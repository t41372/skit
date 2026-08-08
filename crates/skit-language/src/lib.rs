//! Analyze and rewrite supported source files.
//!
//! The analyzers read static source text. They do not execute user code.
//! When a dynamic declaration is not clear, the analyzer returns no field.

#![forbid(unsafe_code)]

use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    str::FromStr,
    sync::LazyLock,
};

use pep440_rs::VersionSpecifiers;
use pep508_rs::{Requirement, VerbatimUrl};
use regex::{Captures, Regex};
use serde_json::Value as JsonValue;
use skit_domain::parameters::{
    ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, ParameterValue, is_secret_name,
    synthesized_placeholder,
};
use skit_i18n::{Localize, Message};
use thiserror::Error;
use toml::Value as TomlValue;

static PYTHON_TYPER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?m)([A-Za-z_][A-Za-z0-9_]*)\s*:\s*(str|int|float|bool)\s*=\s*typer\.(Option|Argument)\s*\(",
    )
    .expect("fixed Typer pattern")
});
static JS_OPTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?ms)([A-Za-z_$][A-Za-z0-9_$]*)\s*:\s*\{([^{}]*?type\s*:\s*['"](string|boolean)['"][^{}]*?)\}"#,
    )
    .expect("fixed parseArgs pattern")
});
static POWERSHELL_PARAM: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?is)(?:\[Parameter\(([^\]]*)\)\]\s*)?\[(string|int|double|float|bool|switch)\]\s*\$([A-Za-z_][A-Za-z0-9_]*)(?:\s*=\s*([^,\r\n\)]+))?",
    )
    .expect("fixed PowerShell parameter pattern")
});
static SHELL_DEFAULT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\$\{([A-Za-z_][A-Za-z0-9_]*)(:-|:=|-|=)([^}]*)\}")
        .expect("fixed shell environment-default pattern")
});
static JS_CONST: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^\s*(?:const|let|var)\s+([A-Za-z_$][A-Za-z0-9_$]*)(?:\s*:\s*[^=]+)?\s*=\s*([^;\r\n]+)\s*;?\s*$",
    )
        .expect("fixed JavaScript constant pattern")
});
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
}

/// Report an invalid Python package or version constraint.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum PythonMetadataError {
    /// A package requirement does not use the PEP 508 grammar.
    #[error("invalid PEP 508 requirement {value:?}: {reason}")]
    InvalidRequirement { value: String, reason: String },
    /// A Python version constraint does not use the PEP 440 grammar.
    #[error("invalid PEP 440 version constraint {value:?}: {reason}")]
    InvalidVersionConstraint { value: String, reason: String },
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
        }
    }
}

impl Localize for PythonMetadataError {
    fn message(&self) -> Message {
        match self {
            Self::InvalidRequirement { value, reason } => {
                Message::new("invalid PEP 508 requirement {}: {}")
                    .quoted(value)
                    .with(reason)
            }
            Self::InvalidVersionConstraint { value, reason } => {
                Message::new("invalid PEP 440 version constraint {}: {}")
                    .quoted(value)
                    .with(reason)
            }
        }
    }
}

/// Effective PEP 723 fields used by Python copy entries.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
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
        .map_err(|error| PythonMetadataError::InvalidRequirement {
            value: value.to_owned(),
            reason: error.to_string(),
        })
}

/// Validate one PEP 440 version-specifier list.
pub fn validate_pep440_specifiers(value: &str) -> Result<(), PythonMetadataError> {
    VersionSpecifiers::from_str(value)
        .map(|_| ())
        .map_err(|error| PythonMetadataError::InvalidVersionConstraint {
            value: value.to_owned(),
            reason: error.to_string(),
        })
}

/// Read managed parameter declarations from the inline metadata block.
#[must_use]
pub fn managed_params(kind: &str, text: &str) -> Vec<ParamDecl> {
    if !parser_accepts(kind, text) {
        return Vec::new();
    }
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
    let insert_at = if text.starts_with("#!") {
        text.find('\n').map_or(text.len(), |index| index + 1)
    } else {
        0
    };
    let mut output = String::with_capacity(text.len() + block.len());
    output.push_str(&text[..insert_at]);
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
    let insert_at = if text.starts_with("#!") {
        text.find('\n').map_or(text.len(), |index| index + 1)
    } else {
        0
    };
    let mut output = String::with_capacity(text.len() + block.len());
    output.push_str(&text[..insert_at]);
    if insert_at == text.len() && text.starts_with("#!") && !text.ends_with(['\n', '\r']) {
        output.push_str(newline);
    }
    output.push_str(&block);
    output.push_str(&text[insert_at..]);
    Ok(output)
}

/// Convert one shell constant to an environment-default expression.
pub fn normalize_shell_default(text: &str, name: &str) -> Result<String, LanguageError> {
    require_valid_source("shell", text)?;
    let name_pattern = regex::escape(name);
    let pattern = Regex::new(&format!(
        r"(?m)^([ \t]*{name_pattern}[ \t]*=[ \t]*)([^#$\r\n][^#\r\n]*?)([ \t]*(?:#.*)?)(\r?)$"
    ))
    .expect("an escaped parameter name is a valid regular expression");
    replace_first(text, &pattern, |captures| {
        let prefix = captures.get(1).map_or("", |item| item.as_str());
        let value = captures.get(2).map_or("", |item| item.as_str()).trim_end();
        let suffix = captures.get(3).map_or("", |item| item.as_str());
        let carriage_return = captures.get(4).map_or("", |item| item.as_str());
        format!("{prefix}${{{name}:-{value}}}{suffix}{carriage_return}")
    })
    .ok_or_else(|| LanguageError::BindingNotFound {
        name: name.to_owned(),
    })
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
    if !parser_accepts(kind, text) {
        return Vec::new();
    }
    match kind {
        "python" => python_cli_params(text),
        "shell" => shell_cli_params(text),
        "js" | "ts" => javascript_cli_params(text),
        "fish" => fish_cli_params(text),
        "powershell" => powershell_cli_params(text),
        _ => Vec::new(),
    }
}

fn python_cli_params(text: &str) -> Vec<ParamDecl> {
    let mut output = Vec::new();
    for body in call_bodies(text, "add_argument") {
        if let Some(parameter) = python_option_from_call(&body, PythonCallKind::Argparse, None) {
            push_unique(&mut output, parameter);
        }
    }
    for body in call_bodies(text, "@click.option") {
        if let Some(parameter) = python_option_from_call(&body, PythonCallKind::Click, None) {
            push_unique(&mut output, parameter);
        }
    }
    for captures in PYTHON_TYPER.captures_iter(text) {
        let full = captures
            .get(0)
            .expect("a regular expression match is complete");
        let open_offset = full
            .as_str()
            .rfind('(')
            .expect("the Typer pattern ends with an opening parenthesis");
        let open = full.start() + open_offset;
        let Some(body) = parenthesized_body(text, open) else {
            continue;
        };
        let name = captures.get(1).map_or("", |value| value.as_str());
        let type_name = captures.get(2).map_or("str", |value| value.as_str());
        let call = captures.get(3).map_or("Option", |value| value.as_str());
        if let Some(parameter) = python_option_from_call(
            body,
            if call == "Argument" {
                PythonCallKind::TyperArgument
            } else {
                PythonCallKind::TyperOption
            },
            Some((name, type_name)),
        ) {
            push_unique(&mut output, parameter);
        }
    }
    output
}

