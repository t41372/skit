use std::collections::{BTreeMap, BTreeSet};

use skit_domain::parameters::{
    ParamDecl, ParameterBinding, ParameterType, ParameterValue, is_secret_name,
};

use super::{
    BindingIdentity, CliSurface, DegradationReason, ParsedDocument, SemanticAnalysis,
    SemanticCandidate, SemanticField, SourceEdit, SourceEditPlan, SourceSpan, canonical_bool,
    dynamic_surface, named_children, render_float, static_surface, text, walk,
};
use crate::LanguageError;

pub(super) fn analysis(document: &ParsedDocument) -> SemanticAnalysis {
    let mutated = mutated_names(document);
    let mut candidates = top_level_candidates(document);
    for candidate in &mut candidates {
        if mutated.contains(&candidate.declaration.name) {
            candidate.demotion = Some(DegradationReason::Accumulator);
        }
    }
    let surface = cli_surface(document);
    SemanticAnalysis {
        frameworks: (!matches!(surface, CliSurface::Absent))
            .then(|| "parseArgs".to_owned())
            .into_iter()
            .collect(),
        uses_argv: ["process.argv", "Deno.args", "Bun.argv"]
            .iter()
            .any(|marker| document.source.contains(marker)),
        candidates,
        ..SemanticAnalysis::default()
    }
}

pub(super) fn cli_surface(document: &ParsedDocument) -> CliSurface {
    let Some(call) = first_parse_args_call(document) else {
        return CliSurface::Absent;
    };
    let Some(arguments) = call.child_by_field_name("arguments") else {
        return CliSurface::Absent;
    };
    let Some(config) = named_children(arguments).into_iter().next() else {
        return CliSurface::Absent;
    };
    if config.kind() != "object" {
        return CliSurface::Absent;
    }
    let Some(options) = object_property(document, config, "options") else {
        return CliSurface::Absent;
    };
    if options.kind() != "object"
        || named_children(options)
            .iter()
            .any(|child| child.kind() == "spread_element")
    {
        return dynamic_surface("parseArgs", DegradationReason::DynamicDeclaration);
    }

    let constants = constant_environment(document);
    let mut fields = Vec::new();
    for pair in named_children(options) {
        if pair.kind() != "pair" {
            continue;
        }
        let Some(key) = pair.child_by_field_name("key") else {
            continue;
        };
        if key.kind() == "computed_property_name" {
            continue;
        }
        let name = property_name(document, key);
        if name.is_empty() {
            continue;
        }
        let mut declaration = ParamDecl::new(&name);
        declaration.flag = format!("--{name}");
        declaration.secret = is_secret_name(&name);
        let mut degradation = None;
        let Some(spec) = pair.child_by_field_name("value") else {
            continue;
        };
        if spec.kind() != "object" {
            declaration.degraded = true;
            degradation = Some(DegradationReason::DynamicDeclaration);
        } else {
            apply_option_spec(
                document,
                spec,
                &constants,
                &mut declaration,
                &mut degradation,
            );
        }
        let occurrence = fields
            .iter()
            .filter(|field: &&SemanticField| field.declaration.name == name)
            .count();
        fields.push(SemanticField {
            identity: BindingIdentity {
                binding: ParameterBinding::None,
                key: name,
                occurrence,
                scope: Vec::new(),
            },
            span: SourceSpan::from_node(pair),
            declaration,
            degradation,
        });
    }
    static_surface("parseArgs", fields)
}

fn top_level_candidates(document: &ParsedDocument) -> Vec<SemanticCandidate> {
    let mut output = Vec::<SemanticCandidate>::new();
    for statement in named_children(document.tree.root_node()) {
        let keyword = match statement.kind() {
            "lexical_declaration" => statement
                .child_by_field_name("kind")
                .map_or("", |node| text(document, node)),
            "variable_declaration" => "var",
            _ => continue,
        };
        for declarator in named_children(statement)
            .into_iter()
            .filter(|node| node.kind() == "variable_declarator")
        {
            let Some(name_node) = declarator.child_by_field_name("name") else {
                continue;
            };
            if name_node.kind() != "identifier" {
                continue;
            }
            let name = text(document, name_node);
            if name.starts_with('_') {
                continue;
            }
            let Some(value) = declarator.child_by_field_name("value") else {
                continue;
            };
            let Some((parameter_type, default)) = literal_shape(document, value) else {
                continue;
            };
            let mut declaration = ParamDecl::new(name);
            declaration.binding = ParameterBinding::Const;
            declaration.delivery = skit_domain::parameters::ParameterDelivery::Inject;
            declaration.parameter_type = parameter_type;
            declaration.default = Some(default);
            declaration.secret = is_secret_name(name);
            let candidate = SemanticCandidate {
                declaration,
                identity: BindingIdentity {
                    binding: ParameterBinding::Const,
                    key: name.to_owned(),
                    occurrence: 0,
                    scope: Vec::new(),
                },
                span: SourceSpan::from_node(declarator),
                demotion: matches!(keyword, "let" | "var")
                    .then_some(DegradationReason::Accumulator),
                empty_uses_default: false,
            };
            if let Some(existing) = output
                .iter_mut()
                .find(|current| current.declaration.name == name)
            {
                *existing = candidate;
            } else {
                output.push(candidate);
            }
        }
    }
    output
}

