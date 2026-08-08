use std::collections::{BTreeMap, BTreeSet};

use tree_sitter::{Node, Parser, Tree};

use crate::{Binding, Delivery, ParamDecl, ParamDefault, ParamType, is_secret_name};

/// One source-detected Python parameter candidate plus onboarding-only signals.
#[derive(Debug, Clone, PartialEq)]
pub struct PythonManagedCandidate {
    pub decl: ParamDecl,
    pub line: usize,
    pub demoted: bool,
    pub demotion: String,
}

/// Python managed-parameter analysis independent of any frontend.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PythonManagedAnalysis {
    pub candidates: Vec<PythonManagedCandidate>,
    pub frameworks: Vec<String>,
    pub syntax_error: bool,
}

/// Detect Python constants, builtin `input()` calls, and CLI frameworks.
///
/// Parsing is lazy at this call boundary. A syntax error returns an empty analysis with
/// `syntax_error=true`; callers must never persist an empty managed set as if every old
/// target genuinely vanished. Module-level literal constants and literal constants in
/// a top-level `if __name__ == "__main__"` guard are injectable. An augmented assignment
/// anywhere, or an assignment inside a loop, demotes that name as an accumulator.
/// `input()` detection is scope-aware: any binding of the name in a scope disables
/// builtin-input candidates only in that scope and descendants.
#[must_use]
pub fn analyze_python_managed(text: &str) -> PythonManagedAnalysis {
    let Some(tree) = parse_python(text) else {
        return syntax_error();
    };
    if tree.root_node().has_error() {
        return syntax_error();
    }

    let root = tree.root_node();
    let mutated = mutated_names(root, text);
    let mut constants = scan_constant_block(root, text, &mutated);
    let module_names = constants
        .iter()
        .map(|candidate| candidate.decl.name.clone())
        .collect::<BTreeSet<_>>();
    if let Some(main_block) = main_guard_block(root, text) {
        constants.extend(
            scan_constant_block(main_block, text, &mutated)
                .into_iter()
                .filter(|candidate| !module_names.contains(&candidate.decl.name)),
        );
    }

    let mut inputs = Vec::new();
    collect_input_scopes(root, text, false, &mut inputs);
    inputs.sort_by_key(|hit| hit.0);
    constants.extend(
        inputs
            .into_iter()
            .enumerate()
            .map(|(order, (_start, prompt, line))| PythonManagedCandidate {
                decl: ParamDecl {
                    name: format!("input-{}", order + 1),
                    binding: Binding::Input,
                    delivery: Delivery::Inject,
                    param_type: ParamType::String,
                    prompt: prompt.clone(),
                    order: i64::try_from(order).unwrap_or(i64::MAX),
                    secret: is_secret_name(&prompt),
                    ..ParamDecl::default()
                },
                line,
                demoted: false,
                demotion: String::new(),
            }),
    );
    constants.sort_by_key(source_sort_key);

    PythonManagedAnalysis {
        candidates: constants,
        frameworks: framework_names(root, text),
        syntax_error: false,
    }
}

fn syntax_error() -> PythonManagedAnalysis {
    PythonManagedAnalysis {
        syntax_error: true,
        ..PythonManagedAnalysis::default()
    }
}

fn parse_python(text: &str) -> Option<Tree> {
    let mut parser = Parser::new();
    let language = tree_sitter_python::LANGUAGE.into();
    parser.set_language(&language).ok()?;
    parser.parse(text, None)
}

