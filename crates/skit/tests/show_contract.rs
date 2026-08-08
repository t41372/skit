use std::fs;
use std::path::Path;
use std::process::Command;

use serde_json::json;
use tempfile::tempdir;

fn write(path: &Path, content: &str) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)?;
    Ok(())
}

fn run_show(
    data: &Path,
    state: &Path,
    config: &Path,
    name: &str,
) -> Result<std::process::Output, Box<dyn std::error::Error>> {
    Ok(Command::new(env!("CARGO_BIN_EXE_skit"))
        .args(["show", name, "--json"])
        .env("SKIT_DATA_DIR", data)
        .env("SKIT_STATE_DIR", state)
        .env("SKIT_CONFIG_DIR", config)
        .output()?)
}

#[test]
fn show_json_declared_exe_keeps_the_stable_top_level_and_field_shape()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let data = root.path().join("data");
    let state = root.path().join("state");
    let config = root.path().join("config");
    write(
        &data.join("scripts/tool/meta.toml"),
        r#"schema = 1
name = "Tool"
kind = "exe"
mode = "reference"
source = "/definitely/missing/skit-tool"
workdir = "origin"
description = "External program"
needs = ["jq"]

[[parameters]]
name = "width"
delivery = "flag"
type = "int"
default = 800
flag = "--width"
help = "target width"

[[parameters]]
name = "API_TOKEN"
delivery = "env"
type = "str"
secret = true
env_source = "TOKEN_FROM_ENV"
"#,
    )?;
    write(
        &state.join("values/tool.toml"),
        r#"[presets.prod]
width = "1200"

[last_run]
at = "2026-08-08T05:00:00+00:00"
exit = 3
"#,
    )?;

    let output = run_show(&data, &state, &config, "Tool")?;
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let actual: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(
        actual,
        json!({
            "name": "Tool",
            "slug": "tool",
            "kind": "exe",
            "mode": "reference",
            "description": "External program",
            "source": "/definitely/missing/skit-tool",
            "workdir": "origin",
            "interpreter": null,
            "missing": true,
            "dependencies": [],
            "requires_python": "",
            "needs": ["jq"],
            "template": null,
            "param_source": "declared",
            "param_origin": "declared",
            "degraded_reason": "",
            "drift": false,
            "fields": [
                {
                    "key": "width",
                    "label": "width",
                    "type": "int",
                    "source": "flag",
                    "required": false,
                    "secret": false,
                    "multiple": false,
                    "repeat": false,
                    "degraded": false,
                    "choices": [],
                    "default": "800",
                    "help": "target width",
                    "flag": "--width",
                    "action": "",
                    "env_source": "",
                    "delivers_empty": false
                },
                {
                    "key": "API_TOKEN",
                    "label": "API_TOKEN",
                    "type": "str",
                    "source": "env",
                    "required": false,
                    "secret": true,
                    "multiple": false,
                    "repeat": false,
                    "degraded": false,
                    "choices": [],
                    "default": null,
                    "help": "",
                    "flag": "",
                    "action": "",
                    "env_source": "TOKEN_FROM_ENV",
                    "delivers_empty": false
                }
            ],
            "presets": ["prod"],
            "last_run_at": "2026-08-08T05:00:00+00:00",
            "last_exit": 3
        })
    );
    Ok(())
}

#[test]
fn show_json_command_synthesizes_undeclared_placeholders_and_secret_hint()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let data = root.path().join("data");
    let state = root.path().join("state");
    let config = root.path().join("config");
    write(
        &data.join("scripts/deploy/meta.toml"),
        r#"name = "deploy"
kind = "command"
template = "echo {target} {api_key}"
params = ["target", "api_key"]
"#,
    )?;

    let output = run_show(&data, &state, &config, "deploy")?;
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let actual: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(actual["param_source"], "command");
    assert_eq!(actual["param_origin"], "command");
    assert_eq!(actual["template"], "echo {target} {api_key}");
    assert_eq!(actual["missing"], false);
    assert_eq!(actual["fields"][0]["key"], "target");
    assert_eq!(actual["fields"][0]["source"], "placeholder");
    assert_eq!(actual["fields"][0]["required"], true);
    assert_eq!(actual["fields"][0]["secret"], false);
    assert_eq!(actual["fields"][1]["key"], "api_key");
    assert_eq!(actual["fields"][1]["secret"], true);
    Ok(())
}

#[test]
fn show_missing_entry_is_failure_and_does_not_emit_json() -> Result<(), Box<dyn std::error::Error>>
{
    let root = tempdir()?;
    let output = run_show(
        &root.path().join("data"),
        &root.path().join("state"),
        &root.path().join("config"),
        "missing",
    )?;
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8(output.stderr)?.contains("not found"));
    Ok(())
}