pub(super) fn plan_injection(
    document: &ParsedDocument,
    declarations: &[ParamDecl],
    values: &BTreeMap<String, String>,
) -> Result<SourceEditPlan, LanguageError> {
    let selected = declarations
        .iter()
        .filter(|declaration| {
            declaration.delivery == skit_domain::parameters::ParameterDelivery::Inject
                && values.contains_key(&declaration.name)
        })
        .collect::<Vec<_>>();
    let mut edits = Vec::new();
    for declaration in selected {
        let targets = constant_targets(document, &declaration.name);
        if targets.is_empty() {
            return Err(LanguageError::BindingNotFound {
                name: declaration.name.clone(),
            });
        }
        let raw = values
            .get(&declaration.name)
            .expect("selected declarations have accepted values");
        let replacement = typed_literal(declaration, raw)?;
        edits.extend(targets.into_iter().map(|target| SourceEdit {
            span: SourceSpan::from_node(target),
            replacement: replacement.clone(),
        }));
    }
    edits.sort_by_key(|edit| (edit.span.start, edit.span.end));
    Ok(SourceEditPlan {
        source: document.source.clone(),
        edits,
    })
}

fn constant_targets<'tree>(
    document: &'tree ParsedDocument,
    expected: &str,
) -> Vec<tree_sitter::Node<'tree>> {
    top_level_declarations(document)
        .into_iter()
        .filter_map(|(declarator, _)| {
            let name = declarator.child_by_field_name("name")?;
            let value = declarator.child_by_field_name("value")?;
            (name.kind() == "identifier"
                && text(document, name) == expected
                && literal_value(document, value).is_some())
            .then_some(value)
        })
        .collect()
}

fn typed_literal(declaration: &ParamDecl, raw: &str) -> Result<String, LanguageError> {
    let invalid = || LanguageError::InvalidValue {
        name: declaration.name.clone(),
        value: raw.to_owned(),
        parameter_type: declaration.parameter_type,
    };
    match declaration.parameter_type {
        ParameterType::Int => raw
            .trim()
            .parse::<i64>()
            .map(|value| value.to_string())
            .map_err(|_| invalid()),
        ParameterType::Float => raw
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite())
            .map(render_float)
            .ok_or_else(invalid),
        ParameterType::Bool => canonical_bool(raw)
            .map(|value| if value { "true" } else { "false" }.to_owned())
            .ok_or_else(invalid),
        ParameterType::Str | ParameterType::Choice | ParameterType::Path => {
            serde_json::to_string(raw).map_err(|_| invalid())
        }
    }
}

fn top_level_declarations(document: &ParsedDocument) -> Vec<(tree_sitter::Node<'_>, &'static str)> {
    let mut output = Vec::new();
    for statement in named_children(document.tree.root_node()) {
        let keyword = match statement.kind() {
            "lexical_declaration" => match statement
                .child_by_field_name("kind")
                .map_or("", |node| text(document, node))
            {
                "const" => "const",
                "let" => "let",
                _ => continue,
            },
            "variable_declaration" => "var",
            _ => continue,
        };
        output.extend(
            named_children(statement)
                .into_iter()
                .filter(|node| node.kind() == "variable_declarator")
                .map(|node| (node, keyword)),
        );
    }
    output
}

fn literal_value(document: &ParsedDocument, node: tree_sitter::Node<'_>) -> Option<ParameterValue> {
    literal_shape(document, node).map(|(_, value)| value)
}

