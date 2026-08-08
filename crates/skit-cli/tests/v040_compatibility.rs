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
    assert_eq!(rows[0]["mode"], "reference");
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
    let stored =
        fs::read_to_string(sandbox.data.path().join("scripts/pipe-tool/script.py")).unwrap();
    assert!(stored.contains("# /// script"));
    assert!(stored.contains("dependencies = [\"requests>=2\"]"));
    assert!(stored.contains("requires-python = \">=3.12\""));
    assert!(stored.ends_with("print('pipe')\n"));
    let meta = fs::read_to_string(sandbox.data.path().join("scripts/pipe-tool/meta.toml")).unwrap();
    assert!(!meta.contains("dependencies ="));
    assert!(!meta.contains("requires_python ="));
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
fn params_json_keeps_discovery_defaults_declared_and_state_surfaces() {
    let sandbox = Sandbox::new();
    let source = sandbox.data.path().join("tool.sh");
    fs::write(&source, "NAME=world\necho \"$NAME\"\n").unwrap();
    sandbox
        .command()
        .args(["add", source.to_str().unwrap(), "--name", "Tool"])
        .assert()
        .success();
    fs::create_dir_all(sandbox.state.path().join("values")).unwrap();
    fs::write(
        sandbox.state.path().join("values/tool.toml"),
        "[values]\nNAME = \"remembered\"\n",
    )
    .unwrap();

    let output = sandbox
        .command()
        .args(["params", "tool", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let record: Value = serde_json::from_slice(&output.stdout).unwrap();
    for key in [
        "params",
        "parameters",
        "current_defaults",
        "last_values",
        "unmanaged",
        "placeholders",
        "declared",
    ] {
        assert!(record.get(key).is_some(), "missing params JSON key: {key}");
    }
    assert_eq!(record["params"], serde_json::json!([]));
    assert_eq!(record["current_defaults"], serde_json::json!({}));
    assert_eq!(record["last_values"]["NAME"], "remembered");
    assert_eq!(record["unmanaged"], serde_json::json!(["NAME"]));

    sandbox
        .command()
        .args(["params", "tool", "--manage", "NAME"])
        .assert()
        .success();
    let output = sandbox
        .command()
        .args(["params", "tool", "--json"])
        .output()
        .unwrap();
    let record: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(record["params"][0]["name"], "NAME");
    assert_eq!(record["current_defaults"]["NAME"], "world");
    assert_eq!(record["unmanaged"], serde_json::json!([]));
}

#[test]
fn params_cli_can_set_every_frontend_neutral_parameter_axis() {
    let sandbox = Sandbox::new();
    sandbox.add_command("Demo", "echo {name}");
    sandbox
        .command()
        .args([
            "params",
            "demo",
            "--binding",
            "name=none",
            "--multiple",
            "name",
            "--repeat",
            "name",
            "--env-target",
            "name=SKIT_NAME",
            "--action",
            "name=store_true",
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
    assert_eq!(field["binding"], "none");
    assert_eq!(field["multiple"], true);
    assert_eq!(field["repeat"], true);
    assert_eq!(field["env_target"], "SKIT_NAME");
    assert_eq!(field["action"], "store_true");

    sandbox
        .command()
        .args([
            "params",
            "demo",
            "--no-multiple",
            "name",
            "--no-repeat",
            "name",
        ])
        .assert()
        .success();
}

#[test]
fn params_refuses_inapplicable_or_order_dependent_operations_without_a_write() {
    let sandbox = Sandbox::new();
    sandbox.add_command("Demo", "echo {name}");
    let meta_path = sandbox.data.path().join("scripts/demo/meta.toml");
    let before = fs::read(&meta_path).unwrap();

    for arguments in [
        vec!["params", "demo", "--runner", ""],
        vec!["params", "demo", "--no-interpolate"],
        vec!["params", "demo", "--interpreter", "bash"],
        vec!["params", "demo", "--workdir", "relative/path"],
        vec!["params", "demo", "--runner", "", "--add", "other"],
        vec!["params", "demo", "--template", "", "--add", "other"],
    ] {
        sandbox.command().args(arguments).assert().code(2);
        assert_eq!(fs::read(&meta_path).unwrap(), before);
    }
}

#[test]
fn show_json_reports_reader_field_sources_and_effective_python_metadata() {
    let sandbox = Sandbox::new();
    let python = sandbox.data.path().join("reader.py");
    fs::write(
        &python,
        "import argparse\np = argparse.ArgumentParser()\np.add_argument('--count', type=int)\n",
    )
    .unwrap();
    sandbox
        .command()
        .args([
            "add",
            python.to_str().unwrap(),
            "--name",
            "Reader",
            "--dep",
            "requests>=2",
            "--python",
            ">=3.12",
        ])
        .assert()
        .success();

    let output = sandbox
        .command()
        .args(["show", "reader", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let record: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(record["dependencies"], serde_json::json!(["requests>=2"]));
    assert_eq!(record["requires_python"], ">=3.12");
    assert_eq!(record["param_source"], "argparse");
    assert_eq!(record["param_origin"], "reader");
    assert_eq!(record["fields"][0]["source"], "flag");
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
fn bare_agent_install_never_creates_an_unselected_third_party_directory() {
    let sandbox = Sandbox::new();
    let home = TempDir::new().unwrap();
    sandbox
        .command()
        .env("HOME", home.path())
        .args(["agent", "install"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("select an agent convention"));
    for directory in [".agents", ".claude", ".codex"] {
        assert!(!home.path().join(directory).exists());
    }
}

#[test]
fn bare_agent_install_uses_the_only_existing_agent_directory() {
    let sandbox = Sandbox::new();
    let home = TempDir::new().unwrap();
    fs::create_dir(home.path().join(".codex")).unwrap();
    sandbox
        .command()
        .env("HOME", home.path())
        .args(["agent", "install"])
        .assert()
        .success();
    assert!(home.path().join(".codex/skills/skit/SKILL.md").is_file());
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
