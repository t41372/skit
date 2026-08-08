use std::fs;

use skit_application::{CreateEntry, EntryMutationRepository, EntryPayload, SourcePermissions};
use skit_domain::{EntryKind, StorageMode};
use skit_store::{FileStore, stored_filename};
use tempfile::TempDir;

#[test]
fn stored_filenames_match_the_v040_library_layout() {
    assert_eq!(stored_filename("python"), Some("script.py"));
    assert_eq!(stored_filename("shell"), Some("script.sh"));
    assert_eq!(stored_filename("js"), Some("script.js"));
    assert_eq!(stored_filename("ts"), Some("script.ts"));
    assert_eq!(stored_filename("fish"), Some("script.fish"));
    assert_eq!(stored_filename("powershell"), Some("script.ps1"));
    assert_eq!(stored_filename("ruby"), Some("script.rb"));
    assert_eq!(stored_filename("perl"), Some("script.pl"));
    assert_eq!(stored_filename("lua"), Some("script.lua"));
    assert_eq!(stored_filename("r"), Some("script.r"));
    assert_eq!(stored_filename("prompt"), Some("prompt.md"));
    assert_eq!(stored_filename("exe"), None);
    assert_eq!(stored_filename("command"), None);
}

#[test]
fn payload_path_uses_the_original_for_references_and_the_stored_copy_for_copies() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let source = root.path().join("source.sh");
    fs::write(&source, b"echo ok\n").unwrap();

    let copied = store
        .create(CreateEntry {
            name: "Copied".to_owned(),
            kind: EntryKind::parse("shell").unwrap(),
            mode: StorageMode::Copy,
            source: source.display().to_string(),
            description: String::new(),
            workdir: "invoke".to_owned(),
            payload: Some(EntryPayload {
                bytes: fs::read(&source).unwrap(),
                stored_name: Some("script.sh".to_owned()),
                permissions: SourcePermissions::default(),
            }),
        })
        .unwrap();
    assert_eq!(
        store.payload_path(&copied).unwrap(),
        root.path().join("scripts/copied/script.sh")
    );
    assert_eq!(
        store.entry_dir_path(&copied.slug),
        root.path().join("scripts/copied")
    );

    let referenced = store
        .create(CreateEntry {
            name: "Referenced".to_owned(),
            kind: EntryKind::parse("shell").unwrap(),
            mode: StorageMode::Reference,
            source: source.display().to_string(),
            description: String::new(),
            workdir: "origin".to_owned(),
            payload: None,
        })
        .unwrap();
    assert_eq!(store.payload_path(&referenced).unwrap(), source);
}
