use std::collections::{BTreeMap, BTreeSet};

use skit_domain::parameters::{
    ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, ParameterValue, is_secret_name,
};

use super::{
    BindingIdentity, CliSurface, DegradationReason, ParsedDocument, SemanticAnalysis,
    SemanticCandidate, SemanticField, SourceEdit, SourceEditPlan, SourceSpan, dynamic_surface,
    match_calls, named_children, render_float, static_surface, text, walk,
};
use crate::{LanguageError, ShellInputError};

const DEFAULT_OPERATORS: [&str; 4] = [":-", ":=", "-", "="];
const VALUE_FLAGS: &str = "adiNntpu";
const REFRAMING_FLAGS: &str = "nNd";

#[derive(Default)]
struct ReadShape {
    secret: bool,
    prompt: String,
    variables: Vec<String>,
    reframing: bool,
    raw: bool,
}

struct ReadSite<'tree> {
    order: i64,
    command: usize,
    node: tree_sitter::Node<'tree>,
    prompt: String,
    secret: bool,
    raw: bool,
}

pub(super) fn analysis(document: &ParsedDocument) -> SemanticAnalysis {
    let mut candidates = constant_candidates(document);
    let mutated = mutated_names(document);
    for candidate in &mut candidates {
        if mutated.contains(&candidate.declaration.name) {
            candidate.demotion = Some(DegradationReason::Accumulator);
        }
    }
    let clobbered = clobbered_names(document);
    candidates.extend(env_default_candidates(document, &clobbered));
    candidates.extend(read_candidates(document));
    let surface = cli_surface(document);
    SemanticAnalysis {
        frameworks: (!matches!(surface, CliSurface::Absent))
            .then(|| "getopts".to_owned())
            .into_iter()
            .collect(),
        uses_argv: uses_argv(document),
        uses_self_location: uses_self_location(document),
        candidates,
        ..SemanticAnalysis::default()
    }
}

pub(super) fn cli_surface(document: &ParsedDocument) -> CliSurface {
    let mut call = None;
    walk(document.tree.root_node(), &mut |node| {
        if call.is_none()
            && node.kind() == "command"
            && command_name(document, node) == Some("getopts")
        {
            call = Some(node);
        }
    });
    let Some(call) = call else {
        return CliSurface::Absent;
    };
    let arguments = command_arguments(call);
    let Some(first) = arguments.first() else {
        return CliSurface::Absent;
    };
    let Some(spec) = literal_text(document, *first) else {
        return dynamic_surface("getopts", DegradationReason::DynamicDeclaration);
    };
    let chars = spec.chars().collect::<Vec<_>>();
    let mut fields = Vec::new();
    let mut seen = BTreeSet::new();
    let mut index = usize::from(chars.first() == Some(&':'));
    while index < chars.len() {
        let letter = chars[index];
        if !letter.is_alphanumeric() {
            index = index.saturating_add(1);
            continue;
        }
        let takes_value = chars.get(index.saturating_add(1)) == Some(&':');
        if seen.insert(letter) {
            let name = letter.to_string();
            let mut declaration = ParamDecl::new(&name);
            declaration.flag = format!("-{letter}");
            declaration.secret = is_secret_name(&name);
            if !takes_value {
                declaration.parameter_type = ParameterType::Bool;
                declaration.action = "store_true".to_owned();
                declaration.default = Some(ParameterValue::Bool(false));
            }
            fields.push(SemanticField {
                identity: BindingIdentity {
                    binding: ParameterBinding::None,
                    key: name,
                    occurrence: 0,
                    scope: Vec::new(),
                },
                span: SourceSpan::from_node(call),
                declaration,
                degradation: None,
            });
        }
        index = index
            .saturating_add(1)
            .saturating_add(usize::from(takes_value));
    }
    static_surface("getopts", fields)
}

