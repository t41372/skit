use std::error::Error as StdError;
use std::fmt;
use std::io;
use std::process::{Child, Command, ExitStatus};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use crate::LaunchPlan;

const INTERRUPTED_EXIT: i32 = 130;
const POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Failures at the process boundary after a launch snapshot has already been built.
#[derive(Debug)]
pub enum RunError {
    EmptyArgv,
    Spawn { program: String, source: io::Error },
    Wait { program: String, source: io::Error },
    Kill { program: String, source: io::Error },
}

impl fmt::Display for RunError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyArgv => formatter.write_str("the launch plan has no program"),
            Self::Spawn { program, source } => write!(formatter, "cannot start {program}: {source}"),
            Self::Wait { program, source } => write!(formatter, "cannot wait for {program}: {source}"),
            Self::Kill { program, source } => write!(formatter, "cannot stop {program}: {source}"),
        }
    }
}

impl StdError for RunError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Spawn { source, .. }
            | Self::Wait { source, .. }
            | Self::Kill { source, .. } => Some(source),
            Self::EmptyArgv => None,
        }
    }
}

/// Spawn an immutable launch plan with inherited stdio and supervise it until exit.
///
/// The caller owns signal policy and flips `interrupted` when cancellation is requested.
/// This layer owns the child lifecycle: once interruption is observed, the child is
/// killed and reaped before the conventional interrupt status (130) is returned.
/// Environment values in the launch plan overlay, rather than replace, the ambient
/// process environment.
///
/// # Errors
///
/// Returns an error for an empty plan or OS failures while spawning, waiting, or
/// terminating the child.
pub fn run_launch(plan: &LaunchPlan, interrupted: &AtomicBool) -> Result<i32, RunError> {
    if interrupted.load(Ordering::SeqCst) {
        return Ok(INTERRUPTED_EXIT);
    }
    let Some(program) = plan.argv.first().filter(|program| !program.is_empty()) else {
        return Err(RunError::EmptyArgv);
    };

    let mut command = Command::new(program);
    command
        .args(&plan.argv[1..])
        .current_dir(&plan.cwd)
        .envs(&plan.env_overlay);
    let mut child = command.spawn().map_err(|source| RunError::Spawn {
        program: program.clone(),
        source,
    })?;
    supervise(&mut child, program, interrupted)
}

fn supervise(
    child: &mut Child,
    program: &str,
    interrupted: &AtomicBool,
) -> Result<i32, RunError> {
    loop {
        if let Some(status) = child.try_wait().map_err(|source| RunError::Wait {
            program: program.to_owned(),
            source,
        })? {
            return Ok(normalize_exit_status(status));
        }
        if interrupted.load(Ordering::SeqCst) {
            let kill_error = child.kill().err();
            child.wait().map_err(|source| RunError::Wait {
                program: program.to_owned(),
                source,
            })?;
            if let Some(source) = kill_error {
                return Err(RunError::Kill {
                    program: program.to_owned(),
                    source,
                });
            }
            return Ok(INTERRUPTED_EXIT);
        }
        thread::sleep(POLL_INTERVAL);
    }
}

fn normalize_exit_status(status: ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        return code;
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return 128 + signal;
        }
    }
    1
}
