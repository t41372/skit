use crate::{Binding, Delivery, ParamDecl, ParamDefault, ParamType, parse_pep723};

const LEADER: &str = "#";

/// Render the frozen comment-stripped `[tool.skit]` table carried by existing Python entries.
#[must_use]
pub fn render_python_params(params: &[ParamDecl]) -> String {
    let mut lines = vec!["[tool.skit]".to_owned(), "schema = 1".to_owned()];
    for param in params {
        lines.push(String::new());
        lines.push("[[tool.skit.params]]".to_owned());
        lines.push(format!("name = {}", toml_string(&param.name)));
        lines.push(format!("kind = {}", toml_string(param.binding.as_str())));
        lines.push(format!("type = {}", toml_string(param.param_type.as_str())));
        if let Some(default) = &param.default {
            lines.push(format!("default = {}", render_default(default)));
        }
        if !param.prompt.is_empty() {
            lines.push(format!("prompt = {}", toml_string(&param.prompt)));
        }
        if param.order >= 0 {
            lines.push(format!("order = {}", param.order));
        }
        if param.secret {
            lines.push("secret = true".to_owned());
        }
        if !param.env_source.is_empty() {
            lines.push(format!("env_source = {}", toml_string(&param.env_source)));
        }
    }
    lines.join("\n") + "\n"
}

/// Replace only the frozen `[tool.skit]` section in a Python PEP 723 block.
///
/// Dependency axes and every unrelated tool table are retained line-for-line. With no
/// metadata block and non-empty params, one block is inserted after the shebang/coding
/// declaration with `dependencies=[]`; no extra blank line is added outside the block.
#[must_use]
pub fn write_python_params(text: &str, params: &[ParamDecl]) -> String {
    let Some((start, end)) = python_block_bounds(text) else {
        if params.is_empty() {
            return text.to_owned();
        }
        return insert_new_python_block(text, params);
    };
    let block = &text[start..end];
    let newline = newline_style(block);
    let has_final_newline = block.ends_with('\n');
    let mut physical = block.split_inclusive('\n').collect::<Vec<_>>();
    if physical.len() < 2 {
        return text.to_owned();
    }
    physical.remove(0);
    physical.pop();

    let mut kept = Vec::<String>::new();
    let mut skipping = false;
    for raw in physical {
        let stripped = strip_comment(raw).trim();
        if stripped.starts_with('[') {
            skipping = stripped.starts_with("[tool.skit]") || stripped.starts_with("[[tool.skit.");
        }
        if !skipping {
            kept.push(raw.to_owned());
        }
    }
    while kept
        .last()
        .is_some_and(|line| matches!(line.trim(), "#" | ""))
    {
        kept.pop();
    }

    let mut rewritten = String::new();
    rewritten.push_str("# /// script");
    rewritten.push_str(newline);
    for line in kept {
        rewritten.push_str(&line);
        if !line.ends_with('\n') {
            rewritten.push_str(newline);
        }
    }
    if !params.is_empty() {
        if rewritten != format!("# /// script{newline}") {
            rewritten.push('#');
            rewritten.push_str(newline);
        }
        for line in render_python_params(params).lines() {
            rewritten.push_str("# ");
            rewritten.push_str(line);
            while rewritten.ends_with(' ') {
                rewritten.pop();
            }
            rewritten.push_str(newline);
        }
    }
    rewritten.push_str("# ///");
    if has_final_newline {
        rewritten.push_str(newline);
    }

    let mut output = String::with_capacity(text.len() + rewritten.len());
    output.push_str(&text[..start]);
    output.push_str(&rewritten);
    output.push_str(&text[end..]);
    output
}

/// Read frozen Python `[tool.skit]` declarations. Malformed shapes degrade to an empty list.
#[must_use]
pub fn read_python_params(text: &str) -> Vec<ParamDecl> {
    let Some(metadata) = parse_pep723(text, LEADER) else {
        return Vec::new();
    };
    let Some(tool) = metadata.extra.get("tool").and_then(toml::Value::as_table) else {
        return Vec::new();
    };
    let Some(skit) = tool.get("skit").and_then(toml::Value::as_table) else {
        return Vec::new();
    };
    let Some(params) = skit.get("params").and_then(toml::Value::as_array) else {
        return Vec::new();
    };
    params
        .iter()
        .filter_map(toml::Value::as_table)
        .map(param_from_frozen)
        .filter(|param| !param.name.is_empty())
        .collect()
}