fn top_level_assignments(document: &ParsedDocument) -> Vec<(tree_sitter::Node<'_>, bool)> {
    let mut output = Vec::new();
    for statement in named_children(document.tree.root_node()) {
        if statement.kind() == "variable_assignment" {
            output.push((statement, false));
            continue;
        }
        if statement.kind() != "declaration_command" {
            continue;
        }
        let keyword = statement.child(0).map_or("", |node| text(document, node));
        if keyword == "local" {
            continue;
        }
        let readonly = keyword == "readonly"
            || named_children(statement).into_iter().any(|node| {
                node.kind() == "word"
                    && text(document, node).starts_with('-')
                    && !text(document, node).starts_with("--")
                    && text(document, node)[1..].contains('r')
            });
        output.extend(
            named_children(statement)
                .into_iter()
                .filter(|node| node.kind() == "variable_assignment")
                .map(|node| (node, readonly)),
        );
    }
    output
}

fn constant_candidates(document: &ParsedDocument) -> Vec<SemanticCandidate> {
    let mut output = Vec::<SemanticCandidate>::new();
    for (assignment, readonly) in top_level_assignments(document) {
        if readonly || assignment_operator(assignment) != "=" {
            continue;
        }
        let Some(name_node) = assignment.child_by_field_name("name") else {
            continue;
        };
        if name_node.kind() != "variable_name" {
            continue;
        }
        let name = text(document, name_node);
        if name.starts_with('_') {
            continue;
        }
        let Some(value_node) = assignment.child_by_field_name("value") else {
            continue;
        };
        let Some(value) = literal_text(document, value_node).filter(|value| !value.is_empty())
        else {
            continue;
        };
        let default = infer_value(&value);
        let mut declaration = ParamDecl::new(name);
        declaration.binding = ParameterBinding::Const;
        declaration.delivery = ParameterDelivery::Inject;
        declaration.parameter_type = parameter_type(&default);
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
            span: SourceSpan::from_node(assignment),
            demotion: None,
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
    output
}

fn env_default_candidates(
    document: &ParsedDocument,
    clobbered: &BTreeSet<String>,
) -> Vec<SemanticCandidate> {
    let mut output = Vec::new();
    let mut seen = BTreeSet::new();
    walk(document.tree.root_node(), &mut |node| {
        if node.kind() != "expansion" {
            return;
        }
        let Some(operator) = node.child_by_field_name("operator") else {
            return;
        };
        let operator_text = text(document, operator);
        if !DEFAULT_OPERATORS.contains(&operator_text) {
            return;
        }
        let Some(name_node) = named_children(node).into_iter().next() else {
            return;
        };
        if name_node.kind() != "variable_name" {
            return;
        }
        let name = text(document, name_node);
        if clobbered.contains(name) || !seen.insert(name.to_owned()) {
            return;
        }
        let default_text = document
            .source
            .get(operator.end_byte()..node.end_byte().saturating_sub(1))
            .unwrap_or_default();
        let default = infer_value(default_text);
        let mut declaration = ParamDecl::new(name);
        declaration.binding = ParameterBinding::EnvDefault;
        declaration.delivery = ParameterDelivery::Env;
        declaration.parameter_type = parameter_type(&default);
        declaration.default = Some(default);
        declaration.secret = is_secret_name(name);
        output.push(SemanticCandidate {
            declaration,
            identity: BindingIdentity {
                binding: ParameterBinding::EnvDefault,
                key: name.to_owned(),
                occurrence: 0,
                scope: Vec::new(),
            },
            span: SourceSpan::from_node(node),
            demotion: None,
            empty_uses_default: matches!(operator_text, ":-" | ":="),
        });
    });
    output
}

fn clobbered_names(document: &ParsedDocument) -> BTreeSet<String> {
    top_level_assignments(document)
        .into_iter()
        .filter_map(|(assignment, _)| {
            let name = assignment.child_by_field_name("name")?;
            (name.kind() == "variable_name").then_some((assignment, name))
        })
        .filter(|(assignment, name)| {
            assignment
                .child_by_field_name("value")
                .is_none_or(|value| !references(document, value, text(document, *name)))
        })
        .map(|(_, name)| text(document, name).to_owned())
        .collect()
}

fn read_candidates(document: &ParsedDocument) -> Vec<SemanticCandidate> {
    let mut output = Vec::new();
    let mut order = 0_i64;
    for (node, shape) in injectable_reads(document) {
        for variable in shape.variables {
            let name = format!("input-{}", order.saturating_add(1));
            let mut declaration = ParamDecl::new(&name);
            declaration.binding = ParameterBinding::Input;
            declaration.delivery = ParameterDelivery::Inject;
            declaration.prompt.clone_from(&shape.prompt);
            declaration.order = order;
            declaration.secret =
                shape.secret || is_secret_name(&shape.prompt) || is_secret_name(&variable);
            output.push(SemanticCandidate {
                declaration,
                identity: BindingIdentity {
                    binding: ParameterBinding::Input,
                    key: shape.prompt.clone(),
                    occurrence: usize::try_from(order).unwrap_or(usize::MAX),
                    scope: Vec::new(),
                },
                span: SourceSpan::from_node(node),
                demotion: None,
                empty_uses_default: false,
            });
            order = order.saturating_add(1);
        }
    }
    output
}

fn read_shape(document: &ParsedDocument, command: tree_sitter::Node<'_>) -> Option<ReadShape> {
    let mut arguments = command_arguments(command);
    match command_name(document, command)? {
        "read" => {}
        "builtin" | "command"
            if arguments
                .first()
                .is_some_and(|node| text(document, *node) == "read") =>
        {
            arguments.remove(0);
        }
        _ => return None,
    }
    let mut shape = ReadShape::default();
    let mut options_done = false;
    let mut index = 0;
    while index < arguments.len() {
        let argument = arguments[index];
        let raw = text(document, argument);
        if !options_done && argument.kind() == "word" && raw == "--" {
            options_done = true;
        } else if !options_done
            && argument.kind() == "word"
            && raw.starts_with('-')
            && raw.len() > 1
        {
            let cluster = &raw[1..];
            for (offset, letter) in cluster.char_indices() {
                if letter == 'r' {
                    shape.raw = true;
                    continue;
                }
                if letter == 's' {
                    shape.secret = true;
                    continue;
                }
                if VALUE_FLAGS.contains(letter) {
                    if REFRAMING_FLAGS.contains(letter) {
                        shape.reframing = true;
                    }
                    let attached = &cluster[offset + letter.len_utf8()..];
                    if letter == 'p' {
                        if attached.is_empty() {
                            if let Some(prompt) = arguments.get(index.saturating_add(1)) {
                                shape.prompt = literal_text(document, *prompt).unwrap_or_default();
                                index = index.saturating_add(1);
                            }
                        } else {
                            shape.prompt = attached.to_owned();
                        }
                    } else if attached.is_empty()
                        && arguments.get(index.saturating_add(1)).is_some()
                    {
                        index = index.saturating_add(1);
                    }
                    break;
                }
            }
        } else if argument.kind() == "word" {
            shape.variables.push(raw.to_owned());
        }
        index = index.saturating_add(1);
    }
    Some(shape)
}

fn injectable_reads(document: &ParsedDocument) -> Vec<(tree_sitter::Node<'_>, ReadShape)> {
    let mut reads = Vec::new();
    walk(document.tree.root_node(), &mut |node| {
        if node.kind() == "command"
            && let Some(shape) = read_shape(document, node)
            && !shape.reframing
            && !has_ifs_prefix(document, node)
            && !is_data_read(document, node)
        {
            reads.push((node, shape));
        }
    });
    reads.sort_by_key(|(node, _)| node.start_byte());
    reads
}

pub(super) fn plan_injection(
    document: &ParsedDocument,
    declarations: &[ParamDecl],
    values: &BTreeMap<String, String>,
    interpreter: Option<&str>,
) -> Result<SourceEditPlan, LanguageError> {
    let selected = declarations
        .iter()
        .filter(|declaration| {
            declaration.delivery == ParameterDelivery::Inject
                && values.contains_key(&declaration.name)
        })
        .collect::<Vec<_>>();
    let mut edits = Vec::new();
    let mut matched = BTreeSet::new();

    for declaration in selected
        .iter()
        .copied()
        .filter(|declaration| declaration.binding == ParameterBinding::Const)
    {
        let targets = constant_targets(document, &declaration.name);
        if targets.is_empty() {
            continue;
        }
        let raw = values
            .get(&declaration.name)
            .expect("selected declarations have accepted values");
        let replacement = typed_constant_literal(declaration, raw)?;
        edits.extend(targets.into_iter().map(|target| SourceEdit {
            span: SourceSpan::from_node(target),
            replacement: replacement.clone(),
        }));
        matched.insert(declaration.name.clone());
    }

    let sites = read_sites(document);
    let current = sites
        .iter()
        .map(|site| (site.order, site.prompt.clone()))
        .collect::<Vec<_>>();
    let stored = selected
        .iter()
        .copied()
        .filter(|declaration| declaration.binding == ParameterBinding::Input)
        .map(|declaration| (declaration.order, declaration.prompt.clone()))
        .collect::<Vec<_>>();
    let bindings = match_calls(&stored, &current);
    let mut queue = BTreeMap::<i64, (&str, bool)>::new();
    for declaration in selected
        .iter()
        .copied()
        .filter(|declaration| declaration.binding == ParameterBinding::Input)
    {
        let Some((resolved, _)) = bindings.get(&declaration.order) else {
            continue;
        };
        if queue.contains_key(resolved) {
            continue;
        }
        let raw = values
            .get(&declaration.name)
            .expect("selected declarations have accepted values");
        queue.insert(*resolved, (raw, declaration.secret));
        matched.insert(declaration.name.clone());
    }

    if let Some(missing) = selected
        .iter()
        .find(|declaration| !matched.contains(&declaration.name))
    {
        return Err(LanguageError::BindingNotFound {
            name: missing.name.clone(),
        });
    }

    let read_edits = read_edits(document, &sites, &queue)?;
    let intercepts_read = !read_edits.is_empty();
    edits.extend(read_edits);
    if intercepts_read {
        let offset = shell_preamble_offset(&document.source);
        let line = document.source[..offset]
            .bytes()
            .filter(|byte| *byte == b'\n')
            .count()
            .saturating_add(1);
        edits.push(SourceEdit {
            span: SourceSpan::insertion(offset, line),
            replacement: shell_read_preamble(shell_fallthrough_keyword(
                interpreter,
                &document.source,
            )),
        });
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
    top_level_assignments(document)
        .into_iter()
        .filter(|(assignment, readonly)| {
            !readonly
                && assignment_operator(*assignment) == "="
                && assignment.child_by_field_name("name").is_some_and(|name| {
                    name.kind() == "variable_name" && text(document, name) == expected
                })
        })
        .filter_map(|(assignment, _)| assignment.child_by_field_name("value"))
        .filter(|value| literal_text(document, *value).is_some_and(|literal| !literal.is_empty()))
        .collect()
}

fn typed_constant_literal(declaration: &ParamDecl, raw: &str) -> Result<String, LanguageError> {
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
        ParameterType::Bool | ParameterType::Str | ParameterType::Choice | ParameterType::Path => {
            Ok(quote(raw))
        }
    }
}

fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn read_sites(document: &ParsedDocument) -> Vec<ReadSite<'_>> {
    let mut sites = Vec::new();
    let mut order = 0_i64;
    for (command, (node, shape)) in injectable_reads(document).into_iter().enumerate() {
        for _variable in &shape.variables {
            sites.push(ReadSite {
                order,
                command,
                node,
                prompt: shape.prompt.clone(),
                secret: shape.secret,
                raw: shape.raw,
            });
            order = order.saturating_add(1);
        }
    }
    sites
}

