use std::fs;
use std::path::Path;

use skit_core::{LibraryRoots, Store};
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

const REGISTRY: &str = r#"[entries.original]
name = "Original"
kind = "shell"
description = "Before"

[entries.other]
name = "Other"
kind = "future-kind"
description = "Keep this row"
custom = "keep"
"#;

fn write(path: &Path, content: &str) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)?;
    Ok(())
}

fn fixture() -> Result<(tempfile::TempDir, Store), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let data = root.path().join("data");
    write(&data.join("scripts/original/meta.toml"), META)?;
    write(&data.join("registry.toml"), REGISTRY)?;
    let state = root.path().join("state");
    let config = root.path().join("config");
    let store = Store::new(LibraryRoots::new(data, state, config));
    Ok((root, store))
}

fn registry(store: &Store) -> Result<toml::Table, Box<dyn std::error::Error>> {
    let text = fs::read_to_string(store.roots().data_dir().join("registry.toml"))?;
    Ok(toml::from_str(&text)?)
}

#[test]
fn metadata_writes_refresh_only_the_existing_registry_row() -> Result<(), Box<dyn std::error::Error>>
{
    let (_root, store) = fixture()?;

    store.update_description("original", "After")?;
    store.rename("original", "Renamed")?;

    let doc = registry(&store)?;
    let entries = doc["entries"].as_table().ok_or("entries is not a table")?;
    let row = entries["original"].as_table().ok_or("row is not a table")?;
    assert_eq!(row["name"].as_str(), Some("Renamed"));
    assert_eq!(row["kind"].as_str(), Some("shell"));
    assert_eq!(row["mode"].as_str(), Some("reference"));
    assert_eq!(row["description"].as_str(), Some("After"));
    assert_eq!(row["target"].as_str(), Some("/missing/original.sh"));
    assert!(row["mtime_ns"].as_integer().is_some());

    let other = entries["other"].as_table().ok_or("other is not a table")?;
    assert_eq!(other["custom"].as_str(), Some("keep"));
    Ok(())
}

#[test]
fn remove_drops_only_the_matching_registry_row() -> Result<(), Box<dyn std::error::Error>> {
    let (_root, store) = fixture()?;

    store.remove("original")?;

    let doc = registry(&store)?;
    let entries = doc["entries"].as_table().ok_or("entries is not a table")?;
    assert!(!entries.contains_key("original"));
    assert_eq!(entries["other"]["name"].as_str(), Some("Other"));
    Ok(())
}

#[test]
fn a_missing_registry_is_not_recreated_by_an_unrelated_metadata_edit()
-> Result<(), Box<dyn std::error::Error>> {
    let (_root, store) = fixture()?;
    fs::remove_file(store.roots().data_dir().join("registry.toml"))?;

    store.update_description("original", "After")?;

    assert!(!store.roots().data_dir().join("registry.toml").exists());
    assert_eq!(store.resolve("original")?.meta.description, "After");
    Ok(())
}
