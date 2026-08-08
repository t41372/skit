use std::collections::BTreeMap;

/// The dependency metadata skit reads from an inline PEP 723 script block.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Pep723Metadata {
    pub dependencies: Vec<String>,
    pub requires_python: String,
    /// Unknown keys and nested tables are retained for callers that later need to
    /// rewrite only skit's dependency axes without destroying user metadata.
    pub extra: BTreeMap<String, toml::Value>,
}

/// Parse one PEP 723-style comment block. Returns `None` for absent or malformed
/// metadata, matching the current Python read path's tolerant behavior.
#[must_use]
pub fn parse_pep723(text: &str, leader: &str) -> Option<Pep723Metadata> {
    let block = block_body(text, leader)?;
    let document = toml::from_str::<toml::Table>(&block).ok()?;
    let dependencies = document
        .get("dependencies")
        .and_then(toml::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(toml::Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let requires_python = document
        .get("requires-python")
        .and_then(toml::Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let extra = document
        .into_iter()
        .filter(|(key, _)| !matches!(key.as_str(), "dependencies" | "requires-python"))
        .collect();
    Some(Pep723Metadata {
        dependencies,
        requires_python,
        extra,
    })
}

/// Whether a syntactically recognizable block is present, even if its TOML body is
/// malformed. Writers use presence rather than parse success so they never duplicate a
/// hand-broken metadata block above the original.
#[must_use]
pub fn has_pep723(text: &str, leader: &str) -> bool {
    block_bounds(text, leader).is_some()
}

/// Generate a PEP 723 comment block with stable TOML escaping.
#[must_use]
pub fn build_pep723(dependencies: &[String], requires_python: &str, leader: &str) -> String {
    let mut lines = vec![format!("{leader} /// script")];
    if !requires_python.is_empty() {
        lines.push(format!(
            "{leader} requires-python = {}",
            toml_basic_string(requires_python)
        ));
    }
    if dependencies.is_empty() {
        lines.push(format!("{leader} dependencies = []"));
    } else {
        lines.push(format!("{leader} dependencies = ["));
        lines.extend(
            dependencies
                .iter()
                .map(|dependency| format!("{leader}     {},", toml_basic_string(dependency))),
        );
        lines.push(format!("{leader} ]"));
    }
    lines.push(format!("{leader} ///"));
    lines.join("\n") + "\n"
}

/// Insert a new block after a shebang and Python coding declaration. Existing blocks
/// are left byte-identical. The inserted block follows the source's newline style.
#[must_use]
pub fn inject_pep723(
    text: &str,
    dependencies: &[String],
    requires_python: &str,
    leader: &str,
) -> String {
    if has_pep723(text, leader) {
        return text.to_owned();
    }
    let newline = source_newline(text);
    let block = build_pep723(dependencies, requires_python, leader).replace('\n', newline);
    let lines = physical_lines(text);
    let mut insert_at = 0;
    if lines
        .first()
        .is_some_and(|line| line.content.starts_with("#!"))
    {
        insert_at = 1;
    }
    if leader == "#"
        && lines
            .get(insert_at)
            .is_some_and(|line| coding_declaration(line.content))
    {
        insert_at += 1;
    }
    let byte_offset = lines.get(insert_at).map_or(text.len(), |line| line.start);
    let prefix = &text[..byte_offset];
    let suffix = &text[byte_offset..];
    let separator = if suffix.is_empty() || suffix.starts_with('\r') || suffix.starts_with('\n') {
        ""
    } else {
        newline
    };
    format!("{prefix}{block}{separator}{suffix}")
}

/// Replace the dependency and Python-version axes inside an existing metadata block while
/// retaining every other comment/TOML line. With no block this falls back to injection.
///
/// The dependency array remover tracks structural brackets only: brackets inside quoted
/// requirement strings or trailing TOML comments cannot extend or truncate the removal span.
/// The rewritten axes follow the source's newline style; retained lines stay byte-identical.
#[must_use]
pub fn set_pep723_axes(
    text: &str,
    dependencies: &[String],
    requires_python: &str,
    leader: &str,
) -> String {
    let Some((start, end)) = block_bounds(text, leader) else {
        return inject_pep723(text, dependencies, requires_python, leader);
    };
    let newline = source_newline(&text[start..end]);
    let block = &text[start..end];
    let lines = block.split_inclusive('\n').collect::<Vec<_>>();
    if lines.len() < 2 {
        return inject_pep723(text, dependencies, requires_python, leader);
    }

    let mut kept = Vec::new();
    let mut in_dependencies = false;
    let mut depth = 0_i32;
    for raw_line in lines.iter().skip(1).take(lines.len().saturating_sub(2)) {
        let Some(content) = strip_comment_prefix(raw_line, leader) else {
            kept.push(*raw_line);
            continue;
        };
        if in_dependencies {
            depth += structural_bracket_delta(content);
            if depth <= 0 {
                in_dependencies = false;
            }
            continue;
        }
        let stripped = content.trim();
        if stripped.starts_with("requires-python") {
            continue;
        }
        if stripped.starts_with("dependencies") {
            let net = structural_bracket_delta(stripped);
            if net > 0 {
                in_dependencies = true;
                depth = net;
            }
            continue;
        }
        kept.push(*raw_line);
    }

    let mut rewritten = String::new();
    rewritten.push_str(leader);
    rewritten.push_str(" /// script");
    rewritten.push_str(newline);
    if !requires_python.is_empty() {
        rewritten.push_str(leader);
        rewritten.push_str(" requires-python = ");
        rewritten.push_str(&toml_basic_string(requires_python));
        rewritten.push_str(newline);
    }
    if dependencies.is_empty() {
        rewritten.push_str(leader);
        rewritten.push_str(" dependencies = []");
        rewritten.push_str(newline);
    } else {
        rewritten.push_str(leader);
        rewritten.push_str(" dependencies = [");
        rewritten.push_str(newline);
        for dependency in dependencies {
            rewritten.push_str(leader);
            rewritten.push_str("     ");
            rewritten.push_str(&toml_basic_string(dependency));
            rewritten.push(',');
            rewritten.push_str(newline);
        }
        rewritten.push_str(leader);
        rewritten.push_str(" ]");
        rewritten.push_str(newline);
    }
    for line in kept {
        rewritten.push_str(line);
    }
    rewritten.push_str(leader);
    rewritten.push_str(" ///");
    if block.ends_with('\n') {
        rewritten.push_str(newline);
    }

    let mut output = String::with_capacity(text.len() + rewritten.len());
    output.push_str(&text[..start]);
    output.push_str(&rewritten);
    output.push_str(&text[end..]);
    output
}

fn block_body(text: &str, leader: &str) -> Option<String> {
    let (start, end) = block_bounds(text, leader)?;
    let block = &text[start..end];
    let mut lines = block.lines();
    lines.next()?;
    let closer = format!("{leader} ///");
    let prefix = format!("{leader} ");
    let mut body = Vec::new();
    for line in lines {
        let clean = line.trim_end_matches('\r');
        if clean.trim_end_matches([' ', '\t']) == closer {
            break;
        }
        if clean == leader {
            body.push(String::new());
        } else {
            let rest = clean.strip_prefix(&prefix)?;
            body.push(rest.to_owned());
        }
    }
    Some(body.join("\n"))
}

fn block_bounds(text: &str, leader: &str) -> Option<(usize, usize)> {
    let opener = format!("{leader} /// script");
    let closer = format!("{leader} ///");
    let prefix = format!("{leader} ");
    let lines = physical_lines(text);
    let mut opening = None;
    for (index, line) in lines.iter().enumerate() {
        let content = line.content.trim_end_matches([' ', '\t', '\r']);
        if opening.is_none() {
            if content == opener {
                opening = Some(line.start);
            }
            continue;
        }
        if content == closer {
            let end = lines.get(index + 1).map_or(text.len(), |next| next.start);
            return opening.map(|start| (start, end));
        }
        if content != leader && !content.starts_with(&prefix) {
            opening = (content == opener).then_some(line.start);
        }
    }
    None
}

fn strip_comment_prefix<'a>(line: &'a str, leader: &str) -> Option<&'a str> {
    let clean = line.trim_end_matches(['\r', '\n']);
    if clean == leader {
        return Some("");
    }
    clean.strip_prefix(leader)?.strip_prefix(' ')
}

fn structural_bracket_delta(text: &str) -> i32 {
    let mut delta = 0_i32;
    let mut quote = None;
    let mut escaped = false;
    for character in text.chars() {
        if let Some(active_quote) = quote {
            if active_quote == '"' && escaped {
                escaped = false;
                continue;
            }
            if active_quote == '"' && character == '\\' {
                escaped = true;
                continue;
            }
            if character == active_quote {
                quote = None;
            }
            continue;
        }
        match character {
            '\'' | '"' => quote = Some(character),
            '#' => break,
            '[' => delta += 1,
            ']' => delta -= 1,
            _ => {}
        }
    }
    delta
}

#[derive(Debug, Clone, Copy)]
struct PhysicalLine<'a> {
    start: usize,
    content: &'a str,
}

fn physical_lines(text: &str) -> Vec<PhysicalLine<'_>> {
    let mut output = Vec::new();
    let mut start = 0;
    for line in text.split_inclusive('\n') {
        let content = line.trim_end_matches('\n');
        output.push(PhysicalLine { start, content });
        start += line.len();
    }
    if start < text.len() || text.is_empty() {
        output.push(PhysicalLine {
            start,
            content: &text[start..],
        });
    }
    output
}

fn coding_declaration(line: &str) -> bool {
    let line = line.trim_end_matches(['\r', '\n']);
    line.starts_with('#') && (line.contains("coding:") || line.contains("coding="))
}

fn source_newline(text: &str) -> &'static str {
    if text.contains("\r\n") { "\r\n" } else { "\n" }
}

fn toml_basic_string(value: &str) -> String {
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
