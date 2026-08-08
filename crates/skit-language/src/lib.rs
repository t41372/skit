//! Analyze and rewrite supported source files.
//!
//! The analyzers read static source text. They do not execute user code.
//! When a dynamic declaration is not clear, the analyzer returns no field.

#![forbid(unsafe_code)]

use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::LazyLock,
};

use regex::{Captures, Regex};
use serde_json::Value as JsonValue;
use skit_domain::parameters::{
    ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, ParameterValue,
    synthesized_placeholder,
};
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
static PYTHON_INPUT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"^\s*([A-Za-z_][A-Za-z0-9_]*)\s*=\s*input\s*\(\s*(?:"([^"]*)"|'([^']*)')\s*\)\s*(?:#.*)?$"#,
    )
    .expect("fixed Python input pattern")
});
static PYTHON_ASSIGN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*([A-Za-z_][A-Za-z0-9_]*)\s*=\s*([^#\r\n]+?)\s*(?:#.*)?$")
        .expect("fixed Python assignment pattern")
});
static SHELL_ENV_DEFAULT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*([A-Za-z_][A-Za-z0-9_]*)\s*=\s*\$\{([A-Za-z_][A-Za-z0-9_]*):-([^}]*)\}\s*$")
        .expect("fixed shell environment-default pattern")
});
static SHELL_READ: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"^\s*read(?:\s+-r)?\s+-p\s+(?:"([^"]*)"|'([^']*)')\s+([A-Za-z_][A-Za-z0-9_]*)\s*$"#,
    )
    .expect("fixed shell read pattern")
});
static SHELL_ASSIGN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*([A-Za-z_][A-Za-z0-9_]*)=([^\s#]+)\s*(?:#.*)?$")
        .expect("fixed shell assignment pattern")
});
static JS_CONST: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*(?:const|let)\s+([A-Za-z_$][A-Za-z0-9_$]*)\s*=\s*([^;\r\n]+)\s*;?\s*$")
        .expect("fixed JavaScript constant pattern")
});
static FISH_ENV_DEFAULT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^\s*set\s+-q\s+([A-Za-z_][A-Za-z0-9_]*);\s*or\s+set\s+([A-Za-z_][A-Za-z0-9_]*)\s+(.+?)\s*$",
    )
    .expect("fixed fish environment-default pattern")
});
static JS_IMPORT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?m)\bfrom\s+['"]([^'"]+)['"]|\bimport\s+['"]([^'"]+)['"]"#)
        .expect("fixed JavaScript import pattern")
});
static JS_REQUIRE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"\brequire\s*\(\s*['"]([^'"]+)['"]\s*\)"#)
        .expect("fixed JavaScript require pattern")
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
    InvalidMetadata { reason: String },
    /// A parser-backed source has syntax errors.
    #[error("source is not valid {kind} syntax")]
    InvalidSource { kind: String },
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

fn shebang_kind(line: &str) -> Option<&'static str> {
    let line = line.trim().strip_prefix("#!")?.trim();
    let mut words = line.split_whitespace();
    let first = words.next()?;
    let program = if basename(first).eq_ignore_ascii_case("env") {
        words.find(|value| !value.starts_with('-'))?
    } else {
        first
    };
    match basename(program).to_ascii_lowercase().as_str() {
        "python" | "python3" => Some("python"),
        "bash" | "sh" | "zsh" | "dash" | "ash" | "ksh" => Some("shell"),
        "fish" => Some("fish"),
        "node" | "deno" | "bun" => Some("js"),
        "pwsh" | "powershell" | "powershell.exe" => Some("powershell"),
        "ruby" => Some("ruby"),
        "perl" => Some("perl"),
        "lua" | "luajit" => Some("lua"),
        "rscript" => Some("r"),
        _ => None,
    }
}

fn basename(value: &str) -> &str {
    value.rsplit(['/', '\\']).next().unwrap_or(value)
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
        let mut block = format!("{leader} /// script{newline}");
        if !kept.is_empty() {
            block.push_str(&kept);
            if !kept.ends_with(newline) {
                block.push_str(newline);
            }
        }
        if !params.is_empty() {
            if !kept.is_empty() {
                block.push_str(leader);
                block.push_str(newline);
            }
            block.push_str(&commentify(&render_managed_toml(params), leader, newline));
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
                    reason: error.to_string(),
                }
            })?
        }
        None => toml::Table::new(),
    };
    if dependencies.is_empty() {
        table.remove("dependencies");
    } else {
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
    }
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
    let encoded = toml::to_string(table).map_err(|error| LanguageError::InvalidMetadata {
        reason: error.to_string(),
    })?;
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
        r"(?ms)^(?:{}) /// script\r?\n(?P<body>.*?)(?P<close>^(?:{}) ///(?:\r?\n|$))",
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
            skipping = stripped.starts_with("[tool.skit]") || stripped.starts_with("[[tool.skit.");
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
        for (key, value) in parameter.to_block_map() {
            lines.push(format!("{key} = {}", json_toml_literal(&value)));
        }
    }
    lines.join("\n") + "\n"
}