#[derive(Clone, Copy)]
enum PythonCallKind {
    Argparse,
    Click,
    TyperOption,
    TyperArgument,
}

fn python_option_from_call(
    body: &str,
    kind: PythonCallKind,
    typer: Option<(&str, &str)>,
) -> Option<ParamDecl> {
    let parts = split_top_level(body, ',');
    let mut positional = Vec::new();
    let mut keywords = BTreeMap::new();
    for part in parts {
        let part = part.trim();
        if let Some((key, value)) = split_keyword(part) {
            keywords.insert(key.to_owned(), value.trim().to_owned());
        } else if !part.is_empty() {
            positional.push(part.to_owned());
        }
    }

    let (name, flag) = match kind {
        PythonCallKind::TyperArgument => (typer?.0.to_owned(), String::new()),
        PythonCallKind::TyperOption => {
            let name = typer?.0.to_owned();
            let flag = positional
                .iter()
                .skip(1)
                .filter_map(|value| parse_quoted(value))
                .find(|value| value.starts_with('-'))
                .unwrap_or_else(|| format!("--{}", name.replace('_', "-")));
            (name, flag)
        }
        PythonCallKind::Argparse | PythonCallKind::Click => {
            let flags = positional
                .iter()
                .filter_map(|value| parse_quoted(value))
                .collect::<Vec<_>>();
            let preferred = flags
                .iter()
                .find(|value| value.starts_with("--"))
                .or_else(|| flags.first())?;
            if preferred.starts_with('-') {
                (flag_name(preferred), preferred.clone())
            } else {
                (preferred.clone(), String::new())
            }
        }
    };

    let mut declaration = ParamDecl::new(name);
    declaration.delivery = ParameterDelivery::Flag;
    declaration.flag = flag;
    if let Some((_, type_name)) = typer {
        declaration.parameter_type = parameter_type(type_name);
    }
    if let Some(value) = keywords.get("type") {
        declaration.parameter_type = parameter_type(value.trim());
    }
    if let Some(value) = keywords.get("required") {
        declaration.required = truthy_source(value);
    }
    if let Some(value) = keywords.get("help") {
        declaration.help = parse_quoted(value).unwrap_or_default();
    }
    if let Some(value) = keywords.get("choices") {
        declaration.choices = parse_list_strings(value);
        if !declaration.choices.is_empty() {
            declaration.parameter_type = ParameterType::Choice;
        }
    }
    if let Some(value) = keywords.get("action").and_then(|value| parse_quoted(value))
        && matches!(value.as_str(), "store_true" | "store_false")
    {
        declaration.parameter_type = ParameterType::Bool;
        declaration.action = value;
    }
    let default_source = match kind {
        PythonCallKind::TyperOption | PythonCallKind::TyperArgument => positional.first(),
        PythonCallKind::Argparse | PythonCallKind::Click => keywords.get("default"),
    };
    if let Some(value) = default_source
        && let Some(default) = parse_parameter_value(value, declaration.parameter_type)
    {
        declaration.default = Some(default);
    }
    Some(declaration)
}

fn shell_cli_params(text: &str) -> Vec<ParamDecl> {
    let pattern = Regex::new(r#"getopts\s+['"]([^'"]+)['"]"#).expect("fixed getopts pattern");
    let Some(spec) = pattern
        .captures(text)
        .and_then(|captures| captures.get(1))
        .map(|value| value.as_str())
    else {
        return Vec::new();
    };
    let chars = spec.chars().collect::<Vec<_>>();
    let mut output = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        let character = chars[index];
        if character == ':' {
            index = index.saturating_add(1);
            continue;
        }
        let takes_value = chars.get(index + 1) == Some(&':');
        let mut declaration = ParamDecl::new(character.to_string());
        declaration.delivery = ParameterDelivery::Flag;
        declaration.flag = format!("-{character}");
        if !takes_value {
            declaration.parameter_type = ParameterType::Bool;
            declaration.action = "store_true".to_owned();
        }
        output.push(declaration);
        index = index
            .saturating_add(usize::from(takes_value))
            .saturating_add(1);
    }
    output
}

fn javascript_cli_params(text: &str) -> Vec<ParamDecl> {
    JS_OPTION
        .captures_iter(text)
        .filter_map(|captures| {
            let name = captures.get(1)?.as_str();
            let type_name = captures.get(3)?.as_str();
            let mut declaration = ParamDecl::new(name);
            declaration.delivery = ParameterDelivery::Flag;
            declaration.flag = format!("--{}", name.replace('_', "-"));
            if type_name == "boolean" {
                declaration.parameter_type = ParameterType::Bool;
                declaration.action = "store_true".to_owned();
            }
            Some(declaration)
        })
        .collect()
}

