//! Deterministic value-token expansion for parameter and extra-argument assembly.
//!
//! The application layer receives all ambient state explicitly: working directory, current-user
//! home, environment values, date, and time. Stored values therefore remain intent strings while
//! callers can expand them afresh for every run without reading process-global state here.

use std::collections::BTreeMap;

use thiserror::Error;

/// All ambient values needed by the token scanner.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TokenContext {
    /// Invoke-time working directory rendered with the caller's native path spelling.
    pub cwd: String,
    /// Current user's home directory; `None` leaves a leading current-user tilde unchanged.
    pub home: Option<String>,
    /// Child-environment source for `{env:NAME}`.
    pub env: BTreeMap<String, String>,
    /// Preformatted local date (`YYYY-MM-DD`).
    pub today: String,
    /// Preformatted local time (`HH-MM-SS`).
    pub now: String,
}

/// A known token could not be resolved.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum TokenError {
    /// The named environment value was absent.
    #[error("The environment variable {name} isn't set (needed by {token}).")]
    MissingEnvironment {
        /// Bare environment-variable name.
        name: String,
        /// Full token spelling from the input.
        token: String,
    },
}

/// Expand known value tokens with one scanner pass.
///
/// Unknown brace expressions pass through untouched. When `brace_escapes` is true, `{{` and `}}`
/// halve to literal braces. When false, each pair stays byte-identical and token matching is skipped
/// inside that pair. Named tokens expand in either mode.
pub fn expand(
    text: &str,
    context: &TokenContext,
    brace_escapes: bool,
) -> Result<String, TokenError> {
    let expanded_home = expand_current_user_home(text, context.home.as_deref());
    let text = expanded_home.as_deref().unwrap_or(text);
    let bytes = text.as_bytes();
    let mut output = String::with_capacity(text.len());
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index..].starts_with(b"{{") {
            output.push_str(if brace_escapes { "{" } else { "{{" });
            index += 2;
            continue;
        }
        if bytes[index..].starts_with(b"}}") {
            output.push_str(if brace_escapes { "}" } else { "}}" });
            index += 2;
            continue;
        }
        if bytes[index] == b'{' {
            if let Some((replacement, end)) = resolve_at(text, index, context)? {
                output.push_str(replacement);
                index = end;
                continue;
            }
        }

        let character = text[index..]
            .chars()
            .next()
            .expect("index is inside the string");
        output.push(character);
        index += character.len_utf8();
    }

    Ok(output)
}

/// Non-raising expansion for live previews.
///
/// On failure the original, unexpanded text is returned together with the exact user-ready error.
#[must_use]
pub fn preview(
    text: &str,
    context: &TokenContext,
    brace_escapes: bool,
) -> (String, Option<String>) {
    match expand(text, context, brace_escapes) {
        Ok(expanded) => (expanded, None),
        Err(error) => (text.to_owned(), Some(error.to_string())),
    }
}

/// Whether expansion can act on this value.
#[must_use]
pub fn has_tokens(text: &str) -> bool {
    text.starts_with('~')
        || text.contains("{{")
        || text.contains("}}")
        || known_token_start(text).is_some()
}

fn expand_current_user_home(text: &str, home: Option<&str>) -> Option<String> {
    let home = home?;
    if text == "~" {
        return Some(home.to_owned());
    }
    let tail = text.strip_prefix("~/").or_else(|| text.strip_prefix("~\\"))?;
    let separator = text.as_bytes().get(1).copied().unwrap_or(b'/') as char;
    Some(format!("{home}{separator}{tail}"))
}

fn resolve_at<'a>(
    text: &'a str,
    start: usize,
    context: &'a TokenContext,
) -> Result<Option<(&'a str, usize)>, TokenError> {
    let rest = &text[start..];
    for (token, value) in [
        ("{cwd}", context.cwd.as_str()),
        ("{today}", context.today.as_str()),
        ("{now}", context.now.as_str()),
    ] {
        if rest.starts_with(token) {
            return Ok(Some((value, start + token.len())));
        }
    }

    let Some(after_prefix) = rest.strip_prefix("{env:") else {
        return Ok(None);
    };
    let Some(close) = after_prefix.find('}') else {
        return Ok(None);
    };
    let name = &after_prefix[..close];
    if !valid_environment_name(name) {
        return Ok(None);
    }
    let token_len = "{env:".len() + name.len() + 1;
    let token = &rest[..token_len];
    let value = context
        .env
        .get(name)
        .ok_or_else(|| TokenError::MissingEnvironment {
            name: name.to_owned(),
            token: token.to_owned(),
        })?;
    Ok(Some((value, start + token_len)))
}

fn known_token_start(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index..].starts_with(b"{{") {
            index += 2;
            continue;
        }
        if bytes[index] == b'{' && known_token_len(&text[index..]).is_some() {
            return Some(index);
        }
        let character = text[index..].chars().next()?;
        index += character.len_utf8();
    }
    None
}

fn known_token_len(rest: &str) -> Option<usize> {
    for token in ["{cwd}", "{today}", "{now}"] {
        if rest.starts_with(token) {
            return Some(token.len());
        }
    }
    let after_prefix = rest.strip_prefix("{env:")?;
    let close = after_prefix.find('}')?;
    let name = &after_prefix[..close];
    valid_environment_name(name).then_some("{env:".len() + name.len() + 1)
}

fn valid_environment_name(name: &str) -> bool {
    let mut characters = name.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && characters.all(|character| character.is_ascii_alphanumeric() || character == '_')
}
