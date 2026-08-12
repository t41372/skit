//! Exact positive serialization pilot for Python `test_registry_lock_serializes_concurrent_holders`.

use std::{
    fs::OpenOptions,
    sync::mpsc,
    thread,
    time::Duration,
};

use skit_application::{
    CreateEntry, EntryMutationRepository as _, EntryPayload, EntryRepository as _, SourcePermissions,
};
use skit_domain::{EntryKind, EntrySettings, StorageMode};
use skit_store::FileStore;
use tempfile::TempDir;

fn request() -> CreateEntry {
    CreateEntry {
        name: "serialized".to_owned(),
        kind: EntryKind::parse("future-kind").unwrap(),
        mode: StorageMode::Copy,
        source: "/original/serialized.tool".to_owned(),
        workdir: "invoke".to_owned(),
        description: String::new(),
        payload: Some(EntryPayload {
            bytes: b"payload\n".to_vec(),
            stored_name: Some("script.tool".to_owned()),
            permissions: SourcePermissions::default(),
        }),
        settings: EntrySettings::default(),
    }
}

#[test]
fn test_registry_lock_serializes_concurrent_holders() {
    let root = TempDir::new().unwrap();
    let lock_path = root.path().join("registry.native.lock");
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .unwrap();
    lock.lock().unwrap();

    let store = FileStore::new(root.path());
    let worker_store = store.clone();
    let (started_tx, started_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let worker = thread::spawn(move || {
        started_tx.send(()).unwrap();
        done_tx.send(worker_store.create(request())).unwrap();
    });
    started_rx.recv().unwrap();

    assert!(
        done_rx.recv_timeout(Duration::from_millis(100)).is_err(),
        "a second registry mutation completed while another holder still owned the native lock"
    );

    drop(lock);
    let created = done_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("registry mutation did not resume after the lock was released")
        .unwrap();
    worker.join().unwrap();

    assert_eq!(created.meta.name, "serialized");
    assert_eq!(
        store.resolve("serialized").unwrap().slug,
        created.slug,
        "the resumed mutation did not commit a coherent registry + entry"
    );
}