fn literal_shape(
    document: &ParsedDocument,
    node: tree_sitter::Node<'_>,
) -> Option<(ParameterType, ParameterValue)> {
    match node.kind() {
        "number" => {
            let raw = text(document, node);
            if raw.chars().all(|character| character.is_ascii_digit()) {
                raw.parse::<i64>()
                    .ok()
                    .map(ParameterValue::Integer)
                    .map(|value| (ParameterType::Int, value))
                    .or_else(|| {
                        Some((ParameterType::Float, ParameterValue::String(raw.to_owned())))
                    })
            } else if raw.split_once('.').is_some_and(|(left, right)| {
                !left.is_empty()
                    && !right.is_empty()
                    && left.chars().all(|character| character.is_ascii_digit())
                    && right.chars().all(|character| character.is_ascii_digit())
            }) {
                raw.parse::<f64>()
                    .ok()
                    .filter(|value| value.is_finite())
                    .map(ParameterValue::Float)
                    .map(|value| (ParameterType::Float, value))
            } else {
                Some((ParameterType::Float, ParameterValue::String(raw.to_owned())))
            }
        }
        "string" => Some((
            ParameterType::Str,
            ParameterValue::String(
                named_children(node)
                    .into_iter()
                    .map(|child| text(document, child))
                    .collect(),
            ),
        )),
        "true" => Some((ParameterType::Bool, ParameterValue::Bool(true))),
        "false" => Some((ParameterType::Bool, ParameterValue::Bool(false))),
        _ => None,
    }
}

fn mutated_names(document: &ParsedDocument) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    walk(document.tree.root_node(), &mut |node| match node.kind() {
        "assignment_expression" | "augmented_assignment_expression" => {
            if let Some(left) = node.child_by_field_name("left")
                && left.kind() == "identifier"
            {
                names.insert(text(document, left).to_owned());
            }
        }
        "update_expression" => {
            if let Some(argument) = node.child_by_field_name("argument")
                && argument.kind() == "identifier"
            {
                names.insert(text(document, argument).to_owned());
            }
        }
        _ => {}
    });
    names
}

fn first_parse_args_call<'tree>(
    document: &'tree ParsedDocument,
) -> Option<tree_sitter::Node<'tree>> {
    let mut found = None;
    walk(document.tree.root_node(), &mut |node| {
        if found.is_some() || node.kind() != "call_expression" {
            return;
        }
        let Some(function) = node.child_by_field_name("function") else {
            return;
        };
        let matches = (function.kind() == "identifier" && text(document, function) == "parseArgs")
            || (function.kind() == "member_expression"
                && function
                    .child_by_field_name("property")
                    .is_some_and(|property| text(document, property) == "parseArgs"));
        if matches {
            found = Some(node);
        }
    });
    found
}

fn object_property<'tree>(
    document: &ParsedDocument,
    object: tree_sitter::Node<'tree>,
    expected: &str,
) -> Option<tree_sitter::Node<'tree>> {
    named_children(object)
        .into_iter()
        .filter(|child| child.kind() == "pair")
        .find_map(|pair| {
            let key = pair.child_by_field_name("key")?;
            (property_name(document, key) == expected)
                .then(|| pair.child_by_field_name("value"))
                .flatten()
        })
}

fn property_name(document: &ParsedDocument, node: tree_sitter::Node<'_>) -> String {
    match node.kind() {
        "property_identifier" => text(document, node).to_owned(),
        "string" => named_children(node)
            .into_iter()
            .map(|child| text(document, child))
            .collect(),
        _ => String::new(),
    }
}

fn constant_environment(document: &ParsedDocument) -> BTreeMap<String, ParameterValue> {
    let candidates = top_level_candidates(document);
    let mutated = mutated_names(document);
    let mut declaration_counts = BTreeMap::<String, usize>::new();
    walk(document.tree.root_node(), &mut |node| match node.kind() {
        "variable_declarator" => {
            if let Some(name) = node.child_by_field_name("name") {
                count_bound_names(document, name, &mut declaration_counts);
            }
        }
        "formal_parameters" => {
            for parameter in named_children(node) {
                count_bound_names(document, parameter, &mut declaration_counts);
            }
        }
        "function_declaration"
        | "function_expression"
        | "generator_function_declaration"
        | "generator_function"
        | "class_declaration"
        | "class" => {
            if let Some(name) = node.child_by_field_name("name") {
                count_one_name(document, name, &mut declaration_counts);
            }
        }
        "catch_clause" => {
            if let Some(parameter) = node.child_by_field_name("parameter") {
                count_bound_names(document, parameter, &mut declaration_counts);
            }
        }
        "import_statement" => count_imports(document, node, &mut declaration_counts),
        _ => {}
    });
    candidates
        .into_iter()
        .filter(|candidate| {
            candidate.demotion.is_none()
                && !candidate.declaration.secret
                && !mutated.contains(&candidate.declaration.name)
                && declaration_counts.get(&candidate.declaration.name) == Some(&1)
        })
        .filter_map(|candidate| Some((candidate.declaration.name, candidate.declaration.default?)))
        .collect()
}

