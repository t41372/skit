use std::collections::BTreeSet;

use skit_domain::parameters::{
    ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, ParameterValue, is_secret_name,
};

use super::{
    BindingIdentity, CliSurface, DegradationReason, ParsedDocument, SemanticAnalysis,
    SemanticCandidate, SemanticField, SourceSpan, dynamic_surface, named_children, static_surface,
    text, walk,
};

const VALUE_OPTIONS: [&str; 8] = [
    "-n",
    "--name",
    "-x",
    "--exclusive",
    "-N",
    "--min-args",
    "-X",
    "--max-args",
];

struct SetCommand {
    conditional: bool,
    query: bool,
    name: String,
    values: Vec<String>,
}

pub(super) fn analysis(document: &ParsedDocument) -> SemanticAnalysis {
    let statements = root_statements(document);
    let clobbered = statements
        .iter()
        .filter_map(|node| classify_set(document, *node))
        .filter(|set| !set.conditional && !set.query)
        .map(|set| set.name)
        .collect::<BTreeSet<_>>();
    let mut candidates = Vec::new();
    let mut seen = BTreeSet::new();
    for pair in statements.windows(2) {
        let Some(query) = classify_set(document, pair[0]) else {
            continue;
        };
        let Some(fallback) = classify_set(document, pair[1]) else {
            continue;
        };
        if !query.query
            || query.conditional
            || fallback.query
            || !fallback.conditional
            || query.name != fallback.name
            || fallback.values.is_empty()
        {
            continue;
        }
        let name = query.name;
        if name.starts_with('_') || clobbered.contains(&name) || !seen.insert(name.clone()) {
            continue;
        }
        let default = infer_value(&fallback.values.join(" "));
        let mut declaration = ParamDecl::new(&name);
        declaration.binding = ParameterBinding::EnvDefault;
        declaration.delivery = ParameterDelivery::Env;
        declaration.parameter_type = parameter_type(&default);
        declaration.default = Some(default);
        declaration.secret = is_secret_name(&name);
        candidates.push(SemanticCandidate {
            declaration,
            identity: BindingIdentity {
                binding: ParameterBinding::EnvDefault,
                key: name,
                occurrence: 0,
                scope: Vec::new(),
            },
            span: SourceSpan::from_node(pair[0]),
            demotion: None,
            empty_uses_default: false,
        });
    }

    let surface = cli_surface(document);
    SemanticAnalysis {
        candidates,
        frameworks: (!matches!(surface, CliSurface::Absent))
            .then(|| "argparse".to_owned())
            .into_iter()
            .collect(),
        uses_argv: has_variable(document, "argv"),
        uses_self_location: uses_self_location(document),
        ..SemanticAnalysis::default()
    }
}

pub(super) fn cli_surface(document: &ParsedDocument) -> CliSurface {
    let mut command = None;
    walk(document.tree.root_node(), &mut |node| {
        if command.is_none()
            && node.kind() == "command"
            && command_name(document, node) == Some("argparse")
        {
            command = Some(node);
        }
    });
    let Some(command) = command else {
        return CliSurface::Absent;
    };
    let words = command_argument_nodes(command)
        .into_iter()
        .map(|node| literal_word(document, node))
        .collect::<Vec<_>>();
    let mut index = 0;
    while index < words.len() {
        let Some(word) = &words[index] else {
            break;
        };
        if word == "--" {
            return static_surface("argparse", Vec::new());
        }
        if !word.starts_with('-') {
            break;
        }
        index = index.saturating_add(
            if VALUE_OPTIONS.contains(&word.as_str()) && index.saturating_add(1) < words.len() {
                2
            } else {
                1
            },
        );
    }
    let specs = words[index..]
        .iter()
        .take_while(|word| word.as_deref() != Some("--"))
        .collect::<Vec<_>>();
    if specs.iter().any(|word| word.is_none()) {
        return dynamic_surface("argparse", DegradationReason::DynamicDeclaration);
    }
    let mut fields = Vec::new();
    for raw in specs {
        let raw = raw
            .as_deref()
            .expect("a dynamic fish specification returned before projection");
        let Some((declaration, degradation)) = parse_spec(raw) else {
            continue;
        };
        fields.push(SemanticField {
            identity: BindingIdentity {
                binding: ParameterBinding::None,
                key: declaration.name.clone(),
                occurrence: 0,
                scope: Vec::new(),
            },
            span: SourceSpan::from_node(command),
            declaration,
            degradation,
        });
    }
    static_surface("argparse", fields)
}

