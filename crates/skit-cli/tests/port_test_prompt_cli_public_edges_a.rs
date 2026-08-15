use std::{fs, path::PathBuf, process::Output};

use assert_cmd::Command;
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
            data: TempDir::new().unwrap(),
            state: TempDir::new().unwrap(),
            config: TempDir::new().unwrap(),
            home: TempDir::new().unwrap(),
        }
    }

    fn command(&self) -> Command {
        let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
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
        command
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command().args(args).output().unwrap()
    }

    fn source(&self, name: &str, body: &str) -> PathBuf {
        let path = self.home.path().join(name);
        fs::write(&path, body).unwrap();
        path
    }
}

fn combined(output: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr))
}

#[test]
fn test_add_prompt_unreadable_file_is_a_store_error() {
    let sandbox = Sandbox::new();
    let trap = sandbox.home.path().join("dir.prompt.md");
    fs::create_dir(&trap).unwrap();
    let output = sandbox.run(&["add", trap.to_str().unwrap(), "--no-input"]);
    assert_eq!(output.status.code(), Some(1), "{}", combined(&output));
    assert!(combined(&output).contains("Not a file"), "{}", combined(&output));
    assert!(!sandbox.data.path().join("scripts/dir").exists());
}

#[test]
fn test_add_runner_flag_refused_on_cmd_edit_exe_lanes() {
    for args in [
        vec!["add", "--cmd", "echo {x}", "-n", "c", "--runner", "claude"],
        vec!["add", "--edit", "--runner", "claude"],
        vec!["add", "x", "--exe", "--runner", "claude"],
    ] {
        let sandbox = Sandbox::new();
        let output = sandbox.run(&args);
        assert_eq!(output.status.code(), Some(2), "args={args:?}\n{}", combined(&output));
        let shown = combined(&output);
        assert!(
            shown.contains("--runner only applies to prompt entries")
                || shown.contains("--runner can't apply here"),
            "args={args:?}\n{shown}"
        );
        assert!(!sandbox.data.path().join("scripts").exists());
    }
}

#[test]
fn test_add_prompt_stdin_lane_reports_store_errors() {
    let sandbox = Sandbox::new();
    let added = sandbox.run(&["add", "--cmd", "echo hi", "--name", "taken"]);
    assert_eq!(added.status.code(), Some(0), "{}", combined(&added));
    let output = sandbox
        .command()
        .args(["add", "-", "--prompt", "-n", "taken"])
        .write_stdin("b\n")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{}", combined(&output));
    assert!(combined(&output).contains("already taken"), "{}", combined(&output));
}

#[test]
fn test_params_view_survives_an_unreadable_reference_body() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("p.prompt.md", "{{a}}\n");
    let output = sandbox.run(&[
        "add",
        source.to_str().unwrap(),
        "--name",
        "p",
        "--ref",
        "--no-input",
    ]);
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    fs::remove_file(&source).unwrap();
    let params = sandbox.run(&["params", "p"]);
    assert_eq!(params.status.code(), Some(0), "{}", combined(&params));
    assert!(combined(&params).contains("a"), "managed record vanished with reference body: {}", combined(&params));
}
