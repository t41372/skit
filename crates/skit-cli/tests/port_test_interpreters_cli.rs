//! CLI-facing ports from Python `tests/test_interpreters.py` at `main@206f9ef`.
//!
//! These tests cross the real CLI and filesystem store. Observable Python v0.4 assertions are kept
//! even when the current Rust implementation disagrees; a red parity test is the intended signal.

use std::{fs, path::PathBuf};

use assert_cmd::Command;
use serde_json::{Value as JsonValue, json};
use skit_application::EntryRepository as _;
use skit_domain::EntrySettings;
use skit_store::FileStore;
use tempfile::TempDir;

const MISSING: &str = "__skit_interpreters_missing_ffmpeg_57f2__";

struct Sandbox {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    home: TempDir,
    empty_path: TempDir,
}

impl Sandbox {
    fn new() -> Self {
        let sandbox = Self {
            data: TempDir::new().unwrap(),
            state: TempDir::new().unwrap(),
            config: TempDir::new().unwrap(),
            home: TempDir::new().unwrap(),
            empty_path: TempDir::new().unwrap(),
        };
        fs::write(
            sandbox.config.path().join("config.toml"),
            "[mirror]\nenabled = false\n",
        )
        .unwrap();
        sandbox
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
            .env("PATH", self.empty_path.path())
            .env_remove("FORCE_COLOR")
            .env_remove("NO_COLOR")
            .env_remove("CLICOLOR")
            .env_remove("CLICOLOR_FORCE")
            .env_remove("PSModulePath")
            .current_dir(self.home.path());
        command
    }

    fn store(&self) -> FileStore {
        FileStore::new(self.data.path())
    }

    fn entry_dir(&self, slug: &str) -> PathBuf {
        self.data.path().join("scripts").join(slug)
    }

    fn add_shell(&self, name: &str) {
        let source = self.home.path().join(format!("{name}.sh"));
        fs::write(&source, "#!/bin/bash\necho hi\n").unwrap();
        self.command()
            .arg("add")
            .arg(&source)
            .args(["--name", name, "--no-input"])
            .assert()
            .success();
    }

    fn add_python(&self, name: &str) {
        let source = self.home.path().join(format!("{name}.py"));
        fs::write(&source, "print(1)\n").unwrap();
        self.command()
            .arg("add")
            .arg(&source)
            .args(["--name", name, "--no-input"])
            .assert()
            .success();
    }

    fn needs(&self, selector: &str) -> Vec<String> {
        let entry = self.store().resolve(selector).unwrap();
        EntrySettings::from_meta(&entry.meta).needs
    }

    fn set_need(&self, selector: &str, need: &str) {
        self.command()
            .args(["deps", selector, "--need", need])
            .assert()
            .success();
    }
}

fn combined(output: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn test_cli_add_shell_script_records_interpreter() {
    let sandbox = Sandbox::new();
    let source = sandbox.home.path().join("deploy.sh");
    fs::write(&source, "#!/usr/bin/env zsh\n# Ship it\necho hi\n").unwrap();
    let output = sandbox
        .command()
        .arg("add")
        .arg(&source)
        .args(["--name", "deploy", "--no-input"])
        .output()
        .unwrap();
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(0), "{text}");

    let entry = sandbox.store().resolve("deploy").unwrap();
    assert_eq!(entry.meta.kind.as_str(), "shell");
    assert_eq!(EntrySettings::from_meta(&entry.meta).interpreter, "zsh");
    assert_eq!(entry.meta.description, "Ship it");
}

#[test]
fn test_cli_add_kind_forces_extensionless_file() {
    let sandbox = Sandbox::new();
    let source = sandbox.home.path().join("build");
    fs::write(&source, "echo building\n").unwrap();
    let output = sandbox
        .command()
        .arg("add")
        .arg(&source)
        .args(["--kind", "shell", "--name", "build", "--no-input"])
        .output()
        .unwrap();
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(0), "{text}");
    assert_eq!(sandbox.store().resolve("build").unwrap().meta.kind.as_str(), "shell");
    assert!(sandbox.entry_dir("build").join("script.sh").exists());
}

#[test]
fn test_cli_add_kind_exe() {
    let sandbox = Sandbox::new();
    let source = sandbox.home.path().join("thing");
    fs::write(&source, "bytes\n").unwrap();
    let output = sandbox
        .command()
        .arg("add")
        .arg(&source)
        .args(["--kind", "exe", "--name", "thing", "--no-input"])
        .output()
        .unwrap();
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(0), "{text}");
    assert_eq!(sandbox.store().resolve("thing").unwrap().meta.kind.as_str(), "exe");
}

#[test]
fn test_cli_add_kind_unknown_is_usage_error() {
    let sandbox = Sandbox::new();
    let source = sandbox.home.path().join("x");
    fs::write(&source, "x\n").unwrap();
    let output = sandbox
        .command()
        .arg("add")
        .arg(&source)
        .args(["--kind", "cobol", "--name", "x", "--no-input"])
        .output()
        .unwrap();
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(2), "{text}");
    assert!(text.contains("shell"), "valid kinds were not listed: {text}");
    assert!(!sandbox.entry_dir("x").exists(), "usage failure created an entry");
}

#[test]
fn test_cli_add_kind_and_exe_conflict() {
    let sandbox = Sandbox::new();
    let source = sandbox.home.path().join("x");
    fs::write(&source, "x\n").unwrap();
    let output = sandbox
        .command()
        .arg("add")
        .arg(&source)
        .args(["--kind", "shell", "--exe", "--name", "x", "--no-input"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2), "{}", combined(&output));
    assert!(!sandbox.entry_dir("x").exists(), "usage failure created an entry");
}

