//! Additional public index/listing ports from Python v0.4 `tests/test_store.py`.
//!
//! `test_a_store_that_cannot_be_written_still_lists` is intentionally not executable here:
//! Python injects a repair-write failure by monkeypatching its private `_save_registry` helper.
//! Rust currently has no deterministic public write-failure seam on the read path; chmod-ing only
//! `registry.toml` is not equivalent because atomic replacement depends on the parent directory.

use std::fs;

use skit_application::{
    CreateEntry, EntryMutationRepository as _, EntryPayload, EntryRepository as _, SourcePermissions,
};
use skit_domain::{EntryKind, EntrySettings, StorageMode};
use skit_store::FileStore;
use tempfile::TempDir;
use toml::{Table, Value};

fn request(name: &str, kind: &str, mode: StorageMode, source: &str) -> CreateEntry {
    let stored = match kind {
        "python" => Some("script.py"),
        "shell" => Some("script.sh"),
        _ => None,
    };
    CreateEntry {
        name: name.to_owned(),
        kind: EntryKind::parse(kind).unwrap(),
        mode,
        source: source.to_owned(),
        workdir: if mode == StorageMode::Reference { "origin" } else { "invoke" }.to_owned(),
        description: String::new(),
        payload: stored.map(|stored_name| EntryPayload {
            bytes: b"payload\n".to_vec(),
            stored_name: Some(stored_name.to_owned()),
            permissions: SourcePermissions::default(),
        }),
        settings: EntrySettings::default(),
    }
}

fn load_registry(root: &TempDir) -> Table {
    toml::from_str(&fs::read_to_string(root.path().join("registry.toml")).unwrap()).unwrap()
}

fn save_registry(root: &TempDir, document: &Table) {
    fs::write(
        root.path().join("registry.toml"),
        toml::to_string_pretty(document).unwrap(),
    )
    .unwrap();
}

fn row_mut<'a>(document: &'a mut Table, slug: &str) -> &'a mut Table {
    document
        .get_mut("entries")
        .and_then(Value::as_table_mut)
        .and_then(|entries| entries.get_mut(slug))
        .and_then(Value::as_table_mut)
        .unwrap()
}

fn replace_with_legacy_row(document: &mut Table, slug: &str, name: &str, kind: &str) {
    document
        .get_mut("entries")
        .and_then(Value::as_table_mut)
        .unwrap()
        .insert(
            slug.to_owned(),
            Value::Table(Table::from_iter([
                ("name".to_owned(), Value::String(name.to_owned())),
                ("kind".to_owned(), Value::String(kind.to_owned())),
                ("description".to_owned(), Value::String(String::new())),
            ])),
        );
}

#[test]
fn test_exe_is_always_reference_mode() {
    let root = TempDir::new().unwrap();
    let source = root.path().join("tool");
    fs::write(&source, b"binary").unwrap();
    let store = FileStore::new(root.path());
    let entry = store
        .create(request("binary", "exe", StorageMode::Reference, &source.display().to_string()))
        .unwrap();
    assert_eq!(entry.meta.mode, StorageMode::Reference);
    assert_eq!(entry.meta.source, source.display().to_string());
    let summary = store.scan().unwrap().entries.into_iter().next().unwrap();
    assert_eq!(summary.mode, StorageMode::Reference);
    assert_eq!(summary.target.as_deref(), Some(source.to_str().unwrap()));
}

#[test]
fn test_an_entry_whose_meta_is_gone_is_not_listed() {
    let root = TempDir::new().unwrap();
    let source = root.path().join("linked.py");
    fs::write(&source, "print(1)\n").unwrap();
    let store = FileStore::new(root.path());
    let linked = store
        .create(request("linked", "python", StorageMode::Reference, &source.display().to_string()))
        .unwrap();
    store
        .create(request("kept", "command", StorageMode::Reference, ""))
        .unwrap();
    fs::remove_dir_all(root.path().join("scripts").join(linked.slug.as_str())).unwrap();

    assert_eq!(
        store.scan().unwrap().entries.iter().map(|summary| summary.name.as_str()).collect::<Vec<_>>(),
        ["kept"]
    );
    assert_eq!(
        store.scan_entries().unwrap().iter().map(|entry| entry.meta.name.as_str()).collect::<Vec<_>>(),
        ["kept"]
    );
    assert!(source.exists());
}

