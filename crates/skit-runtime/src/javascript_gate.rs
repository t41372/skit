//! Resolve JavaScript runtime identity and apply the optional Node syntax gate.

use std::{
    io::Read as _,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};

use skit_i18n::{Localize, Message};
use thiserror::Error;
use wait_timeout::ChildExt as _;

/// Maximum time for the optional `node --check` process.
pub const JAVASCRIPT_SYNTAX_GATE_TIMEOUT: Duration = Duration::from_secs(30);

/// Classify one resolved JavaScript runtime without losing unknown compatible programs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum JavaScriptRuntimeKind {
    /// Deno runtime.
    Deno,
    /// Bun runtime.
    Bun,
    /// Node.js runtime.
    Node,
    /// A user-pinned runtime that skit does not classify.
    Other(String),
}

impl JavaScriptRuntimeKind {
    /// Classify a configured runtime name or path.
    #[must_use]
    pub fn from_candidate(candidate: &str) -> Self {
        let basename = Path::new(candidate)
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or(candidate);
        let lower = basename.to_ascii_lowercase();
        let normalized = lower.strip_suffix(".exe").unwrap_or(&lower);
        match normalized {
            "deno" => Self::Deno,
            "bun" => Self::Bun,
            "node" => Self::Node,
            _ => Self::Other(candidate.to_owned()),
        }
    }

    /// Return the normalized runtime name used by launch and dependency plans.
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::Deno => "deno",
            Self::Bun => "bun",
            Self::Node => "node",
            Self::Other(name) => name,
        }
    }
}

/// Hold the runtime identity and exact executable selected by a program probe.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedJavaScriptRuntime {
    /// Normalized runtime kind.
    pub kind: JavaScriptRuntimeKind,
    /// Exact executable path returned by the probe.
    pub program: PathBuf,
}

/// Captured result of one optional syntax-check process.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JavaScriptSyntaxGateOutput {
    /// Whether the process returned a successful status.
    pub success: bool,
    /// Exact stderr bytes.
    pub stderr: Vec<u8>,
}

/// Report that the optional syntax-check process could not complete.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum JavaScriptSyntaxGateUnavailable {
    /// The process could not start or wait.
    #[error("could not run node syntax check: {reason}")]
    Spawn {
        /// Operating-system detail.
        reason: String,
    },
    /// The process exceeded the bounded gate time.
    #[error("node syntax check timed out")]
    Timeout,
}

/// Run one optional syntax-check process.
pub trait JavaScriptSyntaxGateRunner: std::fmt::Debug {
    /// Run `node --check` for one staged source.
    fn check(
        &self,
        program: &Path,
        source: &Path,
        timeout: Duration,
    ) -> Result<JavaScriptSyntaxGateOutput, JavaScriptSyntaxGateUnavailable>;
}

/// Start syntax-check processes on the local machine.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemJavaScriptSyntaxGateRunner;

impl JavaScriptSyntaxGateRunner for SystemJavaScriptSyntaxGateRunner {
    fn check(
        &self,
        program: &Path,
        source: &Path,
        timeout: Duration,
    ) -> Result<JavaScriptSyntaxGateOutput, JavaScriptSyntaxGateUnavailable> {
        let mut child = Command::new(program)
            .arg("--check")
            .arg(source)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| JavaScriptSyntaxGateUnavailable::Spawn {
                reason: error.to_string(),
            })?;
        let Some(mut stderr) = child.stderr.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(JavaScriptSyntaxGateUnavailable::Spawn {
                reason: "node syntax check did not provide its stderr pipe".to_owned(),
            });
        };
        let reader = match std::thread::Builder::new()
            .name("skit-node-check-stderr".to_owned())
            .spawn(move || {
                let mut bytes = Vec::new();
                stderr.read_to_end(&mut bytes).map(|_| bytes)
            }) {
            Ok(reader) => reader,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(JavaScriptSyntaxGateUnavailable::Spawn {
                    reason: error.to_string(),
                });
            }
        };
        let status = match child.wait_timeout(timeout) {
            Ok(Some(status)) => status,
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                return Err(JavaScriptSyntaxGateUnavailable::Timeout);
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                return Err(JavaScriptSyntaxGateUnavailable::Spawn {
                    reason: error.to_string(),
                });
            }
        };
        let stderr = reader
            .join()
            .map_err(|_| JavaScriptSyntaxGateUnavailable::Spawn {
                reason: "node syntax check stderr reader panicked".to_owned(),
            })?
            .map_err(|error| JavaScriptSyntaxGateUnavailable::Spawn {
                reason: error.to_string(),
            })?;
        Ok(JavaScriptSyntaxGateOutput {
            success: status.success(),
            stderr,
        })
    }
}

/// Report that Node rejected one staged injected source.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error("node rejected the injected copy: {detail}")]
pub struct JavaScriptSyntaxError {
    detail: String,
}

impl JavaScriptSyntaxError {
    /// Create one rejection with the first stderr line.
    #[must_use]
    pub fn new(detail: impl Into<String>) -> Self {
        Self {
            detail: detail.into(),
        }
    }

    /// Return the captured first stderr line.
    #[must_use]
    pub fn detail(&self) -> &str {
        &self.detail
    }
}

impl Localize for JavaScriptSyntaxError {
    fn message(&self) -> Message {
        Message::new("node rejected the injected copy: {}").with(&self.detail)
    }
}

/// Apply the best-effort Node syntax gate to one staged JavaScript source.
///
/// TypeScript suffixes and non-Node runtimes use only the mandatory parser-backed gate. A missing,
/// failed, or timed-out optional process is also non-fatal because the mandatory gate has already
/// accepted the source.
pub fn check_javascript_syntax<R: JavaScriptSyntaxGateRunner>(
    runtime: Option<&ResolvedJavaScriptRuntime>,
    source: &Path,
    runner: &R,
) -> Result<(), JavaScriptSyntaxError> {
    let eligible = source
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| matches!(value, "js" | "mjs" | "cjs"));
    let Some(runtime) = runtime.filter(|runtime| runtime.kind == JavaScriptRuntimeKind::Node)
    else {
        return Ok(());
    };
    if !eligible {
        return Ok(());
    }
    match runner.check(&runtime.program, source, JAVASCRIPT_SYNTAX_GATE_TIMEOUT) {
        Err(_) => Ok(()),
        Ok(output) if output.success => Ok(()),
        Ok(output) => {
            let decoded = String::from_utf8_lossy(&output.stderr);
            let detail = decoded.trim().lines().next().unwrap_or("");
            Err(JavaScriptSyntaxError::new(detail))
        }
    }
}

/// Keep an owned staged source only when the optional syntax gate accepts it.
///
/// On rejection, `source` drops before this function returns. A caller can pass its private staged
/// file guard so the temporary file closes and unlinks on every refusal, including on Windows.
pub fn retain_javascript_source_if_valid<T, R: JavaScriptSyntaxGateRunner>(
    source: T,
    runtime: Option<&ResolvedJavaScriptRuntime>,
    path: &Path,
    runner: &R,
) -> Result<T, JavaScriptSyntaxError> {
    check_javascript_syntax(runtime, path, runner)?;
    Ok(source)
}