#[test]
fn test_cli_add_command_kind_rejected() {
    let sandbox = Sandbox::new();
    let source = sandbox.home.path().join("x");
    fs::write(&source, "x\n").unwrap();
    let output = sandbox
        .command()
        .arg("add")
        .arg(&source)
        .args(["--kind", "command", "--name", "x", "--no-input"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2), "{}", combined(&output));
}

#[test]
fn test_deps_need_replaces_whole_list() {
    let sandbox = Sandbox::new();
    sandbox.add_shell("d");
    sandbox.set_need("d", "jq");
    sandbox.set_need("d", "ffmpeg");
    assert_eq!(sandbox.needs("d"), vec!["ffmpeg".to_owned()]);
}

#[test]
fn test_deps_need_and_clear_needs_conflict() {
    let sandbox = Sandbox::new();
    sandbox.add_shell("d");
    let output = sandbox
        .command()
        .args(["deps", "d", "--need", "jq", "--clear-needs"])
        .output()
        .unwrap();
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(2), "{text}");
    assert!(text.contains("not both"), "{text}");
    assert!(sandbox.needs("d").is_empty(), "refused edit mutated needs");
}

#[test]
fn test_deps_need_works_on_python_too() {
    let sandbox = Sandbox::new();
    sandbox.add_python("a");
    let output = sandbox
        .command()
        .args(["deps", "a", "--need", "ffmpeg"])
        .output()
        .unwrap();
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(0), "{text}");
    assert_eq!(sandbox.needs("a"), vec!["ffmpeg".to_owned()]);
}

#[test]
fn test_deps_dep_on_shell_is_refused() {
    let sandbox = Sandbox::new();
    sandbox.add_shell("d");
    let output = sandbox
        .command()
        .args(["deps", "d", "--dep", "requests"])
        .output()
        .unwrap();
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(2), "{text}");
    assert!(text.contains("doesn't take package dependencies"), "{text}");
}

#[test]
fn test_deps_read_view_shows_needs_for_shell() {
    let sandbox = Sandbox::new();
    sandbox.add_shell("d");
    sandbox.set_need("d", "jq");
    let output = sandbox.command().args(["deps", "d"]).output().unwrap();
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(0), "{text}");
    assert!(text.contains("jq"), "{text}");
}

#[test]
fn test_deps_json_view_includes_needs() {
    let sandbox = Sandbox::new();
    sandbox.add_shell("d");
    sandbox.set_need("d", "jq");
    let output = sandbox
        .command()
        .args(["deps", "d", "--json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    let document: JsonValue = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(document["needs"], json!(["jq"]));
}

#[test]
fn test_deps_read_view_needs_dash_when_empty() {
    let sandbox = Sandbox::new();
    sandbox.add_shell("d");
    let output = sandbox.command().args(["deps", "d"]).output().unwrap();
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(0), "{text}");
    assert!(text.contains('—'), "empty-needs dash disappeared: {text}");
}

#[test]
fn test_doctor_json_needs_missing() {
    let sandbox = Sandbox::new();
    sandbox.add_shell("d");
    sandbox.set_need("d", MISSING);
    let output = sandbox.command().args(["doctor", "--json"]).output().unwrap();
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(0), "{text}");
    let document: JsonValue = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(document["needs_missing"], json!({"d": [MISSING]}));
}

#[test]
fn test_show_human_prints_needs_line() {
    let sandbox = Sandbox::new();
    sandbox.add_shell("d");
    sandbox.command().args(["deps", "d", "--need", "jq", "--need", "ffmpeg"]).assert().success();
    let output = sandbox.command().args(["show", "d"]).output().unwrap();
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(0), "{text}");
    assert!(text.contains("Needs:"), "{text}");
    assert!(text.contains("jq"), "{text}");
}

#[test]
fn test_show_json_includes_needs() {
    let sandbox = Sandbox::new();
    sandbox.add_shell("d");
    sandbox.set_need("d", "jq");
    let output = sandbox
        .command()
        .args(["show", "d", "--json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    let document: JsonValue = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(document["needs"], json!(["jq"]));
}

#[test]
fn test_show_interpreted_header_and_source() {
    let sandbox = Sandbox::new();
    sandbox.add_shell("d");
    let output = sandbox.command().args(["show", "d"]).output().unwrap();
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(0), "{text}");
    assert!(text.contains("Shell · copy"), "{text}");
    assert!(text.contains("Source:"), "{text}");
    assert!(text.contains("skit run d"), "{text}");
}

#[test]
fn test_edit_program_refusal_is_kind_neutral() {
    let sandbox = Sandbox::new();
    let source = sandbox.home.path().join("thing");
    fs::write(&source, "bytes\n").unwrap();
    sandbox
        .command()
        .arg("add")
        .arg(&source)
        .args(["--exe", "--name", "thing", "--no-input"])
        .assert()
        .success();
    let output = sandbox.command().args(["edit", "thing"]).output().unwrap();
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(1), "{text}");
    assert!(text.contains("no editable source"), "{text}");
    assert!(!text.contains("Python"), "kind-specific refusal leaked back in: {text}");
}

#[test]
fn test_edit_command_refusal_is_kind_neutral() {
    let sandbox = Sandbox::new();
    sandbox
        .command()
        .args(["add", "--cmd", "echo hi", "--name", "c", "--no-input"])
        .assert()
        .success();
    let output = sandbox.command().args(["edit", "c"]).output().unwrap();
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(1), "{text}");
    assert!(text.contains("no editable source"), "{text}");
}
