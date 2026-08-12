//! Analyze and rewrite supported source files.
//!
//! The analyzers read static source text. They do not execute user code.
//! When a dynamic declaration is not clear, the analyzer returns no field.

#![forbid(unsafe_code)]

mod description;
mod semantic;
mod source_text;
mod uv_edit;

pub use description::suggest_description;
pub use semantic::{
    BindingIdentity, CliSurface, DegradationReason, DynamicCliSurface, ParseFailure, ParseOutcome,
    ParsedDocument, ReconcilePair, ReconcileReport, SemanticAnalysis, SemanticCandidate,
    SemanticField, SourceEdit, SourceEditPlan, SourceParameterSemantics, SourceSpan,
    StaticCliSurface, parse_document, source_parameter_semantics,
};
pub use source_text::{
    LosslessSource, NewlineStyle, has_uv_metadata_block_bytes, write_managed_params_bytes,
    write_uv_metadata_bytes,
};
pub use uv_edit::{
    UvMetadataEditError, UvMetadataEditPlan, effective_uv_metadata_bytes, plan_uv_metadata_edit,
};

use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    str::FromStr,
};

use pep440_rs::VersionSpecifiers;
use pep508_rs::{Requirement, VerbatimUrl};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use skit_domain::parameters::{ParamDecl, ParameterType, ParameterValue, synthesized_placeholder};
use skit_i18n::{Localize, Message};
use thiserror::Error;
use toml::Value as TomlValue;

/// Report a shell `read` value that cannot be delivered byte-for-byte.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ShellInputError {
    /// A later variable is filled after an empty variable in the same call.
    #[error(
        "{empty} is empty, but {filled} is filled and they are read on the same line — a shell `read` would hand your value to {empty}. Fill {empty} in, or clear {filled}."
    )]
    Gap {
        /// Empty form field.
        empty: String,
        /// Later filled form field.
        filled: String,
    },
    /// A newline terminates the one line consumed by `read`.
    #[error(
        "{name} can't contain a line break: a shell `read` takes ONE line, so everything after the break would be thrown away."
    )]
    LineBreak {
        /// Form field.
        name: String,
    },
    /// Default IFS parsing would split a non-final value.
    #[error(
        "{name} is read on the same line as other values, so its value can't contain spaces or tabs — the shell would split it across the other fields. Only the LAST value on a `read` line may contain spaces."
    )]
    FieldSplit {
        /// Form field.
        name: String,
    },
    /// Default IFS parsing would trim the final value.
    #[error(
        "{name} starts or ends with a space or tab, which a shell `read` strips off the line — the script would receive it trimmed. Remove the surrounding whitespace."
    )]
    EdgeSpace {
        /// Form field.
        name: String,
    },
}

/// Report a source-analysis or rewrite refusal.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum LanguageError {
    /// This operation is not available for the entry kind.
    #[error("source operation is not supported for entry kind {kind}")]
    UnsupportedKind { kind: String },
    /// A selected source binding no longer exists.
    #[error("parameter {name:?} no longer has a matching source binding")]
    BindingNotFound { name: String },
    /// Inline metadata is malformed or cannot be encoded.
    #[error("inline metadata is not valid: {reason}")]
    InvalidMetadata { reason: Message },
    /// A parser-backed source has syntax errors.
    #[error("source is not valid {kind} syntax")]
    InvalidSource { kind: String },
    /// The source no longer matches the version used for semantic planning.
    #[error("source changed after semantic edit planning")]
    SourceChanged,
    /// An accepted form value does not fit the reflected scalar type.
    #[error(
        "value {value:?} for parameter {name:?} is not a valid {}",
        .parameter_type.as_str()
    )]
    InvalidValue {
        /// Parameter key.
        name: String,
        /// Rejected form value.
        value: String,
        /// Expected scalar type.
        parameter_type: ParameterType,
    },
    /// A shell `read` cannot deliver one accepted value byte-for-byte.
    #[error(transparent)]
    ShellInput(#[from] ShellInputError),
}

/// Report an invalid Python package or version constraint.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum PythonMetadataError {
    /// A package requirement does not use the PEP 508 grammar.
    #[error("{value} isn't a package requirement (e.g. \"requests\" or \"rich>=13,<16\").")]
    InvalidRequirement { value: String },
    /// A Python version constraint does not use the PEP 440 grammar.
    #[error("{value} isn't a Python version constraint (e.g. \">=3.11\" or \">=3.12,<3.13\").")]
    InvalidVersionConstraint { value: String },
}

impl Localize for LanguageError {
    fn message(&self) -> Message {
        match self {
            Self::UnsupportedKind { kind } => {
                Message::new("source operation is not supported for entry kind {}").with(kind)
            }
            Self::BindingNotFound { name } => {
                Message::new("parameter {} no longer has a matching source binding").quoted(name)
            }
            Self::InvalidMetadata { reason } => {
                Message::new("inline metadata is not valid: {}").nested(reason.clone())
            }
            Self::InvalidSource { kind } => {
                Message::new("source is not valid {} syntax").with(kind)
            }
            Self::SourceChanged => Message::new("source changed after semantic edit planning"),
            Self::InvalidValue {
                name,
                value,
                parameter_type,
            } => Message::new("value {} for parameter {} is not a valid {}")
                .quoted(value)
                .quoted(name)
                .with(parameter_type.as_str()),
            Self::ShellInput(source) => source.message(),
        }
    }
}

