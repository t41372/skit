//! Direct CLI-level ports from Python v0.4 `tests/test_declared_params.py`.
//!
//! These tests intentionally duplicate some lower-level edit findings at the user-facing boundary:
//! Python's CLI behavior is authoritative where a pure helper contract and a higher-level CLI
//! policy differ. JSON tests parse *all* stdout so a leaked human line cannot hide behind slicing.

use std::fs;

use assert_cmd::Command;
use serde_json::Value;
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
            .env_remove("FORCE_COLOR")
            .env_remove("NO_COLOR")
            .env_remove("CLICOLOR")
            .env_remove("CLICOLOR_FORCE")
            .env_remove("PSModulePath")
            .current_dir(self.home.path());
        command
    }

    fn output(&self, args: &[&str]) -> std::process::Output {
        self.command().args(args).output().unwrap()
    }

    fn ok(&self, args: &[&str]) -> std::process::Output {
        let output = self.output(args);
        assert!(
            output.status.success(),
            "args={args:?}\nstdout={}\nstderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
        output
    }

    fn json_stdout(&self, args: &[&str]) -> Value {
        let output = self.ok(args);
        serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
            panic!(
                "stdout must be exactly one JSON document: {error}\nstdout={}\nstderr={}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
            )
        })
    }

    fn add_exe(&self, name: &str) {
        let source = self.home.path().join(format!("{name}-exe"));
        fs::create_dir(&source).unwrap();
        self.ok(&["add", source.to_str().unwrap(), "--exe", "--name", name]);
    }

    fn add_command(&self, name: &str, template: &str) {
        self.ok(&["add", "--cmd", template, "--name", name]);
    }

    fn add_ruby(&self, name: &str) {
        let source = self.home.path().join(format!("{name}.rb"));
        fs::write(&source, "#!/usr/bin/env ruby\nputs 'hi'\n").unwrap();
        self.ok(&["add", source.to_str().unwrap(), "--name", name, "--no-input"]);
    }
}

fn declared<'a>(document: &'a Value, name: &str) -> &'a Value {
    document["declared"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["name"] == name)
        .unwrap_or_else(|| panic!("missing declared row {name}: {document}"))
}