fn fish_cli_params(text: &str) -> Vec<ParamDecl> {
    let quoted = Regex::new(r#"['"]([^'"]+)['"]"#).expect("fixed fish argparse pattern");
    let mut output = Vec::new();
    for line in text.lines().filter(|line| line.contains("argparse")) {
        for captures in quoted.captures_iter(line) {
            let raw = captures
                .get(1)
                .expect("the fish option pattern captures its value")
                .as_str();
            let takes_value = raw.ends_with('=') || raw.ends_with("=+");
            let raw = raw.trim_end_matches(&['=', '+'][..]);
            let parts = raw.split('/').collect::<Vec<_>>();
            let long = parts.last().copied().unwrap_or(raw);
            let mut declaration = ParamDecl::new(long.replace('-', "_"));
            declaration.delivery = ParameterDelivery::Flag;
            declaration.flag = format!("--{long}");
            if !takes_value {
                declaration.parameter_type = ParameterType::Bool;
                declaration.action = "store_true".to_owned();
            }
            output.push(declaration);
        }
    }
    output
}

fn powershell_cli_params(text: &str) -> Vec<ParamDecl> {
    POWERSHELL_PARAM
        .captures_iter(text)
        .filter_map(|captures| {
            let attributes = captures.get(1).map_or("", |value| value.as_str());
            let type_name = captures.get(2)?.as_str();
            let name = captures.get(3)?.as_str();
            let mut declaration = ParamDecl::new(name);
            declaration.delivery = ParameterDelivery::Flag;
            declaration.flag = format!("-{name}");
            declaration.parameter_type = match type_name.to_ascii_lowercase().as_str() {
                "int" => ParameterType::Int,
                "double" | "float" => ParameterType::Float,
                "bool" | "switch" => ParameterType::Bool,
                _ => ParameterType::Str,
            };
            declaration.required = attributes.to_ascii_lowercase().contains("mandatory=$true");
            if type_name.eq_ignore_ascii_case("switch") {
                declaration.action = "store_true".to_owned();
            }
            if let Some(default) = captures.get(4)
                && let Some(value) =
                    parse_powershell_value(default.as_str().trim(), declaration.parameter_type)
            {
                declaration.default = Some(value);
            }
            Some(declaration)
        })
        .collect()
}

/// Detect source-bound values that skit can manage.
#[must_use]
pub fn detect_candidates(kind: &str, text: &str) -> Vec<ParamDecl> {
    if !parser_accepts(kind, text) {
        return Vec::new();
    }
    match kind {
        "python" => python_candidates(text),
        "shell" => shell_candidates(text),
        "js" | "ts" => javascript_candidates(text),
        "fish" => fish_candidates(text),
        _ => Vec::new(),
    }
}

fn python_candidates(text: &str) -> Vec<ParamDecl> {
    let tree = parsed_tree("python", text).expect("the source passed the Python parser gate");
    let root = tree.root_node();
    let mut positioned = Vec::<(usize, ParamDecl)>::new();
    let mut cursor = root.walk();
    for statement in root.named_children(&mut cursor) {
        collect_python_statement_constants(statement, text, &mut positioned);
        if statement.kind() == "if_statement"
            && is_python_main_guard(statement, text)
            && let Some(body) = statement.child_by_field_name("consequence")
        {
            let mut body_cursor = body.walk();
            for child in body.named_children(&mut body_cursor) {
                collect_python_statement_constants(child, text, &mut positioned);
            }
        }
    }

    for (input_order, node) in python_input_calls(root, text).into_iter().enumerate() {
        let prompt = node
            .child_by_field_name("arguments")
            .and_then(first_named_child)
            .and_then(|argument| node_text(argument, text))
            .and_then(parse_python_string)
            .unwrap_or_default();
        let mut declaration = ParamDecl::new(format!("input-{}", input_order + 1));
        declaration.binding = ParameterBinding::Input;
        declaration.delivery = ParameterDelivery::Inject;
        declaration.prompt = prompt;
        declaration.order = input_order as i64;
        declaration.secret = is_secret_name(&declaration.prompt);
        positioned.push((node.start_byte(), declaration));
    }
    positioned.sort_by_key(|(position, _)| *position);
    positioned
        .into_iter()
        .map(|(_, declaration)| declaration)
        .collect()
}

fn collect_python_statement_constants(
    statement: tree_sitter::Node<'_>,
    text: &str,
    output: &mut Vec<(usize, ParamDecl)>,
) {
    let assignment = if statement.kind() == "expression_statement" {
        first_named_child(statement)
    } else {
        Some(statement)
    };
    let Some(assignment) = assignment.filter(|node| node.kind() == "assignment") else {
        return;
    };
    let left = assignment
        .child_by_field_name("left")
        .expect("a Python assignment has a left field");
    let name = node_text(left, text).expect("tree-sitter byte ranges are inside the source");
    if name.starts_with('_') {
        return;
    }
    if left.kind() != "identifier" {
        return;
    }
    let right = assignment
        .child_by_field_name("right")
        .expect("a Python assignment has a right field");
    let source = node_text(right, text).expect("tree-sitter byte ranges are inside the source");
    let Some((parameter_type, default)) = infer_python_literal(source) else {
        return;
    };
    let mut declaration = ParamDecl::new(name);
    declaration.binding = ParameterBinding::Const;
    declaration.delivery = ParameterDelivery::Inject;
    declaration.parameter_type = parameter_type;
    declaration.default = Some(default);
    declaration.secret = is_secret_name(name);
    if let Some((_, current)) = output
        .iter_mut()
        .find(|(_, current)| current.name == declaration.name)
    {
        *current = declaration;
    } else {
        output.push((assignment.start_byte(), declaration));
    }
}

fn is_python_main_guard(statement: tree_sitter::Node<'_>, text: &str) -> bool {
    statement
        .child_by_field_name("condition")
        .and_then(|condition| node_text(condition, text))
        .is_some_and(|condition| condition.contains("__name__") && condition.contains("__main__"))
}

fn infer_python_literal(source: &str) -> Option<(ParameterType, ParameterValue)> {
    let mut source = source.trim();
    while let Some(inner) = source
        .strip_prefix('(')
        .and_then(|value| value.strip_suffix(')'))
    {
        source = inner.trim();
    }
    infer_literal(source).or_else(|| {
        parse_python_string(source).map(|value| (ParameterType::Str, ParameterValue::String(value)))
    })
}

fn parse_python_string(source: &str) -> Option<String> {
    let source = source.trim();
    let source = source
        .strip_prefix(['r', 'R', 'u', 'U', 'b', 'B', 'f', 'F'])
        .unwrap_or(source);
    parse_quoted(source)
}

fn python_input_calls<'tree>(
    root: tree_sitter::Node<'tree>,
    text: &str,
) -> Vec<tree_sitter::Node<'tree>> {
    if python_scope_binds_input(root, text) {
        return Vec::new();
    }
    let mut calls = Vec::new();
    walk_tree(root, &mut |node| {
        if node.kind() != "call"
            || node
                .child_by_field_name("function")
                .and_then(|function| node_text(function, text))
                != Some("input")
        {
            return;
        }
        let mut ancestor = node.parent();
        while let Some(scope) = ancestor {
            if matches!(scope.kind(), "function_definition" | "lambda")
                && python_scope_binds_input(scope, text)
            {
                return;
            }
            ancestor = scope.parent();
        }
        calls.push(node);
    });
    calls.sort_by_key(tree_sitter::Node::start_byte);
    calls
}

fn python_scope_binds_input(scope: tree_sitter::Node<'_>, text: &str) -> bool {
    if let Some(parameters) = scope.child_by_field_name("parameters")
        && subtree_has_identifier(parameters, text, "input")
    {
        return true;
    }
    let body = scope.child_by_field_name("body").unwrap_or(scope);
    python_body_binds_input(body, text, scope.id() == body.id())
}

fn python_body_binds_input(node: tree_sitter::Node<'_>, text: &str, root_level: bool) -> bool {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "assignment"
            && child
                .child_by_field_name("left")
                .and_then(|left| node_text(left, text))
                == Some("input")
        {
            return true;
        }
        if matches!(child.kind(), "function_definition" | "class_definition") {
            if child
                .child_by_field_name("name")
                .and_then(|name| node_text(name, text))
                == Some("input")
            {
                return true;
            }
            continue;
        }
        if matches!(child.kind(), "import_statement" | "import_from_statement")
            && node_text(child, text).is_some_and(python_import_binds_input)
        {
            return true;
        }
        if !(root_level && matches!(child.kind(), "function_definition" | "class_definition"))
            && python_body_binds_input(child, text, false)
        {
            return true;
        }
    }
    false
}

fn subtree_has_identifier(node: tree_sitter::Node<'_>, text: &str, expected: &str) -> bool {
    if node.kind() == "identifier" && node_text(node, text) == Some(expected) {
        return true;
    }
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .any(|child| subtree_has_identifier(child, text, expected))
}

fn python_import_binds_input(statement: &str) -> bool {
    statement
        .split(|character: char| !character.is_alphanumeric() && character != '_')
        .any(|word| word == "input")
}

