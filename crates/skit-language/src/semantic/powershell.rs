use skit_domain::parameters::{
    ParamDecl, ParameterBinding, ParameterType, ParameterValue, is_secret_name,
};

use super::{
    BindingIdentity, CliSurface, DegradationReason, ParsedDocument, SemanticAnalysis,
    SemanticField, SourceSpan, named_children, static_surface, text, walk,
};

pub(super) fn recoverable_error(source: &str, node: tree_sitter::Node<'_>) -> bool {
    let children = named_children(node);
    let [name] = children.as_slice() else {
        return false;
    };
    if name.kind() != "simple_name" {
        return false;
    }
    let Some(error) = source.get(node.start_byte()..node.end_byte()) else {
        return false;
    };
    let recovered = if let Some(value) = error.trim().strip_prefix('=') {
        value.trim()
    } else if source
        .get(..node.start_byte())
        .is_some_and(|prefix| prefix.trim_end().ends_with('='))
    {
        error.trim()
    } else {
        return false;
    };
    recovered == text_from_source(source, *name)
}

fn text_from_source<'source>(source: &'source str, node: tree_sitter::Node<'_>) -> &'source str {
    source
        .get(node.start_byte()..node.end_byte())
        .unwrap_or_default()
}

pub(super) fn analysis(document: &ParsedDocument) -> SemanticAnalysis {
    let surface = cli_surface(document);
    SemanticAnalysis {
        frameworks: (!matches!(surface, CliSurface::Absent))
            .then(|| "param".to_owned())
            .into_iter()
            .collect(),
        ..SemanticAnalysis::default()
    }
}

pub(super) fn cli_surface(document: &ParsedDocument) -> CliSurface {
    let Some(block) = first_node(document, "param_block") else {
        return CliSurface::Absent;
    };
    let mut parameters = Vec::new();
    walk(block, &mut |node| {
        if node.kind() == "script_parameter" {
            parameters.push(node);
        }
    });
    parameters.sort_by_key(tree_sitter::Node::start_byte);
    let mut fields = Vec::new();
    for parameter in parameters {
        let Some(variable) = named_children(parameter)
            .into_iter()
            .find(|node| node.kind() == "variable")
        else {
            continue;
        };
        let name = text(document, variable)
            .trim_start_matches('$')
            .trim_start_matches('{')
            .trim_end_matches('}')
            .to_owned();
        if name.is_empty() {
            continue;
        }
        let mut declaration = ParamDecl::new(&name);
        declaration.flag = format!("-{name}");
        declaration.secret = is_secret_name(&name);
        let mut degradation = None;
        let attributes = named_children(parameter)
            .into_iter()
            .find(|node| node.kind() == "attribute_list");
        if let Some(attributes) = attributes {
            apply_attributes(document, attributes, &mut declaration, &mut degradation);
        } else {
            declaration.degraded = true;
            degradation = Some(DegradationReason::DynamicType);
        }
        let default = named_children(parameter)
            .into_iter()
            .find(|node| node.kind() == "script_parameter_default")
            .map(|default| text(document, default).to_owned())
            .or_else(|| recovered_default(document, variable));
        if let Some(default) = default {
            if let Some(value) = static_default(&default, declaration.parameter_type) {
                declaration.default = Some(value);
            } else {
                declaration.degraded = true;
                degradation = Some(DegradationReason::DynamicDefault);
            }
        }
        fields.push(SemanticField {
            identity: BindingIdentity {
                binding: ParameterBinding::None,
                key: name,
                occurrence: 0,
                scope: Vec::new(),
            },
            span: SourceSpan::from_node(parameter),
            declaration,
            degradation,
        });
    }
    static_surface("param", fields)
}

