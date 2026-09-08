//! Product-independent traversal and token scans for artifact leak oracles.

use serde_json::Value;

const DRAFT_PREFIX: &str = "skit-new-";
const COUNTER_WIDTH: usize = 6;
const SCRIPT_SUFFIX: &str = ".py";
const PROMPT_SUFFIX: &str = ".prompt.md";

/// The role of a string in a JSON structure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JsonStringRole {
    /// An object member name.
    Key,
    /// A string value.
    Value,
}

/// One string and its stable structural location in a JSON value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JsonStringOccurrence<'a> {
    /// The RFC 6901 pointer to the value or object member.
    pub pointer: String,
    /// Whether the string is a member name or a value.
    pub role: JsonStringRole,
    /// The string contents.
    pub text: &'a str,
}

/// Visit every JSON key and string value in deterministic depth-first order.
///
/// Object keys use ascending UTF-8 order. A key visit precedes the visit of
/// that member's value. Keys and string values for one member have the same
/// pointer and distinct roles.
#[must_use]
pub fn json_string_occurrences(value: &Value) -> Vec<JsonStringOccurrence<'_>> {
    let mut occurrences = Vec::new();
    collect_json_strings(value, "", &mut occurrences);
    occurrences
}

fn collect_json_strings<'a>(
    value: &'a Value,
    pointer: &str,
    occurrences: &mut Vec<JsonStringOccurrence<'a>>,
) {
    match value {
        Value::String(text) => occurrences.push(JsonStringOccurrence {
            pointer: pointer.to_owned(),
            role: JsonStringRole::Value,
            text,
        }),
        Value::Array(values) => {
            for (index, value) in values.iter().enumerate() {
                let child = pointer_with_segment(pointer, &index.to_string());
                collect_json_strings(value, &child, occurrences);
            }
        }
        Value::Object(values) => {
            let mut keys = values.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            for key in keys {
                let child = pointer_with_segment(pointer, &escape_pointer_segment(key));
                occurrences.push(JsonStringOccurrence {
                    pointer: child.clone(),
                    role: JsonStringRole::Key,
                    text: key,
                });
                collect_json_strings(&values[key], &child, occurrences);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn pointer_with_segment(pointer: &str, segment: &str) -> String {
    let mut child = String::with_capacity(pointer.len() + segment.len() + 1);
    child.push_str(pointer);
    child.push('/');
    child.push_str(segment);
    child
}

fn escape_pointer_segment(segment: &str) -> String {
    let mut escaped = String::with_capacity(segment.len());
    for character in segment.chars() {
        match character {
            '~' => escaped.push_str("~0"),
            '/' => escaped.push_str("~1"),
            other => escaped.push(other),
        }
    }
    escaped
}

/// A half-open UTF-8 byte span in text.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct TextSpan {
    /// The inclusive start byte offset.
    pub start: usize,
    /// The exclusive end byte offset.
    pub end: usize,
}

/// Find complete tokens with the host path projection boundaries.
#[must_use]
pub fn exact_fact_spans(text: &str, token: &str) -> Vec<TextSpan> {
    exact_text_spans(text, token)
        .into_iter()
        .filter(|span| accept_host_path_token(&text[..span.start], &text[span.end..]))
        .collect()
}

/// Check the text boundaries around one host path token.
///
/// Projection and exact fact scans use this same predicate. A sentence period or
/// a collapsed-cell marker bounds the right edge only at the end of the text.
#[must_use]
pub fn accept_host_path_token(prefix: &str, suffix: &str) -> bool {
    prefix.chars().next_back().is_none_or(text_path_boundary)
        && (suffix.is_empty()
            || suffix == "."
            || suffix == "~~⟧"
            || suffix.chars().next().is_some_and(text_path_boundary))
}

fn text_path_boundary(character: char) -> bool {
    character.is_whitespace()
        || matches!(
            character,
            '\'' | '"'
                | '('
                | ')'
                | '['
                | ']'
                | '{'
                | '}'
                | ':'
                | ','
                | '='
                | '/'
                | '\\'
                | '，'
                | '。'
                | '；'
                | '：'
                | '！'
                | '？'
                | '、'
                | '（'
                | '）'
                | '【'
                | '】'
                | '「'
                | '」'
                | '『'
                | '』'
                | '《'
                | '》'
        )
}

/// Find every exact occurrence of a nonempty string, including overlaps.
#[must_use]
pub fn exact_text_spans(text: &str, needle: &str) -> Vec<TextSpan> {
    if needle.is_empty() {
        return Vec::new();
    }
    let mut spans = Vec::new();
    let mut search_start = 0;
    while let Some(relative_start) = text[search_start..].find(needle) {
        let start = search_start + relative_start;
        spans.push(TextSpan {
            start,
            end: start + needle.len(),
        });
        let character_width = text[start..]
            .chars()
            .next()
            .expect("a nonempty match starts with a character")
            .len_utf8();
        search_start = start + character_width;
    }
    spans
}

/// One accepted deterministic draft allocator form.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DraftAllocatorForm {
    /// A bare allocator stem.
    Stem,
    /// An allocator stem with the `.py` suffix.
    Script,
    /// An allocator stem with the `.prompt.md` suffix.
    Prompt,
}

/// The reason an allocator token does not match the closed grammar.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DraftAllocatorError {
    /// The token does not start with `skit-new-`.
    Prefix,
    /// The counter does not contain exactly six characters.
    CounterWidth,
    /// The counter contains a character outside lowercase ASCII hexadecimal.
    CounterCharacter,
    /// The suffix is not empty, `.py`, or `.prompt.md`.
    Suffix,
}

/// Parse one complete token from the closed draft allocator grammar.
pub fn parse_draft_allocator_token(token: &str) -> Result<DraftAllocatorForm, DraftAllocatorError> {
    let remainder = token
        .strip_prefix(DRAFT_PREFIX)
        .ok_or(DraftAllocatorError::Prefix)?;
    let suffix_start = remainder.find('.').unwrap_or(remainder.len());
    let (counter, suffix) = remainder.split_at(suffix_start);
    if counter.len() != COUNTER_WIDTH {
        return Err(DraftAllocatorError::CounterWidth);
    }
    if !counter
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(DraftAllocatorError::CounterCharacter);
    }
    match suffix {
        "" => Ok(DraftAllocatorForm::Stem),
        SCRIPT_SUFFIX => Ok(DraftAllocatorForm::Script),
        PROMPT_SUFFIX => Ok(DraftAllocatorForm::Prompt),
        _ => Err(DraftAllocatorError::Suffix),
    }
}
