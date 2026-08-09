use std::{
    fs,
    sync::{
        Arc, Barrier,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};

use skit_application::{
    CreateEntry, EntryMutationRepository, EntryPayload, EntryRepository, SourcePermissions,
    UpdateEntry,
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
        payload: Some(EntryPayload {
            bytes: format!("payload for {name}\n").into_bytes(),
            stored_name: Some(format!("{name}.tool")),
            permissions: SourcePermissions::default(),
        }),
        settings: EntrySettings::default(),
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
#[cfg(any(unix, windows))]
fn rust_rows_carry_a_complete_incarnation_and_projection_proof() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    store
        .create(request("Proof", StorageMode::Reference, "projected"))
        .unwrap();

    let document = registry(&root);
    let cache = row(&document, "proof")
        .get("skit_cache")
        .and_then(Value::as_table)
        .expect("a Rust projection should carry its cache proof");

    assert_eq!(cache.get("schema").and_then(Value::as_integer), Some(1));
    for key in [
        "platform",
        "file_id",
        "file_size",
        "modified_ns",
        "changed_ns",
        "metadata_hash",
        "projection_hash",
    ] {
        assert!(
            cache.get(key).and_then(Value::as_str).is_some(),
            "cache proof is missing {key}"
        );
    }
}

#[test]
fn a_registry_row_never_hides_unreadable_authoritative_metadata() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    store
        .create(request("Fast", StorageMode::Copy, "from the row"))
        .unwrap();

    let meta = root.path().join("scripts/fast/meta.toml");
    fs::remove_file(&meta).unwrap();
    fs::create_dir(&meta).unwrap();
    let scan = store.scan().unwrap();

    assert!(scan.entries.is_empty());
    assert_eq!(scan.diagnostics.len(), 1);
    assert_eq!(scan.diagnostics[0].slug.as_deref(), Some("fast"));
}

#[test]
fn a_registry_summary_never_overrides_authoritative_metadata() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    store
        .create(request("Truth", StorageMode::Copy, "authoritative"))
        .unwrap();
    let mut document = registry(&root);
    row_mut(&mut document, "truth").insert(
        "description".to_owned(),
        Value::String("stale projection".to_owned()),
    );
    write_registry(&root, &document);
    let before = fs::read(root.path().join("registry.toml")).unwrap();

    let scan = store.scan().unwrap();

    assert_eq!(description(&scan, "truth"), "authoritative");
    assert_eq!(fs::read(root.path().join("registry.toml")).unwrap(), before);
}

#[test]
fn restoring_mtime_cannot_hide_a_metadata_edit() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    store
        .create(request("Clock", StorageMode::Copy, "before"))
        .unwrap();
    let meta = root.path().join("scripts/clock/meta.toml");
    let original_mtime = fs::metadata(&meta).unwrap().modified().unwrap();
    set_meta_description(&root, "clock", "after");
    fs::OpenOptions::new()
        .write(true)
        .open(&meta)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(original_mtime))
        .unwrap();
    let before = fs::read(root.path().join("registry.toml")).unwrap();

    let scan = store.scan().unwrap();

    assert_eq!(description(&scan, "clock"), "after");
    assert_eq!(fs::read(root.path().join("registry.toml")).unwrap(), before);
}

#[test]
fn a_same_size_edit_with_restored_mtime_invalidates_the_row() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    store
        .create(request("Clock", StorageMode::Copy, "before"))
        .unwrap();
    let meta = root.path().join("scripts/clock/meta.toml");
    let original = fs::read(&meta).unwrap();
    let original_mtime = fs::metadata(&meta).unwrap().modified().unwrap();

    set_meta_description(&root, "clock", "after!");
    assert_eq!(fs::metadata(&meta).unwrap().len(), original.len() as u64);
    fs::OpenOptions::new()
        .write(true)
        .open(&meta)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(original_mtime))
        .unwrap();
    let before = fs::read(root.path().join("registry.toml")).unwrap();

    let scan = store.scan().unwrap();

    assert_eq!(description(&scan, "clock"), "after!");
    assert_eq!(fs::read(root.path().join("registry.toml")).unwrap(), before);
}

#[test]
fn a_same_size_replacement_with_restored_mtime_invalidates_the_row() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    store
        .create(request("Swap", StorageMode::Copy, "before"))
        .unwrap();
    let meta = root.path().join("scripts/swap/meta.toml");
    let original = fs::read(&meta).unwrap();
    let original_mtime = fs::metadata(&meta).unwrap().modified().unwrap();
    let mut replacement = original.clone();
    let start = replacement
        .windows(b"before".len())
        .position(|window| window == b"before")
        .expect("description should be present");
    replacement[start..start + b"after!".len()].copy_from_slice(b"after!");
    let staged = meta.with_extension("replacement");
    fs::write(&staged, replacement).unwrap();
    fs::OpenOptions::new()
        .write(true)
        .open(&staged)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(original_mtime))
        .unwrap();
    fs::rename(staged, &meta).unwrap();
    let before = fs::read(root.path().join("registry.toml")).unwrap();

    let scan = store.scan().unwrap();

    assert_eq!(description(&scan, "swap"), "after!");
    assert_eq!(fs::read(root.path().join("registry.toml")).unwrap(), before);
}