fn root_statements(document: &ParsedDocument) -> Vec<tree_sitter::Node<'_>> {
    named_children(document.tree.root_node())
        .into_iter()
        .filter(|node| !matches!(node.kind(), "comment" | "shebang"))
        .collect()
}

fn classify_set(document: &ParsedDocument, node: tree_sitter::Node<'_>) -> Option<SetCommand> {
    let (conditional, command) = if node.kind() == "conditional_execution" {
        (
            true,
            named_children(node)
                .into_iter()
                .find(|child| child.kind() == "command")?,
        )
    } else if node.kind() == "command" {
        (false, node)
    } else {
        return None;
    };
    if command_name(document, command) != Some("set") {
        return None;
    }
    let words = command_arguments(document, command);
    let mut flags = Vec::new();
    let mut operands = Vec::new();
    let mut options_done = false;
    for word in words {
        if !options_done && word == "--" {
            options_done = true;
        } else if !options_done && operands.is_empty() && word.starts_with('-') {
            flags.push(word);
        } else {
            operands.push(word);
        }
    }
    let query = flags.iter().any(|flag| {
        flag == "--query"
            || (!flag.starts_with("--")
                && flag
                    .strip_prefix('-')
                    .is_some_and(|letters| letters.contains('q')))
    });
    Ok::<_, ()>(SetCommand {
        conditional,
        query,
        name: operands.first().cloned()?,
        values: operands.into_iter().skip(1).collect(),
    })
    .ok()
}

fn parse_spec(raw: &str) -> Option<(ParamDecl, Option<DegradationReason>)> {
    let spec = raw.split('!').next()?.trim();
    if spec.is_empty() {
        return None;
    }
    let (core, has_value, multiple) =
        if let Some(core) = spec.strip_suffix("=+").or_else(|| spec.strip_suffix("=*")) {
            (core, true, true)
        } else if let Some(core) = spec.strip_suffix("=?") {
            (core, true, false)
        } else if let Some(core) = spec.strip_suffix('=') {
            (core, true, false)
        } else {
            (spec, false, false)
        };
    let (name, flag, numeric) = parse_name(core)?;
    let mut declaration = ParamDecl::new(&name);
    declaration.flag = flag;
    declaration.secret = is_secret_name(&name);
    let degradation = numeric.then_some(DegradationReason::UnsupportedAction);
    if numeric {
        declaration.degraded = true;
    } else if has_value {
        declaration.multiple = multiple;
        declaration.repeat = multiple;
    } else {
        declaration.parameter_type = ParameterType::Bool;
        declaration.action = "store_true".to_owned();
        declaration.default = Some(ParameterValue::Bool(false));
    }
    Some((declaration, degradation))
}

fn parse_name(core: &str) -> Option<(String, String, bool)> {
    if core.is_empty() {
        return None;
    }
    let mut characters = core.char_indices();
    let (_, first) = characters.next()?;
    let second = characters.next();
    if let Some((offset, separator @ ('/' | '-' | '#'))) = second {
        let long = &core[offset + separator.len_utf8()..];
        if long.is_empty() {
            return Some((first.to_string(), format!("-{first}"), separator == '#'));
        }
        return Some((long.to_owned(), format!("--{long}"), separator == '#'));
    }
    if matches!(first, '/' | '-' | '#') {
        return None;
    }
    if core.chars().count() == 1 {
        Some((core.to_owned(), format!("-{core}"), false))
    } else {
        Some((core.to_owned(), format!("--{core}"), false))
    }
}

