use skit_application::{CreateEntry, LibraryService};
use skit_domain::{EntryKind, EntrySettings, StorageMode};
use skit_store::FileStore;
use tempfile::TempDir;

#[test]
fn test_add_script_explicit_workdir_wins_in_reference_mode() {
    let data = TempDir::new().unwrap();
    let source = data.path().join("outside.sh");
    std::fs::write(&source, b"#!/bin/sh\necho hi\n").unwrap();
    let service = LibraryService::new(FileStore::new(data.path()));
    let entry = service
        .add(CreateEntry {
            name: "shell-ref".to_owned(),
            kind: EntryKind::parse("shell").unwrap(),
            mode: StorageMode::Reference,
            source: source.display().to_string(),
            workdir: "invoke".to_owned(),
            description: String::new(),
            payload: None,
            settings: EntrySettings::default(),
        })
        .unwrap();
    assert_eq!(entry.meta.mode, StorageMode::Reference);
    assert_eq!(entry.meta.workdir, "invoke");
    assert_eq!(entry.meta.source, source.display().to_string());
}
