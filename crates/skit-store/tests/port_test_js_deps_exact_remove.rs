//! Exact store-removal dependency-lock ports from Python v0.4 `tests/test_js_deps.py`.

use std::{
    fs::{self, OpenOptions},
    path::Path,
    sync::mpsc,
    thread,
    time::Duration,
};

use skit_application::{
    CreateEntry, EntryMutationRepository as _, EntryPayload, SourcePermissions, payload_stored_name,
};
use skit_domain::{EntryKind, EntrySettings, StorageMode};
use skit_store::FileStore;
use tempfile::TempDir;

fn add_js(root: &TempDir) -> (FileStore, skit_domain::Entry) {
    let store = FileStore::new(root.path());
    let kind = EntryKind::parse("js").unwrap();
    let entry = store
        .create(CreateEntry {
            name: "t".to_owned(),
            kind: kind.clone(),
            mode: StorageMode::Copy,
            source: "t.js".to_owned(),
            workdir: "invoke".to_owned(),
            description: String::new(),
            payload: Some(EntryPayload {
                bytes: b"console.log(1);\n".to_vec(),
                stored_name: Some(payload_stored_name(&kind, Path::new("t.js"))),
                permissions: SourcePermissions::default(),
            }),
            settings: EntrySettings {
                dependencies: vec!["chalk".to_owned()],
                ..EntrySettings::default()
            },
        })
        .unwrap();
    (store, entry)
}

fn dependency_lock_path(root: &TempDir, entry: &skit_domain::Entry) -> std::path::PathBuf {
    root.path()
        .join(".locks")
        .join(format!("{}.skit-deps.lock", entry.slug.as_str()))
}

#[test]
fn test_store_remove_waits_for_a_live_js_install_lock() {
    let root = TempDir::new().unwrap();
    let (store, entry) = add_js(&root);
    let entry_dir = store.entry_dir_path(&entry.slug);
    let lock_path = dependency_lock_path(&root, &entry);
    fs::create_dir_all(lock_path.parent().unwrap()).unwrap();
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .unwrap();
    lock.lock().unwrap();

    let (removed_tx, removed_rx) = mpsc::channel();
    let data = root.path().to_path_buf();
    let held = entry.clone();
    let remover = thread::spawn(move || {
        let store = FileStore::new(data);
        let result = store.remove(&held);
        removed_tx.send(result).unwrap();
    });

    assert!(
        removed_rx.recv_timeout(Duration::from_millis(100)).is_err(),
        "store.remove entered while the JavaScript installer lock was live"
    );
    drop(lock);
    let removed = removed_rx.recv_timeout(Duration::from_secs(2)).unwrap().unwrap();
    remover.join().unwrap();
    assert_eq!(removed, entry.meta.name);
    assert!(!entry_dir.exists());
    assert!(lock_path.is_file(), "persistent dependency lock inode was removed with the entry");
}

#[test]
fn test_store_remove_surfaces_install_lock_failure_without_deleting_entry() {
    let root = TempDir::new().unwrap();
    let (store, entry) = add_js(&root);
    let payload = store.payload_path(&entry).unwrap();
    let lock_path = dependency_lock_path(&root, &entry);
    fs::create_dir_all(&lock_path).unwrap();

    let error = store.remove(&entry).unwrap_err();
    assert!(error.to_string().contains("skit-deps.lock"), "{error}");
    assert!(payload.is_file(), "entry payload was deleted despite dependency-lock refusal");
    let fresh = skit_application::EntryRepository::resolve(&store, entry.slug.as_str()).unwrap();
    assert_eq!(fresh.meta.name, entry.meta.name);
}