impl Localize for ShellInputError {
    fn message(&self) -> Message {
        match self {
            Self::Gap { empty, filled } => Message::new(
                "{} is empty, but {} is filled and they are read on the same line — a shell `read` would hand your value to {}. Fill {} in, or clear {}.",
            )
            .with(empty)
            .with(filled)
            .with(empty)
            .with(empty)
            .with(filled),
            Self::LineBreak { name } => Message::new(
                "{} can't contain a line break: a shell `read` takes ONE line, so everything after the break would be thrown away.",
            )
            .with(name),
            Self::FieldSplit { name } => Message::new(
                "{} is read on the same line as other values, so its value can't contain spaces or tabs — the shell would split it across the other fields. Only the LAST value on a `read` line may contain spaces.",
            )
            .with(name),
            Self::EdgeSpace { name } => Message::new(
                "{} starts or ends with a space or tab, which a shell `read` strips off the line — the script would receive it trimmed. Remove the surrounding whitespace.",
            )
            .with(name),
        }
    }
}

impl Localize for PythonMetadataError {
    fn message(&self) -> Message {
        match self {
            Self::InvalidRequirement { value } => Message::new(
                "{} isn't a package requirement (e.g. \"requests\" or \"rich>=13,<16\").",
            )
            .with(value),
            Self::InvalidVersionConstraint { value } => Message::new(
                "{} isn't a Python version constraint (e.g. \">=3.11\" or \">=3.12,<3.13\").",
            )
            .with(value),
        }
    }
}

/// Effective PEP 723 fields used by Python copy entries.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct UvMetadata {
    /// PEP 508 dependency strings.
    pub dependencies: Vec<String>,
    /// Python version constraint.
    pub requires_python: String,
}

/// Infer a known kind from a path, optional shebang, and executable status.
#[must_use]
pub fn infer_kind(path: &Path, shebang: Option<&str>, executable: bool) -> Option<&'static str> {
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if name.ends_with(".prompt.md") || name.ends_with(".prompt") {
        return Some("prompt");
    }
    if let Some(kind) = extension_kind(&name) {
        return Some(kind);
    }
    if let Some(kind) = shebang.and_then(shebang_kind) {
        return Some(kind);
    }
    executable.then_some("exe")
}

fn extension_kind(name: &str) -> Option<&'static str> {
    [
        (".py", "python"),
        (".sh", "shell"),
        (".bash", "shell"),
        (".zsh", "shell"),
        (".fish", "fish"),
        (".js", "js"),
        (".mjs", "js"),
        (".cjs", "js"),
        (".ts", "ts"),
        (".mts", "ts"),
        (".cts", "ts"),
        (".ps1", "powershell"),
        (".rb", "ruby"),
        (".pl", "perl"),
        (".lua", "lua"),
        (".r", "r"),
    ]
    .into_iter()
    .find_map(|(extension, kind)| name.ends_with(extension).then_some(kind))
}

/// Return the program name from one shebang line.
#[must_use]
pub fn shebang_program(line: &str) -> Option<&str> {
    let line = line.strip_prefix("#!")?.trim();
    let mut words = line.split_whitespace();
    let first = words.next()?;
    let program = if basename(first) == "env" {
        words.find(|value| !value.starts_with('-'))?
    } else {
        first
    };
    Some(basename(program))
}

/// Return the PEP 440 constraint from a versioned Python program name.
#[must_use]
pub fn python_version_pin(program: &str) -> Option<String> {
    let program = basename(program);
    let version = program.strip_prefix("python3.")?;
    let mut parts = version.split('.');
    let minor = parts.next()?;
    if minor.is_empty() || !minor.chars().all(|character| character.is_ascii_digit()) {
        return None;
    }
    let micro = parts.collect::<Vec<_>>();
    if micro
        .iter()
        .any(|part| part.is_empty() || !part.chars().all(|character| character.is_ascii_digit()))
    {
        return None;
    }
    let minor = minor.parse::<u64>().ok()?;
    let next_minor = minor.checked_add(1)?;
    let suffix = if micro.is_empty() {
        String::new()
    } else {
        format!(".{}", micro.join("."))
    };
    Some(format!(">=3.{minor}{suffix},<3.{next_minor}"))
}

fn shebang_kind(line: &str) -> Option<&'static str> {
    let program = shebang_program(line)?;
    let normalized = program.to_ascii_lowercase();
    match normalized.as_str() {
        "python" | "python3" => Some("python"),
        "bash" | "sh" | "zsh" | "dash" | "ash" | "ksh" => Some("shell"),
        "fish" => Some("fish"),
        "node" | "deno" | "bun" => Some("js"),
        "pwsh" | "powershell" | "powershell.exe" => Some("powershell"),
        "ruby" => Some("ruby"),
        "perl" => Some("perl"),
        "lua" | "luajit" => Some("lua"),
        "rscript" => Some("r"),
        _ if python_version_pin(&normalized).is_some() => Some("python"),
        _ => None,
    }
}

fn basename(value: &str) -> &str {
    value.rsplit(['/', '\\']).next().unwrap_or(value)
}

/// Validate one PEP 508 package requirement.
pub fn validate_pep508_requirement(value: &str) -> Result<(), PythonMetadataError> {
    Requirement::<VerbatimUrl>::from_str(value)
        .map(|_| ())
        .map_err(|_| PythonMetadataError::InvalidRequirement {
            value: value.to_owned(),
        })
}

/// Validate one PEP 440 version-specifier list.
pub fn validate_pep440_specifiers(value: &str) -> Result<(), PythonMetadataError> {
    VersionSpecifiers::from_str(value).map(|_| ()).map_err(|_| {
        PythonMetadataError::InvalidVersionConstraint {
            value: value.to_owned(),
        }
    })
}