#[test]
fn a_corrupt_same_size_edit_with_restored_mtime_is_never_cache_hidden() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    store
        .create(request("Broken", StorageMode::Copy, "valid"))
        .unwrap();
    let meta = root.path().join("scripts/broken/meta.toml");
    let original = fs::read(&meta).unwrap();
    let original_mtime = fs::metadata(&meta).unwrap().modified().unwrap();
    fs::write(&meta, vec![b'!'; original.len()]).unwrap();
    fs::OpenOptions::new()
        .write(true)
        .open(&meta)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(original_mtime))
        .unwrap();
    let before = fs::read(root.path().join("registry.toml")).unwrap();

    let scan = store.scan().unwrap();

    assert!(scan.entries.is_empty());
    assert_eq!(scan.diagnostics.len(), 1);
    assert_eq!(scan.diagnostics[0].slug.as_deref(), Some("broken"));
    assert_eq!(fs::read(root.path().join("registry.toml")).unwrap(), before);
}

#[test]
fn a_fresh_python_legacy_row_keeps_the_v040_index_fast_path() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    store
        .create(request("Legacy", StorageMode::Copy, "authoritative"))
        .unwrap();
    let mut document = registry(&root);
    let legacy = row_mut(&mut document, "legacy");
    legacy.remove("skit_cache");
    legacy.insert(
        "description".to_owned(),
        Value::String("from legacy index".to_owned()),
    );
    write_registry(&root, &document);
    let before = fs::read(root.path().join("registry.toml")).unwrap();

    let scan = store.scan().unwrap();

    assert_eq!(description(&scan, "legacy"), "from legacy index");
    assert_eq!(fs::read(root.path().join("registry.toml")).unwrap(), before);
}

#[test]
fn concurrent_atomic_updates_never_mix_cache_and_metadata_generations() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let entry = store
        .create(request("First", StorageMode::Copy, "left"))
        .unwrap();
    let worker_store = store.clone();
    let start = Arc::new(Barrier::new(2));
    let done = Arc::new(AtomicBool::new(false));
    let worker_start = Arc::clone(&start);
    let worker_done = Arc::clone(&done);
    let worker = thread::spawn(move || {
        let mut held = entry;
        worker_start.wait();
        for iteration in 0..200 {
            let (name, description) = if iteration % 2 == 0 {
                ("Second", "right")
            } else {
                ("First", "left")
            };
            held = worker_store
                .update_entry(
                    &held,
                    UpdateEntry {
                        name: name.to_owned(),
                        description: description.to_owned(),
                        settings: EntrySettings::from_meta(&held.meta),
                        workdir: held.meta.workdir.clone(),
                        source: None,
                        expected_source_hash: held.meta.source_hash.clone(),
                    },
                )
                .unwrap();
            thread::yield_now();
        }
        worker_done.store(true, Ordering::Release);
    });

    start.wait();
    let mut observations = 0_usize;
    while !done.load(Ordering::Acquire) || observations < 200 {
        let scan = store.scan().unwrap();
        let summary = scan.entries.first().expect("entry should remain present");
        assert!(
            matches!(
                (summary.name.as_str(), summary.description.as_str()),
                ("First", "left") | ("Second", "right")
            ),
            "cache and metadata generations were mixed: {summary:?}"
        );
        assert!(scan.diagnostics.is_empty());
        observations += 1;
    }
    worker.join().unwrap();
}

#[test]
fn stale_and_malformed_rows_fall_back_without_rewriting_the_registry() {
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
    stale.insert(
        "description".to_owned(),
        Value::String("old row".to_owned()),
    );
    stale.insert("mtime_ns".to_owned(), Value::Integer(0));
    row_mut(&mut document, "malformed").insert("name".to_owned(), Value::Integer(7));
    write_registry(&root, &document);
    let before = fs::read(root.path().join("registry.toml")).unwrap();

    let scan = store.scan().unwrap();

    assert_eq!(description(&scan, "stale"), "after hand edit");
    assert_eq!(description(&scan, "malformed"), "authoritative");
    assert!(scan.diagnostics.is_empty());

    assert_eq!(fs::read(root.path().join("registry.toml")).unwrap(), before);
}

#[test]
fn a_missing_registry_is_not_created_by_a_read() {
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

    assert!(scan.entries.is_empty());
    assert!(scan.diagnostics.is_empty());
    assert!(!root.path().join("registry.toml").exists());
}

#[test]
fn repeated_reads_do_not_repair_a_stale_registry() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    store
        .create(request("Busy", StorageMode::Copy, "before"))
        .unwrap();
    set_meta_description(&root, "busy", "after hand edit");
    let mut document = registry(&root);
    let busy = row_mut(&mut document, "busy");
    busy.insert(
        "description".to_owned(),
        Value::String("old row".to_owned()),
    );
    busy.insert("mtime_ns".to_owned(), Value::Integer(0));
    write_registry(&root, &document);
    let before = fs::read(root.path().join("registry.toml")).unwrap();

    let scan = store.scan().unwrap();
    assert_eq!(description(&scan, "busy"), "after hand edit");
    store.scan().unwrap();
    assert_eq!(fs::read(root.path().join("registry.toml")).unwrap(), before);
}

#[test]
fn invalid_mode_and_missing_reference_target_fall_back_without_self_heal() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    store
        .create(request("Copy", StorageMode::Copy, "copy"))
        .unwrap();
    store
        .create(request("Linked", StorageMode::Reference, "linked"))
        .unwrap();

    let mut document = registry(&root);
    row_mut(&mut document, "copy")
        .insert("mode".to_owned(), Value::String("future-mode".to_owned()));
    row_mut(&mut document, "linked").remove("target");
    write_registry(&root, &document);
    let before = fs::read(root.path().join("registry.toml")).unwrap();

    let scan = store.scan().unwrap();

    assert_eq!(description(&scan, "copy"), "copy");
    assert_eq!(description(&scan, "linked"), "linked");
    assert_eq!(fs::read(root.path().join("registry.toml")).unwrap(), before);
}
