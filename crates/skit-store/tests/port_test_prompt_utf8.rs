//! Store/application boundary ports from Python `tests/test_prompt_utf8.py` at `main@206f9ef`.
//!
//! The storage adapter receives byte snapshots from the application. Copy mode must persist exactly
//! that snapshot, reference mode must hash that snapshot without making a copy, and the generic add
//! boundary must not allow an invalid prompt to bypass the strict prompt invariant.

use std::fs;

use skit_application::{
    CreateEntry, EntryMutationRepository, EntryPayload, EntryRepository, LibraryService,
    RepositoryError, SourcePermissions,
};
use skit_domain::{EntryKind, EntrySettings, StorageMode};
use skit_store::{FileStore, content_hash};
use tempfile::TempDir;

fn prompt_request(
    name: &str,
    mode: StorageMode,
    source: &std::path::Path,
    snapshot: &[u8],
) -> CreateEntry {
    CreateEntry {
        name: name.to_owned(),
        kind: EntryKind::parse("prompt").unwrap(),
        mode,
        source: source.display().to_string(),
        workdir: if mode == StorageMode::Reference {
            "origin"
        } else {
            "invoke"
        }
        .to_owned(),
        description: String::new(),
        payload: Some(EntryPayload {
            bytes: snapshot.to_vec(),
            stored_name: Some("prompt.md".to_owned()),
            permissions: SourcePermissions::default(),
        }),
        settings: EntrySettings::default(),
    }
}

#[test]
fn test_store_accepts_valid_utf8_prompt_byte_exact() {
    let root = TempDir::new().unwrap();
    let source = root.path().join("valid.prompt.md");
    let body = "你好 {{name}} — café\n".as_bytes();
    fs::write(&source, body).unwrap();
    let store = FileStore::new(root.path().join("data"));

    let entry = store
        .create(prompt_request("valid", StorageMode::Copy, &source, body))
        .unwrap();

    assert_eq!(entry.meta.kind.as_str(), "prompt");
    assert_eq!(
        fs::read(store.payload_path(&entry).unwrap()).unwrap(),
        body
    );
}

#[test]
fn test_store_add_prompt_copies_the_validated_snapshot_not_a_second_read() {
    let root = TempDir::new().unwrap();
    let source = root.path().join("race.prompt.md");
    let validated = "validated {{name}}\n".as_bytes();
    fs::write(&source, validated).unwrap();
    let snapshot = fs::read(&source).unwrap();
    fs::write(&source, b"changed after validation\n").unwrap();
    let store = FileStore::new(root.path().join("data"));

    let entry = store
        .create(prompt_request(
            "race",
            StorageMode::Copy,
            &source,
            &snapshot,
        ))
        .unwrap();

    assert_eq!(
        fs::read(store.payload_path(&entry).unwrap()).unwrap(),
        validated
    );
    assert_eq!(fs::read(&source).unwrap(), b"changed after validation\n");
}

#[test]
fn test_store_add_prompt_reference_hash_tracks_the_validated_snapshot() {
    let root = TempDir::new().unwrap();
    let source = root.path().join("linked.prompt.md");
    let validated = "before {{name}}\n".as_bytes();
    fs::write(&source, validated).unwrap();
    let snapshot = fs::read(&source).unwrap();
    fs::write(&source, b"after\xff\n").unwrap();
    let store = FileStore::new(root.path().join("data"));

    let entry = store
        .create(prompt_request(
            "linked",
            StorageMode::Reference,
            &source,
            &snapshot,
        ))
        .unwrap();

    assert_eq!(entry.meta.source_hash, content_hash(validated));
    assert_eq!(entry.meta.source, source.display().to_string());
    let directory = store.data_dir().join("scripts").join(entry.slug.as_str());
    assert!(directory.join("meta.toml").is_file());
    assert!(!directory.join("prompt.md").exists());
}

#[test]
fn test_generic_store_api_also_refuses_invalid_prompt_utf8() {
    let root = TempDir::new().unwrap();
    let source = root.path().join("bad.prompt.md");
    let invalid = b"bad\xff";
    fs::write(&source, invalid).unwrap();
    let service = LibraryService::new(FileStore::new(root.path().join("data")));

    let error = service
        .add(prompt_request(
            "bad",
            StorageMode::Copy,
            &source,
            invalid,
        ))
        .unwrap_err();

    assert!(matches!(error, RepositoryError::InvalidMutation { .. }));
    let message = error.to_string();
    assert!(message.contains("UTF-8"), "{message}");
    assert!(message.contains("3"), "invalid byte offset was lost: {message}");
    assert!(service.list().unwrap().entries.is_empty());
}