/// Read managed parameter declarations from the inline metadata block.
#[must_use]
pub fn managed_params(kind: &str, text: &str) -> Vec<ParamDecl> {
    let Some(leader) = metadata_leader(kind) else {
        return Vec::new();
    };
    let Some(table) = parse_inline_metadata(text, leader) else {
        return Vec::new();
    };
    table
        .get("tool")
        .and_then(TomlValue::as_table)
        .and_then(|tool| tool.get("skit"))
        .and_then(TomlValue::as_table)
        .and_then(|skit| skit.get("params"))
        .and_then(TomlValue::as_array)
        .into_iter()
        .flatten()
        .filter_map(TomlValue::as_table)
        .filter_map(|row| {
            let map = row
                .iter()
                .map(|(key, value)| (key.clone(), toml_to_json(value)))
                .collect::<BTreeMap<_, _>>();
            (!map
                .get("name")
                .and_then(JsonValue::as_str)
                .unwrap_or_default()
                .is_empty())
            .then(|| ParamDecl::from_block_map(&map))
        })
        .collect()
}

/// Write the `[tool.skit]` section in an inline metadata block.
/// Other source text stays in place.
pub fn write_managed_params(
    kind: &str,
    text: &str,
    params: &[ParamDecl],
) -> Result<String, LanguageError> {
    let Some(leader) = metadata_leader(kind) else {
        return Err(LanguageError::UnsupportedKind {
            kind: kind.to_owned(),
        });
    };
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let pattern = block_regex(leader);
    if let Some(captures) = pattern.captures(text) {
        let whole = captures.get(0).expect("block match has a full range");
        let body = captures.name("body").map_or("", |capture| capture.as_str());
        let kept = strip_skit_section(body, leader);
        let table =
            parse_inline_metadata(text, leader).ok_or_else(|| LanguageError::InvalidMetadata {
                reason: Message::new("the inline metadata block is not valid TOML"),
            })?;
        let managed = render_merged_skit_toml(&table, params)?;
        let mut block = format!("{leader} /// script{newline}");
        if !kept.is_empty() {
            block.push_str(&kept);
            if !kept.ends_with(newline) {
                block.push_str(newline);
            }
        }
        if let Some(managed) = managed {
            if !kept.is_empty() {
                block.push_str(leader);
                block.push_str(newline);
            }
            block.push_str(&commentify(&managed, leader, newline));
        }
        block.push_str(leader);
        block.push_str(" ///");
        if captures
            .name("close")
            .is_some_and(|close| close.as_str().ends_with(newline))
        {
            block.push_str(newline);
        }
        let mut output = String::with_capacity(text.len() + block.len());
        output.push_str(&text[..whole.start()]);
        output.push_str(&block);
        output.push_str(&text[whole.end()..]);
        return Ok(output);
    }

    if params.is_empty() {
        return Ok(text.to_owned());
    }
    let block = format!(
        "{leader} /// script{newline}{}{leader} ///{newline}",
        commentify(&render_managed_toml(params), leader, newline)
    );
    let insert_at = metadata_insert_at(kind, text);
    let mut output = String::with_capacity(text.len() + block.len());
    output.push_str(&text[..insert_at]);
    if insert_at == text.len() && insert_at > 0 && !text.ends_with(['\n', '\r']) {
        output.push_str(newline);
    }
    output.push_str(&block);
    output.push_str(&text[insert_at..]);
    Ok(output)
}

/// Read PEP 723 dependency fields without evaluating source code.
#[must_use]
pub fn read_uv_metadata(text: &str) -> Option<UvMetadata> {
    let table = parse_inline_metadata(text, "#")?;
    Some(UvMetadata {
        dependencies: table
            .get("dependencies")
            .and_then(TomlValue::as_array)
            .into_iter()
            .flatten()
            .filter_map(TomlValue::as_str)
            .map(str::to_owned)
            .collect(),
        requires_python: table
            .get("requires-python")
            .and_then(TomlValue::as_str)
            .unwrap_or_default()
            .to_owned(),
    })
}

/// Return whether Python source contains its own PEP 723 metadata fence.
///
/// This predicate deliberately does not parse the body. A caller that cannot decode the complete
/// file still needs to know that an existing block is authoritative and cannot be replaced by
/// metadata outside the source.
#[must_use]
pub fn has_uv_metadata_block(text: &str) -> bool {
    block_regex("#").is_match(text)
}

/// Replace PEP 723 dependency fields and preserve other inline metadata tables.
pub fn write_uv_metadata(
    text: &str,
    dependencies: &[String],
    requires_python: &str,
) -> Result<String, LanguageError> {
    let mut table = match block_regex("#").captures(text) {
        Some(captures) => {
            let body = captures.name("body").map_or("", |capture| capture.as_str());
            let stripped = body
                .lines()
                .map(|line| strip_comment_prefix(line, "#"))
                .collect::<Vec<_>>()
                .join("\n");
            toml::from_str::<toml::Table>(&stripped).map_err(|error| {
                LanguageError::InvalidMetadata {
                    reason: Message::new("the comment metadata block is not valid TOML: {}")
                        .with(error),
                }
            })?
        }
        None => toml::Table::new(),
    };
    table.insert(
        "dependencies".to_owned(),
        TomlValue::Array(
            dependencies
                .iter()
                .cloned()
                .map(TomlValue::String)
                .collect(),
        ),
    );
    if requires_python.is_empty() {
        table.remove("requires-python");
    } else {
        table.insert(
            "requires-python".to_owned(),
            TomlValue::String(requires_python.to_owned()),
        );
    }
    rewrite_inline_metadata(text, "#", &table)
}