fn scan_constant_block(
    block: Node<'_>,
    source: &str,
    mutated: &BTreeSet<String>,
) -> Vec<PythonManagedCandidate> {
    let mut by_name = BTreeMap::<String, PythonManagedCandidate>::new();
    let mut order = Vec::new();
    let mut cursor = block.walk();
    for statement in block.named_children(&mut cursor) {
        let Some(assignment) = assignment_node(statement) else {
            continue;
        };
        let Some(left) = assignment.child_by_field_name("left") else {
            continue;
        };
        if left.kind() != "identifier" {
            continue;
        }
        let Some(name) = node_text(left, source) else {
            continue;
        };
        if name.starts_with('_') {
            continue;
        }
        let Some(right) = assignment.child_by_field_name("right") else {
            continue;
        };
        let Some((param_type, default)) = literal_value(right, source) else {
            continue;
        };
        let candidate = PythonManagedCandidate {
            decl: ParamDecl {
                name: name.to_owned(),
                binding: Binding::Const,
                delivery: Delivery::Inject,
                param_type,
                default: Some(default),
                secret: is_secret_name(name),
                ..ParamDecl::default()
            },
            line: statement.start_position().row + 1,
            demoted: mutated.contains(name),
            demotion: if mutated.contains(name) {
                "accumulator".to_owned()
            } else {
                String::new()
            },
        };
        if let Some(existing) = by_name.get_mut(name) {
            let first_line = existing.line;
            *existing = candidate;
            existing.line = first_line;
        } else {
            order.push(name.to_owned());
            by_name.insert(name.to_owned(), candidate);
        }
    }
    order
        .into_iter()
        .filter_map(|name| by_name.remove(&name))
        .collect()
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

fn literal_value(node: Node<'_>, source: &str) -> Option<(ParamType, ParamDefault)> {
    let text = node_text(node, source)?;
    match node.kind() {
        "true" => Some((ParamType::Boolean, ParamDefault::Boolean(true))),
        "false" => Some((ParamType::Boolean, ParamDefault::Boolean(false))),
        "integer" => parse_python_integer(text)
            .map(|value| (ParamType::Integer, ParamDefault::Integer(value))),
        "float" => {
            parse_python_float(text).map(|value| (ParamType::Float, ParamDefault::Float(value)))
        }
        "unary_operator" => parse_python_integer(text)
            .map(|value| (ParamType::Integer, ParamDefault::Integer(value)))
            .or_else(|| {
                parse_python_float(text).map(|value| (ParamType::Float, ParamDefault::Float(value)))
            }),
        "string" => {
            parse_python_string(text).map(|value| (ParamType::String, ParamDefault::String(value)))
        }
        _ => None,
    }
}

fn parse_python_integer(text: &str) -> Option<i64> {
    let cleaned = text.replace('_', "");
    let (negative, body) = cleaned
        .strip_prefix('-')
        .map_or((false, cleaned.as_str()), |body| (true, body));
    let body = body.strip_prefix('+').unwrap_or(body);
    let (radix, digits) =
        if let Some(digits) = body.strip_prefix("0x").or_else(|| body.strip_prefix("0X")) {
            (16, digits)
        } else if let Some(digits) = body.strip_prefix("0o").or_else(|| body.strip_prefix("0O")) {
            (8, digits)
        } else if let Some(digits) = body.strip_prefix("0b").or_else(|| body.strip_prefix("0B")) {
            (2, digits)
        } else {
            (10, body)
        };
    let magnitude = i64::from_str_radix(digits, radix).ok()?;
    Some(if negative { -magnitude } else { magnitude })
}

fn parse_python_float(text: &str) -> Option<f64> {
    let value = text.replace('_', "").parse::<f64>().ok()?;
    value.is_finite().then_some(value)
}

fn parse_python_string(text: &str) -> Option<String> {
    let quote_at = text.find(['\'', '"'])?;
    let prefix = &text[..quote_at];
    if prefix
        .chars()
        .any(|ch| matches!(ch.to_ascii_lowercase(), 'b' | 'f'))
    {
        return None;
    }
    let raw = prefix.chars().any(|ch| ch.eq_ignore_ascii_case(&'r'));
    let rest = &text[quote_at..];
    let (delimiter, inner) = if rest.starts_with("'''") && rest.ends_with("'''") && rest.len() >= 6
    {
        ("'''", &rest[3..rest.len() - 3])
    } else if rest.starts_with("\"\"\"") && rest.ends_with("\"\"\"") && rest.len() >= 6 {
        ("\"\"\"", &rest[3..rest.len() - 3])
    } else if rest.starts_with('\'') && rest.ends_with('\'') && rest.len() >= 2 {
        ("'", &rest[1..rest.len() - 1])
    } else if rest.starts_with('"') && rest.ends_with('"') && rest.len() >= 2 {
        ("\"", &rest[1..rest.len() - 1])
    } else {
        return None;
    };
    if raw {
        return Some(inner.to_owned());
    }
    decode_python_escapes(inner, delimiter.chars().next()?)
}

fn decode_python_escapes(text: &str, quote: char) -> Option<String> {
    let mut output = String::new();
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            output.push(ch);
            continue;
        }
        let escaped = chars.next()?;
        match escaped {
            '\n' => {}
            '\\' => output.push('\\'),
            '\'' if quote == '\'' => output.push('\''),
            '"' if quote == '"' => output.push('"'),
            'n' => output.push('\n'),
            'r' => output.push('\r'),
            't' => output.push('\t'),
            'a' => output.push('\u{0007}'),
            'b' => output.push('\u{0008}'),
            'f' => output.push('\u{000c}'),
            'v' => output.push('\u{000b}'),
            'x' => output.push(char::from_u32(read_hex(&mut chars, 2)?)?),
            'u' => output.push(char::from_u32(read_hex(&mut chars, 4)?)?),
            'U' => output.push(char::from_u32(read_hex(&mut chars, 8)?)?),
            '0'..='7' => {
                let mut value = escaped.to_digit(8)?;
                for _ in 0..2 {
                    let Some(next) = chars.peek().copied() else {
                        break;
                    };
                    let Some(digit) = next.to_digit(8) else {
                        break;
                    };
                    chars.next();
                    value = value * 8 + digit;
                }
                output.push(char::from_u32(value)?);
            }
            other => {
                output.push('\\');
                output.push(other);
            }
        }
    }
    Some(output)
}

