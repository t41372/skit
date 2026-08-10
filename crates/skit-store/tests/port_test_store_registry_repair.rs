//! Mechanical behavioral port of the registry self-heal cases in
//! `origin/main@206f9ef:tests/test_store.py`.
//!
//! Python latest main treats `registry.toml` as a rebuildable listing projection. A stale or
//! legacy row falls back to authoritative `meta.toml`, then the read path repairs that row only if
//! it can acquire `registry.native.lock` immediately. The read must never wait for that lock.

use std::fs::{self, OpenOptions};

use skit_application::{
    CreateEntry, EntryMutationRepository, EntryPayload, EntryRepository, SourcePermissions,
};
use skit_domain::{EntryKind, EntrySettings, StorageMode};
use skit_store::FileStore;
use tempfile::TempDir;
use toml::{Table, Value};

fn request(name: &str, mode: StorageMode, description: &str) -> CreateEntry {
    CreateEntry {
        name: name.to_owned(),
        kind: EntryKind::parse("future-kind").unwrap(),
        mode,
        source: format!("/original/{name}.tool"),
        workdir: if mode == StorageMode::Reference {
            "origin"
        } else {
            "invoke"
        }
        .to_owned(),
        description: description.to_owned(),
        payload: if mode == StorageMode::Copy {
            Some(EntryPayload {
                bytes: format!("payload for {name}\n").into_bytes(),
                stored_name: Some(format!("{name}.tool")),
                permissions: SourcePermissions::default(),
            })
        } else {
            None
        },
        settings: EntrySettings::default(),
    }
}

fn registry(root: &TempDir) -> Table {
    toml::from_str(&fs::read_to_string(root.path().join("registry.toml")).unwrap()).unwrap()
}

fn write_registry(root: &TempDir, document: &Table) {
    fs::write(
        root.path().join("registry.toml"),
        toml::to_string_pretty(document).unwrap(),
    )
    .unwrap();
}

fn entries_mut(document: &mut Table) -> &mut Table {
    document
        .get_mut("entries")
        .and_then(Value::as_table_mut)
        .unwrap()
}

fn row<'a>(document: &'a Table, slug: &str) -> &'a Table {
    document
        .get("entries")
        .and_then(Value::as_table)
        .and_then(|entries| entries.get(slug))
        .and_then(Value::as_table)
        .unwrap()
}

fn edit_meta(root: &TempDir, slug: &str, key: &str, value: &str) {
    let path = root.path().join("scripts").join(slug).join("meta.toml");
    let mut document = toml::from_str::<Table>(&fs::read_to_string(&path).unwrap()).unwrap();
    document.insert(key.to_owned(), Value::String(value.to_owned()));
    fs::write(path, toml::to_string_pretty(&document).unwrap()).unwrap();
}

#[test]
fn test_a_corrupt_index_lists_nothing_and_preserves_the_bad_bytes() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    store
        .create(request("doomed", StorageMode::Copy, "kept in meta"))
        .unwrap();
    let corrupt = b"entries = [ this is not toml";
    fs::write(root.path().join("registry.toml"), corrupt).unwrap();

    assert!(store.scan().unwrap().entries.is_empty());
    assert!(!root.path().join("registry.toml").exists());
    assert_eq!(
        fs::read(root.path().join("registry.toml.corrupt")).unwrap(),
        corrupt
    );

    assert_eq!(store.rebuild_registry().unwrap(), 1);
    let scan = store.scan().unwrap();
    assert_eq!(scan.entries.len(), 1);
    assert_eq!(scan.entries[0].name, "doomed");
}

#[test]
fn test_an_older_registry_is_widened_the_first_time_it_is_listed() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    store
        .create(request("legacy", StorageMode::Copy, "old row"))
        .unwrap();
    let mut document = registry(&root);
    let legacy = entries_mut(&mut document).get_mut("legacy").unwrap();
    *legacy = Value::Table(Table::from_iter([
        ("name".to_owned(), Value::String("legacy".to_owned())),
        ("kind".to_owned(), Value::String("future-kind".to_owned())),
        (
            "description".to_owned(),
            Value::String("old row".to_owned()),
        ),
    ]));
    write_registry(&root, &document);

    let scan = store.scan().unwrap();
    assert_eq!(scan.entries.len(), 1);
    assert_eq!(scan.entries[0].name, "legacy");

    let repaired = registry(&root);
    let repaired = row(&repaired, "legacy");
    assert_eq!(repaired.get("mode").and_then(Value::as_str), Some("copy"));
    assert!(repaired.get("mtime_ns").and_then(Value::as_integer).is_some());
}