fn rewrite_inline_metadata(
    text: &str,
    leader: &str,
    table: &toml::Table,
) -> Result<String, LanguageError> {
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let encoded = toml::to_string(table).expect("a TOML table always serializes");
    let block = format!(
        "{leader} /// script{newline}{}{leader} ///{newline}",
        commentify(&encoded, leader, newline)
    );
    let pattern = block_regex(leader);
    if let Some(found) = pattern.find(text) {
        let mut output = String::with_capacity(text.len() + block.len());
        output.push_str(&text[..found.start()]);
        output.push_str(&block);
        output.push_str(&text[found.end()..]);
        return Ok(output);
    }
    let insert_at = metadata_insert_at("python", text);
    let mut output = String::with_capacity(text.len() + block.len());
    output.push_str(&text[..insert_at]);
    if insert_at == text.len() && insert_at > 0 && !text.ends_with(['\n', '\r']) {
        output.push_str(newline);
    }
    output.push_str(&block);
    output.push_str(&text[insert_at..]);
    Ok(output)
}

fn metadata_insert_at(kind: &str, text: &str) -> usize {
    let bom_end = usize::from(text.starts_with('\u{feff}')) * '\u{feff}'.len_utf8();
    let first_end = physical_line_end(text, bom_end);
    let first = &text[bom_end..first_end];
    if kind == "python" {
        if python_coding_line(first) {
            return first_end;
        }
        if python_cookie_can_follow(first) && first_end < text.len() {
            let second_end = physical_line_end(text, first_end);
            if python_coding_line(&text[first_end..second_end]) {
                return second_end;
            }
        }
    }
    if first.starts_with("#!") {
        first_end
    } else {
        bom_end
    }
}

fn physical_line_end(text: &str, start: usize) -> usize {
    text[start..]
        .find('\n')
        .map_or(text.len(), |index| start + index + 1)
}

fn python_coding_line(line: &str) -> bool {
    let trimmed = line.trim_start_matches([' ', '\t', '\u{c}']);
    if !trimmed.starts_with('#') {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    lower.contains("coding:") || lower.contains("coding=")
}

fn python_cookie_can_follow(first_line: &str) -> bool {
    let trimmed = first_line.trim_matches([' ', '\t', '\u{c}', '\r', '\n']);
    trimmed.is_empty() || trimmed.starts_with('#')
}

/// Convert one shell constant to an environment-default expression.
pub fn normalize_shell_default(text: &str, name: &str) -> Result<String, LanguageError> {
    let ParseOutcome::Parsed(document) = parse_document("shell", text) else {
        return Err(LanguageError::InvalidSource {
            kind: "shell".to_owned(),
        });
    };
    let output = document.plan_shell_normalization(name)?.apply(text)?;
    match parse_document("shell", &output) {
        ParseOutcome::Parsed(_) => Ok(output),
        ParseOutcome::SyntaxError(_) | ParseOutcome::ParserUnavailable(_) => {
            Err(LanguageError::InvalidSource {
                kind: "shell".to_owned(),
            })
        }
    }
}

fn metadata_leader(kind: &str) -> Option<&'static str> {
    match kind {
        "python" | "shell" | "fish" => Some("#"),
        "js" | "ts" => Some("//"),
        _ => None,
    }
}

fn block_regex(leader: &str) -> Regex {
    Regex::new(&format!(
        r"(?m)^(?:{}) /// script[^\S\n]*\n(?P<body>(?:^(?:{})(?:| [^\r\n]*)\r?\n)*?)(?P<close>^(?:{}) ///[^\S\n]*(?:\n|$))",
        regex::escape(leader),
        regex::escape(leader),
        regex::escape(leader)
    ))
    .expect("escaped block pattern")
}

fn parse_inline_metadata(text: &str, leader: &str) -> Option<toml::Table> {
    let captures = block_regex(leader).captures(text)?;
    let body = captures.name("body")?.as_str();
    let stripped = body
        .lines()
        .map(|line| strip_comment_prefix(line, leader))
        .collect::<Vec<_>>()
        .join("\n");
    toml::from_str(&stripped).ok()
}

fn strip_comment_prefix<'a>(line: &'a str, leader: &str) -> &'a str {
    let line = line.strip_prefix(leader).unwrap_or(line);
    line.strip_prefix(' ').unwrap_or(line)
}

fn strip_skit_section(body: &str, leader: &str) -> String {
    let newline = if body.contains("\r\n") { "\r\n" } else { "\n" };
    let mut output = Vec::new();
    let mut skipping = false;
    for line in body.lines() {
        let stripped = strip_comment_prefix(line, leader).trim();
        if stripped.starts_with('[') {
            // `render_merged_skit_toml` re-emits everything under `tool.skit`, so every
            // shape of that table must leave here. Keeping one would declare it twice.
            skipping = stripped.starts_with("[tool.skit]")
                || stripped.starts_with("[tool.skit.")
                || stripped.starts_with("[[tool.skit.");
        }
        if !skipping {
            output.push(line.trim_end_matches('\r'));
        }
    }
    while output.last().is_some_and(|line| {
        let value = line.trim();
        value.is_empty() || value == leader
    }) {
        output.pop();
    }
    output.join(newline)
}

fn render_managed_toml(params: &[ParamDecl]) -> String {
    let mut lines = vec!["[tool.skit]".to_owned(), "schema = 1".to_owned()];
    for parameter in params {
        lines.push(String::new());
        lines.push("[[tool.skit.params]]".to_owned());
        for (key, value) in parameter.to_block_values() {
            lines.push(format!("{key} = {}", parameter_toml_literal(&value)));
        }
    }
    lines.join("\n") + "\n"
}

