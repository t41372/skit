//! Run bounded, captured commands that validate private injected sources.

use std::{
    ffi::OsString,
    io::Read as _,
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::Duration,
};

use thiserror::Error;
use wait_timeout::ChildExt as _;

/// One bounded command over a private staged source.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InjectedCommand {
    /// Exact executable path selected by launch resolution.
    pub program: PathBuf,
    /// Exact argument vector, excluding `argv[0]`.
    pub args: Vec<OsString>,
    /// Maximum time the optional validation process can run.
    pub timeout: Duration,
}

/// Captured result of one injected-source validation command.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InjectedCommandOutput {
    /// Whether the process returned a successful status.
    pub success: bool,
    /// Exact stderr bytes.
    pub stderr: Vec<u8>,
}

/// Report that an optional injected-source validation command could not complete.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum InjectedCommandUnavailable {
    /// The process could not start, wait, or return captured diagnostics.
    #[error("could not run the injected-source check: {reason}")]
    Spawn {
        /// Operating-system or capture detail.
        reason: String,
    },
    /// The process exceeded its bounded validation time.
    #[error("the injected-source check timed out")]
    Timeout,
}

/// Execute injected-source validation commands without coupling policy to the operating system.
pub trait InjectedCommandRunner: std::fmt::Debug {
    /// Run one complete command and capture its stderr.
    fn run(
        &self,
        command: &InjectedCommand,
    ) -> Result<InjectedCommandOutput, InjectedCommandUnavailable>;
}

/// Execute injected-source validation commands on the local machine.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemInjectedCommandRunner;

impl InjectedCommandRunner for SystemInjectedCommandRunner {
    fn run(
        &self,
        command: &InjectedCommand,
    ) -> Result<InjectedCommandOutput, InjectedCommandUnavailable> {
        let child = Command::new(&command.program)
            .args(&command.args)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| InjectedCommandUnavailable::Spawn {
                reason: error.to_string(),
            })?;
        finish_injected_command_with(
            child,
            command.timeout,
            |child| child.stderr.take(),
            |mut stderr| {
                std::thread::Builder::new()
                    .name("skit-injected-check-stderr".to_owned())
                    .spawn(move || {
                        let mut bytes = Vec::new();
                        stderr.read_to_end(&mut bytes).map(|_| bytes)
                    })
            },
            |child, timeout| {
                child
                    .wait_timeout(timeout)
                    .map(|status| status.map(|status| status.success()))
            },
            stop_injected_command_child,
            std::thread::JoinHandle::join,
        )
    }
}

fn stop_injected_command_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn finish_injected_command_with<C, R, H>(
    mut child: C,
    timeout: Duration,
    take_stderr: impl FnOnce(&mut C) -> Option<R>,
    spawn_reader: impl FnOnce(R) -> std::io::Result<H>,
    wait_timeout: impl FnOnce(&mut C, Duration) -> std::io::Result<Option<bool>>,
    mut stop_child: impl FnMut(&mut C),
    join_reader: impl FnOnce(H) -> std::thread::Result<std::io::Result<Vec<u8>>>,
) -> Result<InjectedCommandOutput, InjectedCommandUnavailable> {
    let Some(stderr) = take_stderr(&mut child) else {
        stop_child(&mut child);
        return Err(InjectedCommandUnavailable::Spawn {
            reason: "the injected-source check did not provide its stderr pipe".to_owned(),
        });
    };
    let reader = match spawn_reader(stderr) {
        Ok(reader) => reader,
        Err(error) => {
            stop_child(&mut child);
            return Err(InjectedCommandUnavailable::Spawn {
                reason: error.to_string(),
            });
        }
    };
    let success = match wait_timeout(&mut child, timeout) {
        Ok(Some(success)) => success,
        Ok(None) => {
            stop_child(&mut child);
            let _ = join_reader(reader);
            return Err(InjectedCommandUnavailable::Timeout);
        }
        Err(error) => {
            stop_child(&mut child);
            let _ = join_reader(reader);
            return Err(InjectedCommandUnavailable::Spawn {
                reason: error.to_string(),
            });
        }
    };
    let stderr = join_reader(reader)
        .map_err(|_| InjectedCommandUnavailable::Spawn {
            reason: "the injected-source stderr reader panicked".to_owned(),
        })?
        .map_err(|error| InjectedCommandUnavailable::Spawn {
            reason: error.to_string(),
        })?;
    Ok(InjectedCommandOutput { success, stderr })
}

#[cfg(test)]
mod private_tests {
    use std::{cell::Cell, io};

    use super::*;

    #[derive(Debug, Default)]
    struct FakeChild;

    fn take_stderr(_: &mut FakeChild) -> Option<io::Empty> {
        Some(io::empty())
    }

    fn spawn_reader(_: io::Empty) -> io::Result<()> {
        Ok(())
    }

    fn join_reader(_: ()) -> std::thread::Result<io::Result<Vec<u8>>> {
        Ok(Ok(Vec::new()))
    }

    #[test]
    fn timeout_and_capture_setup_failures_always_stop_the_child() {
        for (take, spawn, wait, expected) in [
            (
                false,
                true,
                Ok(Some(true)),
                InjectedCommandUnavailable::Spawn {
                    reason: "the injected-source check did not provide its stderr pipe".to_owned(),
                },
            ),
            (
                true,
                false,
                Ok(Some(true)),
                InjectedCommandUnavailable::Spawn {
                    reason: "reader unavailable".to_owned(),
                },
            ),
            (true, true, Ok(None), InjectedCommandUnavailable::Timeout),
        ] {
            let stops = Cell::new(0);
            let result = finish_injected_command_with(
                FakeChild,
                Duration::ZERO,
                |_| take.then(io::empty),
                |_| {
                    if spawn {
                        Ok(())
                    } else {
                        Err(io::Error::other("reader unavailable"))
                    }
                },
                |_, _| wait,
                |_| stops.set(stops.get() + 1),
                join_reader,
            );
            assert_eq!(result.unwrap_err(), expected);
            assert_eq!(stops.get(), 1);
        }

        let output = finish_injected_command_with(
            FakeChild,
            Duration::ZERO,
            take_stderr,
            spawn_reader,
            |_, _| Ok(Some(true)),
            |_| {},
            join_reader,
        )
        .unwrap();
        assert!(output.success);
        assert!(output.stderr.is_empty());
    }
}