fn read_edits(
    document: &ParsedDocument,
    sites: &[ReadSite<'_>],
    queue: &BTreeMap<i64, (&str, bool)>,
) -> Result<Vec<SourceEdit>, LanguageError> {
    let mut groups = BTreeMap::<usize, Vec<&ReadSite<'_>>>::new();
    for site in sites {
        groups.entry(site.command).or_default().push(site);
    }
    let mut edits = Vec::new();
    for (command, group) in groups {
        let supplied = group
            .iter()
            .map(|site| queue.get(&site.order).copied())
            .collect::<Vec<_>>();
        if supplied.iter().all(Option::is_none) {
            continue;
        }
        check_read_values(&group, &supplied)?;
        let line = supplied
            .iter()
            .filter_map(|value| value.map(|(value, _)| feed_read_value(value, group[0].raw)))
            .collect::<Vec<_>>()
            .join(" ");
        let secret = group.iter().zip(&supplied).any(|(site, value)| {
            value.is_some_and(|(_, selected_secret)| site.secret || selected_secret)
        });
        let span = command_name_span(document, group[0].node);
        edits.push(SourceEdit {
            span,
            replacement: format!(
                "_skit_read {command} {} {} {}",
                quote(&line),
                u8::from(secret),
                quote(&group[0].prompt)
            ),
        });
    }
    Ok(edits)
}

fn check_read_values(
    group: &[&ReadSite<'_>],
    supplied: &[Option<(&str, bool)>],
) -> Result<(), LanguageError> {
    let last = supplied.len().saturating_sub(1);
    for (index, value) in supplied.iter().enumerate() {
        let Some((value, _)) = value else {
            continue;
        };
        let error = if value.contains('\n') {
            Some(ShellInputError::LineBreak {
                name: input_name(group[index]),
            })
        } else if index != last && value.contains([' ', '\t']) {
            Some(ShellInputError::FieldSplit {
                name: input_name(group[index]),
            })
        } else if index == last && (value.starts_with([' ', '\t']) || value.ends_with([' ', '\t']))
        {
            Some(ShellInputError::EdgeSpace {
                name: input_name(group[index]),
            })
        } else {
            None
        };
        if let Some(error) = error {
            return Err(error.into());
        }
    }
    for (index, value) in supplied.iter().enumerate() {
        if value.is_some_and(|(value, _)| !value.is_empty() || index == last) {
            continue;
        }
        if let Some(later) = supplied
            .iter()
            .enumerate()
            .skip(index.saturating_add(1))
            .find(|(_, value)| value.is_some_and(|(value, _)| !value.is_empty()))
            .map(|(index, _)| index)
        {
            return Err(ShellInputError::Gap {
                empty: input_name(group[index]),
                filled: input_name(group[later]),
            }
            .into());
        }
        break;
    }
    Ok(())
}

fn input_name(site: &ReadSite<'_>) -> String {
    format!("input-{}", site.order.saturating_add(1))
}

fn feed_read_value(value: &str, raw: bool) -> String {
    if raw {
        value.to_owned()
    } else {
        value.replace('\\', "\\\\")
    }
}

fn command_name_span(document: &ParsedDocument, command: tree_sitter::Node<'_>) -> SourceSpan {
    let name = command
        .child_by_field_name("name")
        .expect("a command has a command name");
    let name_text = text(document, name);
    if matches!(name_text, "builtin" | "command")
        && let Some(read) = command_arguments(command).first()
    {
        return SourceSpan {
            start: name.start_byte(),
            end: read.end_byte(),
            start_line: name.start_position().row.saturating_add(1),
            end_line: read.end_position().row.saturating_add(1),
        };
    }
    SourceSpan::from_node(name)
}

fn shell_preamble_offset(source: &str) -> usize {
    if source.starts_with("#!") {
        source.find('\n').map_or(source.len(), |index| index + 1)
    } else {
        0
    }
}

fn shell_fallthrough_keyword<'a>(interpreter: Option<&'a str>, source: &'a str) -> &'static str {
    let configured = interpreter
        .filter(|value| !value.is_empty())
        .or_else(|| source.lines().next().and_then(crate::shebang_program))
        .unwrap_or("sh");
    let basename = configured.rsplit(['/', '\\']).next().unwrap_or_default();
    let program = basename.strip_suffix(".exe").unwrap_or(basename);
    if matches!(program, "bash" | "zsh") {
        "builtin"
    } else {
        "command"
    }
}

fn shell_read_preamble(keyword: &str) -> String {
    format!(
        concat!(
            "_skit_read() {{\n",
            "  _sk=$1; _sv=$2; _ss=$3; _sp=$4; shift 4\n",
            "  eval \"_su=\\${{_skit_used_$_sk-}}\"\n",
            "  if [ -z \"$_su\" ]; then\n",
            "    eval \"_skit_used_$_sk=1\"\n",
            "    if [ \"$_ss\" = 1 ]; then printf '%s%s\\n' \"$_sp\" '***'; else printf '%s%s\\n' \"$_sp\" \"$_sv\"; fi\n",
            "    {keyword} read \"$@\" <<EOF\n",
            "$_sv\n",
            "EOF\n",
            "  else\n",
            "    {keyword} read \"$@\"\n",
            "  fi\n",
            "}}  # skit:shim\n",
        ),
        keyword = keyword
    )
}

pub(super) fn normalize(
    document: &ParsedDocument,
    name: &str,
) -> Result<SourceEditPlan, LanguageError> {
    let assignments = top_level_assignments(document)
        .into_iter()
        .filter(|(assignment, _)| {
            assignment_operator(*assignment) == "="
                && assignment.child_by_field_name("name").is_some_and(|node| {
                    node.kind() == "variable_name" && text(document, node) == name
                })
        })
        .collect::<Vec<_>>();
    let [(assignment, readonly)] = assignments.as_slice() else {
        return Err(LanguageError::BindingNotFound {
            name: name.to_owned(),
        });
    };
    if *readonly {
        return Err(LanguageError::BindingNotFound {
            name: name.to_owned(),
        });
    }
    let Some(value) = assignment.child_by_field_name("value") else {
        return Err(LanguageError::BindingNotFound {
            name: name.to_owned(),
        });
    };
    if references(document, value, name) {
        return Err(LanguageError::BindingNotFound {
            name: name.to_owned(),
        });
    }
    let Some(literal) = literal_text(document, value).filter(|value| !value.is_empty()) else {
        return Err(LanguageError::BindingNotFound {
            name: name.to_owned(),
        });
    };
    if literal
        .chars()
        .any(|character| "}\"`$\\\n;|&()<>".contains(character))
    {
        return Err(LanguageError::BindingNotFound {
            name: name.to_owned(),
        });
    }
    Ok(SourceEditPlan {
        source: document.source.clone(),
        edits: vec![SourceEdit {
            span: SourceSpan::from_node(value),
            replacement: format!("\"${{{name}:-{literal}}}\""),
        }],
    })
}

fn has_ifs_prefix(document: &ParsedDocument, command: tree_sitter::Node<'_>) -> bool {
    (0..command.child_count()).any(|index| {
        u32::try_from(index)
            .ok()
            .and_then(|index| command.child(index))
            .is_some_and(|child| {
                child.kind() == "variable_assignment"
                    && child
                        .child_by_field_name("name")
                        .is_some_and(|name| text(document, name) == "IFS")
            })
    })
}

fn is_data_read(document: &ParsedDocument, command: tree_sitter::Node<'_>) -> bool {
    if feeds_stdin(document, command) {
        return true;
    }
    let mut node = command;
    while let Some(parent) = node.parent() {
        if parent.kind() == "pipeline" {
            let children = named_children(parent);
            if children
                .first()
                .is_some_and(|first| first.id() != node.id())
            {
                return true;
            }
        }
        if parent.kind() == "redirected_statement"
            && parent
                .child_by_field_name("body")
                .is_some_and(|body| body.id() == node.id())
            && feeds_stdin(document, parent)
        {
            return true;
        }
        node = parent;
    }
    false
}

fn feeds_stdin(document: &ParsedDocument, node: tree_sitter::Node<'_>) -> bool {
    named_children(node)
        .into_iter()
        .any(|child| match child.kind() {
            "heredoc_redirect" | "herestring_redirect" => true,
            "file_redirect" => child
                .child(0)
                .is_some_and(|operator| text(document, operator) == "<"),
            _ => false,
        })
}

fn mutated_names(document: &ParsedDocument) -> BTreeSet<String> {
    let mut output = BTreeSet::new();
    walk(document.tree.root_node(), &mut |node| match node.kind() {
        "variable_assignment" => {
            let Some(name) = node.child_by_field_name("name") else {
                return;
            };
            if name.kind() != "variable_name" {
                return;
            }
            let name_text = text(document, name);
            if assignment_operator(node) == "+="
                || node
                    .child_by_field_name("value")
                    .is_some_and(|value| references(document, value, name_text))
            {
                output.insert(name_text.to_owned());
            }
        }
        "postfix_expression" => {
            let mut found = None;
            walk(node, &mut |child| {
                if found.is_none() && child.kind() == "variable_name" {
                    found = Some(text(document, child).to_owned());
                }
            });
            output.extend(found);
        }
        "binary_expression" => {
            let assignment = (0..node.child_count()).any(|index| {
                u32::try_from(index)
                    .ok()
                    .and_then(|index| node.child(index))
                    .is_some_and(|child| {
                        !child.is_named()
                            && matches!(
                                child.kind(),
                                "=" | "+="
                                    | "-="
                                    | "*="
                                    | "/="
                                    | "%="
                                    | "**="
                                    | "<<="
                                    | ">>="
                                    | "&="
                                    | "|="
                                    | "^="
                            )
                    })
            });
            if assignment
                && let Some(left) = node.child_by_field_name("left")
                && left.kind() == "variable_name"
            {
                output.insert(text(document, left).to_owned());
            }
        }
        "command" if command_name(document, node) == Some("let") => {
            for argument in command_arguments(node) {
                if let Some(name) = leading_assignment_name(text(document, argument)) {
                    output.insert(name.to_owned());
                }
            }
        }
        "for_statement" | "while_statement" | "c_style_for_statement" => {
            walk(node, &mut |child| {
                if child.kind() == "variable_assignment"
                    && let Some(name) = child.child_by_field_name("name")
                    && name.kind() == "variable_name"
                {
                    output.insert(text(document, name).to_owned());
                }
            });
        }
        _ => {}
    });
    output
}

fn leading_assignment_name(raw: &str) -> Option<&str> {
    let end = raw
        .find(|character: char| !character.is_alphanumeric() && character != '_')
        .unwrap_or(raw.len());
    let name = &raw[..end];
    (!name.is_empty()
        && [
            "++", "--", "<<=", ">>=", "+=", "-=", "*=", "/=", "%=", "^=", "&=", "|=", "=",
        ]
        .iter()
        .any(|operator| raw[end..].starts_with(operator)))
    .then_some(name)
}

fn references(document: &ParsedDocument, node: tree_sitter::Node<'_>, expected: &str) -> bool {
    let mut found = false;
    walk(node, &mut |child| {
        if child.kind() == "variable_name" && text(document, child) == expected {
            found = true;
        }
    });
    found
}

fn uses_self_location(document: &ParsedDocument) -> bool {
    let mut found = false;
    walk(document.tree.root_node(), &mut |node| {
        if node.kind() == "variable_name" && matches!(text(document, node), "0" | "BASH_SOURCE") {
            found = true;
        }
    });
    found
}

fn uses_argv(document: &ParsedDocument) -> bool {
    let mut found = false;
    walk(document.tree.root_node(), &mut |node| match node.kind() {
        "variable_name" => {
            let value = text(document, node);
            if value != "0" && value.chars().all(|character| character.is_ascii_digit()) {
                found = true;
            }
        }
        "special_variable_name" if matches!(text(document, node), "@" | "*" | "#") => {
            found = true;
        }
        "command" if matches!(command_name(document, node), Some("getopts" | "shift")) => {
            found = true;
        }
        _ => {}
    });
    found
}

fn command_name<'a>(
    document: &'a ParsedDocument,
    command: tree_sitter::Node<'_>,
) -> Option<&'a str> {
    command
        .child_by_field_name("name")
        .map(|node| text(document, node))
}

