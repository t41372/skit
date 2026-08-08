use std::fs;

use predicates::prelude::*;
use tempfile::TempDir;

struct Sandbox {
    data: TempDir,
    state: TempDir,
    config: TempDir,
}

impl Sandbox {
    fn new() -> Self {
        Self {
            data: TempDir::new().unwrap(),
            state: TempDir::new().unwrap(),
            config: TempDir::new().unwrap(),
        }
    }

    fn command(&self) -> assert_cmd::Command {
        let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
        command
            .env("SKIT_DATA_DIR", self.data.path())
            .env("SKIT_STATE_DIR", self.state.path())
            .env("SKIT_CONFIG_DIR", self.config.path())
            .env("SKIT_LANG", "en");
        command
    }
}

#[test]
fn prompt_without_a_path_reads_a_pipe_in_no_input_mode() {
    let sandbox = Sandbox::new();
    sandbox
        .command()
        .args(["add", "--prompt", "--name", "Review", "--no-input"])
        .write_stdin("Review {{subject}}.\n")
        .assert()
        .success();

    assert_eq!(
        fs::read(sandbox.data.path().join("scripts/review/prompt.md")).unwrap(),
        b"Review {{subject}}.\n"
    );
}

#[test]
fn edit_and_no_input_are_an_explicit_usage_conflict() {
    Sandbox::new()
        .command()
        .args(["add", "--edit", "--name", "Draft", "--no-input"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("standard input"));
}

#[cfg(unix)]
#[test]
fn edit_creates_a_python_draft_and_removes_it_after_copying() {
    use std::os::unix::fs::PermissionsExt as _;

    let sandbox = Sandbox::new();
    let editor = sandbox.config.path().join("editor.sh");
    fs::write(&editor, "#!/bin/sh\nprintf 'print(42)\\n' > \"$1\"\n").unwrap();
    let mut permissions = fs::metadata(&editor).unwrap().permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&editor, permissions).unwrap();
    fs::create_dir_all(sandbox.config.path()).unwrap();
    fs::write(
        sandbox.config.path().join("config.toml"),
        format!("editor = {:?}\n", editor.display().to_string()),
    )
    .unwrap();

    sandbox
        .command()
        .args(["add", "--edit", "--name", "Draft"])
        .assert()
        .success();

    assert_eq!(
        fs::read(sandbox.data.path().join("scripts/draft/script.py")).unwrap(),
        b"print(42)\n"
    );
    let drafts = sandbox.data.path().join("drafts");
    assert!(
        !drafts.exists() || fs::read_dir(drafts).unwrap().next().is_none(),
        "a copied draft must not remain after success"
    );
}

#[test]
fn add_lane_selectors_are_mutually_exclusive() {
    let sandbox = Sandbox::new();
    let source = sandbox.data.path().join("source.py");
    fs::write(&source, "print(1)\n").unwrap();
    sandbox
        .command()
        .args(["add", source.to_str().unwrap(), "--cmd", "echo {name}"])
        .assert()
        .code(2);
    sandbox
        .command()
        .args(["add", source.to_str().unwrap(), "--edit"])
        .assert()
        .code(2);
}

#[test]
fn copied_javascript_adds_static_external_imports_as_private_dependencies() {
    let sandbox = Sandbox::new();
    let source = sandbox.data.path().join("tool.mjs");
    fs::write(
        &source,
        "import chalk from 'chalk';\nimport local from './local.mjs';\n",
    )
    .unwrap();

    sandbox
        .command()
        .args(["add", source.to_str().unwrap(), "--name", "Color"])
        .assert()
        .success();

    let meta = fs::read_to_string(sandbox.data.path().join("scripts/color/meta.toml")).unwrap();
    assert!(meta.contains("dependencies = [\"chalk\"]"));
    assert!(!meta.contains("./local.mjs"));
}