fn render_merged_skit_toml(
    metadata: &toml::Table,
    params: &[ParamDecl],
) -> Result<Option<String>, LanguageError> {
    let existing_tool = metadata.get("tool");
    let existing_skit = match existing_tool {
        None => toml::Table::new(),
        Some(TomlValue::Table(tool)) => match tool.get("skit") {
            None => toml::Table::new(),
            Some(TomlValue::Table(skit)) => skit.clone(),
            Some(_) => {
                return Err(LanguageError::InvalidMetadata {
                    reason: Message::new("tool.skit is not a table"),
                });
            }
        },
        Some(_) => {
            return Err(LanguageError::InvalidMetadata {
                reason: Message::new("tool is not a table"),
            });
        }
    };
    let mut skit = existing_skit;
    let existing_rows = skit
        .remove("params")
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default();
    let mut rows_by_name = BTreeMap::<String, toml::Table>::new();
    let mut anonymous_rows = Vec::new();
    for value in existing_rows {
        let TomlValue::Table(row) = value else {
            anonymous_rows.push(value);
            continue;
        };
        if let Some(name) = row.get("name").and_then(TomlValue::as_str) {
            rows_by_name.insert(name.to_owned(), row);
        } else {
            anonymous_rows.push(TomlValue::Table(row));
        }
    }

    if params.is_empty() {
        skit.remove("schema");
    } else {
        skit.insert("schema".to_owned(), TomlValue::Integer(1));
        let mut rows = params
            .iter()
            .map(|parameter| {
                let mut row = rows_by_name.remove(&parameter.name).unwrap_or_default();
                for key in [
                    "name",
                    "kind",
                    "type",
                    "default",
                    "prompt",
                    "order",
                    "secret",
                    "env_source",
                ] {
                    row.remove(key);
                }
                for (key, value) in parameter.to_block_values() {
                    row.insert(key, parameter_value_to_toml(value));
                }
                TomlValue::Table(row)
            })
            .collect::<Vec<_>>();
        rows.extend(anonymous_rows);
        skit.insert("params".to_owned(), TomlValue::Array(rows));
    }
    if skit.is_empty() {
        return Ok(None);
    }
    let mut tool = toml::Table::new();
    tool.insert("skit".to_owned(), TomlValue::Table(skit));
    let mut root = toml::Table::new();
    root.insert("tool".to_owned(), TomlValue::Table(tool));
    // Every value came from a TOML document or from a validated declaration.
    Ok(Some(
        toml::to_string(&root).expect("skit metadata holds only TOML values"),
    ))
}

fn parameter_value_to_toml(value: ParameterValue) -> TomlValue {
    match value {
        ParameterValue::String(value) => TomlValue::String(value),
        ParameterValue::Integer(value) => TomlValue::Integer(value),
        ParameterValue::Float(value) => TomlValue::Float(value),
        ParameterValue::Bool(value) => TomlValue::Boolean(value),
    }
}

fn parameter_toml_literal(value: &ParameterValue) -> String {
    match value {
        ParameterValue::String(value) => toml_string(value),
        ParameterValue::Integer(value) => value.to_string(),
        ParameterValue::Float(value) => {
            let literal = value.to_string();
            if literal.contains(['.', 'e', 'E']) {
                literal
            } else {
                format!("{literal}.0")
            }
        }
        ParameterValue::Bool(value) => value.to_string(),
    }
}

fn toml_string(value: &str) -> String {
    let mut output = String::from("\"");
    for character in value.chars() {
        match character {
            '\\' => output.push_str("\\\\"),
            '"' => output.push_str("\\\""),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            character
                if character < ' '
                    || matches!(character, '\u{7f}' | '\u{85}' | '\u{2028}' | '\u{2029}') =>
            {
                output.push_str(&format!("\\u{:04X}", u32::from(character)));
            }
            character => output.push(character),
        }
    }
    output.push('"');
    output
}

fn commentify(text: &str, leader: &str, newline: &str) -> String {
    text.lines()
        .map(|line| {
            if line.is_empty() {
                leader.to_owned()
            } else {
                format!("{leader} {line}")
            }
        })
        .collect::<Vec<_>>()
        .join(newline)
        + newline
}

fn toml_to_json(value: &TomlValue) -> JsonValue {
    match value {
        TomlValue::String(value) => JsonValue::String(value.clone()),
        TomlValue::Integer(value) => JsonValue::from(*value),
        TomlValue::Float(value) => serde_json::Number::from_f64(*value)
            .map(JsonValue::Number)
            .unwrap_or(JsonValue::Null),
        TomlValue::Boolean(value) => JsonValue::Bool(*value),
        TomlValue::Datetime(value) => JsonValue::String(value.to_string()),
        TomlValue::Array(values) => JsonValue::Array(values.iter().map(toml_to_json).collect()),
        TomlValue::Table(values) => JsonValue::Object(
            values
                .iter()
                .map(|(key, value)| (key.clone(), toml_to_json(value)))
                .collect(),
        ),
    }
}

/// Read a static CLI declaration for a supported language.
#[must_use]
pub fn cli_params(kind: &str, text: &str) -> Vec<ParamDecl> {
    let ParseOutcome::Parsed(document) = parse_document(kind, text) else {
        return Vec::new();
    };
    match document.cli_surface() {
        CliSurface::Static(surface) => surface
            .fields
            .into_iter()
            .map(|field| field.declaration)
            .collect(),
        CliSurface::Absent | CliSurface::Dynamic(_) => Vec::new(),
    }
}

/// Detect source-bound values that skit can manage.
#[must_use]
pub fn detect_candidates(kind: &str, text: &str) -> Vec<ParamDecl> {
    let ParseOutcome::Parsed(document) = parse_document(kind, text) else {
        return Vec::new();
    };
    document
        .analysis()
        .candidates
        .into_iter()
        .map(|candidate| candidate.declaration)
        .collect()
}

