use std::collections::BTreeMap;

use skit_domain::parameters::{
    ParamDecl, ParameterBinding, ParameterType, ParameterValue, is_secret_name,
};

use super::{
    BindingIdentity, CliSurface, DegradationReason, ParsedDocument, SemanticAnalysis,
    SemanticField, SourceSpan, named_children, static_surface, text, walk,
};

pub(super) fn recoverable_error(source: &str, node: tree_sitter::Node<'_>) -> bool {
    let children = named_children(node);
    let Some(name) = children
        .first()
        .filter(|name| children.len() == 1 && name.kind() == "simple_name")
    else {
        return false;
    };
    let error = &source[node.byte_range()];
    error
        .trim()
        .strip_prefix('=')
        .map(str::trim)
        .is_some_and(|recovered| recovered == text_from_source(source, *name))
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
    let parameter_help = comment_help(document);
    let mut fields = Vec::new();
    for (parameter, variable) in parameters.into_iter().filter_map(|parameter| {
        named_children(parameter)
            .into_iter()
            .find(|node| node.kind() == "variable")
            .map(|variable| (parameter, variable))
    }) {
        let name = text(document, variable)
            .trim_start_matches('$')
            .trim_start_matches('{')
            .trim_end_matches('}')
            .to_owned();
        let mut declaration = ParamDecl::new(&name);
        declaration.flag = format!("-{name}");
        declaration.secret = is_secret_name(&name);
        if let Some(help) = parameter_help.get(&name.to_ascii_uppercase()) {
            declaration.help = help.clone();
        }
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
        if let Some(default_node) = named_children(parameter)
            .into_iter()
            .find(|node| node.kind() == "script_parameter_default")
        {
            // A parsed default: classify the expression the way PowerShell's SafeGetValue does.
            // A readable scalar constant carries onto the scalar model; a readable but non-scalar
            // constant (an `@(...)` array, an `@{...}` hashtable of literals) is left unset without
            // a degrade; a genuinely dynamic expression (a variable, a command, a subexpression)
            // degrades the field. Oracle: cli_reader.py `_apply_default` degrades only when
            // `defaultReadable` is false.
            if let Some(value) = static_scalar_default(document, default_node) {
                declaration.default = Some(value);
            } else if !readable_default(document, default_node) {
                declaration.degraded = true;
                degradation = Some(DegradationReason::DynamicDefault);
            }
        } else if let Some(value) = recovered_default(document, variable)
            .and_then(|default| recovered_scalar_default(&default))
        {
            // An error-recovered bare default has no expression node to classify; keep the
            // established text-based scalar read and its degrade-on-failure fallback.
            declaration.default = Some(value);
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
        if let Some(attribute_name) = descendant_text(document, attribute, "type_identifier")
            && attribute_name.eq_ignore_ascii_case("parameter")
        {
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
        } else if descendant_text(document, attribute, "type_identifier")
            .is_some_and(|name| name.eq_ignore_ascii_case("validateset"))
        {
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

fn static_scalar_default(
    document: &ParsedDocument,
    default_node: tree_sitter::Node<'_>,
) -> Option<ParameterValue> {
    let expression = named_children(default_node).into_iter().next()?;
    scalar_literal(document, expression)
}

fn scalar_literal(
    document: &ParsedDocument,
    node: tree_sitter::Node<'_>,
) -> Option<ParameterValue> {
    if let Some(value) = scalar_literal_text(text(document, node)) {
        return Some(value);
    }
    if matches!(node.kind(), "array_expression" | "hash_literal_expression") {
        return None;
    }
    let children = named_children(node);
    let [child] = children.as_slice() else {
        return None;
    };
    scalar_literal(document, *child)
}

fn scalar_literal_text(raw: &str) -> Option<ParameterValue> {
    let raw = raw.trim();
    if let Some(value) = powershell_string(raw) {
        return Some(ParameterValue::String(value));
    }
    match raw.to_ascii_lowercase().as_str() {
        "$true" => return Some(ParameterValue::Bool(true)),
        "$false" => return Some(ParameterValue::Bool(false)),
        _ => {}
    }
    if let Some(value) = powershell_integer(raw) {
        return Some(ParameterValue::Integer(value));
    }
    raw.parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
        .map(ParameterValue::Float)
}

fn powershell_integer(raw: &str) -> Option<i64> {
    let (sign, unsigned) = if let Some(unsigned) = raw.strip_prefix('-') {
        (-1_i64, unsigned)
    } else {
        (1, raw.strip_prefix('+').unwrap_or(raw))
    };
    let value = if let Some(hex) = unsigned
        .strip_prefix("0x")
        .or_else(|| unsigned.strip_prefix("0X"))
    {
        i64::from_str_radix(hex, 16).ok()?
    } else {
        unsigned.parse::<i64>().ok()?
    };
    value.checked_mul(sign)
}

fn recovered_scalar_default(raw: &str) -> Option<ParameterValue> {
    let raw = raw
        .trim()
        .strip_prefix('=')
        .unwrap_or_else(|| raw.trim())
        .trim();
    scalar_literal_text(raw).or_else(|| {
        raw.chars()
            .all(|character| {
                character.is_alphanumeric() || matches!(character, '_' | '-' | '.' | '/' | '\\')
            })
            .then(|| ParameterValue::String(raw.to_owned()))
    })
}

fn recovered_default(document: &ParsedDocument, variable: tree_sitter::Node<'_>) -> Option<String> {
    let tail = document.source.get(variable.end_byte()..)?.trim_start();
    let tail = tail.strip_prefix('=')?.trim_start();
    let end = tail.find([',', ')', '\r', '\n']).unwrap_or(tail.len());
    let value = tail[..end].trim();
    (!value.is_empty()).then(|| value.to_owned())
}

/// Collect per-parameter comment-based help, keyed by the upper-cased parameter name.
///
/// The oracle reads this from `GetHelpContent().Parameters`; the static reader parses it from a
/// `<# ... #>` block comment. Each `.PARAMETER <name>` section runs until the next `.<SECTION>`
/// line or the closing `#>`, and its text is stripped of surrounding whitespace.
fn comment_help(document: &ParsedDocument) -> BTreeMap<String, String> {
    let mut help = BTreeMap::new();
    walk(document.tree.root_node(), &mut |node| {
        if node.kind() != "comment" {
            return;
        }
        if let Some(body) = text(document, node)
            .strip_prefix("<#")
            .and_then(|inner| inner.strip_suffix("#>"))
        {
            collect_parameter_help(body, &mut help);
        }
    });
    help
}

/// Parse the `.PARAMETER` sections out of one block-comment body into `help`.
fn collect_parameter_help(body: &str, help: &mut BTreeMap<String, String>) {
    let mut current: Option<String> = None;
    let mut lines: Vec<&str> = Vec::new();
    for line in body.lines() {
        if let Some(header) = section_keyword(line) {
            flush_parameter_help(&mut current, &mut lines, help);
            let mut parts = header.split_whitespace();
            let keyword = parts.next().unwrap_or_default();
            if keyword.eq_ignore_ascii_case("parameter")
                && let Some(parameter) = parts.next()
            {
                current = Some(parameter.to_ascii_uppercase());
            }
        } else if current.is_some() {
            lines.push(line);
        }
    }
    flush_parameter_help(&mut current, &mut lines, help);
}

/// A comment-based-help section line (`.<KEYWORD> ...`) with the leading dot removed, or `None`.
fn section_keyword(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    let rest = trimmed.strip_prefix('.')?;
    rest.starts_with(|character: char| character.is_ascii_alphabetic())
        .then_some(rest)
}

/// Store the accumulated lines under the current parameter, stripped of surrounding whitespace.
fn flush_parameter_help(
    current: &mut Option<String>,
    lines: &mut Vec<&str>,
    help: &mut BTreeMap<String, String>,
) {
    if let Some(key) = current.take() {
        help.entry(key)
            .or_insert_with(|| lines.join("\n").trim().to_owned());
    }
    lines.clear();
}

/// Whether a parsed parameter default is a `SafeGetValue`-readable constant.
///
/// A readable constant is a literal, or an `@(...)` array / `@{...}` hashtable whose every element
/// or value is itself a readable constant. `$true`, `$false`, and `$null` are the only variables
/// `SafeGetValue` reads. Anything else — another variable, a command, a subexpression — needs
/// evaluation, so it is dynamic and the field degrades.
fn readable_default(document: &ParsedDocument, default_node: tree_sitter::Node<'_>) -> bool {
    named_children(default_node)
        .into_iter()
        .next()
        .is_some_and(|expression| readable_constant(document, expression))
}

fn readable_constant(document: &ParsedDocument, node: tree_sitter::Node<'_>) -> bool {
    match node.kind() {
        "integer_literal"
        | "decimal_integer_literal"
        | "hexadecimal_integer_literal"
        | "real_literal"
        | "string_literal"
        | "verbatim_string_characters"
        | "expandable_string_literal" => true,
        // `$true`, `$false`, and `$null` are the only variables SafeGetValue evaluates.
        "variable" => matches!(
            text(document, node).to_ascii_lowercase().as_str(),
            "$true" | "$false" | "$null"
        ),
        // A hashtable is readable when every entry value is; keys are names, never evaluated.
        "hash_literal_expression" => {
            let mut readable = true;
            walk(node, &mut |inner| {
                if inner.kind() == "hash_entry" {
                    for child in named_children(inner) {
                        if child.kind() != "key_expression" && !readable_constant(document, child) {
                            readable = false;
                        }
                    }
                }
            });
            readable
        }
        // A parenthesized expression, an `@(...)` array, and the operator/pipeline wrappers are
        // readable when every named child is. A command, a subexpression, or any other node is
        // not listed here, so it falls through to the dynamic arm.
        "array_expression"
        | "parenthesized_expression"
        | "statement_list"
        | "pipeline"
        | "pipeline_chain"
        | "logical_expression"
        | "bitwise_expression"
        | "comparison_expression"
        | "additive_expression"
        | "multiplicative_expression"
        | "format_expression"
        | "range_expression"
        | "array_literal_expression"
        | "unary_expression" => named_children(node)
            .into_iter()
            .all(|child| readable_constant(document, child)),
        _ => false,
    }
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
