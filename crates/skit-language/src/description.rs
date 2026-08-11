//! Derive add-time description suggestions without changing source bytes.

use crate::{ParseOutcome, parse_document};

const PROMPT_DESCRIPTION_LIMIT: usize = 120;

/// Suggest one short description from an exact source snapshot.
///
/// This function uses the latest-main replacement view for arbitrary interpreted bytes. It never
/// returns rewritten source text. Python uses its parsed module docstring. Comment languages use
/// the first prose line in their leading comment block. Prompt entries use their first non-empty
/// Markdown line and keep at most 120 Unicode scalar values.
#[must_use]
pub fn suggest_description(kind: &str, source: &[u8]) -> String {
    let text = String::from_utf8_lossy(source);
    match kind {
        "python" => {
            let ParseOutcome::Parsed(document) = parse_document("python", text.as_ref()) else {
                return String::new();
            };
            document.python_module_description().unwrap_or_default()
        }
        "prompt" => prompt_description(&text),
        "shell" | "fish" | "powershell" | "ruby" | "perl" | "r" => comment_description(&text, "#"),
        "js" | "ts" => comment_description(&text, "//"),
        "lua" => comment_description(&text, "--"),
        _ => String::new(),
    }
}

fn comment_description(text: &str, prefix: &str) -> String {
    for (index, line) in text.lines().enumerate() {
        let stripped = line.trim();
        if index == 0 && stripped.starts_with("#!") {
            continue;
        }
        if stripped.is_empty() {
            continue;
        }
        let Some(content) = stripped.strip_prefix(prefix) else {
            return String::new();
        };
        let content = content.trim();
        if content.starts_with("///") || content.is_empty() {
            continue;
        }
        return content.to_owned();
    }
    String::new()
}

fn prompt_description(text: &str) -> String {
    let Some(line) = text
        .lines()
        .map(|line| line.trim().trim_start_matches('#').trim())
        .find(|line| !line.is_empty())
    else {
        return String::new();
    };
    if line.chars().count() <= PROMPT_DESCRIPTION_LIMIT {
        return line.to_owned();
    }
    let mut description = line
        .chars()
        .take(PROMPT_DESCRIPTION_LIMIT - 1)
        .collect::<String>();
    let trimmed = description.trim_end().len();
    description.truncate(trimmed);
    description.push('\u{2026}');
    description
}