/// Build placeholder fields for command and prompt entries.
#[must_use]
pub fn placeholder_params(kind: &str, text: &str) -> Vec<ParamDecl> {
    let names = match kind {
        "command" => scan_placeholders(text, false),
        "prompt" => scan_placeholders(text, true),
        _ => Vec::new(),
    };
    names
        .into_iter()
        .map(|name| synthesized_placeholder(&name))
        .collect()
}

/// Replace managed prompt placeholders in one pass over the original text.
#[must_use]
pub fn render_prompt_body(
    text: &str,
    values: &BTreeMap<String, String>,
    interpolate: bool,
) -> String {
    if !interpolate {
        return text.to_owned();
    }

    let bytes = text.as_bytes();
    let mut output = String::with_capacity(text.len());
    let mut copied_until = 0;
    let mut index = 0;
    while index < bytes.len() {
        if !bytes[index..].starts_with(b"{{") {
            index = index.saturating_add(1);
            continue;
        }
        let start = index + 2;
        let Some(relative_end) = bytes[start..].windows(2).position(|window| window == b"}}")
        else {
            break;
        };
        let end = start + relative_end;
        let name = &text[start..end];
        if valid_identifier(name)
            && let Some(value) = values.get(name)
        {
            output.push_str(&text[copied_until..index]);
            output.push_str(value);
            copied_until = end + 2;
        }
        index = end.saturating_add(2);
    }
    output.push_str(&text[copied_until..]);
    output
}

fn scan_placeholders(text: &str, doubled: bool) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut output = Vec::new();
    let mut seen = BTreeSet::new();
    let mut index = 0;
    while index < bytes.len() {
        let open: &[u8] = if doubled { b"{{" } else { b"{" };
        let close: &[u8] = if doubled { b"}}" } else { b"}" };
        if !bytes[index..].starts_with(open) {
            index = index.saturating_add(1);
            continue;
        }
        if !doubled && bytes[index..].starts_with(b"{{") {
            index = index.saturating_add(2);
            continue;
        }
        let start = index + open.len();
        let Some(relative_end) = bytes[start..]
            .windows(close.len())
            .position(|window| window == close)
        else {
            break;
        };
        let end = start + relative_end;
        let name = &text[start..end];
        if valid_identifier(name) && seen.insert(name.to_owned()) {
            output.push(name.to_owned());
        }
        index = end.saturating_add(close.len());
    }
    output
}

fn valid_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && chars.all(|character| character.is_ascii_alphanumeric() || character == '_')
}

/// Rewrite source bindings with accepted values.
pub fn inject_values(
    kind: &str,
    text: &str,
    declarations: &[ParamDecl],
    values: &BTreeMap<String, String>,
) -> Result<String, LanguageError> {
    inject_values_for_interpreter(kind, text, declarations, values, None)
}

/// Rewrite source bindings with accepted values and a resolved shell interpreter.
pub fn inject_values_for_interpreter(
    kind: &str,
    text: &str,
    declarations: &[ParamDecl],
    values: &BTreeMap<String, String>,
    interpreter: Option<&str>,
) -> Result<String, LanguageError> {
    let document = match parse_document(kind, text) {
        ParseOutcome::Parsed(document) => document,
        ParseOutcome::SyntaxError(_) => {
            return Err(LanguageError::InvalidSource {
                kind: kind.to_owned(),
            });
        }
        ParseOutcome::ParserUnavailable(_) => {
            return Err(LanguageError::UnsupportedKind {
                kind: kind.to_owned(),
            });
        }
    };
    let output = document
        .plan_injection_for_interpreter(declarations, values, interpreter)?
        .apply(text)?;
    match parse_document(kind, &output) {
        ParseOutcome::Parsed(_) => Ok(output),
        ParseOutcome::SyntaxError(_) | ParseOutcome::ParserUnavailable(_) => {
            Err(LanguageError::InvalidSource {
                kind: kind.to_owned(),
            })
        }
    }
}

fn apply_source_edits(
    text: &str,
    mut edits: Vec<(usize, usize, String)>,
) -> Result<String, LanguageError> {
    edits.sort_by_key(|(start, end, _)| (*start, *end));
    if edits
        .iter()
        .any(|(start, end, _)| start > end || *end > text.len())
        || edits.windows(2).any(|pair| pair[0].1 > pair[1].0)
    {
        return Err(LanguageError::InvalidSource {
            kind: "overlapping source edits".to_owned(),
        });
    }
    let mut output = text.to_owned();
    for (start, end, replacement) in edits.into_iter().rev() {
        output.replace_range(start..end, &replacement);
    }
    Ok(output)
}

/// Return declared package dependencies found in source text.
#[must_use]
pub fn external_dependencies(kind: &str, text: &str) -> Vec<String> {
    external_dependencies_at(kind, text, None)
}

/// Return declared package dependencies and exclude local Python modules.
///
/// `script_dir` is the directory that contains the original script. Python
/// imports from this directory do not name package dependencies.
#[must_use]
pub fn external_dependencies_at(kind: &str, text: &str, script_dir: Option<&Path>) -> Vec<String> {
    // One document serves the gate and the scan, so a refused source never reaches an analyzer.
    let ParseOutcome::Parsed(document) = parse_document(kind, text) else {
        return Vec::new();
    };
    match kind {
        "python" => {
            let inline = parse_inline_metadata(text, "#")
                .and_then(|table| table.get("dependencies").cloned())
                .and_then(|value| value.as_array().cloned())
                .into_iter()
                .flatten()
                .filter_map(|value| value.as_str().map(str::to_owned))
                .collect::<Vec<_>>();
            if inline.is_empty() {
                python_dependencies(document.syntax_tree(), text, script_dir)
            } else {
                inline
            }
        }
        "js" | "ts" => javascript_dependencies(document.syntax_tree(), text),
        _ => Vec::new(),
    }
}

