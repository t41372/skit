use std::{fs, path::Path};

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

    fn command_json(&self, args: &[&str]) -> Value {
        let output = self.command().args(args).output().unwrap();
        assert!(
            output.status.success(),
            "command failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }

    fn write_command_entry(&self) {
        let directory = self.data.path().join("scripts/demo");
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join("meta.toml"),
            r#"
schema = 1
name = "Demo"
kind = "command"
mode = "copy"
source = ""
source_hash = ""
added_at = "2026-08-08T00:00:00Z"
id = "0123456789abcdef0123456789abcdef"
workdir = "invoke"
description = ""
template = "echo {name}"
params = ["name"]
future_field = "keep-me"

[[parameters]]
name = "name"
delivery = "placeholder"
type = "str"
required = true
"#,
        )
        .unwrap();
    }
}

#[test]
fn help_and_version_expose_the_complete_automation_surface() {
    let sandbox = Sandbox::new();
    sandbox
        .command()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("params"))
        .stdout(predicate::str::contains("deps"))
        .stdout(predicate::str::contains("doctor"))
        .stdout(predicate::str::contains("config"))
        .stdout(predicate::str::contains("runner"))
        .stdout(predicate::str::contains("preset"))
        .stdout(predicate::str::contains("agent"))
        .stdout(predicate::str::contains("edit"));

    sandbox
        .command()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("skit 0.5.0"));
}

#[test]
fn add_infers_source_kinds_and_supports_command_and_prompt_entries() {
    let sandbox = Sandbox::new();
    let python = sandbox.data.path().join("hello.py");
    let prompt = sandbox.data.path().join("review.prompt.md");
    fs::write(&python, b"print('hello')\n").unwrap();
    fs::write(&prompt, b"Review {{target}}.\n").unwrap();

    sandbox
        .command()
        .args(["add", python.to_str().unwrap(), "--name", "Python tool"])
        .assert()
        .success();
    sandbox
        .command()
        .args(["add", prompt.to_str().unwrap(), "--name", "Review"])
        .assert()
        .success();
    sandbox
        .command()
        .args(["add", "--cmd", "printf '%s' {value}", "--name", "Print"])
        .assert()
        .success();

    let entries = sandbox.command_json(&["list", "--json"]);
    let entries = entries.as_array().unwrap();
    assert_eq!(entries.len(), 3);
    assert!(entries.iter().any(|item| item["kind"] == "python"));
    assert!(entries.iter().any(|item| item["kind"] == "prompt"));
    assert!(entries.iter().any(|item| item["kind"] == "command"));
    assert_eq!(
        fs::read(sandbox.data.path().join("scripts/python-tool/script.py")).unwrap(),
        b"print('hello')\n"
    );
}

#[test]
fn config_round_trips_known_keys_and_preserves_unknown_toml() {
    let sandbox = Sandbox::new();
    fs::write(
        sandbox.config.path().join("config.toml"),
        "future_key = \"keep-me\"\n",
    )
    .unwrap();

    sandbox
        .command()
        .args(["config", "lang", "zh-TW"])
        .assert()
        .success();
    sandbox
        .command()
        .args(["config", "form", "plain"])
        .assert()
        .success();
    let config = sandbox.command_json(&["config", "--json"]);
    assert_eq!(config["lang"], "zh-TW");
    assert_eq!(config["form"], "plain");

    let text = fs::read_to_string(sandbox.config.path().join("config.toml")).unwrap();
    assert!(text.contains("future_key = \"keep-me\""));
}

#[test]
fn deps_and_params_update_one_identity_without_losing_future_metadata() {
    let sandbox = Sandbox::new();
    sandbox.write_command_entry();

    sandbox
        .command()
        .args(["deps", "demo", "--need", "jq", "--need", "ffmpeg"])
        .assert()
        .success();
    sandbox
        .command()
        .args([
            "params",
            "demo",
            "--add",
            "mode",
            "--type",
            "mode=choice",
            "--choices",
            "mode=fast,safe",
            "--default",
            "mode=fast",
            "--deliver",
            "mode=flag",
            "--flag",
            "mode=--mode",
        ])
        .assert()
        .success();

    let deps = sandbox.command_json(&["deps", "demo", "--json"]);
    assert_eq!(deps["needs"], serde_json::json!(["jq", "ffmpeg"]));
    let params = sandbox.command_json(&["params", "demo", "--json"]);
    let rows = params["parameters"].as_array().unwrap();
    assert!(rows.iter().any(|row| row["name"] == "mode"));

    let metadata = fs::read_to_string(sandbox.data.path().join("scripts/demo/meta.toml")).unwrap();
    assert!(metadata.contains("future_field = \"keep-me\""));
}

