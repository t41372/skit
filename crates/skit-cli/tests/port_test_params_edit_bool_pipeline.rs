//! Exact-name CLI edit-pipeline ports of the boolean-action hygiene section in Python v0.4
//! `tests/test_params_edit.py`.
//!
//! `skit-application::finish_parameter_edit` already has granular unit coverage; these tests prove
//! the complete params edit transaction applies that hygiene at the persisted public boundary and
//! rolls an invalid combined edit back as one row, matching Python `edit_declared`.

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

    fn ok(&self, args: &[&str]) -> std::process::Output {
        let output = self.command().args(args).output().unwrap();
        assert!(
            output.status.success(),
            "args={args:?}\nstdout={}\nstderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
        output
    }

    fn add_exe(&self) {
        let source = self.home.path().join("binary");
        fs::create_dir(&source).unwrap();
        self.ok(&["add", source.to_str().unwrap(), "--exe", "--name", "binary"]);
    }

    fn params(&self) -> Value {
        let output = self.ok(&["params", "binary", "--json"]);
        serde_json::from_slice(&output.stdout).unwrap()
    }
}

fn row<'a>(document: &'a Value, name: &str) -> &'a Value {
    document["parameters"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["name"] == name)
        .unwrap()
}

fn text<'a>(row: &'a Value, key: &str) -> &'a str {
    row[key].as_str().unwrap_or("")
}

#[test]
fn test_type_tweak_to_bool_on_a_flag_sets_store_true() {
    let sandbox = Sandbox::new();
    sandbox.add_exe();
    sandbox.ok(&[
        "params", "binary", "--add", "v", "--flag", "v=--v", "--type", "v=bool",
    ]);
    let document = sandbox.params();
    let v = row(&document, "v");
    assert_eq!(text(v, "type"), "bool");
    assert_eq!(text(v, "action"), "store_true");
}

#[test]
fn test_type_tweak_to_bool_on_a_positional_keeps_empty_action() {
    let sandbox = Sandbox::new();
    sandbox.add_exe();
    sandbox.ok(&[
        "params", "binary", "--add", "b", "--flag", "b=", "--type", "b=bool",
    ]);
    let document = sandbox.params();
    let b = row(&document, "b");
    assert_eq!(text(b, "type"), "bool");
    assert_eq!(text(b, "action"), "");
}

#[test]
fn test_type_tweak_to_bool_on_env_delivery_keeps_empty_action() {
    let sandbox = Sandbox::new();
    sandbox.add_exe();
    sandbox.ok(&[
        "params", "binary", "--add", "v", "--deliver", "v=env", "--flag", "v=--v", "--type", "v=bool",
    ]);
    let document = sandbox.params();
    let v = row(&document, "v");
    assert_eq!(text(v, "type"), "bool");
    assert_eq!(text(v, "delivery"), "env");
    assert_eq!(text(v, "action"), "");
}

#[test]
fn test_type_tweak_off_bool_sheds_stale_action() {
    let sandbox = Sandbox::new();
    sandbox.add_exe();
    sandbox.ok(&[
        "params", "binary", "--add", "v", "--flag", "v=--v", "--type", "v=bool",
    ]);
    assert_eq!(text(row(&sandbox.params(), "v"), "action"), "store_true");
    sandbox.ok(&["params", "binary", "--type", "v=str"]);
    let document = sandbox.params();
    let v = row(&document, "v");
    assert_eq!(text(v, "type"), "str");
    assert_eq!(text(v, "action"), "");
}

#[test]
fn test_non_type_tweak_on_a_bool_leaves_its_action_alone() {
    let sandbox = Sandbox::new();
    sandbox.add_exe();
    sandbox.ok(&[
        "params", "binary", "--add", "c", "--flag", "c=--no-c", "--type", "c=bool", "--action", "c=store_false",
    ]);
    sandbox.ok(&["params", "binary", "--default", "c=true"]);
    let document = sandbox.params();
    assert_eq!(text(row(&document, "c"), "action"), "store_false");
}

#[test]
fn test_non_type_tweak_on_a_str_with_stale_action_clears_it() {
    let sandbox = Sandbox::new();
    sandbox.add_exe();
    sandbox.ok(&[
        "params", "binary", "--add", "a", "--flag", "a=--a", "--action", "a=store_true",
    ]);
    assert_eq!(text(row(&sandbox.params(), "a"), "action"), "");
    // Re-seed a stale action through a single transaction whose type ends as bool, then move off
    // bool in a separate operation before exercising an unrelated tweak.
    sandbox.ok(&["params", "binary", "--type", "a=bool"]);
    assert_eq!(text(row(&sandbox.params(), "a"), "action"), "store_true");
    sandbox.ok(&["params", "binary", "--type", "a=str"]);
    sandbox.ok(&["params", "binary", "--help-text", "a=x"]);
    let document = sandbox.params();
    assert_eq!(text(row(&document, "a"), "action"), "");
}

#[test]
fn test_bool_flag_that_is_on_by_default_is_refused_not_stamped() {
    let sandbox = Sandbox::new();
    sandbox.add_exe();
    sandbox.ok(&["params", "binary", "--add", "v", "--flag", "v=--v"]);
    let before = row(&sandbox.params(), "v").clone();

    let output = sandbox.ok(&[
        "params", "binary", "--type", "v=bool", "--default", "v=true",
    ]);
    let message = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        message.contains("v is on by default") && message.contains("turn it OFF"),
        "the refusal warning must explain the unusable positive flag:\n{message}"
    );
    let document = sandbox.params();
    assert_eq!(
        row(&document, "v"),
        &before,
        "the entire invalid combined edit must roll back"
    );
}

#[test]
fn test_bool_flag_that_is_off_by_default_still_gets_store_true() {
    let sandbox = Sandbox::new();
    sandbox.add_exe();
    sandbox.ok(&[
        "params", "binary", "--add", "v", "--flag", "v=--v", "--type", "v=bool", "--default", "v=false",
    ]);
    let document = sandbox.params();
    let v = row(&document, "v");
    assert_eq!(text(v, "type"), "bool");
    assert_eq!(v["default"], false);
    assert_eq!(text(v, "action"), "store_true");
}
