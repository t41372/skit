//! One stand-in program a test writes for the product to find and run.
//!
//! A Unix host runs a file whose first line names an interpreter, once the file carries an execute
//! bit. Windows has neither. It finds a program by trying the suffixes in `PATHEXT`, and it runs a
//! `.cmd` through the command interpreter. The product follows the host rule on both sides:
//! `program_names` in `skit-runtime/src/launch.rs` appends every `PATHEXT` suffix to a bare name,
//! and `is_executable` there asks only whether the path is a file when the host is not Unix.
//!
//! So a test cannot write one script and expect both hosts to run it. The makers here write the
//! dialect the host runs, name the file the way the host finds it, and return the path they wrote.
//! A caller must use that returned path and never spell the file name itself, because the two hosts
//! spell it differently.

use std::{
    fs,
    path::{Path, PathBuf},
};

/// What a stand-in program does when the product runs it.
///
/// Each case names a behavior rather than a script, because the two hosts cannot share script text.
#[derive(Clone, Copy, Debug)]
#[allow(
    dead_code,
    reason = "each including file uses the cases its own shims need"
)]
pub(crate) enum Shim {
    /// Exit with this status and do nothing else.
    Exit(i32),
    /// Make this directory below the working directory, then exit zero.
    MakeDirectory(&'static str),
    /// Make the empty file the named environment variable points to, then exit zero.
    TouchFromEnvironment(&'static str),
}

/// The suffix the host needs on a program name.
#[cfg(windows)]
const SUFFIX: &str = ".cmd";

/// The suffix the host needs on a program name.
#[cfg(not(windows))]
const SUFFIX: &str = "";

/// Write `shim` into `directory` as `name`, and return the path that was written.
///
/// The returned path carries the host's own suffix. Use it; do not rebuild the name.
#[allow(
    dead_code,
    reason = "each including file uses the makers its own shims need"
)]
pub(crate) fn write_shim(directory: &Path, name: &str, shim: Shim) -> PathBuf {
    let path = directory.join(format!("{name}{SUFFIX}"));
    write_shim_at(&path, shim);
    path
}

/// Write `shim` at an exact path the caller already chose.
///
/// The caller owns the suffix here, so this is for a program the product reaches by path rather
/// than by name.
#[allow(
    dead_code,
    reason = "each including file uses the makers its own shims need"
)]
pub(crate) fn write_shim_at(path: &Path, shim: Shim) {
    fs::write(path, body(shim)).unwrap();
    make_runnable(path);
}

#[cfg(not(windows))]
fn body(shim: Shim) -> String {
    match shim {
        Shim::Exit(status) => format!("#!/bin/sh\nexit {status}\n"),
        // Name the program by path. These stand-ins run with a PATH that holds only the directory
        // they live in, so a bare name would not resolve.
        Shim::MakeDirectory(name) => format!("#!/bin/sh\n/bin/mkdir -p '{name}'\nexit 0\n"),
        Shim::TouchFromEnvironment(variable) => {
            format!("#!/bin/sh\n: > \"${variable}\"\nexit 0\n")
        }
    }
}

#[cfg(windows)]
fn body(shim: Shim) -> String {
    // `exit /b` sets the status of the script itself, which is what the parent reads.
    match shim {
        Shim::Exit(status) => format!("@echo off\r\nexit /b {status}\r\n"),
        Shim::MakeDirectory(name) => {
            format!("@echo off\r\nif not exist \"{name}\" mkdir \"{name}\"\r\nexit /b 0\r\n")
        }
        Shim::TouchFromEnvironment(variable) => {
            format!("@echo off\r\ntype nul > \"%{variable}%\"\r\nexit /b 0\r\n")
        }
    }
}

/// Give the file the execute bit the host needs, where the host has one.
#[cfg(unix)]
fn make_runnable(path: &Path) {
    use std::os::unix::fs::PermissionsExt as _;

    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

/// Windows carries no execute bit: the suffix in the name is what makes a file runnable.
#[cfg(not(unix))]
fn make_runnable(_path: &Path) {}