fn shell_candidates(text: &str) -> Vec<ParamDecl> {
    let mut positioned = Vec::<(usize, ParamDecl)>::new();
    let mut inputs = Vec::<(usize, ParamDecl)>::new();
    let mut assigned = BTreeSet::new();
    let mut defaults = Vec::<(usize, ParamDecl)>::new();
    let mut input_order = 0_i64;
    let mut depth = 0_usize;
    let mut offset = 0_usize;

    for line_with_end in text.split_inclusive('\n') {
        let line = line_with_end.trim_end_matches(['\r', '\n']);
        let trimmed = line.trim();
        if shell_closes_block(trimmed) {
            depth = depth.saturating_sub(1);
        }

        for captures in SHELL_DEFAULT.captures_iter(line) {
            let name = captures.get(1).expect("shell default name").as_str();
            if name.starts_with('_') || defaults.iter().any(|(_, row)| row.name == name) {
                continue;
            }
            let raw_default = captures.get(3).map_or("", |value| value.as_str());
            let (parameter_type, default) = infer_shell_value(raw_default);
            let mut declaration = ParamDecl::new(name);
            declaration.binding = ParameterBinding::EnvDefault;
            declaration.delivery = ParameterDelivery::Env;
            declaration.parameter_type = parameter_type;
            declaration.default = Some(default);
            declaration.secret = is_secret_name(name);
            defaults.push((
                offset + captures.get(0).expect("shell default").start(),
                declaration,
            ));
        }

        if let Some((prompt, variables, secret)) = shell_read(line) {
            for variable in variables {
                let mut declaration = ParamDecl::new(format!("input-{}", input_order + 1));
                declaration.binding = ParameterBinding::Input;
                declaration.delivery = ParameterDelivery::Inject;
                declaration.prompt = prompt.clone();
                declaration.order = input_order;
                declaration.secret =
                    secret || is_secret_name(&declaration.prompt) || is_secret_name(&variable);
                inputs.push((offset, declaration));
                input_order += 1;
            }
        }

        if depth == 0
            && let Some((name, value, self_default)) = shell_assignment(line)
        {
            if !self_default {
                assigned.insert(name.clone());
            }
            if !name.starts_with('_')
                && !self_default
                && !value.is_empty()
                && !value.contains('$')
                && !value.starts_with('(')
            {
                let (parameter_type, default) = infer_shell_value(&value);
                let mut declaration = ParamDecl::new(&name);
                declaration.binding = ParameterBinding::Const;
                declaration.delivery = ParameterDelivery::Inject;
                declaration.parameter_type = parameter_type;
                declaration.default = Some(default);
                declaration.secret = is_secret_name(&name);
                if let Some((_, current)) = positioned
                    .iter_mut()
                    .find(|(_, current)| current.name == name)
                {
                    *current = declaration;
                } else {
                    positioned.push((offset, declaration));
                }
            }
        }

        if shell_opens_block(trimmed) {
            depth += 1;
        }
        offset += line_with_end.len();
    }

    positioned.extend(
        defaults
            .into_iter()
            .filter(|(_, declaration)| !assigned.contains(&declaration.name)),
    );
    positioned.sort_by_key(|(position, _)| *position);
    inputs.sort_by_key(|(position, _)| *position);
    positioned.extend(inputs);
    positioned
        .into_iter()
        .map(|(_, declaration)| declaration)
        .collect()
}

fn shell_assignment(line: &str) -> Option<(String, String, bool)> {
    let words = shlex::split(line)?;
    let first = words.first()?.as_str();
    let (prefix, assignment) = if first.contains('=') {
        (None, first)
    } else if matches!(first, "export" | "declare" | "typeset") {
        let assignment = words.iter().skip(1).find(|word| word.contains('='))?;
        (Some(first), assignment.as_str())
    } else {
        return None;
    };
    if matches!(prefix, Some("declare" | "typeset"))
        && words
            .iter()
            .take_while(|word| !word.contains('='))
            .any(|word| word.starts_with('-') && word.contains('r'))
    {
        return None;
    }
    let (name, value) = assignment.split_once('=')?;
    if !valid_identifier(name) {
        return None;
    }
    let self_default = SHELL_DEFAULT
        .captures(value)
        .and_then(|captures| captures.get(1))
        .is_some_and(|target| target.as_str() == name);
    Some((name.to_owned(), value.to_owned(), self_default))
}

fn shell_read(line: &str) -> Option<(String, Vec<String>, bool)> {
    let words = shlex::split(line)?;
    if words.first().map(String::as_str) != Some("read") {
        return None;
    }
    let mut prompt = String::new();
    let mut secret = false;
    let mut index = 1;
    let mut interactive = false;
    while let Some(option) = words.get(index).filter(|word| word.starts_with('-')) {
        let flags = option.trim_start_matches('-');
        secret |= flags.contains('s');
        interactive |= flags.contains('s') || flags.contains('p');
        index = index.saturating_add(1);
        if flags.contains('p') {
            prompt = words.get(index).cloned().unwrap_or_default();
            if prompt.contains('$') {
                prompt.clear();
            }
            index = index.saturating_add(1);
        }
    }
    if !interactive {
        return None;
    }
    let variables = words[index..]
        .iter()
        .filter(|word| valid_identifier(word))
        .cloned()
        .collect::<Vec<_>>();
    (!variables.is_empty()).then_some((prompt, variables, secret))
}

fn shell_closes_block(line: &str) -> bool {
    matches!(line, "}" | "done" | "fi" | "esac")
}

fn shell_opens_block(line: &str) -> bool {
    (line.ends_with('{') && (line.contains("()") || line.starts_with("function ")))
        || ["if ", "while ", "until ", "for ", "select ", "case "]
            .iter()
            .any(|prefix| line.starts_with(prefix))
}

fn infer_shell_value(value: &str) -> (ParameterType, ParameterValue) {
    infer_literal(value).unwrap_or_else(|| {
        (
            ParameterType::Str,
            ParameterValue::String(unquote_shell(value)),
        )
    })
}

fn javascript_candidates(text: &str) -> Vec<ParamDecl> {
    let mut output = Vec::new();
    for line in text.lines() {
        let Some(captures) = JS_CONST.captures(line) else {
            continue;
        };
        let name = captures
            .get(1)
            .expect("the JavaScript constant pattern captures its name")
            .as_str();
        let source = captures.get(2).map_or("", |value| value.as_str().trim());
        let Some((parameter_type, default)) = infer_javascript_literal(source) else {
            continue;
        };
        let mut declaration = ParamDecl::new(name);
        declaration.binding = ParameterBinding::Const;
        declaration.delivery = ParameterDelivery::Inject;
        declaration.parameter_type = parameter_type;
        declaration.default = Some(default);
        declaration.secret = is_secret_name(name);
        push_unique(&mut output, declaration);
    }
    output
}

