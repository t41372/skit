use std::collections::{BTreeMap, BTreeSet};
use std::error::Error as StdError;
use std::fmt;

use tree_sitter::{Node, Parser};

use crate::python_managed::python_input_sites;
use crate::{Binding, ParamDecl, ParamType, analyze_python_managed, match_calls};

/// A managed Python value cannot be safely injected into the current source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PythonInjectError {
    Syntax,
    ManagedInputUnsupported(String),
    AmbiguousInput(String),
    MissingTarget(String),
    InvalidValue { name: String, value: String },
}

impl fmt::Display for PythonInjectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Syntax => formatter.write_str("the Python source does not parse"),
            Self::ManagedInputUnsupported(name) => write!(
                formatter,
                "managed input() injection is not enabled by this const-only API: {name}"
            ),
            Self::AmbiguousInput(name) => write!(
                formatter,
                "managed input() target is ambiguous after source drift: {name}"
            ),
            Self::MissingTarget(name) => {
                write!(formatter, "managed Python target no longer exists: {name}")
            }
            Self::InvalidValue { name, value } => {
                write!(formatter, "{name} cannot accept the value {value:?}")
            }
        }
    }
}

impl StdError for PythonInjectError {}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Replacement {
    start: usize,
    end: usize,
    text: String,
}

/// Rewrite only managed Python constants. Supplying an input-bound value is refused.
///
/// This narrow API remains useful to callers that deliberately promise const-only
/// behavior. Full run preparation uses `inject_python_managed` instead.
///
/// # Errors
///
/// Returns the same source/value errors as full managed injection plus an explicit
/// refusal when a supplied value belongs to `input()`.
pub fn inject_python_consts(
    text: &str,
    specs: &[ParamDecl],
    values: &BTreeMap<String, String>,
) -> Result<String, PythonInjectError> {
    let by_name = specs
        .iter()
        .map(|spec| (spec.name.as_str(), spec))
        .collect::<BTreeMap<_, _>>();
    if let Some(name) = values.keys().find(|name| {
        by_name
            .get(name.as_str())
            .is_some_and(|spec| spec.binding == Binding::Input)
    }) {
        return Err(PythonInjectError::ManagedInputUnsupported(name.clone()));
    }
    inject_python_managed(text, specs, values)
}