#[test]
fn test_a_hand_edited_meta_shows_up_on_the_next_listing() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    store
        .create(request("job", StorageMode::Copy, "the old text"))
        .unwrap();

    edit_meta(&root, "job", "description", "edited by hand");

    let scan = store.scan().unwrap();
    assert_eq!(scan.entries[0].description, "edited by hand");
    let repaired = registry(&root);
    assert_eq!(
        row(&repaired, "job")
            .get("description")
            .and_then(Value::as_str),
        Some("edited by hand")
    );
    let after_first = fs::read(root.path().join("registry.toml")).unwrap();
    assert_eq!(store.scan().unwrap().entries[0].description, "edited by hand");
    assert_eq!(
        fs::read(root.path().join("registry.toml")).unwrap(),
        after_first
    );
}

#[test]
fn test_a_listing_never_blocks_on_the_registry_lock() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    store
        .create(request("legacy", StorageMode::Copy, "old"))
        .unwrap();
    let mut document = registry(&root);
    let legacy = entries_mut(&mut document).get_mut("legacy").unwrap();
    *legacy = Value::Table(Table::from_iter([
        ("name".to_owned(), Value::String("legacy".to_owned())),
        ("kind".to_owned(), Value::String("future-kind".to_owned())),
        ("description".to_owned(), Value::String("old".to_owned())),
    ]));
    write_registry(&root, &document);
    let legacy_bytes = fs::read(root.path().join("registry.toml")).unwrap();

    let lock_path = root.path().join("registry.native.lock");
    let held = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .unwrap();
    if held.metadata().unwrap().len() == 0 {
        held.set_len(1).unwrap();
    }
    held.lock().unwrap();

    let scan = store.scan().unwrap();
    assert_eq!(scan.entries[0].name, "legacy");
    assert_eq!(fs::read(root.path().join("registry.toml")).unwrap(), legacy_bytes);

    drop(held);
    store.scan().unwrap();
    assert_eq!(
        row(&registry(&root), "legacy")
            .get("mode")
            .and_then(Value::as_str),
        Some("copy")
    );
}

#[test]
fn test_a_reference_row_that_lost_its_target_is_repaired_once() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    store
        .create(request("linked", StorageMode::Reference, "linked"))
        .unwrap();
    let mut document = registry(&root);
    entries_mut(&mut document)
        .get_mut("linked")
        .and_then(Value::as_table_mut)
        .unwrap()
        .remove("target");
    write_registry(&root, &document);

    let scan = store.scan().unwrap();
    assert_eq!(scan.entries[0].target.as_deref(), Some("/original/linked.tool"));
    assert_eq!(
        row(&registry(&root), "linked")
            .get("target")
            .and_then(Value::as_str),
        Some("/original/linked.tool")
    );

    let repaired = fs::read(root.path().join("registry.toml")).unwrap();
    assert_eq!(
        store.scan().unwrap().entries[0].target.as_deref(),
        Some("/original/linked.tool")
    );
    assert_eq!(fs::read(root.path().join("registry.toml")).unwrap(), repaired);
}

#[test]
fn test_repair_never_drops_an_entry_added_meanwhile() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    store
        .create(request("legacy", StorageMode::Copy, "old"))
        .unwrap();
    store
        .create(request("raced", StorageMode::Copy, "new"))
        .unwrap();
    let mut document = registry(&root);
    let raced = entries_mut(&mut document).get("raced").unwrap().clone();
    let legacy = entries_mut(&mut document).get_mut("legacy").unwrap();
    *legacy = Value::Table(Table::from_iter([
        ("name".to_owned(), Value::String("legacy".to_owned())),
        ("kind".to_owned(), Value::String("future-kind".to_owned())),
        ("description".to_owned(), Value::String("old".to_owned())),
    ]));
    write_registry(&root, &document);

    store.scan().unwrap();

    let repaired = registry(&root);
    let entries = repaired.get("entries").and_then(Value::as_table).unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries.get("raced"), Some(&raced));
    assert_eq!(
        row(&repaired, "legacy")
            .get("mode")
            .and_then(Value::as_str),
        Some("copy")
    );
}