fn command_arguments(command: tree_sitter::Node<'_>) -> Vec<tree_sitter::Node<'_>> {
    (0..command.child_count())
        .filter_map(|index| {
            let index = u32::try_from(index).ok()?;
            (command.field_name_for_child(index) == Some("argument"))
                .then(|| command.child(index))
                .flatten()
        })
        .collect()
}

fn assignment_operator(node: tree_sitter::Node<'_>) -> &str {
    (0..node.child_count())
        .filter_map(|index| {
            u32::try_from(index)
                .ok()
                .and_then(|index| node.child(index))
        })
        .find(|child| matches!(child.kind(), "=" | "+="))
        .map_or("=", |child| child.kind())
}

fn literal_text(document: &ParsedDocument, node: tree_sitter::Node<'_>) -> Option<String> {
    match node.kind() {
        "word" | "number" if node.named_child_count() == 0 => Some(text(document, node).to_owned()),
        "raw_string" => text(document, node)
            .get(1..text(document, node).len().saturating_sub(1))
            .map(ToOwned::to_owned),
        "string"
            if named_children(node)
                .iter()
                .all(|child| child.kind() == "string_content") =>
        {
            Some(
                named_children(node)
                    .into_iter()
                    .map(|child| text(document, child))
                    .collect(),
            )
        }
        _ => None,
    }
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