/// Rewrite managed Python constants and builtin `input()` values in memory.
///
/// Constant RHS spans use the same source-anchor rules as add-time analysis. Input
/// call sites come from the analyzer's exact scope-aware scanner and are matched with
/// prompt-first, one-to-one call matching. A matched input callee is rewritten to a
/// one-shot wrapper: the first call returns the supplied value and echoes the prompt
/// plus the value (or `***` for secrets); repeated execution of that same source call
/// site falls through to the real builtin `input()`.
///
/// Position-only fallback with a recorded prompt is treated as ambiguous and refused
/// until the shared warning channel lands; silently rebinding a secret is not allowed.
///
/// # Errors
///
/// Returns a named error for syntax failure, vanished/ambiguous source anchors, or an
/// invalid typed constant value.
pub fn inject_python_managed(
    text: &str,
    specs: &[ParamDecl],
    values: &BTreeMap<String, String>,
) -> Result<String, PythonInjectError> {
    if values.is_empty() {
        return Ok(text.to_owned());
    }
    let analysis = analyze_python_managed(text);
    if analysis.syntax_error {
        return Err(PythonInjectError::Syntax);
    }
    let current_consts = analysis
        .candidates
        .iter()
        .filter(|candidate| candidate.decl.binding == Binding::Const)
        .map(|candidate| candidate.decl.name.as_str())
        .collect::<BTreeSet<_>>();
    let input_sites = python_input_sites(text).ok_or(PythonInjectError::Syntax)?;
    let current_inputs = input_sites
        .iter()
        .map(|site| (site.order, site.prompt.clone()))
        .collect::<Vec<_>>();
    let stored_inputs = specs
        .iter()
        .filter(|spec| spec.binding == Binding::Input)
        .map(|spec| (spec.order, spec.prompt.clone()))
        .collect::<Vec<_>>();
    let input_bindings = match_calls(&stored_inputs, &current_inputs);
    let input_by_order = input_sites
        .iter()
        .map(|site| (site.order, site))
        .collect::<BTreeMap<_, _>>();

    let mut parser = Parser::new();
    let language = tree_sitter_python::LANGUAGE.into();
    parser
        .set_language(&language)
        .map_err(|_| PythonInjectError::Syntax)?;
    let tree = parser.parse(text, None).ok_or(PythonInjectError::Syntax)?;
    if tree.root_node().has_error() {
        return Err(PythonInjectError::Syntax);
    }

    let by_name = specs
        .iter()
        .map(|spec| (spec.name.as_str(), spec))
        .collect::<BTreeMap<_, _>>();
    let root = tree.root_node();
    let mut replacements = Vec::new();
    let mut input_queue = BTreeMap::<i64, (String, bool)>::new();
    for (name, raw) in values {
        let Some(spec) = by_name.get(name.as_str()).copied() else {
            return Err(PythonInjectError::MissingTarget(name.clone()));
        };
        match spec.binding {
            Binding::Const => {
                if !current_consts.contains(name.as_str()) {
                    return Err(PythonInjectError::MissingTarget(name.clone()));
                }
                let literal = render_value(spec, raw)?;
                let mut targets = const_targets(root, text, name);
                if let Some(main) = main_guard_block(root, text) {
                    targets.extend(const_targets(main, text, name));
                }
                if targets.is_empty() {
                    return Err(PythonInjectError::MissingTarget(name.clone()));
                }
                replacements.extend(targets.into_iter().map(|node| Replacement {
                    start: node.start_byte(),
                    end: node.end_byte(),
                    text: literal.clone(),
                }));
            }
            Binding::Input => {
                let Some((current_order, ambiguous)) = input_bindings.get(&spec.order).copied()
                else {
                    return Err(PythonInjectError::MissingTarget(name.clone()));
                };
                if ambiguous || input_queue.contains_key(&current_order) {
                    return Err(PythonInjectError::AmbiguousInput(name.clone()));
                }
                let Some(site) = input_by_order.get(&current_order).copied() else {
                    return Err(PythonInjectError::MissingTarget(name.clone()));
                };
                replacements.push(Replacement {
                    start: site.callee_start,
                    end: site.callee_end,
                    text: format!("_skit_i[{current_order}]"),
                });
                input_queue.insert(current_order, (raw.clone(), spec.secret));
            }
            Binding::EnvDefault | Binding::None => {}
        }
    }

    replacements.sort_by_key(|replacement| replacement.start);
    if replacements
        .windows(2)
        .any(|pair| pair[0].end > pair[1].start)
    {
        return Err(PythonInjectError::Syntax);
    }
    let insert_at = (!input_queue.is_empty()).then(|| input_preamble_offset(root, text));
    let mut output = text.to_owned();
    for replacement in replacements.into_iter().rev() {
        output.replace_range(replacement.start..replacement.end, &replacement.text);
    }
    if let Some(insert_at) = insert_at {
        output.insert_str(insert_at, &input_preamble(&input_queue));
    }
    Ok(output)
}