fn command_name<'a>(
    document: &'a ParsedDocument,
    command: tree_sitter::Node<'_>,
) -> Option<&'a str> {
    command
        .child_by_field_name("name")
        .map(|node| text(document, node))
}

fn command_arguments(document: &ParsedDocument, command: tree_sitter::Node<'_>) -> Vec<String> {
    command_argument_nodes(command)
        .into_iter()
        .filter_map(|node| literal_word(document, node))
        .collect()
}

fn command_argument_nodes(command: tree_sitter::Node<'_>) -> Vec<tree_sitter::Node<'_>> {
    (0..command.child_count())
        .map_while(|index| u32::try_from(index).ok())
        .filter_map(|index| {
            (command.field_name_for_child(index) == Some("argument"))
                .then(|| command.child(index))
                .flatten()
        })
        .collect()
}

fn literal_word(document: &ParsedDocument, node: tree_sitter::Node<'_>) -> Option<String> {
    match node.kind() {
        "word" | "integer" | "float" if node.named_child_count() == 0 => {
            Some(decode_fish_word(text(document, node)))
        }
        "single_quote_string" | "double_quote_string"
            if named_children(node)
                .iter()
                .all(|child| child.kind() == "string_content") =>
        {
            Some(decode_fish_word(text(document, node)))
        }
        "concatenation"
            if named_children(node).iter().all(|child| {
                matches!(
                    child.kind(),
                    "word"
                        | "integer"
                        | "float"
                        | "escape_sequence"
                        | "single_quote_string"
                        | "double_quote_string"
                )
            }) =>
        {
            Some(decode_fish_word(text(document, node)))
        }
        _ => None,
    }
}

fn decode_fish_word(raw: &str) -> String {
    let (body, quote) = if raw.len() >= 2 && raw.starts_with('\'') && raw.ends_with('\'') {
        (&raw[1..raw.len() - 1], Some('\''))
    } else if raw.len() >= 2 && raw.starts_with('"') && raw.ends_with('"') {
        (&raw[1..raw.len() - 1], Some('"'))
    } else {
        (raw, None)
    };
    // The literal-word gate accepts quoted words only when the parser reports plain
    // `string_content`. Quote escape nodes are dynamic and never reach this decoder. An unquoted
    // concatenation can contain `escape_sequence`, so only that form needs backslash decoding.
    if quote.is_some() {
        return body.to_owned();
    }
    let mut output = String::new();
    let mut characters = body.chars().peekable();
    while let Some(character) = characters.next() {
        if character == '\\'
            && let Some(next) = characters.peek().copied()
        {
            output.push(next);
            characters.next();
            continue;
        }
        output.push(character);
    }
    output
}

fn has_variable(document: &ParsedDocument, expected: &str) -> bool {
    let mut found = false;
    walk(document.tree.root_node(), &mut |node| {
        if node.kind() == "variable_name" && text(document, node) == expected {
            found = true;
        }
    });
    found
}

fn uses_self_location(document: &ParsedDocument) -> bool {
    let mut found = false;
    walk(document.tree.root_node(), &mut |node| {
        if found || node.kind() != "command" || command_name(document, node) != Some("status") {
            return;
        }
        if command_arguments(document, node)
            .iter()
            .any(|argument| matches!(argument.as_str(), "filename" | "dirname"))
        {
            found = true;
        }
    });
    found
}

fn infer_value(value: &str) -> ParameterValue {
    value
        .parse::<i64>()
        .ok()
        .map(ParameterValue::Integer)
        .or_else(|| {
            value
                .parse::<f64>()
                .ok()
                .filter(|number| number.is_finite() && value.contains('.'))
                .map(ParameterValue::Float)
        })
        .unwrap_or_else(|| ParameterValue::String(value.to_owned()))
}

fn parameter_type(value: &ParameterValue) -> ParameterType {
    match value {
        ParameterValue::Integer(_) => ParameterType::Int,
        ParameterValue::Float(_) => ParameterType::Float,
        ParameterValue::String(_) | ParameterValue::Bool(_) => ParameterType::Str,
    }
}