fn fish_candidates(text: &str) -> Vec<ParamDecl> {
    let statements = fish_top_level_statements(text);
    let mut candidates = Vec::<ParamDecl>::new();
    let mut clobbered = BTreeSet::new();

    for words in &statements {
        if words.first().map(String::as_str) != Some("set") || fish_is_query(words) {
            continue;
        }
        if let Some(name) = fish_set_name(words) {
            clobbered.insert(name.to_owned());
        }
    }

    for pair in statements.windows(2) {
        let query = &pair[0];
        let guarded = &pair[1];
        if query.first().map(String::as_str) != Some("set") || !fish_is_query(query) {
            continue;
        }
        let Some(query_name) = fish_set_name(query) else {
            continue;
        };
        if query_name.starts_with('_')
            || clobbered.contains(query_name)
            || candidates
                .iter()
                .any(|candidate| candidate.name == query_name)
        {
            continue;
        }
        let Some((guarded_name, values)) = fish_guarded_set(guarded) else {
            continue;
        };
        if guarded_name != query_name || values.is_empty() {
            continue;
        }
        let value = values.join(" ");
        let (parameter_type, default) = infer_shell_value(&value);
        let mut declaration = ParamDecl::new(query_name);
        declaration.binding = ParameterBinding::EnvDefault;
        declaration.delivery = ParameterDelivery::Env;
        declaration.parameter_type = parameter_type;
        declaration.default = Some(default);
        declaration.secret = is_secret_name(query_name);
        candidates.push(declaration);
    }
    candidates
}

fn fish_top_level_statements(text: &str) -> Vec<Vec<String>> {
    let mut output = Vec::new();
    let mut depth = 0_usize;
    for line in text.lines() {
        for segment in fish_line_segments(line) {
            let Some(words) = shlex::split(&segment) else {
                continue;
            };
            debug_assert!(!words.is_empty(), "a nonempty shell segment has one word");
            if words.first().map(String::as_str) == Some("end") {
                depth = depth.saturating_sub(1);
                continue;
            }
            if depth == 0 {
                output.push(words.clone());
            }
            if matches!(
                words.first().map(String::as_str),
                Some("function" | "if" | "while" | "for" | "begin" | "switch")
            ) {
                depth += 1;
            }
        }
    }
    output
}

fn fish_line_segments(line: &str) -> Vec<String> {
    let mut output = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut escaped = false;
    let mut previous_whitespace = true;
    for character in line.chars() {
        if escaped {
            current.push(character);
            escaped = false;
            previous_whitespace = character.is_whitespace();
            continue;
        }
        if character == '\\' {
            current.push(character);
            escaped = true;
            previous_whitespace = false;
            continue;
        }
        if matches!(character, '\'' | '"') {
            if quote == Some(character) {
                quote = None;
            } else if quote.is_none() {
                quote = Some(character);
            }
            current.push(character);
            previous_whitespace = false;
            continue;
        }
        if quote.is_none() && character == '#' && previous_whitespace {
            break;
        }
        if quote.is_none() && character == ';' {
            if !current.trim().is_empty() {
                output.push(current.trim().to_owned());
            }
            current.clear();
            previous_whitespace = true;
            continue;
        }
        previous_whitespace = character.is_whitespace();
        current.push(character);
    }
    if !current.trim().is_empty() {
        output.push(current.trim().to_owned());
    }
    output
}

fn fish_is_query(words: &[String]) -> bool {
    words
        .iter()
        .skip(1)
        .take_while(|word| word.starts_with('-'))
        .any(|word| word == "--query" || word.trim_start_matches('-').contains('q'))
}

fn fish_set_name(words: &[String]) -> Option<&str> {
    words
        .iter()
        .skip(1)
        .find(|word| !word.starts_with('-'))
        .map(String::as_str)
}

fn fish_guarded_set(words: &[String]) -> Option<(&str, &[String])> {
    if words.first().map(String::as_str) != Some("or")
        || words.get(1).map(String::as_str) != Some("set")
    {
        return None;
    }
    let index = words
        .iter()
        .enumerate()
        .skip(2)
        .find(|(_, word)| !word.starts_with('-'))?
        .0;
    Some((words[index].as_str(), &words[index + 1..]))
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
    require_valid_source(kind, text)?;
    if kind == "python" {
        let output = inject_python_values(text, declarations, values)?;
        require_valid_source(kind, &output)?;
        return Ok(output);
    }
    if kind == "shell" {
        let output = inject_shell_values(text, declarations, values)?;
        require_valid_source(kind, &output)?;
        return Ok(output);
    }
    let mut output = text.to_owned();
    for declaration in declarations {
        if declaration.delivery != ParameterDelivery::Inject {
            continue;
        }
        let Some(value) = values.get(&declaration.name) else {
            continue;
        };
        output = match kind {
            "js" | "ts" => inject_javascript(&output, declaration, value)?,
            _ => {
                return Err(LanguageError::UnsupportedKind {
                    kind: kind.to_owned(),
                });
            }
        };
    }
    require_valid_source(kind, &output)?;
    Ok(output)
}

fn inject_python_values(
    text: &str,
    declarations: &[ParamDecl],
    values: &BTreeMap<String, String>,
) -> Result<String, LanguageError> {
    let tree = parsed_tree("python", text).expect("the source passed the Python parser gate");
    let selected = declarations
        .iter()
        .filter(|declaration| {
            declaration.delivery == ParameterDelivery::Inject
                && values.contains_key(&declaration.name)
        })
        .collect::<Vec<_>>();
    if selected.is_empty() {
        return Ok(text.to_owned());
    }

    let mut edits = Vec::<(usize, usize, String)>::new();
    let mut matched = BTreeSet::new();
    walk_tree(tree.root_node(), &mut |node| {
        if node.kind() != "assignment" {
            return;
        }
        let left = node
            .child_by_field_name("left")
            .expect("a Python assignment has a left field");
        let name = node_text(left, text).expect("tree-sitter byte ranges are inside the source");
        let Some(declaration) = selected.iter().find(|declaration| {
            declaration.binding == ParameterBinding::Const && declaration.name == name
        }) else {
            return;
        };
        let right = node
            .child_by_field_name("right")
            .expect("a Python assignment has a right field");
        let source = node_text(right, text).expect("tree-sitter byte ranges are inside the source");
        if infer_python_literal(source).is_none() {
            return;
        }
        let value = values
            .get(&declaration.name)
            .expect("selected declarations have values");
        edits.push((
            right.start_byte(),
            right.end_byte(),
            replacement_literal(
                declaration.parameter_type,
                value,
                quote_python_string,
                python_bool_literal,
            ),
        ));
        matched.insert(declaration.name.clone());
    });
    for (input_order, node) in python_input_calls(tree.root_node(), text)
        .into_iter()
        .enumerate()
    {
        if let Some(declaration) = selected.iter().find(|declaration| {
            declaration.binding == ParameterBinding::Input
                && declaration.order == input_order as i64
        }) {
            let value = values
                .get(&declaration.name)
                .expect("selected declarations have values");
            edits.push((
                node.start_byte(),
                node.end_byte(),
                quote_python_string(value),
            ));
            matched.insert(declaration.name.clone());
        }
    }

    if let Some(missing) = selected
        .iter()
        .find(|declaration| !matched.contains(&declaration.name))
    {
        return Err(LanguageError::BindingNotFound {
            name: missing.name.clone(),
        });
    }
    apply_source_edits(text, edits)
}

