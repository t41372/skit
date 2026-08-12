//! Exact corrupt-registry ports from Python `tests/test_store_fix.py` at `main@206f9ef`.

use std::fs;

use skit_application::{
    CreateEntry, EntryMutationRepository as _, EntryPayload, EntryRepository as _, SourcePermissions,
};
use skit_domain::{EntryKind, EntrySettings, StorageMode};
use skit_store::FileStore;
use tempfile::TempDir;

fn request() -> CreateEntry {
    CreateEntry {
        name: "a".to_owned(),
        kind: EntryKind::parse("future-kind").unwrap(),
        mode: StorageMode::Copy,
        source: "/original/a.tool".to_owned(),
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
fn test_corrupt_registry_is_backed_up_and_degrades_to_empty() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    store.create(request()).unwrap();
    let path = root.path().join("registry.toml");
    let corrupt = b"not valid toml [[[";
    fs::write(&path, corrupt).unwrap();

    assert!(store.scan().unwrap().entries.is_empty());
    let backup = root.path().join("registry.toml.corrupt");
    assert_eq!(fs::read(&backup).unwrap(), corrupt);
    assert!(
        !path.exists(),
        "corrupt registry stayed live and would re-trigger parsing on every read"
    );
}

#[test]
fn test_corrupt_registry_recovers_fully_via_doctor_rebuild() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    store.create(request()).unwrap();
    fs::write(root.path().join("registry.toml"), "not valid toml [[[").unwrap();

    let report = store.rebuild_registry_report().unwrap();
    assert_eq!(report.entry_count, 1);
    assert!(report.problems.is_empty(), "{:?}", report.problems);
    let entries = store.scan().unwrap().entries;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "a");
}