fn read_hex(
    chars: &mut std::iter::Peekable<impl Iterator<Item = char>>,
    count: usize,
) -> Option<u32> {
    let mut value = 0_u32;
    for _ in 0..count {
        value = value.checked_mul(16)? + chars.next()?.to_digit(16)?;
    }
    Some(value)
}

fn mutated_names(root: Node<'_>, source: &str) -> BTreeSet<String> {
    let mut output = BTreeSet::new();
    collect_mutations(root, source, false, &mut output);
    output
}

fn collect_mutations(node: Node<'_>, source: &str, in_loop: bool, output: &mut BTreeSet<String>) {
    let now_in_loop = in_loop || matches!(node.kind(), "for_statement" | "while_statement");
    if (node.kind() == "augmented_assignment" || (now_in_loop && node.kind() == "assignment"))
        && let Some(left) = node.child_by_field_name("left")
    {
        collect_identifiers(left, source, output);
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_mutations(child, source, now_in_loop, output);
    }
}

fn collect_identifiers(node: Node<'_>, source: &str, output: &mut BTreeSet<String>) {
    if node.kind() == "identifier" {
        if let Some(name) = node_text(node, source) {
            output.insert(name.to_owned());
        }
        return;
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_identifiers(child, source, output);
    }
}

fn collect_input_scopes(
    scope: Node<'_>,
    source: &str,
    inherited_shadow: bool,
    output: &mut Vec<(usize, String, usize)>,
) {
    let shadowed = inherited_shadow || scope_binds_input(scope, source);
    collect_scope_calls(scope, source, shadowed, output);

    let mut cursor = scope.walk();
    for child in scope.named_children(&mut cursor) {
        recurse_nested_scopes(child, source, shadowed, output);
    }
}

fn recurse_nested_scopes(
    node: Node<'_>,
    source: &str,
    inherited_shadow: bool,
    output: &mut Vec<(usize, String, usize)>,
) {
    if is_scope_node(node) {
        collect_input_scopes(node, source, inherited_shadow, output);
        return;
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        recurse_nested_scopes(child, source, inherited_shadow, output);
    }
}

fn collect_scope_calls(
    node: Node<'_>,
    source: &str,
    shadowed: bool,
    output: &mut Vec<(usize, String, usize)>,
) {
    if node.kind() == "call"
        && !shadowed
        && let Some(function) = node.child_by_field_name("function")
        && function.kind() == "identifier"
        && node_text(function, source) == Some("input")
    {
        let prompt = input_prompt(node, source).unwrap_or_default();
        output.push((node.start_byte(), prompt, node.start_position().row + 1));
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if is_scope_node(child) {
            continue;
        }
        collect_scope_calls(child, source, shadowed, output);
    }
}

fn input_prompt(call: Node<'_>, source: &str) -> Option<String> {
    let arguments = call.child_by_field_name("arguments")?;
    let mut cursor = arguments.walk();
    let first = arguments.named_children(&mut cursor).next()?;
    (first.kind() == "string")
        .then(|| node_text(first, source))??
        .pipe(parse_python_string)
}

trait Pipe: Sized {
    fn pipe<T>(self, f: impl FnOnce(Self) -> T) -> T {
        f(self)
    }
}
impl<T> Pipe for T {}

fn scope_binds_input(scope: Node<'_>, source: &str) -> bool {
    if matches!(scope.kind(), "function_definition" | "lambda")
        && scope
            .child_by_field_name("parameters")
            .is_some_and(|params| contains_identifier(params, source, "input"))
    {
        return true;
    }
    if is_comprehension_scope(scope) && contains_binding_identifier(scope, source, "input") {
        return true;
    }
    scope_binding_walk(scope, source)
}

