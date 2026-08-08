use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use serde_json::json;
use tempfile::tempdir;

const META: &str = r#"schema = 1
name = "Demo"
kind = "command"
mode = "copy"
source = ""
source_hash = ""
added_at = "2026-07-22T00:00:00+00:00"
workdir = "invoke"
description = ""
template = "echo {NAME}"
params = ["NAME"]
"#;

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

fn fixture(state: Option<&str>) -> Result<tempfile::TempDir, Box<dyn std::error::Error>> {
    let root = tempdir()?;
    write(&root.path().join("data/scripts/demo/meta.toml"), META)?;
    if let Some(state) = state {
        write(&root.path().join("state/values/demo.toml"), state)?;
    }
    Ok(root)
}

#[test]
fn preset_list_json_matches_the_existing_machine_contract() -> Result<(), Box<dyn std::error::Error>>
{
    let root = fixture(Some(
        "[presets.daily]\nNAME = \"Ada\"\nCOUNT = \"2\"\n\n[presets.fast]\nNAME = \"Lin\"\n",
    ))?;
    let output = run(root.path(), &["preset", "list", "Demo", "--json"])?;
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let actual: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(
        actual,
        json!({
            "daily": {"COUNT": "2", "NAME": "Ada"},
            "fast": {"NAME": "Lin"}
        })
    );
    Ok(())
}

#[test]
fn preset_list_without_values_keeps_the_existing_guidance() -> Result<(), Box<dyn std::error::Error>>
{
    let root = fixture(None)?;
    let output = run(root.path(), &["preset", "list", "Demo"])?;
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout)?,
        "No presets for Demo yet. Create one with: skit run Demo --save-preset <preset>\n"
    );
    Ok(())
}

#[test]
fn preset_save_from_last_captures_exact_snapshot() -> Result<(), Box<dyn std::error::Error>> {
    let root = fixture(Some(
        "[last_run]\nat = \"2026-08-08T05:00:00+00:00\"\nexit = 0\n\n[last_run.values]\nNAME = \"Ada\"\n",
    ))?;
    let output = run(
        root.path(),
        &["preset", "save", "Demo", "prod", "--from-last"],
    )?;
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(
        String::from_utf8(output.stdout)?,
        "Preset \"prod\" saved for Demo.\n"
    );
    let listed = run(root.path(), &["preset", "list", "Demo", "--json"])?;
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&listed.stdout)?,
        json!({"prod": {"NAME": "Ada"}})
    );
    Ok(())
}

#[test]
fn preset_save_from_last_preserves_an_exact_empty_snapshot()
-> Result<(), Box<dyn std::error::Error>> {
    let root = fixture(Some(
        "[last_run]\nat = \"2026-08-08T05:00:00+00:00\"\nexit = 0\n\n[last_run.values]\n",
    ))?;
    let output = run(
        root.path(),
        &["preset", "save", "Demo", "empty", "--from-last"],
    )?;
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let listed = run(root.path(), &["preset", "list", "Demo", "--json"])?;
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&listed.stdout)?,
        json!({"empty": {}})
    );
    Ok(())
}

#[test]
fn preset_save_from_last_refuses_before_first_run() -> Result<(), Box<dyn std::error::Error>> {
    let root = fixture(None)?;
    let output = run(
        root.path(),
        &["preset", "save", "Demo", "prod", "--from-last"],
    )?;
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr)?,
        "Demo has no remembered values yet — run it once first.\n"
    );
    Ok(())
}

#[test]
fn preset_save_from_last_refuses_legacy_stamp_without_snapshot()
-> Result<(), Box<dyn std::error::Error>> {
    let root = fixture(Some(
        "[values]\nNAME = \"last-used\"\n\n[last_run]\nat = \"2026-07-01T00:00:00+00:00\"\nexit = 0\n",
    ))?;
    let output = run(
        root.path(),
        &["preset", "save", "Demo", "prod", "--from-last"],
    )?;
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8(output.stderr)?.contains("run it once first"));
    let listed = run(root.path(), &["preset", "list", "Demo", "--json"])?;
    assert_eq!(serde_json::from_slice::<serde_json::Value>(&listed.stdout)?, json!({}));
    Ok(())
}

#[test]
fn preset_save_without_from_last_is_usage_error_and_writes_nothing()
-> Result<(), Box<dyn std::error::Error>> {
    let root = fixture(Some("[values]\nNAME = \"Ada\"\n"))?;
    let output = run(root.path(), &["preset", "save", "Demo", "prod"])?;
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8(output.stderr)?.contains("pass --from-last"));
    let listed = run(root.path(), &["preset", "list", "Demo", "--json"])?;
    assert_eq!(serde_json::from_slice::<serde_json::Value>(&listed.stdout)?, json!({}));
    Ok(())
}

#[test]
fn preset_delete_matches_the_existing_success_copy() -> Result<(), Box<dyn std::error::Error>> {
    let root = fixture(Some("[presets.daily]\nNAME = \"Ada\"\n"))?;
    let output = run(root.path(), &["preset", "delete", "Demo", "daily"])?;
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout)?,
        "Preset \"daily\" deleted from Demo.\n"
    );
    let state = fs::read_to_string(root.path().join("state/values/demo.toml"))?;
    assert!(!state.contains("daily"));
    Ok(())
}

#[test]
fn preset_delete_unknown_exits_one_and_lists_available_names()
-> Result<(), Box<dyn std::error::Error>> {
    let root = fixture(Some("[presets.daily]\nNAME = \"Ada\"\n"))?;
    let output = run(root.path(), &["preset", "delete", "Demo", "missing"])?;
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr)?,
        "Unknown preset \"missing\". Available: daily\n"
    );
    Ok(())
}