/// Report whether a parser accepts the complete source text.
///
/// Kinds without a parser-backed analyzer accept all source text. Parser-backed
/// analyzer, injector, and dependency features use this same gate.
#[must_use]
pub fn source_is_valid(kind: &str, text: &str) -> bool {
    !matches!(parse_document(kind, text), ParseOutcome::SyntaxError(_))
}

fn node_text<'text>(node: tree_sitter::Node<'_>, text: &'text str) -> Option<&'text str> {
    text.get(node.byte_range())
}

fn first_named_child(node: tree_sitter::Node<'_>) -> Option<tree_sitter::Node<'_>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).next()
}

fn walk_tree<'tree>(
    node: tree_sitter::Node<'tree>,
    visit: &mut impl FnMut(tree_sitter::Node<'tree>),
) {
    visit(node);
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        walk_tree(child, visit);
    }
}

fn python_dependencies(
    tree: &tree_sitter::Tree,
    text: &str,
    script_dir: Option<&Path>,
) -> Vec<String> {
    let mut output = BTreeSet::new();
    walk_tree(tree.root_node(), &mut |node| match node.kind() {
        "import_statement" => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                let name_node = if child.kind() == "aliased_import" {
                    first_named_child(child)
                } else {
                    Some(child)
                };
                if let Some(name) = name_node.and_then(|item| node_text(item, text)) {
                    add_python_dependency(&mut output, name, script_dir);
                }
            }
        }
        "import_from_statement" => {
            if let Some(module) = node
                .child_by_field_name("module_name")
                .and_then(|item| node_text(item, text))
                && !module.starts_with('.')
            {
                add_python_dependency(&mut output, module, script_dir);
            }
        }
        _ => {}
    });
    output.into_iter().collect()
}

fn add_python_dependency(output: &mut BTreeSet<String>, import: &str, script_dir: Option<&Path>) {
    let name = import.split('.').next().unwrap_or_default();
    if name.is_empty()
        || name.starts_with('_')
        || PYTHON_STDLIB.contains(&name)
        || is_local_python_module(script_dir, name)
    {
        return;
    }
    let package = python_package_name(name);
    if validate_pep508_requirement(package).is_ok() {
        output.insert(package.to_owned());
    }
}

fn is_local_python_module(script_dir: Option<&Path>, module: &str) -> bool {
    let Some(directory) = script_dir else {
        return false;
    };
    if directory.join(format!("{module}.py")).is_file() {
        return true;
    }
    directory.join(module).read_dir().is_ok_and(|entries| {
        entries.filter_map(Result::ok).any(|entry| {
            entry.path().is_file()
                && entry.path().extension().and_then(|value| value.to_str()) == Some("py")
        })
    })
}

fn python_package_name(import: &str) -> &str {
    match import {
        "PIL" => "Pillow",
        "cv2" => "opencv-python",
        "yaml" => "PyYAML",
        "bs4" => "beautifulsoup4",
        "sklearn" => "scikit-learn",
        "skimage" => "scikit-image",
        "dotenv" => "python-dotenv",
        "dateutil" => "python-dateutil",
        "serial" => "pyserial",
        "jwt" => "PyJWT",
        "docx" => "python-docx",
        "pptx" => "python-pptx",
        "fitz" => "PyMuPDF",
        "OpenSSL" => "pyOpenSSL",
        "Crypto" => "pycryptodome",
        "Cryptodome" => "pycryptodomex",
        "git" => "GitPython",
        "attr" => "attrs",
        "slugify" => "python-slugify",
        "usb" => "pyusb",
        "win32com" | "win32api" => "pywin32",
        _ => import,
    }
}

const PYTHON_STDLIB: &[&str] = &[
    "__future__",
    "_thread",
    "abc",
    "argparse",
    "array",
    "ast",
    "asyncio",
    "base64",
    "binascii",
    "bisect",
    "builtins",
    "bz2",
    "calendar",
    "cmath",
    "cmd",
    "code",
    "codecs",
    "collections",
    "colorsys",
    "compileall",
    "concurrent",
    "configparser",
    "contextlib",
    "contextvars",
    "copy",
    "csv",
    "ctypes",
    "curses",
    "dataclasses",
    "datetime",
    "decimal",
    "difflib",
    "dis",
    "doctest",
    "email",
    "encodings",
    "enum",
    "errno",
    "faulthandler",
    "fcntl",
    "filecmp",
    "fileinput",
    "fnmatch",
    "fractions",
    "ftplib",
    "functools",
    "gc",
    "getopt",
    "getpass",
    "gettext",
    "glob",
    "graphlib",
    "grp",
    "gzip",
    "hashlib",
    "heapq",
    "hmac",
    "html",
    "http",
    "imaplib",
    "importlib",
    "inspect",
    "io",
    "ipaddress",
    "itertools",
    "json",
    "keyword",
    "linecache",
    "locale",
    "logging",
    "lzma",
    "mailbox",
    "math",
    "mimetypes",
    "mmap",
    "multiprocessing",
    "netrc",
    "numbers",
    "operator",
    "os",
    "pathlib",
    "pdb",
    "pickle",
    "pickletools",
    "pkgutil",
    "platform",
    "plistlib",
    "pprint",
    "profile",
    "pstats",
    "pty",
    "pwd",
    "py_compile",
    "pyclbr",
    "queue",
    "quopri",
    "random",
    "re",
    "readline",
    "reprlib",
    "resource",
    "rlcompleter",
    "runpy",
    "sched",
    "secrets",
    "select",
    "selectors",
    "shelve",
    "shlex",
    "shutil",
    "signal",
    "site",
    "smtplib",
    "socket",
    "socketserver",
    "sqlite3",
    "ssl",
    "stat",
    "statistics",
    "string",
    "stringprep",
    "struct",
    "subprocess",
    "sys",
    "sysconfig",
    "tarfile",
    "tempfile",
    "textwrap",
    "threading",
    "time",
    "timeit",
    "tkinter",
    "token",
    "tokenize",
    "tomllib",
    "trace",
    "traceback",
    "tracemalloc",
    "tty",
    "turtle",
    "types",
    "typing",
    "unicodedata",
    "unittest",
    "urllib",
    "uuid",
    "venv",
    "warnings",
    "wave",
    "weakref",
    "webbrowser",
    "xml",
    "xmlrpc",
    "zipapp",
    "zipfile",
    "zipimport",
    "zlib",
    "zoneinfo",
];

