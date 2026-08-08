use std::fs;
use std::path::Path;
use std::process::Command;

use serde_json::json;
use tempfile::tempdir;

const META: &str = r#"schema = 1
name = "Missing Ref"
kind = "shell"
mode = "reference"
source = "/definitely/missing/skit-script.sh"
source_hash = "sha256:0123456789abcdef"
added_at = "2026-07-22T00:00:00+00:00"
workdir = "origin"
description = "Broken source"
"#;

const STATE: &str = r#"[last_run]
at = "2026-07-22T01:02:03+00:00"
exit = 7
"#;

fn write(path: &Path, content: &str) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)?;
    Ok(())
}

#[test]
fn version_uses_the_stable_machine_spelling() -> Result<(), Box<dyn std::error::Error>> {
    let output = Command::new(env!("CARGO_BIN_EXE_skit"))
        .arg("--version")
        .output()?;

    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout)?, "skit 0.5.0\n");
    assert!(output.stderr.is_empty());
    Ok(())
}

#[test]
fn list_json_keeps_the_existing_machine_contract() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let data = root.path().join("data");
    let state = root.path().join("state");
    let config = root.path().join("config");

    write(&data.join("scripts/missing-ref/meta.toml"), META)?;
    write(&state.join("values/missing-ref.toml"), STATE)?;

    let output = Command::new(env!("CARGO_BIN_EXE_skit"))
        .args(["list", "--json"])
        .env("SKIT_DATA_DIR", &data)
        .env("SKIT_STATE_DIR", &state)
        .env("SKIT_CONFIG_DIR", &config)
        .output()?;

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let actual: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(
        actual,
        json!([{
            "name": "Missing Ref",
            "slug": "missing-ref",
            "kind": "shell",
            "mode": "reference",
            "description": "Broken source",
            "missing": true,
            "last_run_at": "2026-07-22T01:02:03+00:00",
            "last_exit": 7
        }])
    );
    Ok(())
}