fn apply_source_edits(
    text: &str,
    mut edits: Vec<(usize, usize, String)>,
) -> Result<String, LanguageError> {
    edits.sort_by_key(|(start, _, _)| *start);
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

fn inject_shell_values(
    text: &str,
    declarations: &[ParamDecl],
    values: &BTreeMap<String, String>,
) -> Result<String, LanguageError> {
    let selected = declarations
        .iter()
        .filter(|declaration| {
            declaration.delivery == ParameterDelivery::Inject
                && values.contains_key(&declaration.name)
        })
        .collect::<Vec<_>>();
    if selected.is_empty() {
        return Ok(text.to_owned());
    }

    let mut edits = Vec::<(usize, usize, String)>::new();
    let mut matched = BTreeSet::new();
    let mut input_order = 0_i64;
    let mut offset = 0_usize;
    for line_with_end in text.split_inclusive('\n') {
        let line = line_with_end.trim_end_matches(['\r', '\n']);
        if let Some((_, variables, _)) = shell_read(line) {
            let first_order = input_order;
            input_order += variables.len() as i64;
            let rows = variables
                .iter()
                .enumerate()
                .map(|(index, variable)| {
                    selected
                        .iter()
                        .find(|declaration| {
                            declaration.binding == ParameterBinding::Input
                                && declaration.order == first_order + index as i64
                        })
                        .map(|declaration| (*declaration, variable))
                })
                .collect::<Vec<_>>();
            if !rows.iter().all(Option::is_some) {
                offset += line_with_end.len();
                continue;
            }
            let indent = &line[..line.len() - line.trim_start().len()];
            let replacement = rows
                .into_iter()
                .flatten()
                .map(|(declaration, variable)| {
                    matched.insert(declaration.name.clone());
                    format!(
                        "{variable}={}",
                        quote_posix(
                            values
                                .get(&declaration.name)
                                .expect("selected declarations have values")
                        )
                    )
                })
                .collect::<Vec<_>>()
                .join("; ");
            edits.push((
                offset,
                offset + line.len(),
                format!("{indent}{replacement}"),
            ));
        }

        if let Some((name, _, self_default)) = shell_assignment(line)
            && !self_default
            && let Some(declaration) = selected.iter().find(|declaration| {
                declaration.binding == ParameterBinding::Const && declaration.name == name
            })
            && let Some(value_start) = shell_assignment_value_start(line, &name)
        {
            let before_comment = shell_comment_start(line).unwrap_or(line.len());
            let value_end = line[..before_comment].trim_end().len();
            if value_start <= value_end {
                edits.push((
                    offset + value_start,
                    offset + value_end,
                    quote_posix(
                        values
                            .get(&declaration.name)
                            .expect("selected declarations have values"),
                    ),
                ));
                matched.insert(declaration.name.clone());
            }
        }
        offset += line_with_end.len();
    }

    if let Some(missing) = selected
        .iter()
        .find(|declaration| !matched.contains(&declaration.name))
    {
        return Err(LanguageError::BindingNotFound {
            name: missing.name.clone(),
        });
    }
    apply_source_edits(text, edits)
}

fn shell_assignment_value_start(line: &str, name: &str) -> Option<usize> {
    let pattern = format!("{name}=");
    line.find(&pattern).map(|index| index + pattern.len())
}

fn shell_comment_start(line: &str) -> Option<usize> {
    let mut quote = None;
    let mut escaped = false;
    let mut previous_whitespace = true;
    for (index, character) in line.char_indices() {
        if escaped {
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if matches!(character, '\'' | '"') {
            if quote == Some(character) {
                quote = None;
            } else if quote.is_none() {
                quote = Some(character);
            }
        } else if quote.is_none() && character == '#' && previous_whitespace {
            return Some(index);
        }
        previous_whitespace = character.is_whitespace();
    }
    None
}

fn inject_javascript(
    text: &str,
    declaration: &ParamDecl,
    value: &str,
) -> Result<String, LanguageError> {
    let name = regex::escape(&declaration.name);
    let pattern = Regex::new(&format!(
        r"(?m)^([ \t]*(?:const|let|var)[ \t]+{name}(?:[ \t]*:[^=\r\n]+)?[ \t]*=[ \t]*)([^;\r\n]+)(;?[ \t]*)(\r?)$"
    ))
    .expect("escaped JavaScript constant pattern");
    if pattern.captures(text).is_none() {
        return Err(LanguageError::BindingNotFound {
            name: declaration.name.clone(),
        });
    }
    let literal = replacement_literal(
        declaration.parameter_type,
        value,
        quote_javascript_string,
        javascript_bool_literal,
    );
    Ok(replace_first(text, &pattern, |captures| {
        format!(
            "{}{}{}{}",
            captures.get(1).map_or("", |item| item.as_str()),
            literal,
            captures.get(3).map_or("", |item| item.as_str()),
            captures.get(4).map_or("", |item| item.as_str())
        )
    })
    .expect("the same JavaScript constant pattern matched before replacement"))
}

fn replace_first<F>(text: &str, pattern: &Regex, replacement: F) -> Option<String>
where
    F: FnOnce(&Captures<'_>) -> String,
{
    let captures = pattern.captures(text)?;
    let whole = captures.get(0)?;
    let mut output = String::with_capacity(text.len());
    output.push_str(&text[..whole.start()]);
    output.push_str(&replacement(&captures));
    output.push_str(&text[whole.end()..]);
    Some(output)
}

/// Render one accepted value as a literal of the target language.
///
/// `bool_literal` belongs to the language, not to the stored constant: Python accepts only
/// `True`/`False`, so a Boolean parameter over `FLAG = 1` must still inject `True`.
fn replacement_literal(
    parameter_type: ParameterType,
    value: &str,
    string_quote: fn(&str) -> String,
    bool_literal: fn(bool) -> &'static str,
) -> String {
    match parameter_type {
        ParameterType::Int => value
            .trim()
            .parse::<i64>()
            .map_or_else(|_| string_quote(value), |value| value.to_string()),
        ParameterType::Float => value.trim().parse::<f64>().map_or_else(
            |_| string_quote(value),
            |value| {
                let mut rendered = value.to_string();
                if !rendered.contains(['.', 'e', 'E']) {
                    rendered.push_str(".0");
                }
                rendered
            },
        ),
        ParameterType::Bool => canonical_bool(value).map_or_else(
            || string_quote(value),
            |value| bool_literal(value).to_owned(),
        ),
        ParameterType::Str | ParameterType::Choice | ParameterType::Path => string_quote(value),
    }
}

const fn python_bool_literal(value: bool) -> &'static str {
    if value { "True" } else { "False" }
}

const fn javascript_bool_literal(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

fn canonical_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "y" | "on" => Some(true),
        "false" | "0" | "no" | "n" | "off" => Some(false),
        _ => None,
    }
}

fn quote_python_string(value: &str) -> String {
    format!(
        "'{}'",
        value
            .replace('\\', "\\\\")
            .replace('\'', "\\'")
            .replace('\n', "\\n")
            .replace('\r', "\\r")
            .replace('\t', "\\t")
    )
}

fn quote_javascript_string(value: &str) -> String {
    format!(
        "'{}'",
        value
            .replace('\\', "\\\\")
            .replace('\'', "\\'")
            .replace('\n', "\\n")
            .replace('\r', "\\r")
    )
}

fn quote_posix(value: &str) -> String {
    if !value.is_empty()
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "_@%+=:,./-".contains(character))
    {
        value.to_owned()
    } else {
        format!("'{}'", value.replace('\'', "'\"'\"'"))
    }
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
    // One parse serves the gate and the scan, so a refused source never reaches an analyzer.
    match (kind, parsed_tree(kind, text)) {
        ("python", Some(tree)) => {
            let inline = parse_inline_metadata(text, "#")
                .and_then(|table| table.get("dependencies").cloned())
                .and_then(|value| value.as_array().cloned())
                .into_iter()
                .flatten()
                .filter_map(|value| value.as_str().map(str::to_owned))
                .collect::<Vec<_>>();
            if inline.is_empty() {
                python_dependencies(&tree, text, script_dir)
            } else {
                inline
            }
        }
        ("js" | "ts", Some(tree)) => javascript_dependencies(&tree, text),
        _ => Vec::new(),
    }
}

