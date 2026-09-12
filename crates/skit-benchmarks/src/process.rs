//! Bounded process-tree execution with complete environment construction.

use std::{collections::BTreeMap, io, path::PathBuf, sync::OnceLock, time::Duration};

use processkit::Command;
use thiserror::Error;
use tokio::runtime::{Builder, Runtime};

static PROCESS_RUNTIME: OnceLock<Runtime> = OnceLock::new();

/// One exact process invocation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessSpec {
    /// Program followed by its arguments.
    pub argv: Vec<String>,
    /// Child working directory.
    pub cwd: PathBuf,
    /// Complete environment. The parent environment is not inherited.
    pub env: BTreeMap<String, String>,
    /// Hard deadline.
    pub timeout: Duration,
    /// Treat a non-zero status as an error.
    pub check: bool,
}

/// Captured output from a process tree.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessOutput {
    /// Captured standard output bytes.
    pub stdout: Vec<u8>,
    /// Captured standard error bytes.
    pub stderr: Vec<u8>,
    /// Terminal process status.
    pub status: ProcessStatus,
}

/// Portable terminal status for a process tree.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProcessStatus {
    code: Option<i32>,
    success: bool,
}

impl ProcessStatus {
    /// Return true when the process exited with code zero.
    #[must_use]
    pub const fn success(self) -> bool {
        self.success
    }

    /// Return the exit code, or `None` for a signal termination.
    #[must_use]
    pub const fn code(self) -> Option<i32> {
        self.code
    }
}

/// A process could not start, finish, or satisfy its status contract.
#[derive(Debug, Error)]
pub enum ProcessError {
    /// No program was supplied.
    #[error("process argv is empty")]
    EmptyArgv,
    /// The process runtime could not start.
    #[error("could not initialize the process runtime: {source}")]
    Runtime {
        /// Runtime initialization error.
        #[source]
        source: io::Error,
    },
    /// Spawn, capture, or wait failed.
    #[error("could not run {argv:?}: {source}")]
    Run {
        /// Attempted argv.
        argv: Vec<String>,
        /// Structured process lifecycle error.
        #[source]
        source: processkit::Error,
    },
    /// Deadline expired. The complete process tree was killed and reaped.
    #[error("process timed out after {seconds}s: {argv:?}")]
    Timeout {
        /// Attempted argv.
        argv: Vec<String>,
        /// Deadline in seconds.
        seconds: f64,
    },
    /// Checked process exited non-zero.
    #[error("process exited {code}: {argv:?}\n{stderr}")]
    Exit {
        /// Attempted argv.
        argv: Vec<String>,
        /// Exit status or signal marker.
        code: String,
        /// Bounded diagnostic text.
        stderr: String,
    },
}

/// Run one process tree with captured stdout/stderr and a hard timeout.
pub fn run(spec: &ProcessSpec) -> Result<ProcessOutput, ProcessError> {
    let (program, args) = spec.argv.split_first().ok_or(ProcessError::EmptyArgv)?;
    let command = Command::new(program)
        .args(args)
        .current_dir(&spec.cwd)
        .env_clear()
        .envs(&spec.env)
        .timeout(spec.timeout);
    let result = process_runtime()?
        .block_on(command.output_bytes())
        .map_err(|source| ProcessError::Run {
            argv: spec.argv.clone(),
            source,
        })?;
    if result.timed_out() {
        return Err(ProcessError::Timeout {
            argv: spec.argv.clone(),
            seconds: spec.timeout.as_secs_f64(),
        });
    }
    let output = ProcessOutput {
        stdout: result.stdout().clone(),
        stderr: result.stderr().as_bytes().to_vec(),
        status: ProcessStatus {
            code: result.code(),
            success: result.is_success(),
        },
    };
    if spec.check && !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = stderr
            .chars()
            .rev()
            .take(2_000)
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        return Err(ProcessError::Exit {
            argv: spec.argv.clone(),
            code: output
                .status
                .code()
                .map_or_else(|| "signal".to_owned(), |code| code.to_string()),
            stderr,
        });
    }
    Ok(output)
}

fn process_runtime() -> Result<&'static Runtime, ProcessError> {
    if let Some(runtime) = PROCESS_RUNTIME.get() {
        return Ok(runtime);
    }
    let runtime = Builder::new_multi_thread()
        .enable_all()
        .thread_name("skit-benchmark-process")
        .build()
        .map_err(runtime_error)?;
    let _ = PROCESS_RUNTIME.set(runtime);
    Ok(PROCESS_RUNTIME
        .get()
        .expect("the process runtime was initialized or won a concurrent race"))
}

fn runtime_error(source: io::Error) -> ProcessError {
    ProcessError::Runtime { source }
}

#[cfg(all(test, unix))]
mod tests {
    use std::{collections::BTreeMap, io, path::PathBuf, time::Duration};

    use super::{ProcessError, ProcessSpec};

    fn spec(argv: &[&str], check: bool) -> ProcessSpec {
        ProcessSpec {
            argv: argv.iter().map(|value| (*value).to_owned()).collect(),
            cwd: PathBuf::from("/"),
            env: BTreeMap::from([("PATH".to_owned(), "/usr/bin:/bin".to_owned())]),
            timeout: Duration::from_secs(5),
            check,
        }
    }

    #[test]
    fn process_contract_distinguishes_empty_spawn_and_checked_exit_failures() {
        assert!(matches!(
            super::run(&spec(&[], true)),
            Err(ProcessError::EmptyArgv)
        ));
        assert!(matches!(
            super::run(&spec(&["/definitely/absent/skit-benchmark"], true)),
            Err(ProcessError::Run { .. })
        ));

        let unchecked = super::run(&spec(
            &["/bin/sh", "-c", "printf out; printf err >&2; exit 7"],
            false,
        ))
        .unwrap();
        assert_eq!(unchecked.stdout, b"out");
        assert_eq!(unchecked.stderr, b"err");
        assert!(!unchecked.status.success());
        assert_eq!(unchecked.status.code(), Some(7));

        let checked = super::run(&spec(
            &["/bin/sh", "-c", "printf '%02050d' 0 >&2; exit 9"],
            true,
        ))
        .unwrap_err();
        assert!(matches!(checked, ProcessError::Exit { .. }));
        if let ProcessError::Exit { code, stderr, .. } = checked {
            assert_eq!(code, "9");
            assert_eq!(stderr.chars().count(), 2_000);
        }
    }

    #[test]
    fn signal_status_has_no_exit_code() {
        let output = super::run(&spec(&["/bin/sh", "-c", "kill -TERM $$"], false)).unwrap();
        assert!(!output.status.success());
        assert_eq!(output.status.code(), None);
    }

    #[test]
    fn runtime_initialization_errors_keep_the_io_source() {
        let error = super::runtime_error(io::Error::other("runtime unavailable"));
        assert!(matches!(error, ProcessError::Runtime { .. }));
        assert!(error.to_string().contains("runtime unavailable"));
    }
}
