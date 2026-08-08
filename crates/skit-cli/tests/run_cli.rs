use std::fs;

use predicates::prelude::*;
use tempfile::TempDir;

fn command_library(template: &str, parameters: &str) -> (TempDir, TempDir) {
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let dir = data.path().join("scripts").join("demo");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("meta.toml"),
        format!(
            r#"
schema = 1
name = "Demo"
kind = "command"
mode = "copy"
source = ""
source_hash = ""
added_at = "2026-08-07T00:00:00Z"
id = "0123456789abcdef0123456789abcdef"
workdir = "invoke"
description = ""
template = {template:?}
params = ["name"]
{parameters}
"#
        ),
    )
    .unwrap();
    (data, state)
}

fn skit(data: &TempDir, state: &TempDir) -> assert_cmd::Command {
    let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
    command
        .env("SKIT_DATA_DIR", data.path())
        .env("SKIT_STATE_DIR", state.path());
    command
}

#[test]
fn dry_run_uses_set_values_and_does_not_write_state() {
    let (data, state) = command_library(
        "echo {name}",
        r#"
[[parameters]]
name = "name"
delivery = "placeholder"
type = "str"
required = true
"#,
    );

    skit(&data, &state)
        .args(["run", "demo", "--set", "name=Ada Lovelace", "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Ada Lovelace"));

    assert!(!state.path().join("values/demo.toml").exists());
}

#[test]
fn a_missing_required_value_is_a_usage_error_before_spawn() {
    let (data, state) = command_library(
        "echo {name}",
        r#"
[[parameters]]
name = "name"
delivery = "placeholder"
type = "str"
required = true
"#,
    );

    skit(&data, &state)
        .args(["run", "demo", "--no-input"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("required"));
}

#[test]
fn preset_then_set_uses_the_same_precedence_as_the_form_state_service() {
    let (data, state) = command_library(
        "echo {name}",
        r#"
[[parameters]]
name = "name"
delivery = "placeholder"
type = "str"
required = true
"#,
    );
    fs::create_dir_all(state.path().join("values")).unwrap();
    fs::write(
        state.path().join("values/demo.toml"),
        r#"
[values]
name = "last"

[presets.work]
name = "preset"
"#,
    )
    .unwrap();

    skit(&data, &state)
        .args([
            "run",
            "demo",
            "--preset",
            "work",
            "--set",
            "name=this-run",
            "--dry-run",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("this-run"))
        .stdout(predicate::str::contains("preset").not());
}

#[test]
fn dry_run_masks_secret_placeholder_values() {
    let (data, state) = command_library(
        "echo {name}",
        r#"
[[parameters]]
name = "name"
delivery = "placeholder"
type = "str"
required = true
secret = true
"#,
    );

    skit(&data, &state)
        .args(["run", "demo", "--set", "name=super-secret", "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("super-secret").not())
        .stdout(predicate::str::contains("•••"));
}

#[test]
fn child_exit_status_is_the_skit_process_status() {
    let template = if cfg!(windows) { "exit /b 7" } else { "exit 7" };
    let (data, state) = command_library(template, "");
    fs::write(
        data.path().join("scripts/demo/meta.toml"),
        format!(
            r#"
schema = 1
name = "Demo"
kind = "command"
mode = "copy"
source = ""
source_hash = ""
added_at = "2026-08-07T00:00:00Z"
id = "0123456789abcdef0123456789abcdef"
workdir = "invoke"
description = ""
template = {template:?}
params = []
"#
        ),
    )
    .unwrap();

    skit(&data, &state)
        .args(["run", "demo", "--no-input"])
        .assert()
        .code(7);

    let saved = fs::read_to_string(state.path().join("values/demo.toml")).unwrap();
    assert!(saved.contains("exit = 7"));
}

#[test]
fn malformed_set_and_unknown_preset_are_usage_errors() {
    let (data, state) = command_library("echo {name}", "");

    skit(&data, &state)
        .args(["run", "demo", "--set", "not-an-assignment", "--dry-run"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("NAME=VALUE"));

    skit(&data, &state)
        .args(["run", "demo", "--preset", "missing", "--dry-run"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("preset"));
}
