use std::{fs, path::Path};

use predicates::prelude::*;
use serde_json::Value;
use skit_store::FileStore;
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
        FileStore::new(self.data.path()).rebuild_registry().unwrap();
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
        .stdout(predicate::str::contains("edit"))
        .stdout(predicate::str::contains("--install-completion"))
        .stdout(predicate::str::contains("--show-completion"));

    sandbox
        .command()
        .arg("--version")
        .assert()
        .success()
        .stdout("skit 0.5.0\n");

    sandbox
        .command()
        .arg("-V")
        .assert()
        .success()
        .stdout("skit 0.5.0\n");

    sandbox
        .command()
        .args(["--version", "list"])
        .assert()
        .success()
        .stdout("skit 0.5.0\n");

    for arguments in [["--version", "unknown"], ["-V", "unknown"]] {
        sandbox
            .command()
            .args(arguments)
            .assert()
            .failure()
            .stdout(predicate::str::is_empty());
    }
}

#[test]
fn completion_scripts_are_available_without_opening_the_tui() {
    let sandbox = Sandbox::new();
    sandbox
        .command()
        .env("SHELL", "/bin/bash")
        .arg("--show-completion")
        .assert()
        .success()
        .stdout(predicate::str::contains("_skit"));
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
    let command = sandbox.command_json(&["show", "print", "--json"]);
    let fields = command["fields"].as_array().unwrap();
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0]["key"], "value");
    let command_meta =
        fs::read_to_string(sandbox.data.path().join("scripts/print/meta.toml")).unwrap();
    assert!(command_meta.contains("params = [\"value\"]"));
    assert!(!command_meta.contains("[[parameters]]"));
    assert_eq!(
        fs::read(sandbox.data.path().join("scripts/python-tool/script.py")).unwrap(),
        b"print('hello')\n"
    );
}

#[test]
fn prompt_add_flood_guard_does_not_manage_more_than_thirty_detected_holes() {
    let sandbox = Sandbox::new();
    let prompt = sandbox.data.path().join("large.prompt.md");
    let body = (0..31)
        .map(|index| format!("{{{{field_{index}}}}}"))
        .collect::<Vec<_>>()
        .join(" ");
    fs::write(&prompt, body).unwrap();

    sandbox
        .command()
        .args(["add", prompt.to_str().unwrap(), "--name", "Large prompt"])
        .assert()
        .success();

    let shown = sandbox.command_json(&["show", "large-prompt", "--json"]);
    assert!(shown["fields"].as_array().unwrap().is_empty());
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
fn preset_commands_use_the_existing_state_shape_and_keep_direct_delete_automation() {
    let sandbox = Sandbox::new();
    sandbox.write_command_entry();
    let skill = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("skills/skit/SKILL.md"),
    )
    .unwrap();
    assert!(skill.contains("skit preset delete <name> nightly -y"));
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
    assert_eq!(presets["favorite"]["name"], "Ada");
    sandbox
        .command()
        .args(["preset", "delete", "demo", "favorite", "--no-input"])
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
    let reviewer = runners
        .as_array()
        .unwrap()
        .iter()
        .find(|runner| runner["name"] == "reviewer")
        .unwrap();
    assert_eq!(
        reviewer["argv"],
        serde_json::json!(["review-cli", "--prompt", "{{prompt}}"])
    );
    sandbox
        .command()
        .args(["runner", "remove", "reviewer", "--yes"])
        .assert()
        .success();

    fs::write(
        sandbox.config.path().join("config.toml"),
        r#"[prompt]
runners_seeded = true
runners = [
  { name = "broken", argv = ["no marker"], keep = 1 },
  { name = "valid", argv = ["agent", "{{prompt}}"] },
]
"#,
    )
    .unwrap();
    let rows = sandbox.command_json(&["runner", "list", "--all", "--json"]);
    assert_eq!(rows[0]["row"], 0);
    assert_eq!(rows[0]["valid"], false);
    sandbox
        .command()
        .args(["runner", "remove", "--row", "0", "--yes"])
        .assert()
        .success();
    let rows = sandbox.command_json(&["runner", "list", "--all", "--json"]);
    assert_eq!(rows.as_array().unwrap().len(), 1);
    assert_eq!(rows[0]["name"], "valid");
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
fn doctor_keeps_the_v040_fresh_install_uv_check() {
    let empty = Sandbox::new();
    empty
        .command()
        .env("PATH", empty.data.path())
        .args(["doctor", "--json"])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("\"uv\":null"));

    // A non-empty library with no python entries runs fine without uv: the report
    // still shows uv as absent, but the exit code is 0 (uv "not required").
    let commands = Sandbox::new();
    commands.write_command_entry();
    commands
        .command()
        .env("PATH", commands.data.path())
        .args(["doctor", "--json"])
        .assert()
        .code(0)
        .stdout(predicate::str::contains("\"uv\":null"));

    let python = Sandbox::new();
    let source = python.data.path().join("tool.py");
    fs::write(&source, "print(1)\n").unwrap();
    python
        .command()
        .args(["add", source.to_str().unwrap(), "--name", "Tool"])
        .assert()
        .success();
    python
        .command()
        .env("PATH", python.data.path())
        .args(["doctor", "--json"])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("\"uv\":null"));
    python
        .command()
        .env("PATH", python.data.path())
        .args(["doctor"])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("ERROR uv: not found"));
}

