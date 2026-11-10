//! Derive add-time description and name suggestions without changing source bytes.

use skit_i18n::{Locale, text};

use crate::{ParseOutcome, parse_document};

const PROMPT_DESCRIPTION_LIMIT: usize = 120;
/// The most words that a suggested prompt name keeps.
const PROMPT_NAME_WORDS: usize = 4;
/// The most Unicode scalar values that a suggested prompt name keeps.
const PROMPT_NAME_LIMIT: usize = 32;
/// The sentence marks that a suggested prompt name does not end with.
const PROMPT_NAME_END_MARKS: &[char] = &[
    '.', ',', ':', ';', '!', '?', '\u{2026}', '\u{2014}', '\u{2013}', '-', '\u{3002}', '\u{ff0c}',
    '\u{ff1a}', '\u{ff1b}', '\u{ff01}', '\u{ff1f}',
];

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

/// Suggest a name for a new prompt draft from the first words that the person wrote.
///
/// A new draft lives in a temporary file, and its file name means nothing to the person (C12 of
/// the 2026-11 audit). The name comes from the same line as the description: the first non-empty
/// Markdown line after the starter line that skit writes. It keeps at most four words and 32
/// Unicode scalar values, drops the braces of `{{name}}` placeholders and the `*` and `` ` ``
/// marks, and does not end with a sentence mark. A line with no letter or digit names nothing, so
/// the next line is used. `None` means that no line names the prompt.
#[must_use]
pub fn suggest_prompt_name(source: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(source);
    prompt_lines(&text).find_map(prompt_name)
}

/// Iterate the trimmed, non-empty prompt lines after any starter line, without heading marks.
fn prompt_lines(text: &str) -> impl Iterator<Item = &str> {
    text.lines()
        .map(str::trim)
        .filter(|line| !is_new_prompt_starter(line))
        .map(|line| line.trim_start_matches('#').trim())
        .filter(|line| !line.is_empty())
}

/// Report whether a line is the heading that skit writes into a new prompt draft.
fn is_new_prompt_starter(line: &str) -> bool {
    [Locale::En, Locale::ZhCn, Locale::ZhTw, Locale::Pseudo]
        .into_iter()
        .any(|locale| text(locale, "# New prompt").trim() == line)
}

fn prompt_name(line: &str) -> Option<String> {
    let cleaned = line
        .replace("{{", "")
        .replace("}}", "")
        .replace(['*', '`'], "");
    let mut name = String::new();
    for word in cleaned.split_whitespace().take(PROMPT_NAME_WORDS) {
        let length = name.chars().count();
        if name.is_empty() {
            name = word.chars().take(PROMPT_NAME_LIMIT).collect();
        } else if length + 1 + word.chars().count() <= PROMPT_NAME_LIMIT {
            name.push(' ');
            name.push_str(word);
        } else {
            break;
        }
    }
    let name = name
        .trim_start_matches(['>', '-', '+'])
        .trim_end_matches(PROMPT_NAME_END_MARKS)
        .trim();
    name.chars()
        .any(char::is_alphanumeric)
        .then(|| name.to_owned())
}

fn prompt_description(text: &str) -> String {
    let Some(line) = prompt_lines(text).next() else {
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
