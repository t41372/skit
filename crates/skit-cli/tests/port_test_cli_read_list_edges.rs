use std::{fs, path::{Path, PathBuf}, process::{Command, Output}};

use assert_cmd::Command as AssertCommand;
use tempfile::TempDir;

struct Sandbox {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    home: TempDir,
}

impl Sandbox {
    fn new() -> Self {
        Self {
            data: TempDir::new().expect("data"),
            state: TempDir::new().expect("state"),
            config: TempDir::new().expect("config"),
            home: TempDir::new().expect("home"),
        }
    }

    fn command(&self) -> AssertCommand {
        let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
        self.configure_assert(&mut command);
        command
    }

    fn configure_assert(&self, command: &mut AssertCommand) {
        command
            .env("SKIT_DATA_DIR", self.data.path())
            .env("SKIT_STATE_DIR", self.state.path())
            .env("SKIT_CONFIG_DIR", self.config.path())
            .env("SKIT_LANG", "en")
            .env("HOME", self.home.path())
            .env("USERPROFILE", self.home.path())
            .env("XDG_CONFIG_HOME", self.home.path().join("xdg-config"))
            .env("XDG_DATA_HOME", self.home.path().join("xdg-data"))
            .env("XDG_STATE_HOME", self.home.path().join("xdg-state"))
            .current_dir(self.home.path());
    }

    fn configure_std(&self, command: &mut Command) {
        command
            .env("SKIT_DATA_DIR", self.data.path())
            .env("SKIT_STATE_DIR", self.state.path())
            .env("SKIT_CONFIG_DIR", self.config.path())
            .env("SKIT_LANG", "en")
            .env("HOME", self.home.path())
            .env("USERPROFILE", self.home.path())
            .env("XDG_CONFIG_HOME", self.home.path().join("xdg-config"))
            .env("XDG_DATA_HOME", self.home.path().join("xdg-data"))
            .env("XDG_STATE_HOME", self.home.path().join("xdg-state"))
            .current_dir(self.home.path());
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command().args(args).output().expect("run skit")
    }

    fn source(&self, relative: &str, body: &[u8]) -> PathBuf {
        let path = self.home.path().join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("source parent");
        }
        fs::write(&path, body).expect("source");
        path
    }

    fn add_copy(&self, source: &Path, name: &str, description: Option<&str>) {
        let mut args = vec!["add", source.to_str().unwrap(), "--name", name, "--no-input"];
        if let Some(description) = description {
            args.extend(["--description", description]);
        }
        assert_success(&self.run(&args));
    }

    fn row_for(&self, needle: &str) -> String {
        let output = self.run(&["list"]);
        assert_success(&output);
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .find(|line| line.contains(needle))
            .unwrap_or_else(|| panic!("missing row {needle:?} in {}", combined(&output)))
            .trim()
            .to_owned()
    }
}

fn combined(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn assert_code(output: &Output, code: i32) {
    assert_eq!(output.status.code(), Some(code), "{}", combined(output));
}

fn assert_success(output: &Output) {
    assert_code(output, 0);
}

#[test]
fn test_add_read_error_reports_clean_message() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("invalid.py", b"print(1)\n\xff\xfe\n");
    let output = sandbox.run(&["add", source.to_str().unwrap()]);
    assert_code(&output, 1);
    let shown = combined(&output);
    assert!(shown.contains("Can't read"), "{shown}");
    assert!(!shown.contains("panicked at"), "{shown}");
    assert!(!shown.contains("stack backtrace"), "{shown}");
}

