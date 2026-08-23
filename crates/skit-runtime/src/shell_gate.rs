//! Apply the optional interpreter syntax gate to staged shell sources.

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    time::Duration,
};

use skit_i18n::{Localize, Message};
use thiserror::Error;

use crate::{InjectedCommand, InjectedCommandRunner};

/// Maximum time for the optional `<shell> -n` process.
pub const SHELL_SYNTAX_GATE_TIMEOUT: Duration = Duration::from_secs(30);

/// Hold the user-facing interpreter identity and exact executable selected by launch resolution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedShellInterpreter {
    name: String,
    program: PathBuf,
}

impl ResolvedShellInterpreter {
    /// Create one resolved shell identity.
    #[must_use]
    pub fn new(name: impl Into<String>, program: PathBuf) -> Self {
        Self {
            name: name.into(),
            program,
        }
    }

    /// Return the interpreter spelling selected by entry settings or the shell default.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Return the exact executable path selected by the launch probe.
    #[must_use]
    pub fn program(&self) -> &Path {
        &self.program
    }
}

/// Report that the resolved shell rejected one staged injected source.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error("{shell} rejected the injected copy: {detail}")]
pub struct ShellSyntaxError {
    shell: String,
    detail: String,
}

impl ShellSyntaxError {
    /// Return the entry's interpreter spelling.
    #[must_use]
    pub fn shell(&self) -> &str {
        &self.shell
    }

    /// Return the first captured stderr line, which can be empty.
    #[must_use]
    pub fn detail(&self) -> &str {
        &self.detail
    }
}

impl Localize for ShellSyntaxError {
    fn message(&self) -> Message {
        Message::new("{} rejected the injected copy: {}")
            .with(&self.shell)
            .with(&self.detail)
    }
}

/// Apply the best-effort interpreter syntax gate to one staged shell source.
///
/// A missing interpreter is owned by launch preflight. Spawn, capture, wait, and timeout failures
/// are non-fatal because the mandatory parser-backed gate has already accepted the rewritten text.
pub fn check_shell_syntax<R: InjectedCommandRunner>(
    interpreter: Option<&ResolvedShellInterpreter>,
    source: &Path,
    runner: &R,
) -> Result<(), ShellSyntaxError> {
    let Some(interpreter) = interpreter else {
        return Ok(());
    };
    let command = InjectedCommand {
        program: interpreter.program.clone(),
        args: vec![OsString::from("-n"), source.as_os_str().to_owned()],
        timeout: SHELL_SYNTAX_GATE_TIMEOUT,
    };
    match runner.run(&command) {
        Err(_) => Ok(()),
        Ok(output) if output.success => Ok(()),
        Ok(output) => {
            let decoded = String::from_utf8_lossy(&output.stderr);
            let detail = decoded.trim().lines().next().unwrap_or("").to_owned();
            Err(ShellSyntaxError {
                shell: interpreter.name.clone(),
                detail,
            })
        }
    }
}

/// Keep an owned staged shell source only when the optional interpreter gate accepts it.
pub fn retain_shell_source_if_valid<T, R: InjectedCommandRunner>(
    source: T,
    interpreter: Option<&ResolvedShellInterpreter>,
    path: &Path,
    runner: &R,
) -> Result<T, ShellSyntaxError> {
    check_shell_syntax(interpreter, path, runner)?;
    Ok(source)
}

/// Return the v0.4 warning for a staged shell source that reads its own location.
#[must_use]
pub fn shell_self_location_warning(uses_self_location: bool) -> Option<Message> {
    uses_self_location.then(|| {
        Message::new(
            "⚠ This script reads its own location ($0 / $BASH_SOURCE), and the injected values run from a temporary copy — so it sees the copy's path, not the original's. Rewriting a constant as NAME=\"${NAME:-value}\" delivers the value through the environment instead, with no copy at all (`skit params <script> --normalize NAME` does the rewrite for you on a stored copy).",
        )
    })
}
