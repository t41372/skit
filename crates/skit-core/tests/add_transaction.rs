use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Barrier};
use std::thread;

use skit_core::{EntryDraft, Error, LibraryRoots, ScriptMeta, Store};
use tempfile::tempdir;

fn roots(root: &Path) -> LibraryRoots {
    LibraryRoots::new(root.join("data"), root.join("state"), root.join("config"))
}

fn meta(name: &str, kind: &str, mode: &str) -> ScriptMeta {
    ScriptMeta {
        schema: 1,
        name: name.to_owned(),
        kind: kind.to_owned(),
        mode: mode.to_owned(),
        source: "/origin/tool.sh".to_owned(),
        source_hash: "sha256:abc".to_owned(),
        added_at: "2026-08-08T00:00:00+00:00".to_owned(),
        workdir: if mode == "reference" {
            "origin".to_owned()
        } else {
            "invoke".to_owned()
        },
        description: "demo".to_owned(),
        template: String::new(),
        dependencies: None,
        requires_python: String::new(),
        params: None,
        interpreter: "bash".to_owned(),
        runner: String::new(),
        interpolate: true,
        needs: None,
        parameters: None,
        extra: BTreeMap::new(),
    }
}

#[test]
fn copied_entry_writes_payload_meta_and_registry_as_one_add_operation()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let store = Store::new(roots(root.path()));
    let payload = b"#!/bin/sh\necho ok\n".to_vec();

    let entry = store.insert_entry(EntryDraft::new(
        meta("My Tool", "shell", "copy"),
        Some(payload.clone()),
    ))?;

    assert_eq!(entry.slug, "my-tool");
    assert_eq!(entry.script_path(), entry.dir.join("script.sh"));
    assert_eq!(fs::read(entry.script_path())?, payload);
    assert_eq!(store.resolve("My Tool")?.slug, "my-tool");

    let registry: toml::Table = toml::from_str(&fs::read_to_string(
        store.roots().data_dir().join("registry.toml"),
    )?)?;
    let row = registry["entries"]["my-tool"]
        .as_table()
        .ok_or("registry row is not a table")?;
    assert_eq!(row["name"].as_str(), Some("My Tool"));
    assert_eq!(row["mode"].as_str(), Some("copy"));
    assert!(!row.contains_key("target"));
    Ok(())
}

#[test]
fn reference_entry_writes_no_payload_and_keeps_target_in_registry()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let store = Store::new(roots(root.path()));
    let entry = store.insert_entry(EntryDraft::new(meta("Linked", "shell", "reference"), None))?;

    assert_eq!(entry.slug, "linked");
    assert!(!entry.dir.join("script.sh").exists());
    let registry: toml::Table = toml::from_str(&fs::read_to_string(
        store.roots().data_dir().join("registry.toml"),
    )?)?;
    assert_eq!(
        registry["entries"]["linked"]["target"].as_str(),
        Some("/origin/tool.sh")
    );
    Ok(())
}

#[test]
fn slug_allocation_uses_registry_and_nonempty_filesystem_truth()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let store = Store::new(roots(root.path()));
    let scripts = store.roots().data_dir().join("scripts");
    fs::create_dir_all(scripts.join("tool-2"))?;
    fs::write(scripts.join("tool-2/orphan"), "protect")?;

    let first = store.insert_entry(EntryDraft::new(
        meta("Tool", "shell", "copy"),
        Some(b"one".to_vec()),
    ))?;
    let second = store.insert_entry(EntryDraft::new(
        meta("Other", "shell", "copy"),
        Some(b"two".to_vec()),
    ))?;
    store.rename(&second.slug, "Other Renamed")?;

    assert_eq!(first.slug, "tool");

    let mut third_meta = meta("Tool!", "shell", "copy");
    third_meta.name = "Tool!".to_owned();
    let third = store.insert_entry(EntryDraft::new(third_meta, Some(b"three".to_vec())))?;
    assert_eq!(third.slug, "tool-3");
    assert_eq!(
        fs::read_to_string(scripts.join("tool-2/orphan"))?,
        "protect"
    );
    Ok(())
}

#[test]
fn duplicate_display_name_is_refused_even_when_registry_is_missing()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let store = Store::new(roots(root.path()));
    let existing = store.roots().data_dir().join("scripts/existing/meta.toml");
    fs::create_dir_all(existing.parent().ok_or("missing parent")?)?;
    fs::write(
        &existing,
        toml::to_string(&meta("Taken", "shell", "reference"))?,
    )?;

    let result = store.insert_entry(EntryDraft::new(
        meta("Taken", "shell", "copy"),
        Some(b"new".to_vec()),
    ));
    assert!(matches!(result, Err(Error::NameConflict { name }) if name == "Taken"));
    assert!(!store.roots().data_dir().join("scripts/taken").exists());
    Ok(())
}

#[test]
fn empty_unregistered_slug_directory_is_reused() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let store = Store::new(roots(root.path()));
    let empty = store.roots().data_dir().join("scripts/tool");
    fs::create_dir_all(&empty)?;

    let entry = store.insert_entry(EntryDraft::new(
        meta("Tool", "shell", "copy"),
        Some(b"ok".to_vec()),
    ))?;

    assert_eq!(entry.slug, "tool");
    assert_eq!(fs::read(entry.dir.join("script.sh"))?, b"ok");
    Ok(())
}

#[test]
fn registry_read_failure_does_not_leave_a_half_entry() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let store = Store::new(roots(root.path()));
    let registry = store.roots().data_dir().join("registry.toml");
    fs::create_dir_all(&registry)?;

    let result = store.insert_entry(EntryDraft::new(
        meta("Tool", "shell", "copy"),
        Some(b"ok".to_vec()),
    ));

    assert!(result.is_err());
    assert!(!store.roots().data_dir().join("scripts/tool").exists());
    Ok(())
}

#[test]
fn concurrent_adds_with_same_name_create_exactly_one_entry()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let store = Store::new(roots(root.path()));
    let barrier = Arc::new(Barrier::new(3));

    let left_store = store.clone();
    let left_barrier = Arc::clone(&barrier);
    let left = thread::spawn(move || {
        left_barrier.wait();
        left_store.insert_entry(EntryDraft::new(
            meta("Same", "shell", "copy"),
            Some(b"left".to_vec()),
        ))
    });

    let right_store = store.clone();
    let right_barrier = Arc::clone(&barrier);
    let right = thread::spawn(move || {
        right_barrier.wait();
        right_store.insert_entry(EntryDraft::new(
            meta("Same", "shell", "copy"),
            Some(b"right".to_vec()),
        ))
    });

    barrier.wait();
    let left_result = left.join().map_err(|_| "left thread panicked")?;
    let right_result = right.join().map_err(|_| "right thread panicked")?;
    let successes = usize::from(left_result.is_ok()) + usize::from(right_result.is_ok());
    assert_eq!(successes, 1);
    assert_eq!(store.list()?.len(), 1);
    Ok(())
}
