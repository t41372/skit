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
            data: TempDir::new().expect("data"),
            state: TempDir::new().expect("state"),
            config: TempDir::new().expect("config"),
            home: TempDir::new().expect("home"),
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
        self.command().args(args).output().expect("run skit")
    }

    fn source(&self, name: &str, body: &str) -> PathBuf {
        let path = self.home.path().join(name);
        fs::write(&path, body).expect("source");
        path
    }

    fn meta(&self, slug: &str) -> String {
        fs::read_to_string(self.data.path().join("scripts").join(slug).join("meta.toml"))
            .expect("meta.toml")
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
fn test_add_prompt_missing_file_is_clean_on_the_panel_face() {
    let sandbox = Sandbox::new();
    let missing = sandbox.home.path().join("typo.prompt.md");
    let output = sandbox.run(&["add", missing.to_str().unwrap()]);
    assert_code(&output, 1);
    let shown = combined(&output);
    assert!(shown.contains("File not found"), "{shown}");
    assert!(!shown.contains("panicked at"), "{shown}");
    assert!(!shown.contains("stack backtrace"), "{shown}");
}

#[test]
fn test_add_prompt_unknown_runner_flag_is_usage_error() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("x.prompt.md", "{{a}}\n");
    let output = sandbox.run(&[
        "add",
        source.to_str().unwrap(),
        "-n",
        "x",
        "--runner",
        "ghost",
        "--no-input",
    ]);
    assert_code(&output, 2);
    assert!(combined(&output).contains("Unknown runner"), "{}", combined(&output));
}

#[test]
fn test_add_runner_flag_without_prompt_is_refused() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("s.py", "print(1)\n");
    let output = sandbox.run(&[
        "add",
        source.to_str().unwrap(),
        "--runner",
        "claude",
        "--no-input",
    ]);
    assert_code(&output, 2);
    assert!(
        combined(&output).contains("--runner only applies to prompt entries"),
        "{}",
        combined(&output)
    );
}

#[test]
fn test_add_prompt_conflicts_with_other_kind_flags() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("x.prompt.md", "{{a}}\n");
    for flags in [
        vec!["--exe"],
        vec!["--kind", "shell"],
        vec!["--edit"],
        vec!["--cmd", "echo {x}"],
    ] {
        let mut args = vec!["add", source.to_str().unwrap(), "--prompt"];
        args.extend(flags.iter().copied());
        let output = sandbox.run(&args);
        assert_code(&output, 2);
        assert!(
            combined(&output).contains("drop --edit/--exe/--kind/--cmd"),
            "flags={flags:?}: {}",
            combined(&output)
        );
    }
}

#[test]
fn test_add_prompt_flag_forces_the_kind_on_any_extension() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("notes.txt", "Do {{thing}}\n");
    let output = sandbox.run(&[
        "add",
        source.to_str().unwrap(),
        "--prompt",
        "--no-input",
    ]);
    assert_success(&output);
    let meta = sandbox.meta("notes");
    assert!(meta.contains("kind = \"prompt\""), "{meta}");
}

#[test]
fn test_add_bare_md_no_input_requires_explicit_prompt() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("notes.md", "hello {{x}}\n");
    let output = sandbox.run(&["add", source.to_str().unwrap(), "--no-input"]);
    assert_code(&output, 2);
    assert!(combined(&output).contains("--prompt"), "{}", combined(&output));
}

#[test]
fn test_missing_bare_md_is_refused_before_the_prompt_confirmation() {
    let sandbox = Sandbox::new();
    let missing = sandbox.home.path().join("missing.md");
    let output = sandbox.run(&["add", missing.to_str().unwrap()]);
    assert_code(&output, 1);
    let shown = combined(&output);
    assert!(shown.contains("File not found:"), "{shown}");
    assert!(shown.contains("missing.md"), "{shown}");
}

#[test]
fn test_executable_lane_preserves_the_existing_non_file_contract() {
    let sandbox = Sandbox::new();
    let directory = sandbox.home.path().join("tool-dir");
    fs::create_dir(&directory).expect("tool dir");
    let output = sandbox.run(&[
        "add",
        directory.to_str().unwrap(),
        "--exe",
        "--no-input",
    ]);
    assert_success(&output);
    let meta = sandbox.meta("tool-dir");
    assert!(meta.contains("kind = \"exe\""), "{meta}");
    assert!(meta.contains("source_hash = \"\""), "{meta}");
}