#[test]
fn test_widening_gives_up_on_a_row_it_would_reject_again() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let entry = store
        .create(request("odd", "python", StorageMode::Copy, "/origin/odd.py"))
        .unwrap();
    let meta_path = root.path().join("scripts/odd/meta.toml");
    let mut meta: Table = toml::from_str(&fs::read_to_string(&meta_path).unwrap()).unwrap();
    meta.insert("mode".to_owned(), Value::String("sideways".to_owned()));
    fs::write(&meta_path, toml::to_string_pretty(&meta).unwrap()).unwrap();
    let mut registry = load_registry(&root);
    replace_with_legacy_row(&mut registry, entry.slug.as_str(), "odd", "python");
    save_registry(&root, &registry);
    let before = fs::read(root.path().join("registry.toml")).unwrap();

    for _ in 0..3 {
        assert_eq!(
            store.scan().unwrap().entries.iter().map(|summary| summary.name.as_str()).collect::<Vec<_>>(),
            ["odd"]
        );
    }
    assert_eq!(fs::read(root.path().join("registry.toml")).unwrap(), before);
}

#[test]
fn test_a_renamed_legacy_row_is_upgraded_not_patched() {
    let root = TempDir::new().unwrap();
    let source = root.path().join("linked.py");
    fs::write(&source, "print(1)\n").unwrap();
    let store = FileStore::new(root.path());
    let entry = store
        .create(request("linked", "python", StorageMode::Reference, &source.display().to_string()))
        .unwrap();
    let mut registry = load_registry(&root);
    replace_with_legacy_row(&mut registry, entry.slug.as_str(), "linked", "python");
    save_registry(&root, &registry);

    let renamed = store.rename(&entry, "renamed").unwrap();
    let summary = store.scan().unwrap().entries.into_iter().next().unwrap();
    assert_eq!(summary.name, "renamed");
    assert_eq!(summary.mode, StorageMode::Reference);
    assert_eq!(summary.target.as_deref(), Some(source.to_str().unwrap()));
    assert_eq!(store.resolve("renamed").unwrap().slug, renamed.slug);
}

#[test]
fn test_a_reference_row_without_a_target_falls_back_to_its_meta() {
    let root = TempDir::new().unwrap();
    let source = root.path().join("orig.py");
    fs::write(&source, "print(1)\n").unwrap();
    let store = FileStore::new(root.path());
    let entry = store
        .create(request("linked", "python", StorageMode::Reference, &source.display().to_string()))
        .unwrap();
    fs::remove_file(&source).unwrap();
    let mut registry = load_registry(&root);
    row_mut(&mut registry, entry.slug.as_str()).remove("target");
    save_registry(&root, &registry);

    let summary = store.scan().unwrap().entries.into_iter().next().unwrap();
    assert_eq!(summary.target.as_deref(), Some(source.to_str().unwrap()));
    assert!(!std::path::Path::new(summary.target.as_deref().unwrap()).exists());
}

#[test]
fn test_a_command_row_keeps_an_empty_target() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let entry = store
        .create(request("tmpl", "command", StorageMode::Reference, ""))
        .unwrap();
    let registry = load_registry(&root);
    assert_eq!(
        registry["entries"][entry.slug.as_str()]["target"].as_str(),
        Some("")
    );
    let summary = store.scan().unwrap().entries.into_iter().next().unwrap();
    assert_eq!(summary.mode, StorageMode::Reference);
    assert_eq!(summary.target.as_deref(), Some(""));
    let before = fs::read(root.path().join("registry.toml")).unwrap();
    assert_eq!(store.scan().unwrap().entries[0].target.as_deref(), Some(""));
    assert_eq!(fs::read(root.path().join("registry.toml")).unwrap(), before);
}

#[test]
fn test_a_fresh_stamped_row_with_broken_fields_falls_back() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let entry = store
        .create(request("real", "python", StorageMode::Copy, "/origin/real.py"))
        .unwrap();
    let meta_path = root.path().join("scripts/real/meta.toml");
    let mtime = fs::metadata(&meta_path).unwrap().modified().unwrap();
    let ns = mtime
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos() as i64;
    let mut registry = load_registry(&root);
    let row = row_mut(&mut registry, entry.slug.as_str());
    row.insert("description".to_owned(), Value::Integer(7));
    row.insert("mtime_ns".to_owned(), Value::Integer(ns));
    save_registry(&root, &registry);

    let summary = store.scan().unwrap().entries.into_iter().next().unwrap();
    assert_eq!(summary.description, "");
}
