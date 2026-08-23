//! Prompt-runner command rules shared by all frontends and storage adapters.

use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

/// Quoting convention used by one editable argv line.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EditableArgvDialect {
    /// POSIX `shlex` syntax on Unix-like hosts.
    Posix,
    /// Microsoft C runtime argument syntax on Windows.
    Windows,
}

impl EditableArgvDialect {
    /// Return the host platform's editable argument convention.
    #[must_use]
    #[cfg(windows)]
    pub const fn host() -> Self {
        Self::Windows
    }

    /// Return the host platform's editable argument convention.
    #[must_use]
    #[cfg(not(windows))]
    pub const fn host() -> Self {
        Self::Posix
    }
}

/// One invalid runner command that a form can correct.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RunnerCommandError {
    /// The one-line representation has an open quote.
    UnbalancedQuotes,
    /// The command has no program or contains an empty argument.
    EmptyCommand,
    /// `{{prompt}}` does not occur exactly once.
    PromptSlotCount,
    /// `{{prompt}}` occurs in argv zero.
    PromptInProgram,
    /// A double-brace hole other than `{{prompt}}` occurs.
    UnsupportedHole,
}

/// One invalid direct argv runner template.
///
/// This type does not contain editable-line syntax failures. Stored argv adapters
/// can use it without inventing a result for quote parsing.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RunnerArgvError {
    /// The command has no program or contains an empty argument.
    EmptyCommand,
    /// `{{prompt}}` does not occur exactly once.
    PromptSlotCount,
    /// `{{prompt}}` occurs in argv zero.
    PromptInProgram,
    /// A double-brace hole other than `{{prompt}}` occurs.
    UnsupportedHole,
}

impl RunnerArgvError {
    /// Return the stable configuration reason used by CLI and JSON surfaces.
    #[must_use]
    pub const fn reason_code(self) -> &'static str {
        match self {
            Self::EmptyCommand => "empty",
            Self::PromptSlotCount => "prompt-slot-count",
            Self::PromptInProgram => "prompt-in-binary",
            Self::UnsupportedHole => "stray-hole",
        }
    }
}

impl From<RunnerArgvError> for RunnerCommandError {
    fn from(value: RunnerArgvError) -> Self {
        match value {
            RunnerArgvError::EmptyCommand => Self::EmptyCommand,
            RunnerArgvError::PromptSlotCount => Self::PromptSlotCount,
            RunnerArgvError::PromptInProgram => Self::PromptInProgram,
            RunnerArgvError::UnsupportedHole => Self::UnsupportedHole,
        }
    }
}

impl RunnerCommandError {
    /// Return the stable configuration reason used by CLI and JSON surfaces.
    #[must_use]
    pub const fn reason_code(self) -> &'static str {
        match self {
            Self::UnbalancedQuotes => "unbalanced-quotes",
            Self::EmptyCommand => "empty",
            Self::PromptSlotCount => "prompt-slot-count",
            Self::PromptInProgram => "prompt-in-binary",
            Self::UnsupportedHole => "stray-hole",
        }
    }
}

/// Split one editable command line into the argv that the launcher stores.
///
/// This only decodes an argv representation. A shell never executes the line.
pub fn split_editable_argv(
    value: &str,
    dialect: EditableArgvDialect,
) -> Result<Vec<String>, RunnerCommandError> {
    match dialect {
        EditableArgvDialect::Posix => {
            shlex::split(value).ok_or(RunnerCommandError::UnbalancedQuotes)
        }
        EditableArgvDialect::Windows => {
            validate_windows_quotes(value)?;
            Ok(windows_args::Args::parse_args(value).collect())
        }
    }
}

/// Render argv through the quoting convention paired with [`split_editable_argv`].
#[must_use]
pub fn join_editable_argv(arguments: &[String], dialect: EditableArgvDialect) -> String {
    match dialect {
        EditableArgvDialect::Posix => {
            shlex::try_join(arguments.iter().map(String::as_str)).unwrap_or_default()
        }
        EditableArgvDialect::Windows => join_windows_argv(arguments),
    }
}

/// Validate the runner template contract shared by CLI, TUI, and storage.
pub fn validate_runner_argv(arguments: &[String]) -> Result<(), RunnerArgvError> {
    if arguments.is_empty() || arguments.iter().any(String::is_empty) {
        return Err(RunnerArgvError::EmptyCommand);
    }
    let mut prompt_slots = 0;
    for (index, argument) in arguments.iter().enumerate() {
        for hole in double_brace_holes(argument) {
            if hole != "prompt" {
                return Err(RunnerArgvError::UnsupportedHole);
            }
            if index == 0 {
                return Err(RunnerArgvError::PromptInProgram);
            }
            prompt_slots += 1;
        }
    }
    if prompt_slots != 1 {
        return Err(RunnerArgvError::PromptSlotCount);
    }
    Ok(())
}

fn double_brace_holes(value: &str) -> impl Iterator<Item = &str> {
    static TOKEN: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"\{\{([^{}]*)\}\}").expect("the prompt runner token pattern is valid")
    });
    TOKEN.captures_iter(value).filter_map(|captures| {
        let token = captures.get(0)?;
        let brace_adjacent = token
            .start()
            .checked_sub(1)
            .is_some_and(|index| value.as_bytes()[index] == b'{')
            || value
                .as_bytes()
                .get(token.end())
                .is_some_and(|byte| *byte == b'}');
        (!brace_adjacent).then(|| captures.get(1).expect("the token has one capture").as_str())
    })
}

fn validate_windows_quotes(value: &str) -> Result<(), RunnerCommandError> {
    let mut quoted = false;
    let mut backslashes = 0;
    for character in value.chars() {
        if character == '\\' {
            backslashes += 1;
            continue;
        }
        if character == '"' && backslashes % 2 == 0 {
            quoted = !quoted;
        }
        backslashes = 0;
    }
    if quoted {
        Err(RunnerCommandError::UnbalancedQuotes)
    } else {
        Ok(())
    }
}

fn join_windows_argv(arguments: &[String]) -> String {
    let mut command = String::new();
    for argument in arguments {
        if !command.is_empty() {
            command.push(' ');
        }
        let quote = argument.is_empty() || argument.contains([' ', '\t']);
        if quote {
            command.push('"');
        }
        let mut backslashes = 0;
        for character in argument.chars() {
            if character == '\\' {
                backslashes += 1;
                continue;
            }
            if character == '"' {
                command.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
            } else {
                command.extend(std::iter::repeat_n('\\', backslashes));
            }
            backslashes = 0;
            command.push(character);
        }
        command.extend(std::iter::repeat_n(
            '\\',
            if quote { backslashes * 2 } else { backslashes },
        ));
        if quote {
            command.push('"');
        }
    }
    command
}
