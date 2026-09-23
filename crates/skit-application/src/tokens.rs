//! Deterministic value-token expansion for parameter and extra-argument assembly.
//!
//! The application layer receives all ambient state explicitly: working directory, current-user
//! home, environment values, date, and time. Stored values therefore remain intent strings while
//! callers can expand them afresh for every run without reading process-global state here.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use skit_i18n::{Localize, Message};
use thiserror::Error;

/// All ambient values needed by the token scanner.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
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

impl Localize for TokenError {
    fn message(&self) -> Message {
        match self {
            Self::MissingEnvironment { name, token } => {
                Message::new("The environment variable {} isn't set (needed by {}).")
                    .with(name)
                    .with(token)
            }
        }
    }
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
    let mut output = String::with_capacity(text.len());
    let mut rest = text;

    while !rest.is_empty() {
        if let Some(tail) = rest.strip_prefix("{{") {
            output.push_str(if brace_escapes { "{" } else { "{{" });
            rest = tail;
            continue;
        }
        if let Some(tail) = rest.strip_prefix("}}") {
            output.push_str(if brace_escapes { "}" } else { "}}" });
            rest = tail;
            continue;
        }
        if rest.starts_with('{')
            && let Some((replacement, tail)) = resolve_prefix(rest, context)?
        {
            output.push_str(replacement);
            rest = tail;
            continue;
        }

        let character = rest
            .chars()
            .next()
            .expect("the remaining text is not empty");
        output.push(character);
        rest = rest
            .strip_prefix(character)
            .expect("the character is the prefix of the remaining text");
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

/// Return the typed failure form for a frontend that localizes after serialization.
#[must_use]
pub fn preview_typed(
    text: &str,
    context: &TokenContext,
    brace_escapes: bool,
) -> (String, Option<TokenError>) {
    match expand(text, context, brace_escapes) {
        Ok(expanded) => (expanded, None),
        Err(error) => (text.to_owned(), Some(error)),
    }
}

/// Whether expansion can act on this value.
#[must_use]
pub fn has_tokens(text: &str) -> bool {
    text.starts_with('~')
        || contains_known_token(text)
        || text.contains("{{")
        || text.contains("}}")
}

/// Build one environment token with the same name grammar used by expansion.
#[must_use]
pub fn environment_token(name: &str) -> Option<String> {
    valid_environment_name(name).then(|| format!("{{env:{name}}}"))
}

fn expand_current_user_home(text: &str, home: Option<&str>) -> Option<String> {
    let home = home?;
    if text == "~" {
        return Some(home.to_owned());
    }
    let tail = text
        .strip_prefix("~/")
        .or_else(|| text.strip_prefix("~\\"))?;
    let separator = text.as_bytes().get(1).copied().unwrap_or(b'/') as char;
    Some(format!("{home}{separator}{tail}"))
}

fn resolve_prefix<'text, 'context>(
    rest: &'text str,
    context: &'context TokenContext,
) -> Result<Option<(&'context str, &'text str)>, TokenError> {
    for (token, value) in [
        ("{cwd}", context.cwd.as_str()),
        ("{today}", context.today.as_str()),
        ("{now}", context.now.as_str()),
    ] {
        if let Some(tail) = rest.strip_prefix(token) {
            return Ok(Some((value, tail)));
        }
    }

    let Some(after_prefix) = rest.strip_prefix("{env:") else {
        return Ok(None);
    };
    let Some((name, tail)) = after_prefix.split_once('}') else {
        return Ok(None);
    };
    if !valid_environment_name(name) {
        return Ok(None);
    }
    let token = format!("{{env:{name}}}");
    let value = context
        .env
        .get(name)
        .ok_or_else(|| TokenError::MissingEnvironment {
            name: name.to_owned(),
            token,
        })?;
    Ok(Some((value, tail)))
}

fn contains_known_token(text: &str) -> bool {
    let mut rest = text;
    while !rest.is_empty() {
        if let Some(tail) = rest.strip_prefix("{{") {
            rest = tail;
            continue;
        }
        if rest.starts_with('{') && is_known_token(rest) {
            return true;
        }
        let character = rest
            .chars()
            .next()
            .expect("the remaining text is not empty");
        rest = rest
            .strip_prefix(character)
            .expect("the character is the prefix of the remaining text");
    }
    false
}

fn is_known_token(rest: &str) -> bool {
    for token in ["{cwd}", "{today}", "{now}"] {
        if rest.starts_with(token) {
            return true;
        }
    }
    let Some(after_prefix) = rest.strip_prefix("{env:") else {
        return false;
    };
    let Some((name, _)) = after_prefix.split_once('}') else {
        return false;
    };
    valid_environment_name(name)
}

fn valid_environment_name(name: &str) -> bool {
    let mut characters = name.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && characters.all(|character| character.is_ascii_alphanumeric() || character == '_')
}