fn require_valid_source(kind: &str, text: &str) -> Result<(), LanguageError> {
    if parser_accepts(kind, text) {
        Ok(())
    } else {
        Err(LanguageError::InvalidSource {
            kind: kind.to_owned(),
        })
    }
}

/// Report whether a parser accepts the complete source text.
///
/// Kinds without a parser-backed analyzer accept all source text. Parser-backed
/// analyzer, injector, and dependency features use this same gate.
#[must_use]
pub fn source_is_valid(kind: &str, text: &str) -> bool {
    parser_accepts(kind, text)
}

fn parser_accepts(kind: &str, text: &str) -> bool {
    !matches!(kind, "python" | "shell" | "js" | "ts") || parsed_tree(kind, text).is_some()
}

fn parsed_tree(kind: &str, text: &str) -> Option<tree_sitter::Tree> {
    let language = match kind {
        "python" => tree_sitter_python::LANGUAGE,
        "shell" => tree_sitter_bash::LANGUAGE,
        "js" => tree_sitter_javascript::LANGUAGE,
        "ts" => tree_sitter_typescript::LANGUAGE_TYPESCRIPT,
        _ => return None,
    };
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&language.into()).ok()?;
    parser
        .parse(text, None)
        .filter(|tree| !tree.root_node().has_error())
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

fn call_bodies(text: &str, needle: &str) -> Vec<String> {
    let mut output = Vec::new();
    let mut offset = 0;
    while let Some(relative) = text[offset..].find(needle) {
        let start = offset + relative + needle.len();
        let Some(open) = text[start..]
            .find('(')
            .map(|relative_open| start + relative_open)
        else {
            break;
        };
        if let Some(body) = parenthesized_body(text, open) {
            output.push(body.to_owned());
        }
        offset = open.saturating_add(1);
    }
    output
}

fn parenthesized_body(text: &str, open: usize) -> Option<&str> {
    if text.as_bytes().get(open) != Some(&b'(') {
        return None;
    }
    let bytes = text.as_bytes();
    let mut depth = 0_i32;
    let mut quote = None;
    let mut escaped = false;
    let mut index = open;
    while index < bytes.len() {
        let character = bytes[index] as char;
        if escaped {
            escaped = false;
            index = index.saturating_add(1);
            continue;
        }
        if character == '\\' && quote.is_some() {
            escaped = true;
            index = index.saturating_add(1);
            continue;
        }
        if matches!(character, '\'' | '"') {
            if quote == Some(character) {
                quote = None;
            } else if quote.is_none() {
                quote = Some(character);
            }
            index = index.saturating_add(1);
            continue;
        }
        if quote.is_none() {
            if character == '(' {
                depth += 1;
            } else if character == ')' {
                depth -= 1;
                if depth == 0 {
                    return text.get(open + 1..index);
                }
            }
        }
        index = index.saturating_add(1);
    }
    None
}

fn split_top_level(text: &str, delimiter: char) -> Vec<&str> {
    let mut output = Vec::new();
    let mut quote = None;
    let mut escaped = false;
    let mut square = 0_i32;
    let mut round = 0_i32;
    let mut curly = 0_i32;
    let mut start = 0;
    for (index, character) in text.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' && quote.is_some() {
            escaped = true;
            continue;
        }
        if matches!(character, '\'' | '"') {
            if quote == Some(character) {
                quote = None;
            } else if quote.is_none() {
                quote = Some(character);
            }
            continue;
        }
        if quote.is_some() {
            continue;
        }
        match character {
            '[' => square += 1,
            ']' => square -= 1,
            '(' => round += 1,
            ')' => round -= 1,
            '{' => curly += 1,
            '}' => curly -= 1,
            value if value == delimiter && square == 0 && round == 0 && curly == 0 => {
                output.push(&text[start..index]);
                start = index + character.len_utf8();
            }
            _ => {}
        }
    }
    output.push(&text[start..]);
    output
}

fn split_keyword(value: &str) -> Option<(&str, &str)> {
    let parts = split_top_level(value, '=');
    (parts.len() == 2).then(|| (parts[0].trim(), parts[1].trim()))
}

fn parse_quoted(value: &str) -> Option<String> {
    let value = value.trim();
    if value.len() < 2 {
        return None;
    }
    let quote = value.as_bytes()[0] as char;
    if !matches!(quote, '\'' | '"') || value.as_bytes()[value.len() - 1] as char != quote {
        return None;
    }
    let inner = &value[1..value.len() - 1];
    Some(
        inner
            .replace("\\n", "\n")
            .replace("\\r", "\r")
            .replace("\\t", "\t")
            .replace(&format!("\\{quote}"), &quote.to_string())
            .replace("\\\\", "\\"),
    )
}

fn parse_list_strings(value: &str) -> Vec<String> {
    let value = value.trim();
    let Some(inner) = value
        .strip_prefix('[')
        .and_then(|item| item.strip_suffix(']'))
        .or_else(|| {
            value
                .strip_prefix('(')
                .and_then(|item| item.strip_suffix(')'))
        })
    else {
        return Vec::new();
    };
    split_top_level(inner, ',')
        .into_iter()
        .filter_map(parse_quoted)
        .collect()
}

fn parse_parameter_value(value: &str, parameter_type: ParameterType) -> Option<ParameterValue> {
    let value = value.trim();
    if matches!(value, "None" | "...") {
        return None;
    }
    match parameter_type {
        ParameterType::Int => value.parse().ok().map(ParameterValue::Integer),
        ParameterType::Float => value
            .parse::<f64>()
            .ok()
            .filter(|number| number.is_finite())
            .map(ParameterValue::Float),
        ParameterType::Bool => match value.to_ascii_lowercase().as_str() {
            "true" | "1" => Some(ParameterValue::Bool(true)),
            "false" | "0" => Some(ParameterValue::Bool(false)),
            _ => None,
        },
        ParameterType::Str | ParameterType::Choice | ParameterType::Path => {
            parse_quoted(value).map(ParameterValue::String)
        }
    }
}

