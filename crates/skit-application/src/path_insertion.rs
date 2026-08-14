//! Path insertion rules shared by terminal and future graphical frontends.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// How one accepted filesystem path changes a run-form value.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunPathInsertMode {
    /// Replace a scalar field's complete value.
    Replace,
    /// Append one POSIX-shlex piece to a multiple-value parameter.
    Shlex,
    /// Append one native editable-argv piece to the extra-argument tail.
    Arguments,
}

/// Quoting grammar for an editable argument tail.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArgumentDialect {
    /// POSIX `shlex` syntax.
    Posix,
    /// Microsoft C runtime syntax paired with Python's `subprocess.list2cmdline`.
    Windows,
}

/// A real filesystem path could not be represented in the target text grammar.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum PathInsertionError {
    /// NUL cannot occur in a shell word.
    #[error("a path cannot contain a NUL byte")]
    Nul,
}

/// Apply one picked path with the current platform's editable-argument grammar.
pub fn insert_picked_path(
    existing: &str,
    picked: &str,
    mode: RunPathInsertMode,
) -> Result<String, PathInsertionError> {
    insert_picked_path_for_dialect(existing, picked, mode, current_argument_dialect())
}

/// Apply one picked path with an explicit grammar. This supports deterministic cross-platform
/// compatibility tests without changing the host platform.
pub fn insert_picked_path_for_dialect(
    existing: &str,
    picked: &str,
    mode: RunPathInsertMode,
    dialect: ArgumentDialect,
) -> Result<String, PathInsertionError> {
    if mode == RunPathInsertMode::Replace {
        return Ok(picked.to_owned());
    }

    let literal = escape_glob_metacharacters(picked);
    let piece = match (mode, dialect) {
        (RunPathInsertMode::Shlex, _) | (RunPathInsertMode::Arguments, ArgumentDialect::Posix) => {
            shlex::try_quote(&literal)
                .map_err(|_| PathInsertionError::Nul)?
                .into_owned()
        }
        (RunPathInsertMode::Arguments, ArgumentDialect::Windows) => {
            quote_windows_argument(&literal)
        }
        (RunPathInsertMode::Replace, _) => unreachable!("replace returned before quoting"),
    };
    let existing = existing.trim();
    Ok(if existing.is_empty() {
        piece
    } else {
        format!("{existing} {piece}")
    })
}

fn escape_glob_metacharacters(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        if matches!(character, '*' | '?' | '[') {
            escaped.push('[');
            escaped.push(character);
            escaped.push(']');
        } else {
            escaped.push(character);
        }
    }
    escaped
}

const fn current_argument_dialect() -> ArgumentDialect {
    if cfg!(windows) {
        ArgumentDialect::Windows
    } else {
        ArgumentDialect::Posix
    }
}

fn quote_windows_argument(argument: &str) -> String {
    let quote = argument.is_empty() || argument.contains([' ', '\t']);
    let mut output = String::new();
    if quote {
        output.push('"');
    }
    let mut backslashes = 0_usize;
    for character in argument.chars() {
        if character == '\\' {
            backslashes = backslashes.saturating_add(1);
            continue;
        }
        if character == '"' {
            output.extend(std::iter::repeat_n('\\', backslashes.saturating_mul(2) + 1));
        } else {
            output.extend(std::iter::repeat_n('\\', backslashes));
        }
        backslashes = 0;
        output.push(character);
    }
    output.extend(std::iter::repeat_n(
        '\\',
        if quote {
            backslashes.saturating_mul(2)
        } else {
            backslashes
        },
    ));
    if quote {
        output.push('"');
    }
    output
}
