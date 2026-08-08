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

    fn add_command(&self, name: &str, template: &str) {
        self.command()
            .args(["add", "--cmd", template, "--name", name])
            .assert()
            .success();
    }
}

#[test]
fn list_json_keeps_the_v040_array_and_complete_row_shape() {
    let sandbox = Sandbox::new();
    sandbox.add_command("Demo", "printf '%s' {name}");

    let output = sandbox.command().args(["list", "--json"]).output().unwrap();
    assert!(output.status.success());
    let rows: Value = serde_json::from_slice(&output.stdout).unwrap();
    let rows = rows.as_array().expect("v0.4 list JSON is an array");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["name"], "Demo");
    assert_eq!(rows[0]["slug"], "demo");
    assert_eq!(rows[0]["kind"], "command");
    assert_eq!(rows[0]["mode"], "copy");
    assert_eq!(rows[0]["missing"], false);
    assert!(rows[0].get("last_run_at").is_some());
    assert!(rows[0].get("last_exit").is_some());
}

#[test]
fn show_json_keeps_the_v040_automation_record() {
    let sandbox = Sandbox::new();
    sandbox.add_command("Demo", "printf '%s' {name}");
    let output = sandbox
        .command()
        .args(["show", "demo", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let record: Value = serde_json::from_slice(&output.stdout).unwrap();

    for key in [
        "name",
        "slug",
        "kind",
        "mode",
        "description",
        "source",
        "workdir",
        "interpreter",
        "missing",
        "dependencies",
        "requires_python",
        "needs",
        "template",
        "param_source",
        "param_origin",
        "degraded_reason",
        "drift",
        "fields",
        "presets",
        "last_run_at",
        "last_exit",
    ] {
        assert!(record.get(key).is_some(), "missing JSON key: {key}");
    }
    assert_eq!(record["param_source"], "command");
    assert_eq!(record["fields"][0]["key"], "name");
}

#[test]
fn add_reads_stdin_and_accepts_python_dependency_options_without_prompting() {
    let sandbox = Sandbox::new();
    sandbox
        .command()
        .args([
            "add",
            "-",
            "--name",
            "Pipe tool",
            "--kind",
            "python",
            "--dep",
            "requests>=2",
            "--python",
            ">=3.12",
            "--no-input",
        ])
        .write_stdin("print('pipe')\n")
        .assert()
        .success();
    assert_eq!(
        fs::read(sandbox.data.path().join("scripts/pipe-tool/script.py")).unwrap(),
        b"print('pipe')\n"
    );
    let deps = sandbox
        .command()
        .args(["deps", "pipe-tool", "--json"])
        .output()
        .unwrap();
    let deps: Value = serde_json::from_slice(&deps.stdout).unwrap();
    assert_eq!(deps["dependencies"], serde_json::json!(["requests>=2"]));
    assert_eq!(deps["requires_python"], ">=3.12");
}

#[test]
fn params_supports_the_remaining_declared_schema_options() {
    let sandbox = Sandbox::new();
    sandbox.add_command("Demo", "echo {name}");
    sandbox
        .command()
        .args([
            "params",
            "demo",
            "--help-text",
            "name=Shown beside the field.",
            "--prompt",
            "name=Your name",
            "--env-source",
            "name=SKIT_TEST_NAME",
            "--secret",
            "name",
        ])
        .assert()
        .success();
    let output = sandbox
        .command()
        .args(["params", "demo", "--json"])
        .output()
        .unwrap();
    let record: Value = serde_json::from_slice(&output.stdout).unwrap();
    let field = &record["parameters"][0];
    assert_eq!(field["help"], "Shown beside the field.");
    assert_eq!(field["prompt"], "Your name");
    assert_eq!(field["env_source"], "SKIT_TEST_NAME");
    assert_eq!(field["secret"], true);
}

#[test]
fn run_uses_user_configured_prompt_runner_rows() {
    let sandbox = Sandbox::new();
    let prompt = sandbox.data.path().join("review.prompt.md");
    fs::write(&prompt, "Review {{subject}}.").unwrap();
    sandbox
        .command()
        .args(["add", prompt.to_str().unwrap(), "--name", "Review"])
        .assert()
        .success();
    sandbox
        .command()
        .args(["runner", "add", "custom", "printf", "%s", "{{prompt}}"])
        .assert()
        .success();
    sandbox
        .command()
        .args([
            "run",
            "review",
            "--runner",
            "custom",
            "--set",
            "subject=Rust",
            "--no-input",
        ])
        .assert()
        .success()
        .stdout("Review Rust.");
}

#[test]
fn raw_mode_refuses_template_artifacts_without_writing_state() {
    let sandbox = Sandbox::new();
    sandbox.add_command("Demo", "echo {name}");
    sandbox
        .command()
        .args(["run", "demo", "--raw"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("does not apply"));
    assert!(!sandbox.state.path().join("values/demo.toml").exists());
}

#[test]
fn agent_explicit_to_remains_the_v040_skills_directory_contract() {
    let sandbox = Sandbox::new();
    let target = sandbox.data.path().join("skills");
    sandbox
        .command()
        .args(["agent", "install", "--to", target.to_str().unwrap()])
        .assert()
        .success();
    assert!(target.join("skit/SKILL.md").is_file());
    assert!(!target.join("skills/skit/SKILL.md").exists());
}

#[test]
fn deps_refuses_package_axes_for_kinds_without_package_management() {
    let sandbox = Sandbox::new();
    sandbox.add_command("Demo", "echo ok");
    sandbox
        .command()
        .args(["deps", "demo", "--dep", "requests"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "does not take package dependencies",
        ));
    sandbox
        .command()
        .args(["deps", "demo", "--need", "printf"])
        .assert()
        .success();
}
