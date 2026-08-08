use std::collections::{BTreeMap, BTreeSet};
use std::error::Error as StdError;
use std::fmt;

use tree_sitter::{Node, Parser};

use crate::{Binding, ParamDecl, ParamType, analyze_python_managed};

/// A managed Python value cannot be safely injected into the current source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PythonInjectError {
    Syntax,
    ManagedInputUnsupported(String),
    MissingTarget(String),
    InvalidValue { name: String, value: String },
}

impl fmt::Display for PythonInjectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Syntax => formatter.write_str("the Python source does not parse"),
            Self::ManagedInputUnsupported(name) => write!(
                formatter,
                "managed input() injection is not enabled in the Rust rewrite yet: {name}"
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

/// Rewrite managed Python constant values in memory without touching the stored source.
///
/// Only module-level literal assignments and literal assignments directly inside a
/// top-level `if __name__ == "__main__"` guard are eligible, matching add-time managed
/// detection. Every same-named eligible assignment is replaced. A supplied managed
/// `input()` value is explicitly refused until the call-site one-shot wrapper lands.
///
/// # Errors
///
/// Returns a named error for syntax failure, a vanished target, an invalid typed value,
/// or a managed `input()` value that this vertical slice cannot safely deliver yet.
pub fn inject_python_consts(
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
    for (name, raw) in values {
        let Some(spec) = by_name.get(name.as_str()).copied() else {
            return Err(PythonInjectError::MissingTarget(name.clone()));
        };
        match spec.binding {
            Binding::Input => {
                return Err(PythonInjectError::ManagedInputUnsupported(name.clone()));
            }
            Binding::Const => {}
            Binding::EnvDefault | Binding::None => continue,
        }
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

    replacements.sort_by_key(|replacement| replacement.start);
    if replacements
        .windows(2)
        .any(|pair| pair[0].end > pair[1].start)
    {
        return Err(PythonInjectError::Syntax);
    }
    let mut output = text.to_owned();
    for replacement in replacements.into_iter().rev() {
        output.replace_range(replacement.start..replacement.end, &replacement.text);
    }
    Ok(output)
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
        if is_injectable_literal(right) {
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

fn is_injectable_literal(node: Node<'_>) -> bool {
    matches!(
        node.kind(),
        "string" | "integer" | "float" | "true" | "false" | "unary_operator"
    )
}

fn main_guard_block<'a>(root: Node<'a>, source: &str) -> Option<Node<'a>> {
    let mut cursor = root.walk();
    root.named_children(&mut cursor)
        .find(|node| node.kind() == "if_statement" && is_main_guard(*node, source))
        .and_then(|node| node.child_by_field_name("consequence"))
}

fn is_main_guard(node: Node<'_>, source: &str) -> bool {
    let condition = node.child_by_field_name("condition")?;
    let text = node_text(condition, source)?;
    let compact = text
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    Some(matches!(
        compact.as_str(),
        "__name__==\"__main__\""
            | "__name__=='__main__'"
            | "\"__main__\"==__name__"
            | "'__main__'==__name__"
    ))
    .unwrap_or(false)
}

fn node_text<'a>(node: Node<'_>, source: &'a str) -> Option<&'a str> {
    source.get(node.start_byte()..node.end_byte())
}