fn scope_binding_walk(node: Node<'_>, source: &str) -> bool {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if is_scope_node(child) {
            if matches!(child.kind(), "function_definition" | "class_definition")
                && child
                    .child_by_field_name("name")
                    .and_then(|name| node_text(name, source))
                    == Some("input")
            {
                return true;
            }
            continue;
        }
        if node_binds_name(child, source, "input") || scope_binding_walk(child, source) {
            return true;
        }
    }
    false
}

fn node_binds_name(node: Node<'_>, source: &str, wanted: &str) -> bool {
    match node.kind() {
        "assignment" | "augmented_assignment" => node
            .child_by_field_name("left")
            .is_some_and(|left| contains_identifier(left, source, wanted)),
        "for_statement" => node
            .child_by_field_name("left")
            .is_some_and(|left| contains_identifier(left, source, wanted)),
        "import_statement" | "import_from_statement" => import_binds_name(node, source, wanted),
        _ => false,
    }
}

fn import_binds_name(node: Node<'_>, source: &str, wanted: &str) -> bool {
    let Some(text) = node_text(node, source) else {
        return false;
    };
    if node.kind() == "import_statement" {
        return text.trim_start_matches("import").split(',').any(|part| {
            let mut words = part.split_whitespace();
            let imported = words
                .next()
                .unwrap_or_default()
                .split('.')
                .next()
                .unwrap_or_default();
            let alias = if words.next() == Some("as") {
                words.next()
            } else {
                None
            };
            alias.unwrap_or(imported) == wanted
        });
    }
    let Some((_, imported)) = text.split_once(" import ") else {
        return false;
    };
    imported.trim() == "*"
        || imported.split(',').any(|part| {
            let mut words = part.split_whitespace();
            let name = words.next().unwrap_or_default();
            let alias = if words.next() == Some("as") {
                words.next()
            } else {
                None
            };
            alias.unwrap_or(name) == wanted
        })
}

fn contains_binding_identifier(node: Node<'_>, source: &str, wanted: &str) -> bool {
    if matches!(node.kind(), "for_in_clause")
        && node
            .child_by_field_name("left")
            .is_some_and(|left| contains_identifier(left, source, wanted))
    {
        return true;
    }
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .any(|child| contains_binding_identifier(child, source, wanted))
}

fn contains_identifier(node: Node<'_>, source: &str, wanted: &str) -> bool {
    if node.kind() == "identifier" && node_text(node, source) == Some(wanted) {
        return true;
    }
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .any(|child| contains_identifier(child, source, wanted))
}

fn framework_names(root: Node<'_>, source: &str) -> Vec<String> {
    const FRAMEWORKS: &[&str] = &["argparse", "click", "typer", "docopt", "fire"];
    let mut found = Vec::<(usize, String)>::new();
    collect_frameworks(root, source, &mut found);
    found.sort_by_key(|(start, _)| *start);
    let mut seen = BTreeSet::new();
    found
        .into_iter()
        .filter_map(|(_, name)| {
            (FRAMEWORKS.contains(&name.as_str()) && seen.insert(name.clone())).then_some(name)
        })
        .collect()
}

fn collect_frameworks(node: Node<'_>, source: &str, output: &mut Vec<(usize, String)>) {
    if node.kind() == "import_statement" {
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            let name = if child.kind() == "aliased_import" {
                child.child_by_field_name("name")
            } else if child.kind() == "dotted_name" {
                Some(child)
            } else {
                None
            };
            if let Some(name) = name
                && let Some(text) = node_text(name, source)
                && let Some(top) = text.split('.').next()
            {
                output.push((name.start_byte(), top.to_owned()));
            }
        }
    } else if node.kind() == "import_from_statement"
        && let Some(module) = node.child_by_field_name("module_name")
        && module.kind() == "dotted_name"
        && let Some(text) = node_text(module, source)
        && let Some(top) = text.split('.').next()
    {
        output.push((module.start_byte(), top.to_owned()));
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_frameworks(child, source, output);
    }
}

fn is_scope_node(node: Node<'_>) -> bool {
    matches!(
        node.kind(),
        "function_definition"
            | "class_definition"
            | "lambda"
            | "list_comprehension"
            | "set_comprehension"
            | "dictionary_comprehension"
            | "generator_expression"
    )
}

fn is_comprehension_scope(node: Node<'_>) -> bool {
    matches!(
        node.kind(),
        "list_comprehension"
            | "set_comprehension"
            | "dictionary_comprehension"
            | "generator_expression"
    )
}

fn node_text<'a>(node: Node<'_>, source: &'a str) -> Option<&'a str> {
    source.get(node.start_byte()..node.end_byte())
}

fn source_sort_key(candidate: &PythonManagedCandidate) -> usize {
    candidate.line
}
