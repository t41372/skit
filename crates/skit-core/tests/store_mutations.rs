use std::fs;
use std::path::Path;
use std::sync::{Arc, Barrier};
use std::thread;

use skit_core::{Error, LibraryRoots, Store};
use tempfile::tempdir;

const META: &str = r#"schema = 1
name = "Original"
kind = "shell"
mode = "reference"
source = "SOURCE_PATH"
source_hash = "sha256:0123456789abcdef"
added_at = "2026-07-22T00:00:00+00:00"
workdir = "origin"
description = "Before"
future_key = "keep me"
"#;

fn write(path: &Path, content: &str) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)?;
    Ok(())
}

fn fixture() -> Result<(tempfile::TempDir, Store, std::path::PathBuf), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let data = root.path().join("data");
    let state = root.path().join("state");
    let config = root.path().join("config");
    let source = root.path().join("original.sh");
    write(&source, "#!/bin/sh\necho ok\n")?;
    let meta = META.replace("SOURCE_PATH", &source.to_string_lossy());
    write(&data.join("scripts/original/meta.toml"), &meta)?;
    write(
        &state.join("values/original.toml"),
        "[values]\nNAME = \"Ada\"\n",
    )?;
    Ok((
        root,
        Store::new(LibraryRoots::new(data, state, config)),
        source,
    ))
}

#[test]
fn description_and_rename_preserve_forward_fields_and_slug() -> Result<(), Box<dyn std::error::Error>> {
    let (_root, store, _source) = fixture()?;

    let described = store.update_description("original", "After")?;
    assert_eq!(described.slug, "original");
    assert_eq!(described.meta.description, "After");

    let renamed = store.rename("original", "Renamed")?;
    assert_eq!(renamed.slug, "original");
    assert_eq!(renamed.meta.name, "Renamed");
    assert_eq!(renamed.meta.description, "After");
    assert_eq!(
        renamed.meta.extra.get("future_key").and_then(toml::Value::as_str),
        Some("keep me")
    );

    assert!(store.roots().data_dir().join("scripts/original").is_dir());
    assert!(!store.roots().data_dir().join("scripts/renamed").exists());
    assert!(store.roots().state_dir().join("values/original.toml").is_file());
    Ok(())
}

#[test]
fn concurrent_metadata_updates_do_not_erase_each_other() -> Result<(), Box<dyn std::error::Error>> {
    let (_root, store, _source) = fixture()?;
    let barrier = Arc::new(Barrier::new(3));

    let rename_store = store.clone();
    let rename_barrier = Arc::clone(&barrier);
    let rename_thread = thread::spawn(move || {
        rename_barrier.wait();
        rename_store.rename("original", "Concurrent")
    });

    let describe_store = store.clone();
    let describe_barrier = Arc::clone(&barrier);
    let describe_thread = thread::spawn(move || {
        describe_barrier.wait();
        describe_store.update_description("original", "Concurrent description")
    });

    barrier.wait();
    rename_thread.join().map_err(|_| "rename thread panicked")??;
    describe_thread
        .join()
        .map_err(|_| "description thread panicked")??;

    let entry = store.resolve("original")?;
    assert_eq!(entry.meta.name, "Concurrent");
    assert_eq!(entry.meta.description, "Concurrent description");
    Ok(())
}

#[test]
fn rename_refuses_a_duplicate_display_name() -> Result<(), Box<dyn std::error::Error>> {
    let (_root, store, _source) = fixture()?;
    let other = store.roots().data_dir().join("scripts/other/meta.toml");
    write(
        &other,
        "schema = 1\nname = \"Taken\"\nkind = \"command\"\nmode = \"copy\"\nsource = \"\"\nsource_hash = \"\"\nadded_at = \"\"\nworkdir = \"origin\"\ndescription = \"\"\ntemplate = \"echo ok\"\n",
    )?;

    match store.rename("original", "Taken") {
        Err(Error::NameConflict { name }) => assert_eq!(name, "Taken"),
        other => panic!("unexpected rename result: {other:?}"),
    }
    assert_eq!(store.resolve("original")?.meta.name, "Original");
    Ok(())
}

#[test]
fn removing_a_reference_never_deletes_the_original_file() -> Result<(), Box<dyn std::error::Error>> {
    let (_root, store, source) = fixture()?;

    let removed_name = store.remove("original")?;

    assert_eq!(removed_name, "Original");
    assert!(source.is_file());
    assert!(!store.roots().data_dir().join("scripts/original").exists());
    assert!(!store.roots().state_dir().join("values/original.toml").exists());
    Ok(())
}