fn combined(output: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn test_cli_declared_edit_with_json_emits_the_final_read_view() {
    let sandbox = Sandbox::new();
    sandbox.add_exe("prog");
    let document = sandbox.json_stdout(&[
        "params",
        "prog",
        "--add",
        "width",
        "--deliver",
        "width=flag",
        "--flag",
        "width=--width",
        "--json",
    ]);
    let width = declared(&document, "width");
    assert_eq!(width["name"], "width");
    assert_eq!(width["delivery"], "flag");
}

#[test]
fn test_cli_env_source_on_non_secret_declared_param_warns() {
    let sandbox = Sandbox::new();
    sandbox.add_exe("prog");
    sandbox.ok(&[
        "params",
        "prog",
        "--add",
        "WIDTH",
        "--deliver",
        "WIDTH=env",
    ]);

    let human = sandbox.output(&["params", "prog", "--env-source", "WIDTH=COLS"]);
    assert!(human.status.success(), "{}", combined(&human));
    assert!(
        String::from_utf8_lossy(&human.stderr).contains("WIDTH isn't secret"),
        "warning must ride stderr:\n{}",
        combined(&human)
    );

    let json = sandbox.output(&[
        "params",
        "prog",
        "--env-source",
        "WIDTH=COLS",
        "--json",
    ]);
    assert!(json.status.success(), "{}", combined(&json));
    let document: Value = serde_json::from_slice(&json.stdout).unwrap_or_else(|error| {
        panic!(
            "--json stdout must stay pure even when the edit warns: {error}\n{}",
            combined(&json)
        )
    });
    assert_eq!(declared(&document, "WIDTH")["name"], "WIDTH");
    assert!(
        String::from_utf8_lossy(&json.stderr).contains("WIDTH isn't secret"),
        "warning must stay off stdout:\n{}",
        combined(&json)
    );
    assert!(
        declared(&document, "WIDTH")["env_source"]
            .as_str()
            .unwrap_or("")
            .is_empty(),
        "a warning-only no-op must not persist env_source: {}",
        declared(&document, "WIDTH")
    );
}

#[test]
fn test_cli_bad_type_warns_and_skips() {
    let sandbox = Sandbox::new();
    sandbox.add_exe("prog");
    sandbox.ok(&["params", "prog", "--add", "w"]);
    let output = sandbox.output(&["params", "prog", "--type", "w=integer"]);
    assert!(
        output.status.success(),
        "bad type is a Python soft warning, not usage failure:\n{}",
        combined(&output)
    );
    assert!(combined(&output).contains("unknown type"), "{}", combined(&output));
    let document = sandbox.json_stdout(&["params", "prog", "--json"]);
    assert_eq!(declared(&document, "w")["type"], "str");
}

#[test]
fn test_cli_declared_malformed_value_warns() {
    let sandbox = Sandbox::new();
    sandbox.add_exe("prog");
    let output = sandbox.output(&["params", "prog", "--type", "NOEQUALS"]);
    assert!(output.status.success(), "{}", combined(&output));
    assert!(
        combined(&output).contains("Ignored a malformed value"),
        "{}",
        combined(&output)
    );
}

#[test]
fn test_cli_rm_declared_param() {
    let sandbox = Sandbox::new();
    sandbox.add_exe("prog");
    sandbox.ok(&[
        "params", "prog", "--add", "a", "--add", "b",
    ]);
    sandbox.ok(&["params", "prog", "--rm", "a"]);
    let document = sandbox.json_stdout(&["params", "prog", "--json"]);
    assert_eq!(
        document["declared"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["b"]
    );
}

#[test]
fn test_declared_add_on_interpreted_meta_kind_defaults_to_deliverable_flag() {
    let sandbox = Sandbox::new();
    sandbox.add_ruby("rb");
    sandbox.ok(&["params", "rb", "--add", "SIZE"]);
    let document = sandbox.json_stdout(&["params", "rb", "--json"]);
    assert_eq!(declared(&document, "SIZE")["delivery"], "flag");
    let dry = sandbox.ok(&[
        "run", "rb", "--set", "SIZE=5", "--dry-run", "--no-input",
    ]);
    let text = String::from_utf8_lossy(&dry.stdout);
    assert!(text.contains("5"), "declared value did not reach dry-run argv:\n{text}");
}

#[test]
fn test_template_add_of_a_non_placeholder_name_creates_a_deliverable_env_row() {
    let sandbox = Sandbox::new();
    sandbox.add_command("tpl", "greet {WHO}");
    sandbox.ok(&["params", "tpl", "--add", "RETRIES"]);
    let document = sandbox.json_stdout(&["params", "tpl", "--json"]);
    assert_eq!(
        declared(&document, "RETRIES")["delivery"],
        "env",
        "a non-slot command parameter must use the delivery a template can actually honor"
    );

    let dry = sandbox.ok(&[
        "run",
        "tpl",
        "--set",
        "WHO=ada",
        "--set",
        "RETRIES=3",
        "--dry-run",
        "--no-input",
    ]);
    let text = String::from_utf8_lossy(&dry.stdout).replace('\n', "");
    assert!(
        text.contains("RETRIES=3"),
        "the env row must actually reach the dry-run delivery surface: {text}"
    );
}

#[test]
fn test_template_add_of_a_real_placeholder_name_still_fills_the_slot() {
    let sandbox = Sandbox::new();
    sandbox.add_command("tpl2", "greet {WHO}");
    sandbox.ok(&["params", "tpl2", "--add", "WHO", "--type", "WHO=str"]);
    let document = sandbox.json_stdout(&["params", "tpl2", "--json"]);
    assert_eq!(declared(&document, "WHO")["delivery"], "placeholder");

    let dry = sandbox.ok(&[
        "run", "tpl2", "--set", "WHO=ada", "--dry-run", "--no-input",
    ]);
    let text = String::from_utf8_lossy(&dry.stdout);
    assert!(text.contains("ada"), "placeholder value did not fill the command: {text}");
}

#[test]
fn test_cli_exe_declared_show_json_param_origin() {
    let sandbox = Sandbox::new();
    sandbox.add_exe("prog");
    sandbox.ok(&[
        "params", "prog", "--add", "w", "--deliver", "w=flag", "--flag", "w=--w", "--type", "w=int",
    ]);
    let document = sandbox.json_stdout(&["show", "prog", "--json"]);
    assert_eq!(document["param_source"], "declared");
    assert_eq!(document["param_origin"], "declared");
    let field = document["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|field| field["key"] == "w")
        .unwrap();
    assert_eq!(field["source"], "flag");
}

#[test]
fn test_cli_exe_no_declared_show_json_param_origin_none() {
    let sandbox = Sandbox::new();
    sandbox.add_exe("prog");
    let document = sandbox.json_stdout(&["show", "prog", "--json"]);
    assert_eq!(document["param_source"], "none");
    assert_eq!(document["param_origin"], "none");
}
