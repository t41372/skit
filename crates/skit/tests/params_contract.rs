use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use serde_json::json;
use tempfile::tempdir;

fn write(path: &Path, content: &str) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)?;
    Ok(())
}

fn run(root: &Path, args: &[&str]) -> Result<Output, Box<dyn std::error::Error>> {
    Ok(Command::new(env!("CARGO_BIN_EXE_skit"))
        .args(args)
        .env("SKIT_DATA_DIR", root.join("data"))
        .env("SKIT_STATE_DIR", root.join("state"))
        .env("SKIT_CONFIG_DIR", root.join("config"))
        .output()?)
}

fn command_fixture(root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    write(
        &root.join("data/scripts/demo/meta.toml"),
        r#"schema = 1
name = "Demo"
kind = "command"
mode = "copy"
workdir = "invoke"
template = "convert {size}"
params = ["size"]
"#,
    )
}

fn exe_fixture(root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    write(
        &root.join("data/scripts/tool/meta.toml"),
        r#"schema = 1
name = "Tool"
kind = "exe"
mode = "reference"
source = "/missing/tool"
workdir = "origin"
"#,
    )
}

#[test]
fn command_placeholder_can_be_declared_and_typed_in_one_call()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    command_fixture(root.path())?;

    let output = run(
        root.path(),
        &[
            "params",
            "Demo",
            "--add",
            "size",
            "--type",
            "size=choice",
            "--choices",
            "size=s,m",
            "--default",
            "size=m",
            "--required",
            "size",
            "--json",
        ],
    )?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let actual: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(actual["param_source"], "command");
    assert_eq!(actual["fields"][0]["key"], "size");
    assert_eq!(actual["fields"][0]["source"], "placeholder");
    assert_eq!(actual["fields"][0]["type"], "choice");
    assert_eq!(actual["fields"][0]["required"], true);
    assert_eq!(actual["fields"][0]["default"], "m");
    assert_eq!(actual["fields"][0]["choices"], json!(["s", "m"]));

    let meta = fs::read_to_string(root.path().join("data/scripts/demo/meta.toml"))?;
    assert!(meta.contains("params = [\"size\"]"));
    assert!(meta.contains("[[parameters]]"));
    assert!(meta.contains("delivery = \"placeholder\""));
    Ok(())
}

#[test]
fn exe_declared_flag_roundtrips_through_machine_schema() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    exe_fixture(root.path())?;

    let output = run(
        root.path(),
        &[
            "params",
            "Tool",
            "--add",
            "width",
            "--type",
            "width=int",
            "--flag",
            "width=  --width  ",
            "--required",
            "width",
            "--json",
        ],
    )?;
    assert!(output.status.success());
    let actual: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(actual["fields"][0]["key"], "width");
    assert_eq!(actual["fields"][0]["source"], "flag");
    assert_eq!(actual["fields"][0]["type"], "int");
    assert_eq!(actual["fields"][0]["flag"], "--width");
    assert_eq!(actual["fields"][0]["required"], true);
    Ok(())
}

#[test]
fn invalid_type_is_usage_and_does_not_materialize_parameter_rows()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    command_fixture(root.path())?;
    let before = fs::read_to_string(root.path().join("data/scripts/demo/meta.toml"))?;

    let output = run(
        root.path(),
        &["params", "Demo", "--add", "size", "--type", "size=integer"],
    )?;
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8(output.stderr)?.contains("Unknown parameter type"));
    assert_eq!(
        fs::read_to_string(root.path().join("data/scripts/demo/meta.toml"))?,
        before
    );
    Ok(())
}

#[test]
fn secret_transition_scrubs_every_old_state_value_surface()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    command_fixture(root.path())?;
    write(
        &root.path().join("state/values/demo.toml"),
        r#"[values]
size = "m"
TOKEN = "old-secret"

[presets.prod]
TOKEN = "old-secret"

[last_run]
at = "2026-08-08T12:00:00+00:00"
exit = 0

[last_run.values]
TOKEN = "old-secret"
"#,
    )?;

    let output = run(
        root.path(),
        &[
            "params",
            "Demo",
            "--add",
            "TOKEN",
            "--deliver",
            "TOKEN=env",
            "--secret",
            "TOKEN",
            "--env-source",
            "TOKEN= API_TOKEN ",
            "--json",
        ],
    )?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let actual: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    let token = actual["fields"]
        .as_array()
        .and_then(|fields| fields.iter().find(|field| field["key"] == "TOKEN"))
        .ok_or("TOKEN field missing")?;
    assert_eq!(token["secret"], true);
    assert_eq!(token["env_source"], "API_TOKEN");

    let raw_state = fs::read_to_string(root.path().join("state/values/demo.toml"))?;
    assert!(!raw_state.contains("old-secret"));
    assert!(!raw_state.contains("TOKEN"));
    assert!(raw_state.contains("size = \"m\""));
    Ok(())
}

#[test]
fn source_managed_kind_editing_is_explicitly_refused_without_writing()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    write(
        &root.path().join("data/scripts/py/meta.toml"),
        r#"name = "Py"
kind = "python"
mode = "copy"
workdir = "invoke"
"#,
    )?;
    write(&root.path().join("data/scripts/py/script.py"), "print(1)\n")?;
    let before = fs::read_to_string(root.path().join("data/scripts/py/meta.toml"))?;

    let output = run(root.path(), &["params", "Py", "--add", "x"])?;
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8(output.stderr)?.contains("not enabled for python"));
    assert_eq!(
        fs::read_to_string(root.path().join("data/scripts/py/meta.toml"))?,
        before
    );
    Ok(())
}