#[cfg(unix)]
#[test]
fn test_add_unreadable_file_clean_error_not_traceback() {
    use std::os::unix::{fs::PermissionsExt as _, process::CommandExt as _};

    let sandbox = Sandbox::new();
    let source = sandbox.source("locked.py", b"print(1)\n");
    fs::set_permissions(&source, fs::Permissions::from_mode(0o000)).expect("lock source");

    let uid = Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|value| value.trim().parse::<u32>().ok())
        .expect("POSIX id -u");

    // Under root, execute the real skit child as an unprivileged numeric uid so chmod(000)
    // remains a real permission refusal instead of a vacuous root-readable fixture.
    if uid == 0 {
        for root in [
            sandbox.data.path(),
            sandbox.state.path(),
            sandbox.config.path(),
            sandbox.home.path(),
        ] {
            fs::set_permissions(root, fs::Permissions::from_mode(0o777)).expect("share temp root");
        }
    }

    let mut command = Command::new(env!("CARGO_BIN_EXE_skit"));
    sandbox.configure_std(&mut command);
    command.args(["add", source.to_str().unwrap()]);
    if uid == 0 {
        command.uid(65_534).gid(65_534);
    }
    let output = command.output().expect("run unreadable add");
    fs::set_permissions(&source, fs::Permissions::from_mode(0o644)).expect("restore source");

    assert_code(&output, 1);
    let shown = combined(&output);
    assert!(shown.contains("Can't read"), "{shown}");
    assert!(!shown.contains("File not found"), "{shown}");
    assert!(!shown.contains("panicked at"), "{shown}");
}

#[test]
fn test_list_description_exact_marker_when_no_description() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("gone.py", b"print(1)\n");
    sandbox.add_copy(&source, "gone", None);
    let stored = sandbox.data.path().join("scripts/gone/script.py");
    fs::remove_file(&stored).expect("remove stored script");
    let row = sandbox.row_for("gone");
    let marker = format!("⚠ missing: {}", stored.display());
    assert!(row.contains(&marker), "{row}");
    assert!(!row.contains(&format!("—  {marker}")), "missing-only description must not gain a dash prefix: {row}");
}

#[test]
fn test_list_description_appends_marker_after_description() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("gone2.py", b"print(1)\n");
    sandbox.add_copy(&source, "gone2", Some("My job."));
    let stored = sandbox.data.path().join("scripts/gone2/script.py");
    fs::remove_file(&stored).expect("remove stored script");
    let row = sandbox.row_for("gone2");
    let marker = format!("⚠ missing: {}", stored.display());
    let description = row.find("My job.").expect("description");
    let warning = row.find(&marker).expect("missing marker");
    assert!(description < warning, "description must precede the missing marker: {row}");
    assert!(row.contains(&format!("My job.  {marker}")), "{row}");
}

#[test]
fn test_list_description_healthy_and_command_entries_untouched() {
    let sandbox = Sandbox::new();
    let healthy = sandbox.source("healthy.py", b"print(1)\n");
    sandbox.add_copy(&healthy, "healthy", Some("Fine."));
    let command = sandbox.run(&["add", "--cmd", "echo hi", "--name", "cmdbare"]);
    assert_success(&command);

    let healthy_row = sandbox.row_for("healthy");
    assert!(healthy_row.contains("Fine."), "{healthy_row}");
    assert!(!healthy_row.contains("missing"), "{healthy_row}");
    let command_row = sandbox.row_for("cmdbare");
    assert!(command_row.contains('—'), "a bare command keeps the empty-description marker: {command_row}");
    assert!(!command_row.contains("missing"), "{command_row}");
}

#[test]
fn test_list_description_escapes_markup_in_description() {
    let sandbox = Sandbox::new();
    assert_success(&sandbox.run(&[
        "add",
        "--cmd",
        "echo hi",
        "--name",
        "mkup",
        "--description",
        "[red]DANGER[/red]",
    ]));
    let row = sandbox.row_for("mkup");
    assert!(row.contains("[red]DANGER[/red]"), "{row}");
}

#[test]
fn test_list_description_escapes_markup_in_missing_path() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("[red]boom[bold]/tool", b"#!/bin/sh\necho hi\n");
    let output = sandbox.run(&[
        "add",
        source.to_str().unwrap(),
        "--exe",
        "--ref",
        "--name",
        "mkup-path",
        "--no-input",
    ]);
    assert_success(&output);
    fs::remove_file(&source).expect("remove reference source");
    let row = sandbox.row_for("mkup-path");
    assert!(row.contains("[red]boom[bold]"), "{row}");
    assert!(row.contains("missing"), "{row}");
}