fn parse_powershell_value(value: &str, parameter_type: ParameterType) -> Option<ParameterValue> {
    if parameter_type == ParameterType::Bool {
        return match value.to_ascii_lowercase().as_str() {
            "$true" => Some(ParameterValue::Bool(true)),
            "$false" => Some(ParameterValue::Bool(false)),
            _ => None,
        };
    }
    parse_parameter_value(value, parameter_type).or_else(|| match parameter_type {
        ParameterType::Str | ParameterType::Choice | ParameterType::Path => Some(
            ParameterValue::String(value.trim_matches(&['\'', '"'][..]).to_owned()),
        ),
        ParameterType::Int | ParameterType::Float | ParameterType::Bool => None,
    })
}

fn parameter_type(value: &str) -> ParameterType {
    match value.trim().trim_matches(&['\'', '"'][..]) {
        "int" => ParameterType::Int,
        "float" => ParameterType::Float,
        "bool" => ParameterType::Bool,
        _ => ParameterType::Str,
    }
}

fn truthy_source(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "true" | "1" | "$true"
    )
}

fn flag_name(flag: &str) -> String {
    flag.trim_start_matches('-').replace('-', "_")
}

fn infer_literal(source: &str) -> Option<(ParameterType, ParameterValue)> {
    if let Some(value) = parse_quoted(source) {
        return Some((ParameterType::Str, ParameterValue::String(value)));
    }
    if let Ok(value) = source.parse::<i64>() {
        return Some((ParameterType::Int, ParameterValue::Integer(value)));
    }
    if let Ok(value) = source.parse::<f64>()
        && value.is_finite()
    {
        return Some((ParameterType::Float, ParameterValue::Float(value)));
    }
    match source {
        "True" | "true" => Some((ParameterType::Bool, ParameterValue::Bool(true))),
        "False" | "false" => Some((ParameterType::Bool, ParameterValue::Bool(false))),
        _ => None,
    }
}

fn infer_javascript_literal(source: &str) -> Option<(ParameterType, ParameterValue)> {
    infer_literal(source)
}

fn unquote_shell(value: &str) -> String {
    let value = value.trim();
    if value.len() >= 2
        && ((value.starts_with('\'') && value.ends_with('\''))
            || (value.starts_with('"') && value.ends_with('"')))
    {
        value[1..value.len() - 1].to_owned()
    } else {
        value.to_owned()
    }
}

fn push_unique(output: &mut Vec<ParamDecl>, parameter: ParamDecl) {
    if let Some(existing) = output
        .iter_mut()
        .find(|existing| existing.name == parameter.name)
    {
        *existing = parameter;
    } else {
        output.push(parameter);
    }
}

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
    fn injected_literals_fall_back_to_a_quoted_string_when_a_value_does_not_parse() {
        assert_eq!(
            replacement_literal(
                ParameterType::Int,
                "many",
                toml_string,
                javascript_bool_literal
            ),
            "\"many\""
        );
        assert_eq!(
            replacement_literal(
                ParameterType::Float,
                "many",
                toml_string,
                javascript_bool_literal
            ),
            "\"many\""
        );
        assert_eq!(
            replacement_literal(
                ParameterType::Bool,
                "maybe",
                toml_string,
                javascript_bool_literal
            ),
            "\"maybe\""
        );
        // A Python source spelling keeps its capitals; every other language uses lowercase.
        assert_eq!(
            replacement_literal(ParameterType::Bool, "no", toml_string, python_bool_literal),
            "False"
        );
        assert_eq!(
            replacement_literal(
                ParameterType::Bool,
                "no",
                toml_string,
                javascript_bool_literal
            ),
            "false"
        );
        assert_eq!(
            replacement_literal(
                ParameterType::Float,
                "3",
                toml_string,
                javascript_bool_literal
            ),
            "3.0"
        );
    }

    #[test]
    fn text_scanners_handle_quotes_escapes_nesting_and_invalid_input() {
        assert_eq!(toml_string("a\rb"), "\"a\\rb\"");

        assert_eq!(parenthesized_body("plain", 0), None);
        assert_eq!(
            parenthesized_body("call('a\\\'b', nested(1)) tail", 4),
            Some("'a\\\'b', nested(1)")
        );
        assert_eq!(parenthesized_body("call('unterminated", 4), None);

        assert_eq!(
            split_top_level("a='x\\\'y', b=[1,2], c=(3,4), d={5,6}", ','),
            ["a='x\\\'y'", " b=[1,2]", " c=(3,4)", " d={5,6}"]
        );
        assert_eq!(parse_list_strings("('one', 'two')"), ["one", "two"]);
        assert!(parse_list_strings("not-a-list").is_empty());
    }

    #[test]
    fn scalar_readers_distinguish_boolean_and_powershell_values() {
        assert_eq!(
            parse_parameter_value("true", ParameterType::Bool),
            Some(ParameterValue::Bool(true))
        );
        assert_eq!(
            parse_parameter_value("0", ParameterType::Bool),
            Some(ParameterValue::Bool(false))
        );
        assert_eq!(parse_parameter_value("maybe", ParameterType::Bool), None);
        assert_eq!(
            parse_powershell_value("$false", ParameterType::Bool),
            Some(ParameterValue::Bool(false))
        );
        assert_eq!(parse_powershell_value("maybe", ParameterType::Bool), None);
        assert_eq!(
            parse_powershell_value("bare", ParameterType::Str),
            Some(ParameterValue::String("bare".to_owned()))
        );
        assert_eq!(parse_powershell_value("bare", ParameterType::Float), None);
        assert_eq!(unquote_shell("'quoted value'"), "quoted value");
    }

    #[test]
    fn internal_guards_reject_unsupported_parsers_and_invalid_edits() {
        assert!(parsed_tree("ruby", "puts 1").is_none());
        assert!(matches!(
            apply_source_edits("abc", vec![(2, 1, "x".to_owned())]),
            Err(LanguageError::InvalidSource { .. })
        ));
        assert!(matches!(
            apply_source_edits("abc", vec![(0, 2, "x".to_owned()), (1, 3, "y".to_owned())]),
            Err(LanguageError::InvalidSource { .. })
        ));
        assert!(fish_guarded_set(&["or".to_owned(), "echo".to_owned()]).is_none());
    }

    #[test]
    fn source_helpers_preserve_comments_and_boolean_literal_style() {
        assert_eq!(shell_comment_start("NAME=old\\ value # note"), Some(16));
        assert_eq!(shell_comment_start("NAME='old # value'"), None);
        assert_eq!(
            replacement_literal(
                ParameterType::Bool,
                "maybe",
                quote_python_string,
                python_bool_literal,
            ),
            "'maybe'"
        );
        assert_eq!(
            scan_placeholders("{{escaped}} {open", false),
            Vec::<String>::new()
        );
    }
}