fn apply_attributes(
    document: &ParsedDocument,
    attributes: tree_sitter::Node<'_>,
    declaration: &mut ParamDecl,
    degradation: &mut Option<DegradationReason>,
) {
    let mut static_type = None;
    let mut choices = Vec::new();
    for attribute in named_children(attributes)
        .into_iter()
        .filter(|node| node.kind() == "attribute")
    {
        if let Some(type_literal) = named_children(attribute)
            .into_iter()
            .find(|node| node.kind() == "type_literal")
        {
            static_type = descendant_text(document, type_literal, "type_identifier");
            continue;
        }
        let Some(attribute_name) = descendant_text(document, attribute, "type_identifier") else {
            continue;
        };
        if attribute_name.eq_ignore_ascii_case("parameter") {
            walk(attribute, &mut |argument| {
                if argument.kind() != "attribute_argument" {
                    return;
                }
                let source = text(document, argument).trim();
                let (name, value) = source
                    .split_once('=')
                    .map_or((source, None), |(name, value)| {
                        (name.trim(), Some(value.trim()))
                    });
                if name.eq_ignore_ascii_case("mandatory")
                    && value.is_none_or(|value| value.eq_ignore_ascii_case("$true"))
                {
                    declaration.required = true;
                }
            });
        } else if attribute_name.eq_ignore_ascii_case("validateset") {
            walk(attribute, &mut |node| {
                if node.kind() == "string_literal"
                    && let Some(value) = powershell_string(text(document, node))
                {
                    choices.push(value);
                }
            });
        }
    }
    if !choices.is_empty() {
        declaration.parameter_type = ParameterType::Choice;
        declaration.choices = choices;
        return;
    }
    match static_type
        .map(|value| value.to_ascii_lowercase())
        .as_deref()
    {
        Some("string") => declaration.parameter_type = ParameterType::Str,
        Some("int" | "int32" | "int64" | "long") => {
            declaration.parameter_type = ParameterType::Int;
        }
        Some("double" | "single" | "float") => {
            declaration.parameter_type = ParameterType::Float;
        }
        Some("bool" | "boolean") => {
            declaration.parameter_type = ParameterType::Bool;
        }
        Some("switch" | "switchparameter") => {
            declaration.parameter_type = ParameterType::Bool;
            declaration.action = "store_true".to_owned();
            declaration.default = Some(ParameterValue::Bool(false));
        }
        _ => {
            declaration.degraded = true;
            *degradation = Some(DegradationReason::DynamicType);
        }
    }
}

fn static_default(raw: &str, parameter_type: ParameterType) -> Option<ParameterValue> {
    let raw = raw
        .trim()
        .strip_prefix('=')
        .unwrap_or_else(|| raw.trim())
        .trim();
    match parameter_type {
        ParameterType::Int => raw.parse::<i64>().ok().map(ParameterValue::Integer),
        ParameterType::Float => raw
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite())
            .map(ParameterValue::Float),
        ParameterType::Bool => match raw.to_ascii_lowercase().as_str() {
            "$true" => Some(ParameterValue::Bool(true)),
            "$false" => Some(ParameterValue::Bool(false)),
            _ => None,
        },
        ParameterType::Str | ParameterType::Choice | ParameterType::Path => powershell_string(raw)
            .or_else(|| {
                raw.chars()
                    .all(|character| {
                        character.is_alphanumeric()
                            || matches!(character, '_' | '-' | '.' | '/' | '\\')
                    })
                    .then(|| raw.to_owned())
            })
            .map(ParameterValue::String),
    }
}

fn recovered_default(document: &ParsedDocument, variable: tree_sitter::Node<'_>) -> Option<String> {
    let tail = document.source.get(variable.end_byte()..)?.trim_start();
    let tail = tail.strip_prefix('=')?.trim_start();
    let end = tail.find([',', ')', '\r', '\n']).unwrap_or(tail.len());
    let value = tail[..end].trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn powershell_string(raw: &str) -> Option<String> {
    if raw.len() < 2 {
        return None;
    }
    if raw.starts_with('\'') && raw.ends_with('\'') {
        return Some(raw[1..raw.len() - 1].replace("''", "'"));
    }
    if raw.starts_with('"') && raw.ends_with('"') {
        let body = &raw[1..raw.len() - 1];
        if body.contains('$') {
            return None;
        }
        return Some(body.replace("`\"", "\"").replace("``", "`"));
    }
    None
}

fn descendant_text<'a>(
    document: &'a ParsedDocument,
    root: tree_sitter::Node<'_>,
    expected_kind: &str,
) -> Option<&'a str> {
    let mut found = None;
    walk(root, &mut |node| {
        if found.is_none() && node.kind() == expected_kind {
            found = Some(text(document, node));
        }
    });
    found
}

fn first_node<'tree>(
    document: &'tree ParsedDocument,
    expected_kind: &str,
) -> Option<tree_sitter::Node<'tree>> {
    let mut found = None;
    walk(document.tree.root_node(), &mut |node| {
        if found.is_none() && node.kind() == expected_kind {
            found = Some(node);
        }
    });
    found
}