fn json_toml_literal(value: &JsonValue) -> String {
    match value {
        JsonValue::String(value) => toml_string(value),
        JsonValue::Bool(value) => value.to_string(),
        JsonValue::Number(value) => value.to_string(),
        _ => toml_string(&value.to_string()),
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
        let Some(full) = captures.get(0) else {
            continue;
        };
        let Some(open_offset) = full.as_str().rfind('(') else {
            continue;
        };
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
            index += 1;
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
        index += usize::from(takes_value) + 1;
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
            let Some(raw) = captures.get(1).map(|value| value.as_str()) else {
                continue;
            };
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
    let mut output = Vec::new();
    for line in text.lines() {
        if let Some(captures) = PYTHON_INPUT.captures(line) {
            let Some(name) = captures.get(1).map(|value| value.as_str()) else {
                continue;
            };
            let mut declaration = ParamDecl::new(name);
            declaration.binding = ParameterBinding::Input;
            declaration.delivery = ParameterDelivery::Inject;
            declaration.prompt = captures
                .get(2)
                .or_else(|| captures.get(3))
                .map_or("", |value| value.as_str())
                .to_owned();
            declaration.order = output.len() as i64;
            push_unique(&mut output, declaration);
            continue;
        }
        let Some(captures) = PYTHON_ASSIGN.captures(line) else {
            continue;
        };
        let Some(name) = captures.get(1).map(|value| value.as_str()) else {
            continue;
        };
        let Some(source) = captures.get(2).map(|value| value.as_str().trim()) else {
            continue;
        };
        let Some((parameter_type, default)) = infer_literal(source) else {
            continue;
        };
        let mut declaration = ParamDecl::new(name);
        declaration.binding = ParameterBinding::Const;
        declaration.delivery = ParameterDelivery::Inject;
        declaration.parameter_type = parameter_type;
        declaration.default = Some(default);
        push_unique(&mut output, declaration);
    }
    output
}

fn shell_candidates(text: &str) -> Vec<ParamDecl> {
    let mut output = Vec::new();
    for line in text.lines() {
        if let Some(captures) = SHELL_ENV_DEFAULT.captures(line) {
            let Some(name) = captures.get(1).map(|value| value.as_str()) else {
                continue;
            };
            if captures.get(2).map(|value| value.as_str()) != Some(name) {
                continue;
            }
            let default = captures.get(3).map_or("", |value| value.as_str());
            let mut declaration = ParamDecl::new(name);
            declaration.binding = ParameterBinding::EnvDefault;
            declaration.delivery = ParameterDelivery::Env;
            declaration.default = Some(ParameterValue::String(unquote_shell(default)));
            declaration.env_target = name.to_owned();
            push_unique(&mut output, declaration);
            continue;
        }
        if let Some(captures) = SHELL_READ.captures(line) {
            let Some(name) = captures.get(3).map(|value| value.as_str()) else {
                continue;
            };
            let mut declaration = ParamDecl::new(name);
            declaration.binding = ParameterBinding::Input;
            declaration.delivery = ParameterDelivery::Inject;
            declaration.prompt = captures
                .get(1)
                .or_else(|| captures.get(2))
                .map_or("", |value| value.as_str())
                .to_owned();
            declaration.order = output.len() as i64;
            push_unique(&mut output, declaration);
            continue;
        }
        let Some(captures) = SHELL_ASSIGN.captures(line) else {
            continue;
        };
        let Some(name) = captures.get(1).map(|value| value.as_str()) else {
            continue;
        };
        let source = captures.get(2).map_or("", |value| value.as_str());
        if source.contains('$') {
            continue;
        }
        let mut declaration = ParamDecl::new(name);
        declaration.binding = ParameterBinding::Const;
        declaration.delivery = ParameterDelivery::Inject;
        declaration.default = Some(ParameterValue::String(unquote_shell(source)));
        push_unique(&mut output, declaration);
    }
    output
}

fn javascript_candidates(text: &str) -> Vec<ParamDecl> {
    let mut output = Vec::new();
    for line in text.lines() {
        let Some(captures) = JS_CONST.captures(line) else {
            continue;
        };
        let Some(name) = captures.get(1).map(|value| value.as_str()) else {
            continue;
        };
        let source = captures.get(2).map_or("", |value| value.as_str().trim());
        let Some((parameter_type, default)) = infer_javascript_literal(source) else {
            continue;
        };
        let mut declaration = ParamDecl::new(name);
        declaration.binding = ParameterBinding::Const;
        declaration.delivery = ParameterDelivery::Inject;
        declaration.parameter_type = parameter_type;
        declaration.default = Some(default);
        push_unique(&mut output, declaration);
    }
    output
}

fn fish_candidates(text: &str) -> Vec<ParamDecl> {
    FISH_ENV_DEFAULT
        .captures_iter(text)
        .filter_map(|captures| {
            let name = captures.get(1)?.as_str();
            if captures.get(2)?.as_str() != name {
                return None;
            }
            let mut declaration = ParamDecl::new(name);
            declaration.binding = ParameterBinding::EnvDefault;
            declaration.delivery = ParameterDelivery::Env;
            declaration.env_target = name.to_owned();
            declaration.default = Some(ParameterValue::String(unquote_shell(
                captures.get(3)?.as_str().trim(),
            )));
            Some(declaration)
        })
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

fn scan_placeholders(text: &str, doubled: bool) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut output = Vec::new();
    let mut seen = BTreeSet::new();
    let mut index = 0;
    while index < bytes.len() {
        let open: &[u8] = if doubled { b"{{" } else { b"{" };
        let close: &[u8] = if doubled { b"}}" } else { b"}" };
        if !bytes[index..].starts_with(open) {
            index += 1;
            continue;
        }
        if !doubled && bytes[index..].starts_with(b"{{") {
            index += 2;
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
        index = end + close.len();
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
    let mut output = text.to_owned();
    for declaration in declarations {
        if declaration.delivery != ParameterDelivery::Inject {
            continue;
        }
        let Some(value) = values.get(&declaration.name) else {
            continue;
        };
        output = match kind {
            "python" => inject_python(&output, declaration, value)?,
            "shell" => inject_shell(&output, declaration, value)?,
            "js" | "ts" => inject_javascript(&output, declaration, value)?,
            _ => {
                return Err(LanguageError::UnsupportedKind {
                    kind: kind.to_owned(),
                });
            }
        };
    }
    Ok(output)
}

fn inject_python(
    text: &str,
    declaration: &ParamDecl,
    value: &str,
) -> Result<String, LanguageError> {
    let name = regex::escape(&declaration.name);
    if declaration.binding == ParameterBinding::Input {
        let pattern = Regex::new(&format!(
            r"(?m)^([ \t]*{name}[ \t]*=[ \t]*)input[ \t]*\([^\r\n]*\)([ \t]*(?:#.*)?$)"
        ))
        .expect("escaped Python input pattern");
        return replace_first(text, &pattern, |captures| {
            format!(
                "{}{}{}",
                captures.get(1).map_or("", |item| item.as_str()),
                quote_python_string(value),
                captures.get(2).map_or("", |item| item.as_str())
            )
        })
        .ok_or_else(|| LanguageError::BindingNotFound {
            name: declaration.name.clone(),
        });
    }

    let pattern = Regex::new(&format!(
        r"(?m)^([ \t]*{name}[ \t]*=[ \t]*)([^#\r\n]+?)([ \t]*(?:#.*)?$)"
    ))
    .expect("escaped Python assignment pattern");
    let Some(captures) = pattern.captures(text) else {
        return Err(LanguageError::BindingNotFound {
            name: declaration.name.clone(),
        });
    };
    let source = captures
        .get(2)
        .map_or("", |capture| capture.as_str().trim());
    let literal = replacement_literal(source, value, quote_python_string);
    replace_first(text, &pattern, |captures| {
        format!(
            "{}{}{}",
            captures.get(1).map_or("", |item| item.as_str()),
            literal,
            captures.get(3).map_or("", |item| item.as_str())
        )
    })
    .ok_or_else(|| LanguageError::BindingNotFound {
        name: declaration.name.clone(),
    })
}

fn inject_shell(text: &str, declaration: &ParamDecl, value: &str) -> Result<String, LanguageError> {
    let name = regex::escape(&declaration.name);
    let pattern = if declaration.binding == ParameterBinding::Input {
        Regex::new(&format!(
            r"(?m)^([ \t]*)read(?:[ \t]+-r)?[ \t]+-p[ \t]+[^\r\n]+[ \t]+{name}[ \t]*$"
        ))
        .expect("escaped shell read pattern")
    } else {
        Regex::new(&format!(r"(?m)^([ \t]*{name}=)[^\r\n]*$"))
            .expect("escaped shell assignment pattern")
    };
    let Some(captures) = pattern.captures(text) else {
        return Err(LanguageError::BindingNotFound {
            name: declaration.name.clone(),
        });
    };
    let replacement = if declaration.binding == ParameterBinding::Input {
        format!(
            "{}{}={}",
            captures.get(1).map_or("", |item| item.as_str()),
            declaration.name,
            quote_posix(value)
        )
    } else {
        format!(
            "{}{}",
            captures.get(1).map_or("", |item| item.as_str()),
            quote_posix(value)
        )
    };
    replace_first(text, &pattern, |_| replacement.clone()).ok_or_else(|| {
        LanguageError::BindingNotFound {
            name: declaration.name.clone(),
        }
    })
}

fn inject_javascript(
    text: &str,
    declaration: &ParamDecl,
    value: &str,
) -> Result<String, LanguageError> {
    let name = regex::escape(&declaration.name);
    let pattern = Regex::new(&format!(
        r"(?m)^([ \t]*(?:const|let)[ \t]+{name}[ \t]*=[ \t]*)([^;\r\n]+)(;?[ \t]*)$"
    ))
    .expect("escaped JavaScript constant pattern");
    let Some(captures) = pattern.captures(text) else {
        return Err(LanguageError::BindingNotFound {
            name: declaration.name.clone(),
        });
    };
    let source = captures
        .get(2)
        .map_or("", |capture| capture.as_str().trim());
    let literal = replacement_literal(source, value, quote_javascript_string);
    replace_first(text, &pattern, |captures| {
        format!(
            "{}{}{}",
            captures.get(1).map_or("", |item| item.as_str()),
            literal,
            captures.get(3).map_or("", |item| item.as_str())
        )
    })
    .ok_or_else(|| LanguageError::BindingNotFound {
        name: declaration.name.clone(),
    })
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

fn replacement_literal(source: &str, value: &str, string_quote: fn(&str) -> String) -> String {
    if source.trim().parse::<i64>().is_ok() && value.trim().parse::<i64>().is_ok() {
        return value.trim().to_owned();
    }
    if source.trim().parse::<f64>().is_ok() && value.trim().parse::<f64>().is_ok() {
        return value.trim().to_owned();
    }
    if matches!(source.trim(), "true" | "false" | "True" | "False") {
        let lower = value.trim().to_ascii_lowercase();
        if matches!(lower.as_str(), "true" | "false") {
            let title_case = source.starts_with('T') || source.starts_with('F');
            return if title_case {
                if lower == "true" { "True" } else { "False" }.to_owned()
            } else {
                lower
            };
        }
    }
    string_quote(value)
}

fn quote_python_string(value: &str) -> String {
    format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'"))
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
    if !parser_accepts(kind, text) {
        return Vec::new();
    }
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
                python_dependencies(text)
            } else {
                inline
            }
        }
        "js" | "ts" => javascript_dependencies(text),
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

fn parser_accepts(kind: &str, text: &str) -> bool {
    let language = match kind {
        "shell" => Some(tree_sitter_bash::LANGUAGE),
        "js" => Some(tree_sitter_javascript::LANGUAGE),
        "ts" => Some(tree_sitter_typescript::LANGUAGE_TYPESCRIPT),
        _ => None,
    };
    let Some(language) = language else {
        return true;
    };
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&language.into()).is_ok()
        && parser
            .parse(text, None)
            .is_some_and(|tree| !tree.root_node().has_error())
}

fn python_dependencies(text: &str) -> Vec<String> {
    let mut output = BTreeSet::new();
    for line in text.lines().map(str::trim) {
        if let Some(imports) = line.strip_prefix("import ") {
            for item in imports.split(',') {
                if let Some(name) = item.split_whitespace().next() {
                    add_python_dependency(&mut output, name);
                }
            }
        } else if let Some(from) = line.strip_prefix("from ")
            && let Some(name) = from.split_whitespace().next()
            && !name.starts_with('.')
        {
            add_python_dependency(&mut output, name);
        }
    }
    output.into_iter().collect()
}

fn add_python_dependency(output: &mut BTreeSet<String>, import: &str) {
    let name = import.split('.').next().unwrap_or_default();
    if !name.is_empty() && !PYTHON_STDLIB.contains(&name) {
        output.insert(name.to_owned());
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

fn javascript_dependencies(text: &str) -> Vec<String> {
    let mut output = BTreeSet::new();
    for captures in JS_IMPORT.captures_iter(text) {
        if let Some(specifier) = captures.get(1).or_else(|| captures.get(2))
            && let Some(package) = package_name(specifier.as_str())
        {
            output.insert(package);
        }
    }
    for captures in JS_REQUIRE.captures_iter(text) {
        if let Some(specifier) = captures.get(1)
            && let Some(package) = package_name(specifier.as_str())
        {
            output.insert(package);
        }
    }
    output.into_iter().collect()
}

fn package_name(specifier: &str) -> Option<String> {
    if specifier.starts_with('.')
        || specifier.starts_with('/')
        || specifier.starts_with("node:")
        || specifier.starts_with("http:")
        || specifier.starts_with("https:")
    {
        return None;
    }
    if specifier.starts_with('@') {
        let mut parts = specifier.split('/');
        let scope = parts.next()?;
        let package = parts.next()?;
        return Some(format!("{scope}/{package}"));
    }
    specifier.split('/').next().map(str::to_owned)
}

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
        offset = open + 1;
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
            index += 1;
            continue;
        }
        if character == '\\' && quote.is_some() {
            escaped = true;
            index += 1;
            continue;
        }
        if matches!(character, '\'' | '"') {
            if quote == Some(character) {
                quote = None;
            } else if quote.is_none() {
                quote = Some(character);
            }
            index += 1;
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
        index += 1;
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
    parse_parameter_value(value, parameter_type).or_else(|| {
        matches!(
            parameter_type,
            ParameterType::Str | ParameterType::Choice | ParameterType::Path
        )
        .then(|| ParameterValue::String(value.trim_matches(&['\'', '"'][..]).to_owned()))
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