fn input_preamble(queue: &BTreeMap<i64, (String, bool)>) -> String {
    let values = queue
        .iter()
        .map(|(order, (value, secret))| {
            format!(
                "{order}: ({}, {})",
                python_string(value),
                if *secret { "True" } else { "False" }
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let keys = queue
        .keys()
        .map(i64::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "import sys as _skit_s; _skit_o = input; _skit_q = {{{values}}}; _skit_i = {{k: (lambda p='', /, k=k: ((_skit_s.stdout.write(str(p) + ('***' if _skit_q[k][1] else _skit_q[k][0]) + chr(10)), _skit_q.pop(k)[0])[1] if k in _skit_q else _skit_o(p))) for k in [{keys}]}}  # skit:shim\n"
    )
}

fn input_preamble_offset(root: Node<'_>, source: &str) -> usize {
    let mut cursor = root.walk();
    let statements = root.named_children(&mut cursor).collect::<Vec<_>>();
    let mut index = 0;
    if statements.first().is_some_and(|statement| {
        statement.kind() == "expression_statement"
            && statement
                .named_child(0)
                .is_some_and(|child| child.kind() == "string")
    }) {
        index = 1;
    }
    while statements.get(index).is_some_and(|statement| {
        node_text(*statement, source)
            .is_some_and(|text| text.trim_start().starts_with("from __future__ import "))
    }) {
        index += 1;
    }
    statements
        .get(index)
        .map_or(source.len(), |node| node.start_byte())
}

fn render_value(spec: &ParamDecl, raw: &str) -> Result<String, PythonInjectError> {
    let invalid = || PythonInjectError::InvalidValue {
        name: spec.name.clone(),
        value: raw.to_owned(),
    };
    match spec.param_type {
        ParamType::Integer => raw
            .parse::<i64>()
            .map(|value| value.to_string())
            .map_err(|_| invalid()),
        ParamType::Float => raw
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite())
            .map(|value| value.to_string())
            .ok_or_else(invalid),
        ParamType::Boolean => parse_bool(raw)
            .map(|value| if value { "True" } else { "False" }.to_owned())
            .ok_or_else(invalid),
        ParamType::String | ParamType::Choice | ParamType::Path => Ok(python_string(raw)),
    }
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "y" | "on" => Some(true),
        "false" | "0" | "no" | "n" | "off" => Some(false),
        _ => None,
    }
}

fn python_string(value: &str) -> String {
    let mut output = String::with_capacity(value.len() + 2);
    output.push('"');
    for character in value.chars() {
        match character {
            '\\' => output.push_str("\\\\"),
            '"' => output.push_str("\\\""),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            character if character < ' ' || character == '\u{7f}' => {
                use std::fmt::Write as _;
                let _ = write!(output, "\\u{:04X}", u32::from(character));
            }
            character => output.push(character),
        }
    }
    output.push('"');
    output
}

fn const_targets<'a>(block: Node<'a>, source: &str, wanted: &str) -> Vec<Node<'a>> {
    let mut output = Vec::new();
    let mut cursor = block.walk();
    for statement in block.named_children(&mut cursor) {
        let Some(assignment) = assignment_node(statement) else {
            continue;
        };
        let Some(left) = assignment.child_by_field_name("left") else {
            continue;
        };
        if left.kind() != "identifier" || node_text(left, source) != Some(wanted) {
            continue;
        }
        let Some(right) = assignment.child_by_field_name("right") else {
            continue;
        };
        if is_injectable_literal(right, source) {
            output.push(right);
        }
    }
    output
}

fn assignment_node(statement: Node<'_>) -> Option<Node<'_>> {
    if statement.kind() == "assignment" {
        return Some(statement);
    }
    if statement.kind() != "expression_statement" {
        return None;
    }
    let mut cursor = statement.walk();
    statement
        .named_children(&mut cursor)
        .find(|child| child.kind() == "assignment")
}

fn is_injectable_literal(node: Node<'_>, source: &str) -> bool {
    match node.kind() {
        "string" | "integer" | "float" | "true" | "false" => true,
        "unary_operator" => node_text(node, source).is_some_and(|text| {
            let cleaned = text.replace('_', "");
            cleaned.parse::<i64>().is_ok()
                || cleaned.parse::<f64>().is_ok_and(|value| value.is_finite())
        }),
        _ => false,
    }
}

fn main_guard_block<'a>(root: Node<'a>, source: &str) -> Option<Node<'a>> {
    let mut cursor = root.walk();
    root.named_children(&mut cursor)
        .find(|node| node.kind() == "if_statement" && is_main_guard(*node, source))
        .and_then(|node| node.child_by_field_name("consequence"))
}

fn is_main_guard(node: Node<'_>, source: &str) -> bool {
    let Some(condition) = node.child_by_field_name("condition") else {
        return false;
    };
    let Some(text) = node_text(condition, source) else {
        return false;
    };
    let compact = text
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    matches!(
        compact.as_str(),
        "__name__==\"__main__\""
            | "__name__=='__main__'"
            | "\"__main__\"==__name__"
            | "'__main__'==__name__"
    )
}

fn node_text<'a>(node: Node<'_>, source: &'a str) -> Option<&'a str> {
    source.get(node.start_byte()..node.end_byte())
}