fn param_from_frozen(row: &toml::Table) -> ParamDecl {
    let binding = match scalar_text(row.get("kind")).as_str() {
        "input" => Binding::Input,
        "envdefault" => Binding::EnvDefault,
        "none" => Binding::None,
        _ => Binding::Const,
    };
    let delivery = match binding {
        Binding::Const | Binding::Input => Delivery::Inject,
        Binding::EnvDefault => Delivery::Env,
        Binding::None => Delivery::Flag,
    };
    let param_type = match scalar_text(row.get("type")).as_str() {
        "int" => ParamType::Integer,
        "float" => ParamType::Float,
        "bool" => ParamType::Boolean,
        "choice" => ParamType::Choice,
        "path" => ParamType::Path,
        _ => ParamType::String,
    };
    ParamDecl {
        name: scalar_text(row.get("name")),
        binding,
        delivery,
        param_type,
        default: row.get("default").and_then(frozen_default),
        prompt: scalar_text(row.get("prompt")),
        order: row
            .get("order")
            .and_then(toml::Value::as_integer)
            .unwrap_or(-1),
        secret: scalar_bool(row.get("secret")),
        env_source: scalar_text(row.get("env_source")),
        ..ParamDecl::default()
    }
}

fn frozen_default(value: &toml::Value) -> Option<ParamDefault> {
    match value {
        toml::Value::String(value) => Some(ParamDefault::String(value.clone())),
        toml::Value::Integer(value) => Some(ParamDefault::Integer(*value)),
        toml::Value::Float(value) if value.is_finite() => Some(ParamDefault::Float(*value)),
        toml::Value::Boolean(value) => Some(ParamDefault::Boolean(*value)),
        _ => None,
    }
}

fn scalar_text(value: Option<&toml::Value>) -> String {
    match value {
        Some(toml::Value::String(value)) => value.clone(),
        Some(toml::Value::Integer(value)) => value.to_string(),
        Some(toml::Value::Float(value)) if value.is_finite() => value.to_string(),
        Some(toml::Value::Boolean(value)) => value.to_string(),
        _ => String::new(),
    }
}

fn scalar_bool(value: Option<&toml::Value>) -> bool {
    match value {
        Some(toml::Value::Boolean(value)) => *value,
        Some(toml::Value::Integer(value)) => *value != 0,
        Some(toml::Value::String(value)) => matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "true" | "1" | "yes" | "y" | "on"
        ),
        _ => false,
    }
}

fn insert_new_python_block(text: &str, params: &[ParamDecl]) -> String {
    let newline = newline_style(text);
    let offset = insertion_offset(text);
    let mut block = String::new();
    block.push_str("# /// script");
    block.push_str(newline);
    block.push_str("# dependencies = []");
    block.push_str(newline);
    block.push('#');
    block.push_str(newline);
    for line in render_python_params(params).lines() {
        block.push_str("# ");
        block.push_str(line);
        while block.ends_with(' ') {
            block.pop();
        }
        block.push_str(newline);
    }
    block.push_str("# ///");
    block.push_str(newline);
    format!("{}{}{}", &text[..offset], block, &text[offset..])
}

fn insertion_offset(text: &str) -> usize {
    let lines = physical_lines(text);
    let mut index = 0;
    if lines
        .first()
        .is_some_and(|(_, line)| line.starts_with("#!"))
    {
        index = 1;
    }
    if lines.get(index).is_some_and(|(_, line)| {
        line.starts_with('#') && (line.contains("coding:") || line.contains("coding="))
    }) {
        index += 1;
    }
    lines.get(index).map_or(text.len(), |(offset, _)| *offset)
}

fn python_block_bounds(text: &str) -> Option<(usize, usize)> {
    let lines = physical_lines(text);
    let mut opening = None;
    for (index, (start, raw)) in lines.iter().enumerate() {
        let content = raw.trim_end_matches([' ', '\t', '\r']);
        if opening.is_none() {
            if content == "# /// script" {
                opening = Some(*start);
            }
            continue;
        }
        if content == "# ///" {
            let end = lines.get(index + 1).map_or(text.len(), |(start, _)| *start);
            return opening.map(|start| (start, end));
        }
        if content != "#" && !content.starts_with("# ") {
            opening = (content == "# /// script").then_some(*start);
        }
    }
    None
}

fn physical_lines(text: &str) -> Vec<(usize, &str)> {
    let mut lines = Vec::new();
    let mut start = 0;
    for line in text.split_inclusive('\n') {
        lines.push((start, line.trim_end_matches('\n')));
        start += line.len();
    }
    if start < text.len() || text.is_empty() {
        lines.push((start, &text[start..]));
    }
    lines
}

fn strip_comment(line: &str) -> &str {
    let clean = line.trim_end_matches(['\r', '\n']);
    if clean == "#" {
        ""
    } else {
        clean.strip_prefix("# ").unwrap_or(clean)
    }
}

fn newline_style(text: &str) -> &'static str {
    if text.contains("\r\n") { "\r\n" } else { "\n" }
}

fn render_default(default: &ParamDefault) -> String {
    match default {
        ParamDefault::String(value) => toml_string(value),
        ParamDefault::Integer(value) => value.to_string(),
        ParamDefault::Float(value) => value.to_string(),
        ParamDefault::Boolean(value) => value.to_string(),
    }
}

fn toml_string(value: &str) -> String {
    let mut output = String::with_capacity(value.len() + 2);
    output.push('"');
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
                use std::fmt::Write as _;
                let _ = write!(output, "\\u{:04X}", u32::from(character));
            }
            character => output.push(character),
        }
    }
    output.push('"');
    output
}
