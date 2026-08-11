use std::fs;

use predicates::prelude::*;
use serde_json::Value;
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
fn parameter_edit_keeps_boolean_flag_actions_truthful_and_rolls_back_refusals() {
    let sandbox = Sandbox::new();
    let executable = sandbox.data.path().join("tool");
    fs::write(&executable, b"tool bytes\n").unwrap();
    sandbox
        .command()
        .args([
            "add",
            executable.to_str().unwrap(),
            "--exe",
            "--name",
            "Tool",
            "--no-input",
        ])
        .assert()
        .success();
    sandbox
        .command()
        .args([
            "params",
            "tool",
            "--add",
            "verbose",
            "--flag",
            "verbose=--verbose",
        ])
        .assert()
        .success();

    let metadata = sandbox.data.path().join("scripts/tool/meta.toml");
    let before = fs::read(&metadata).unwrap();
    sandbox
        .command()
        .args([
            "params",
            "tool",
            "--type",
            "verbose=bool",
            "--default",
            "verbose=true",
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains("is on by default"));
    assert_eq!(fs::read(&metadata).unwrap(), before);

    sandbox
        .command()
        .args([
            "params",
            "tool",
            "--type",
            "verbose=bool",
            "--default",
            "verbose=false",
        ])
        .assert()
        .success();
    let output = sandbox
        .command()
        .args(["params", "tool", "--json"])
        .output()
        .unwrap();
    let document: Value = serde_json::from_slice(&output.stdout).unwrap();
    let row = &document["parameters"][0];
    assert_eq!(row["type"], "bool");
    assert_eq!(row["default"], false);
    assert_eq!(row["action"], "store_true");

    sandbox
        .command()
        .args(["params", "tool", "--type", "verbose=str"])
        .assert()
        .success();
    let output = sandbox
        .command()
        .args(["params", "tool", "--json"])
        .output()
        .unwrap();
    let document: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(document["parameters"][0]["action"], "");
}
