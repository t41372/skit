use std::{
    fs::{self, OpenOptions},
    sync::mpsc,
    thread,
    time::{Duration, UNIX_EPOCH},
};

use skit_application::{
    CreateEntry, EntryMutationRepository, EntryPayload, EntryRepository, SourcePermissions,
};
use skit_domain::{EntryKind, StorageMode};
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
        payload: Some(EntryPayload {
            bytes: format!("payload for {name}\n").into_bytes(),
            stored_name: Some(format!("{name}.tool")),
            permissions: SourcePermissions::default(),
        }),
    }
}

fn registry(root: &TempDir) -> Table {
    toml::from_str(
        &fs::read_to_string(root.path().join("registry.toml")).expect("registry should exist"),
    )
    .expect("registry should stay valid TOML")
}

fn write_registry(root: &TempDir, document: &Table) {
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
        .expect("entry row should be a table")
}

fn row<'a>(document: &'a Table, slug: &str) -> &'a Table {
    document
        .get("entries")
        .and_then(Value::as_table)
        .and_then(|entries| entries.get(slug))
        .and_then(Value::as_table)
        .expect("entry row should be a table")
}

fn mtime_ns(path: &std::path::Path) -> i64 {
    let nanos = fs::metadata(path)
        .unwrap()
        .modified()
        .unwrap()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    i64::try_from(nanos).unwrap()
}

fn set_meta_description(root: &TempDir, slug: &str, description: &str) {
    let path = root.path().join("scripts").join(slug).join("meta.toml");
    let mut document = toml::from_str::<Table>(&fs::read_to_string(&path).unwrap()).unwrap();
    document.insert(
        "description".to_owned(),
        Value::String(description.to_owned()),
    );
    fs::write(path, toml::to_string_pretty(&document).unwrap()).unwrap();
}

fn description(scan: &skit_application::LibraryScan, slug: &str) -> String {
    scan.entries
        .iter()
        .find(|entry| entry.slug.as_str() == slug)
        .expect("entry should be listed")
        .description
        .clone()
}

#[test]
fn a_fresh_registry_row_lists_without_opening_meta_toml() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    store
        .create(request("Fast", StorageMode::Copy, "from the row"))
        .unwrap();

    let meta = root.path().join("scripts/fast/meta.toml");
    fs::remove_file(&meta).unwrap();
    fs::create_dir(&meta).unwrap();
    let mut document = registry(&root);
    row_mut(&mut document, "fast").insert(
        "mtime_ns".to_owned(),
        Value::Integer(mtime_ns(&meta)),
    );
    write_registry(&root, &document);

    let scan = store.scan().unwrap();

    assert_eq!(description(&scan, "fast"), "from the row");
    assert!(scan.diagnostics.is_empty());
}

#[test]
fn stale_and_malformed_rows_fall_back_per_entry_and_repair_together() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    store
        .create(request("Stale", StorageMode::Copy, "before"))
        .unwrap();
    store
        .create(request("Malformed", StorageMode::Copy, "authoritative"))
        .unwrap();
    set_meta_description(&root, "stale", "after hand edit");

    let mut document = registry(&root);
    let stale = row_mut(&mut document, "stale");
    stale.insert("description".to_owned(), Value::String("old row".to_owned()));
    stale.insert("mtime_ns".to_owned(), Value::Integer(0));
    row_mut(&mut document, "malformed").insert("name".to_owned(), Value::Integer(7));
    write_registry(&root, &document);

    let scan = store.scan().unwrap();

    assert_eq!(description(&scan, "stale"), "after hand edit");
    assert_eq!(description(&scan, "malformed"), "authoritative");
    assert!(scan.diagnostics.is_empty());

    let repaired = registry(&root);
    assert_eq!(
        row(&repaired, "stale")
            .get("description")
            .and_then(Value::as_str),
        Some("after hand edit")
    );
    assert_eq!(
        row(&repaired, "stale")
            .get("mtime_ns")
            .and_then(Value::as_integer),
        Some(mtime_ns(&root.path().join("scripts/stale/meta.toml")))
    );
    assert_eq!(
        row(&repaired, "malformed")
            .get("name")
            .and_then(Value::as_str),
        Some("Malformed")
    );
}

#[test]
fn missing_registry_rows_are_rebuilt_from_authoritative_metadata() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    store
        .create(request("First", StorageMode::Copy, "first"))
        .unwrap();
    store
        .create(request("Second", StorageMode::Copy, "second"))
        .unwrap();
    fs::remove_file(root.path().join("registry.toml")).unwrap();

    let scan = store.scan().unwrap();

    assert_eq!(description(&scan, "first"), "first");
    assert_eq!(description(&scan, "second"), "second");
    let repaired = registry(&root);
    let entries = repaired.get("entries").and_then(Value::as_table).unwrap();
    assert!(entries.contains_key("first"));
    assert!(entries.contains_key("second"));
}

#[test]
fn a_busy_registry_lock_never_blocks_listing_and_defers_self_heal() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    store
        .create(request("Busy", StorageMode::Copy, "before"))
        .unwrap();
    set_meta_description(&root, "busy", "after hand edit");
    let mut document = registry(&root);
    let busy = row_mut(&mut document, "busy");
    busy.insert("description".to_owned(), Value::String("old row".to_owned()));
    busy.insert("mtime_ns".to_owned(), Value::Integer(0));
    write_registry(&root, &document);

    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .open(root.path().join("registry.native.lock"))
        .unwrap();
    lock.lock().unwrap();
    let worker_store = store.clone();
    let (sender, receiver) = mpsc::channel();
    let worker = thread::spawn(move || sender.send(worker_store.scan()).unwrap());

    let received = receiver.recv_timeout(Duration::from_secs(2));
    drop(lock);
    worker.join().unwrap();
    let scan = received.expect("listing blocked on registry.native.lock").unwrap();
    assert_eq!(description(&scan, "busy"), "after hand edit");
    assert_eq!(
        row(&registry(&root), "busy")
            .get("description")
            .and_then(Value::as_str),
        Some("old row")
    );

    store.scan().unwrap();
    assert_eq!(
        row(&registry(&root), "busy")
            .get("description")
            .and_then(Value::as_str),
        Some("after hand edit")
    );
}

#[test]
fn invalid_mode_and_missing_reference_target_fall_back_and_self_heal() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    store
        .create(request("Copy", StorageMode::Copy, "copy"))
        .unwrap();
    store
        .create(request("Linked", StorageMode::Reference, "linked"))
        .unwrap();

    let mut document = registry(&root);
    row_mut(&mut document, "copy").insert(
        "mode".to_owned(),
        Value::String("future-mode".to_owned()),
    );
    row_mut(&mut document, "linked").remove("target");
    write_registry(&root, &document);

    let scan = store.scan().unwrap();

    assert_eq!(description(&scan, "copy"), "copy");
    assert_eq!(description(&scan, "linked"), "linked");
    let repaired = registry(&root);
    assert_eq!(
        row(&repaired, "copy").get("mode").and_then(Value::as_str),
        Some("copy")
    );
    assert_eq!(
        row(&repaired, "linked")
            .get("target")
            .and_then(Value::as_str),
        Some("/original/Linked.tool")
    );
}