#[test]
fn preset_commands_use_the_existing_state_shape_and_delete_with_confirmation() {
    let sandbox = Sandbox::new();
    sandbox.write_command_entry();
    fs::create_dir_all(sandbox.state.path().join("values")).unwrap();
    fs::write(
        sandbox.state.path().join("values/demo.toml"),
        r#"
[values]
name = "Ada"

[last_run]
at = "2026-08-08T00:00:00Z"
exit = 0

[last_run.values]
name = "Ada"
"#,
    )
    .unwrap();

    sandbox
        .command()
        .args(["preset", "save", "demo", "favorite", "--from-last"])
        .assert()
        .success();
    let presets = sandbox.command_json(&["preset", "list", "demo", "--json"]);
    assert_eq!(presets["presets"]["favorite"]["name"], "Ada");
    sandbox
        .command()
        .args(["preset", "delete", "demo", "favorite", "--no-input"])
        .assert()
        .code(2);
    sandbox
        .command()
        .args(["preset", "delete", "demo", "favorite", "--yes"])
        .assert()
        .success();
}

#[test]
fn runner_commands_preserve_argv_tokens_and_remove_by_name() {
    let sandbox = Sandbox::new();
    sandbox
        .command()
        .args([
            "runner",
            "add",
            "reviewer",
            "review-cli",
            "--prompt",
            "{{prompt}}",
        ])
        .assert()
        .success();
    let runners = sandbox.command_json(&["runner", "list", "--json"]);
    assert_eq!(
        runners["runners"]["reviewer"],
        serde_json::json!(["review-cli", "--prompt", "{{prompt}}"])
    );
    sandbox
        .command()
        .args(["runner", "remove", "reviewer", "--yes"])
        .assert()
        .success();
}

#[test]
fn doctor_rebuilds_the_registry_and_reports_all_owned_paths() {
    let sandbox = Sandbox::new();
    sandbox.write_command_entry();
    let report = sandbox.command_json(&["doctor", "--rebuild", "--json"]);
    assert_eq!(report["entries"], 1);
    assert_eq!(report["rebuilt"], 1);
    assert_eq!(
        report["location"],
        sandbox.data.path().join("scripts").display().to_string()
    );
    for key in [
        "uv",
        "missing",
        "drift",
        "needs_missing",
        "launch_blocked",
        "runner_rows_invalid",
        "rebuild_problems",
        "mirror",
        "size_bytes",
    ] {
        assert!(report.get(key).is_some(), "missing doctor key: {key}");
    }
    assert_eq!(report["mirror"]["enabled"], false);
    assert!(report.get("state_location").is_some());
    assert!(report.get("config_location").is_some());
    assert!(sandbox.data.path().join("registry.toml").is_file());
}

#[test]
fn doctor_requires_uv_only_for_empty_or_python_libraries() {
    let empty = Sandbox::new();
    empty
        .command()
        .env("PATH", empty.data.path())
        .args(["doctor", "--json"])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("\"uv\":null"));

    let commands = Sandbox::new();
    commands.write_command_entry();
    commands
        .command()
        .env("PATH", commands.data.path())
        .args(["doctor", "--json"])
        .assert()
        .success();
}

#[test]
fn agent_install_writes_the_exact_bundled_skill_to_an_explicit_target() {
    let sandbox = Sandbox::new();
    let target = sandbox.data.path().join("agent");
    sandbox
        .command()
        .args(["agent", "install", "--to", target.to_str().unwrap()])
        .assert()
        .success();

    let installed = target.join("skit/SKILL.md");
    assert!(installed.is_file());
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("skills/skit/SKILL.md");
    assert_eq!(fs::read(installed).unwrap(), fs::read(source).unwrap());
}

#[test]
fn edit_uses_the_configured_editor_and_refuses_payloadless_entries() {
    let sandbox = Sandbox::new();
    sandbox.write_command_entry();
    sandbox
        .command()
        .args(["config", "editor", "true"])
        .assert()
        .success();
    sandbox
        .command()
        .args(["edit", "demo"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("editable source"));
}
