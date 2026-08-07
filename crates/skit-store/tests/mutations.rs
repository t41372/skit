use std::{
    fs,
    sync::{Arc, Barrier},
    thread,
};

use skit_application::{
    CreateEntry, EntryMutationRepository, EntryPayload, EntryRepository, RepositoryError,
    SourcePermissions,
};
use skit_domain::{EntryKind, StorageMode};
use skit_store::{FileStore, content_hash};
use tempfile::TempDir;

fn request(name: &str, bytes: &[u8]) -> CreateEntry {
    CreateEntry {
        name: name.to_owned(),
        kind: EntryKind::parse("python").unwrap(),
        mode: StorageMode::Copy,
        source: format!("/original/{name}.py"),
        workdir: "invoke".to_owned(),
        description: String::new(),
        payload: Some(EntryPayload {
            bytes: bytes.to_vec(),
            stored_name: Some("script.py".to_owned()),
            permissions: SourcePermissions {
                readonly: false,
                unix_mode: Some(0o755),
            },
        }),
    }
}

fn write_legacy_meta(root: &TempDir, slug: &str, name: &str) {
    let dir = root.path().join("scripts").join(slug);
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("meta.toml"),
        format!(
            "name = {name:?}\nkind = \"python\"\nmode = \"copy\"\nsource = \"/old.py\"\nsource_hash = \"\"\n"
        ),
    )
    .unwrap();
    fs::write(dir.join("script.py"), b"print('old')\n").unwrap();
}

#[test]
fn content_hash_is_the_existing_sha256_contract() {
    assert_eq!(
        content_hash(b""),
        "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        content_hash(b"abc"),
        "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn create_is_atomic_mints_identity_and_preserves_payload_bytes() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let bytes = b"#!/usr/bin/env python3\r\nprint('hello')\r\n";

    let entry = store.create(request("Hello", bytes)).unwrap();

    assert_eq!(entry.slug.as_str(), "hello");
    assert!(entry.meta.id.is_some());
    assert_eq!(entry.meta.source_hash, content_hash(bytes));
    assert_eq!(
        fs::read(root.path().join("scripts/hello/script.py")).unwrap(),
        bytes
    );
    assert!(
        fs::read_to_string(root.path().join("scripts/hello/meta.toml"))
            .unwrap()
            .contains("id = ")
    );
    let staging = root.path().join(".staging");
    assert!(
        !staging.exists() || fs::read_dir(staging).unwrap().next().is_none(),
        "successful create must not leave a staged directory"
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(root.path().join("scripts/hello/script.py"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o755
        );
    }
}

#[test]
fn create_refuses_conflicts_and_path_traversal_without_partial_entries() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    store.create(request("Hello", b"first")).unwrap();

    let conflict = store.create(request("Hello", b"second")).unwrap_err();
    assert!(matches!(conflict, RepositoryError::Conflict { .. }));
    assert_eq!(
        fs::read(root.path().join("scripts/hello/script.py")).unwrap(),
        b"first"
    );

    let mut invalid = request("Escape", b"payload");
    invalid.payload.as_mut().unwrap().stored_name = Some("../outside".to_owned());
    let error = store.create(invalid).unwrap_err();
    assert!(matches!(error, RepositoryError::InvalidMutation { .. }));
    assert!(!root.path().join("scripts/escape").exists());
    assert!(!root.path().join("outside").exists());
}

#[test]
fn legacy_claim_stamps_once_and_old_handles_cannot_touch_a_reincarnation() {
    let root = TempDir::new().unwrap();
    write_legacy_meta(&root, "legacy", "Legacy");
    let store = FileStore::new(root.path());
    let held = store.resolve("legacy").unwrap();
    assert!(held.meta.id.is_none());

    let claimed = store.claim_identity(&held).unwrap();
    let old_id = claimed.meta.id.clone().unwrap();
    assert_eq!(
        store.resolve("legacy").unwrap().meta.id,
        Some(old_id.clone())
    );

    store.remove(&claimed).unwrap();
    let replacement = store.create(request("Legacy", b"replacement")).unwrap();
    assert_ne!(replacement.meta.id, Some(old_id));

    let error = store.describe(&claimed, "must not land").unwrap_err();
    assert!(matches!(error, RepositoryError::StaleEntry { .. }));
    assert_eq!(store.resolve("legacy").unwrap().meta.description, "");
}

#[test]
fn rename_describe_and_remove_preserve_identity_and_payload() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let created = store.create(request("Before", b"payload")).unwrap();
    let claimed = store.claim_identity(&created).unwrap();

    let described = store.describe(&claimed, "useful").unwrap();
    assert_eq!(described.meta.description, "useful");
    assert_eq!(described.meta.id, claimed.meta.id);

    let renamed = store.rename(&described, "After Name").unwrap();
    assert_eq!(renamed.slug.as_str(), "after-name");
    assert_eq!(renamed.meta.name, "After Name");
    assert_eq!(renamed.meta.id, claimed.meta.id);
    assert_eq!(
        fs::read(root.path().join("scripts/after-name/script.py")).unwrap(),
        b"payload"
    );
    assert!(matches!(
        store.resolve("before"),
        Err(RepositoryError::NotFound { .. })
    ));

    assert_eq!(store.remove(&renamed).unwrap(), "After Name");
    assert!(!root.path().join("scripts/after-name").exists());
}

#[test]
fn copy_edit_is_identity_and_source_compare_and_swap() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let created = store.create(request("Edit", b"base")).unwrap();
    let claimed = store.claim_identity(&created).unwrap();

    let stale = store
        .commit_copy_edit(&claimed, b"wrong", "sha256:not-the-base")
        .unwrap_err();
    assert!(matches!(stale, RepositoryError::SourceChanged { .. }));
    assert_eq!(
        fs::read(root.path().join("scripts/edit/script.py")).unwrap(),
        b"base"
    );

    let edited = store
        .commit_copy_edit(&claimed, b"next", &claimed.meta.source_hash)
        .unwrap();
    assert_eq!(edited.meta.source_hash, content_hash(b"next"));
    assert_eq!(
        fs::read(root.path().join("scripts/edit/script.py")).unwrap(),
        b"next"
    );
}

#[test]
fn concurrent_copy_edits_allow_exactly_one_source_cas_winner() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let claimed = store
        .claim_identity(&store.create(request("Race", b"base")).unwrap())
        .unwrap();
    let expected = claimed.meta.source_hash.clone();
    let barrier = Arc::new(Barrier::new(3));

    let handles = [b"left".as_slice(), b"right".as_slice()].map(|payload| {
        let store = store.clone();
        let held = claimed.clone();
        let expected = expected.clone();
        let barrier = Arc::clone(&barrier);
        let payload = payload.to_vec();
        thread::spawn(move || {
            barrier.wait();
            store.commit_copy_edit(&held, &payload, &expected)
        })
    });
    barrier.wait();
    let results = handles.map(|handle| handle.join().unwrap());

    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, Err(RepositoryError::SourceChanged { .. })))
            .count(),
        1
    );
    let bytes = fs::read(root.path().join("scripts/race/script.py")).unwrap();
    assert!(bytes == b"left" || bytes == b"right");
}