fn count_one_name(
    document: &ParsedDocument,
    node: tree_sitter::Node<'_>,
    counts: &mut BTreeMap<String, usize>,
) {
    if matches!(
        node.kind(),
        "identifier" | "shorthand_property_identifier_pattern"
    ) {
        *counts.entry(text(document, node).to_owned()).or_default() += 1;
    }
}

fn count_bound_names(
    document: &ParsedDocument,
    node: tree_sitter::Node<'_>,
    counts: &mut BTreeMap<String, usize>,
) {
    match node.kind() {
        "identifier" | "shorthand_property_identifier_pattern" => {
            count_one_name(document, node, counts);
        }
        "required_parameter" | "optional_parameter" => {
            if let Some(pattern) = node.child_by_field_name("pattern") {
                count_bound_names(document, pattern, counts);
            }
        }
        "assignment_pattern" => {
            if let Some(left) = node.child_by_field_name("left") {
                count_bound_names(document, left, counts);
            }
        }
        "pair_pattern" => {
            if let Some(value) = node.child_by_field_name("value") {
                count_bound_names(document, value, counts);
            }
        }
        _ => {
            for child in named_children(node) {
                count_bound_names(document, child, counts);
            }
        }
    }
}

fn count_imports(
    document: &ParsedDocument,
    statement: tree_sitter::Node<'_>,
    counts: &mut BTreeMap<String, usize>,
) {
    let Some(clause) = named_children(statement)
        .into_iter()
        .find(|node| node.kind() == "import_clause")
    else {
        return;
    };
    for child in named_children(clause) {
        match child.kind() {
            "identifier" => count_one_name(document, child, counts),
            "namespace_import" => {
                if let Some(name) = named_children(child)
                    .into_iter()
                    .rev()
                    .find(|node| node.kind() == "identifier")
                {
                    count_one_name(document, name, counts);
                }
            }
            "named_imports" => {
                for specifier in named_children(child) {
                    let name = specifier
                        .child_by_field_name("alias")
                        .or_else(|| specifier.child_by_field_name("name"));
                    if let Some(name) = name {
                        count_one_name(document, name, counts);
                    }
                }
            }
            _ => {}
        }
    }
}

fn apply_option_spec(
    document: &ParsedDocument,
    spec: tree_sitter::Node<'_>,
    constants: &BTreeMap<String, ParameterValue>,
    declaration: &mut ParamDecl,
    degradation: &mut Option<DegradationReason>,
) {
    let mut properties = BTreeMap::new();
    for pair in named_children(spec)
        .into_iter()
        .filter(|node| node.kind() == "pair")
    {
        let Some(key) = pair.child_by_field_name("key") else {
            continue;
        };
        let Some(value) = pair.child_by_field_name("value") else {
            continue;
        };
        let name = property_name(document, key);
        if !name.is_empty() {
            properties.insert(name, value);
        }
    }
    if let Some(value) = properties.get("type") {
        match literal_value(document, *value) {
            Some(ParameterValue::String(value)) if value == "boolean" => {
                declaration.parameter_type = ParameterType::Bool;
                declaration.action = "store_true".to_owned();
                declaration.default = Some(ParameterValue::Bool(false));
            }
            Some(ParameterValue::String(value)) if value == "string" => {}
            _ => {
                declaration.degraded = true;
                *degradation = Some(DegradationReason::DynamicType);
            }
        }
    }
    if let Some(value) = properties.get("default") {
        let default = literal_value(document, *value).or_else(|| {
            (value.kind() == "identifier")
                .then(|| constants.get(text(document, *value)).cloned())
                .flatten()
        });
        if let Some(default) = default {
            declaration.default = Some(default);
        } else {
            declaration.degraded = true;
            *degradation = Some(DegradationReason::DynamicDefault);
        }
    }
    if properties
        .get("multiple")
        .is_some_and(|value| value.kind() == "true")
    {
        declaration.multiple = true;
        declaration.repeat = true;
    }
}
