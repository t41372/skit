use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use tempfile::tempdir;

const META: &str = r#"schema = 1
name = "Original"
kind = "shell"
mode = "reference"
source = "/missing/original.sh"
source_hash = "sha256:abc"
added_at = "2026-07-22T00:00:00+00:00"
workdir = "origin"
description = "Before"
"#;

fn write(path: &Path, content: &str) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)?;
    Ok(())
}

fn run(root: &Path, args: &[&str]) -> Result<Output, Box<dyn std::error::Error>> {
    let data = root.join("data");
    let state = root.join("state");
    let config = root.join("config");
    Ok(Command::new(env!("CARGO_BIN_EXE_skit"))
        .args(args)
        .env("SKIT_DATA_DIR", data)
        .env("SKIT_STATE_DIR", state)
        .env("SKIT_CONFIG_DIR", config)
        .output()?)
}

fn fixture() -> Result<tempfile::TempDir, Box<dyn std::error::Error>> {
    let root = tempdir()?;
    write(&root.path().join("data/scripts/original/meta.toml"), META)?;
    write(
        &root.path().join("data/registry.toml"),
        "[entries.original]\nname = \"Original\"\nkind = \"shell\"\ndescription = \"Before\"\n",
    )?;
    Ok(root)
}

#[test]
fn describe_matches_the_existing_success_copy() -> Result<(), Box<dyn std::error::Error>> {
    let root = fixture()?;
    let output = run(root.path(), &["describe", "Original", "After"])?;
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout)?,
        "Description updated for Original.\n"
    );
    assert!(output.stderr.is_empty());
    Ok(())
}

#[test]
fn describe_can_clear_the_description() -> Result<(), Box<dyn std::error::Error>> {
    let root = fixture()?;
    let output = run(root.path(), &["describe", "Original", ""])?;
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout)?,
        "Description cleared for Original.\n"
    );
    Ok(())
}

#[test]
fn rename_matches_the_existing_success_copy() -> Result<(), Box<dyn std::error::Error>> {
    let root = fixture()?;
    let output = run(root.path(), &["rename", "Original", "Renamed"])?;
    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout)?, "Renamed to Renamed.\n");
    Ok(())
}

#[test]
fn remove_yes_skips_confirmation_and_keeps_the_original_contract()
-> Result<(), Box<dyn std::error::Error>> {
    let root = fixture()?;
    let output = run(root.path(), &["remove", "Original", "--yes"])?;
    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout)?, "Removed: Original\n");
    assert!(!root.path().join("data/scripts/original").exists());
    Ok(())
}

#[test]
fn missing_management_target_exits_one() -> Result<(), Box<dyn std::error::Error>> {
    let root = fixture()?;
    let output = run(root.path(), &["rename", "Missing", "New"])?;
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(!output.stderr.is_empty());
    Ok(())
}