#[test]
fn doctor_rebuild_keeps_the_uv_exit_rule_over_a_clean_report() {
    // --rebuild does not change the uv exit rule, and the rule reads only uv. A python library
    // that rebuilds with no problem at all still exits 1 without uv, on both output paths. The
    // sibling `doctor_keeps_the_v040_fresh_install_uv_check` owns the same rule without
    // --rebuild.
    let sandbox = Sandbox::new();
    let source = sandbox.data.path().join("tool.py");
    fs::write(&source, "print(1)\n").unwrap();
    sandbox
        .command()
        .args(["add", source.to_str().unwrap(), "--name", "Tool"])
        .assert()
        .success();

    // PATH holds the data directory, which carries no uv, and there is no private uv below it.
    let output = sandbox
        .command()
        .env("PATH", sandbox.data.path())
        .args(["doctor", "--rebuild", "--json"])
        .output()
        .unwrap();
    let report: Value = serde_json::from_slice(&output.stdout).expect("doctor --json is one doc");
    // The report names no problem, so only the missing uv can explain the exit code.
    assert_eq!(report["uv"], Value::Null);
    assert_eq!(report["entries"], 1);
    assert_eq!(report["rebuilt"], 1);
    assert_eq!(report["rebuild_problems"], serde_json::json!([]));
    assert_eq!(report["diagnostics"], serde_json::json!([]));
    assert_eq!(report["missing"], serde_json::json!([]));
    assert_eq!(report["drift"], serde_json::json!([]));
    assert_eq!(output.status.code(), Some(1), "{report}");

    // The human path reports the same rebuild and the same exit code.
    sandbox
        .command()
        .env("PATH", sandbox.data.path())
        .args(["doctor", "--rebuild"])
        .assert()
        .code(1)
        .stdout(
            predicate::str::contains("ERROR uv: not found")
                .and(predicate::str::contains("Index rebuilt: 1 entry")),
        );
}

#[test]
fn doctor_rebuild_keeps_the_uv_exit_rule_for_every_library_shape() {
    // The rule that selects the exit code reads the library the same way with --rebuild as
    // without it: an empty library needs uv, and a library with no python entry does not.
    let empty = Sandbox::new();
    empty
        .command()
        .env("PATH", empty.data.path())
        .args(["doctor", "--rebuild", "--json"])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("\"uv\":null"));
    empty
        .command()
        .env("PATH", empty.data.path())
        .args(["doctor", "--rebuild"])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("ERROR uv: not found"));

    let commands = Sandbox::new();
    commands.write_command_entry();
    commands
        .command()
        .env("PATH", commands.data.path())
        .args(["doctor", "--rebuild", "--json"])
        .assert()
        .code(0)
        .stdout(predicate::str::contains("\"uv\":null"));
}

#[test]
fn doctor_accepts_the_v040_private_uv_location() {
    let sandbox = Sandbox::new();
    let bin = sandbox.data.path().join("bin");
    fs::create_dir_all(&bin).unwrap();
    let uv = bin.join(if cfg!(windows) { "uv.exe" } else { "uv" });
    fs::write(&uv, "private uv").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mut permissions = fs::metadata(&uv).unwrap().permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&uv, permissions).unwrap();
    }

    sandbox
        .command()
        .env("PATH", "")
        .args(["doctor", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains(uv.display().to_string()));
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
    sandbox
        .command()
        .args(["add", "--cmd", "echo ok", "--name", "Command"])
        .assert()
        .success();
    let executable = sandbox.data.path().join("program.bin");
    fs::write(&executable, "program\n").unwrap();
    sandbox
        .command()
        .args([
            "add",
            executable.to_str().unwrap(),
            "--exe",
            "--name",
            "Program",
        ])
        .assert()
        .success();
    sandbox
        .command()
        .args(["config", "editor", "true"])
        .assert()
        .success();
    for selector in ["command", "program"] {
        sandbox
            .command()
            .args(["edit", selector, "--no-input"])
            .assert()
            .code(1)
            .stderr(predicate::str::contains("editable source"));
    }

    let missing = sandbox.data.path().join("missing.py");
    fs::write(&missing, "print('gone')\n").unwrap();
    sandbox
        .command()
        .args(["add", missing.to_str().unwrap(), "--name", "Missing"])
        .assert()
        .success();
    fs::remove_file(sandbox.data.path().join("scripts/missing/script.py")).unwrap();
    sandbox
        .command()
        .args(["edit", "missing", "--no-input"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("no stored copy to edit"));
}