fn javascript_dependencies(tree: &tree_sitter::Tree, text: &str) -> Vec<String> {
    let mut output = BTreeSet::new();
    walk_tree(tree.root_node(), &mut |node| {
        if let Some(specifier) = javascript_import_source(node, text)
            && let Some(package) = package_name(&specifier)
        {
            output.insert(package);
        }
    });
    output.into_iter().collect()
}

fn javascript_import_source(node: tree_sitter::Node<'_>, text: &str) -> Option<String> {
    if matches!(node.kind(), "import_statement" | "export_statement") {
        return node
            .child_by_field_name("source")
            .and_then(|source| javascript_string_value(source, text));
    }
    if node.kind() != "call_expression" {
        return None;
    }
    let function = node.child_by_field_name("function")?;
    if !matches!(node_text(function, text), Some("require" | "import")) {
        return None;
    }
    let arguments = node.child_by_field_name("arguments")?;
    if arguments.named_child_count() != 1 {
        return None;
    }
    javascript_string_value(arguments.named_child(0)?, text)
}

fn javascript_string_value(node: tree_sitter::Node<'_>, text: &str) -> Option<String> {
    if node.kind() != "string" {
        return None;
    }
    let mut output = String::new();
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        output.push_str(node_text(child, text)?);
    }
    Some(output)
}

fn package_name(specifier: &str) -> Option<String> {
    if specifier.is_empty()
        || specifier.starts_with('.')
        || specifier.starts_with('/')
        || specifier.starts_with('#')
        || NON_PACKAGE_PREFIXES
            .iter()
            .any(|prefix| specifier.starts_with(prefix))
    {
        return None;
    }
    let mut parts = specifier.split('/');
    let first = parts.next()?;
    let package = if specifier.starts_with('@') {
        let name = parts.next()?;
        if first.len() < 2 || name.is_empty() {
            return None;
        }
        format!("{first}/{name}")
    } else {
        first.to_owned()
    };
    if NODE_BUILTINS.contains(&package.as_str()) {
        return None;
    }
    Some(package)
}

const NON_PACKAGE_PREFIXES: &[&str] = &[
    "node:", "npm:", "jsr:", "http:", "https:", "data:", "file:", "bun:",
];

const NODE_BUILTINS: &[&str] = &[
    "assert",
    "async_hooks",
    "buffer",
    "child_process",
    "cluster",
    "console",
    "constants",
    "crypto",
    "dgram",
    "diagnostics_channel",
    "dns",
    "domain",
    "events",
    "fs",
    "http",
    "http2",
    "https",
    "inspector",
    "module",
    "net",
    "os",
    "path",
    "perf_hooks",
    "process",
    "punycode",
    "querystring",
    "readline",
    "repl",
    "stream",
    "string_decoder",
    "sys",
    "timers",
    "tls",
    "trace_events",
    "tty",
    "url",
    "util",
    "v8",
    "vm",
    "wasi",
    "worker_threads",
    "zlib",
];

#[cfg(test)]
mod private_tests {
    use super::*;

    #[test]
    fn declaration_values_map_every_scalar_shape_to_toml() {
        assert_eq!(
            parameter_value_to_toml(ParameterValue::String("text".to_owned())),
            TomlValue::String("text".into())
        );
        assert_eq!(
            parameter_value_to_toml(ParameterValue::Integer(7)),
            TomlValue::Integer(7)
        );
        assert_eq!(
            parameter_value_to_toml(ParameterValue::Float(0.5)),
            TomlValue::Float(0.5)
        );
        assert_eq!(
            parameter_value_to_toml(ParameterValue::Bool(true)),
            TomlValue::Boolean(true)
        );
        assert_eq!(parameter_toml_literal(&ParameterValue::Float(0.0)), "0.0");
        assert_eq!(parameter_toml_literal(&ParameterValue::Float(1.5)), "1.5");
    }

    #[test]
    fn toml_strings_escape_carriage_returns() {
        assert_eq!(toml_string("a\rb"), "\"a\\rb\"");
    }

    #[test]
    fn internal_guards_reject_unsupported_parsers_and_invalid_edits() {
        assert!(matches!(
            parse_document("ruby", "puts 1"),
            ParseOutcome::ParserUnavailable(_)
        ));
        assert!(matches!(
            apply_source_edits("abc", vec![(2, 1, "x".to_owned())]),
            Err(LanguageError::InvalidSource { .. })
        ));
        assert!(matches!(
            apply_source_edits("abc", vec![(0, 2, "x".to_owned()), (1, 3, "y".to_owned())]),
            Err(LanguageError::InvalidSource { .. })
        ));
    }

    #[test]
    fn source_helpers_preserve_placeholder_literal_style() {
        assert_eq!(
            scan_placeholders("{{escaped}} {open", false),
            Vec::<String>::new()
        );
    }
}
