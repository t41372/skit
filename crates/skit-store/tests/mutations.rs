use std::{
    fs::{self, OpenOptions},
    sync::mpsc,
    sync::{Arc, Barrier},
    thread,
    time::Duration,
};

use skit_application::{
    CreateEntry, EntryMutationRepository, EntryPayload, EntryRepository, RepositoryError,
    SourcePermissions, UpdateEntry,
};
use skit_domain::{EntryKind, EntrySettings, StorageMode};
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
        settings: EntrySettings::default(),
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

    let mut create = request("Hello", bytes);
    create.settings.dependencies = vec!["requests>=2".to_owned()];
    create.settings.interpreter = "python3.14".to_owned();
    let entry = store.create(create).unwrap();

    assert_eq!(entry.slug.as_str(), "hello");
    assert!(entry.meta.id.is_some());
    assert_eq!(entry.meta.source_hash, content_hash(bytes));
    let settings = EntrySettings::from_meta(&entry.meta);
    assert_eq!(settings.dependencies, ["requests>=2"]);
    assert_eq!(settings.interpreter, "python3.14");
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
fn a_legacy_mutation_claims_identity_before_it_writes() {
    let root = TempDir::new().unwrap();
    write_legacy_meta(&root, "legacy", "Legacy");
    let store = FileStore::new(root.path());
    let held = store.resolve("legacy").unwrap();
    assert!(held.meta.id.is_none());

    let described = store.describe(&held, "claimed during mutation").unwrap();

    assert!(described.meta.id.is_some());
    assert_eq!(described.meta.description, "claimed during mutation");
    assert_eq!(store.resolve("legacy").unwrap().meta.id, described.meta.id);
}

#[test]
fn metadata_mutations_preserve_every_unknown_toml_value() {
    let root = TempDir::new().unwrap();
    let directory = root.path().join("scripts/extensions");
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join("meta.toml"),
        r#"name = "Extensions"
kind = "future-kind"
mode = "reference"
source = "/tmp/tool"
release_at = 2026-08-08T12:34:56Z
limit = inf
future = { enabled = true, values = [1, 2, 3] }
"#,
    )
    .unwrap();
    let store = FileStore::new(root.path());

    let held = store.resolve("extensions").unwrap();
    store.describe(&held, "updated").unwrap();

    let document = fs::read_to_string(directory.join("meta.toml"))
        .unwrap()
        .parse::<toml::Table>()
        .unwrap();
    assert_eq!(
        document["release_at"].as_datetime().unwrap().to_string(),
        "2026-08-08T12:34:56Z"
    );
    assert!(document["limit"].as_float().unwrap().is_infinite());
    assert_eq!(document["future"]["enabled"].as_bool(), Some(true));
    assert_eq!(document["future"]["values"].as_array().unwrap().len(), 3);
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
    assert_eq!(renamed.slug.as_str(), "before");
    assert_eq!(renamed.meta.name, "After Name");
    assert_eq!(renamed.meta.id, claimed.meta.id);
    assert_eq!(
        fs::read(root.path().join("scripts/before/script.py")).unwrap(),
        b"payload"
    );
    assert_eq!(store.resolve("before").unwrap().meta.name, "After Name");
    assert_eq!(store.resolve("After Name").unwrap().slug.as_str(), "before");

    assert_eq!(store.remove(&renamed).unwrap(), "After Name");
    assert!(!root.path().join("scripts/before").exists());
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
fn combined_update_commits_metadata_and_source_under_one_identity_check() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let claimed = store
        .claim_identity(&store.create(request("Combined", b"before")).unwrap())
        .unwrap();
    let settings = EntrySettings {
        dependencies: vec!["requests".to_owned()],
        ..EntrySettings::default()
    };

    let updated = store
        .update_entry(
            &claimed,
            UpdateEntry {
                name: "After".to_owned(),
                description: "complete".to_owned(),
                settings,
                workdir: "store".to_owned(),
                source: Some(b"after".to_vec()),
                expected_source_hash: claimed.meta.source_hash.clone(),
            },
        )
        .unwrap();

    assert_eq!(updated.slug, claimed.slug);
    assert_eq!(updated.meta.name, "After");
    assert_eq!(updated.meta.description, "complete");
    assert_eq!(updated.meta.workdir, "store");
    assert_eq!(
        fs::read(root.path().join("scripts/combined/script.py")).unwrap(),
        b"after"
    );
    assert_eq!(store.resolve("After").unwrap(), updated);

    let before_meta = fs::read(root.path().join("scripts/combined/meta.toml")).unwrap();
    let error = store
        .update_entry(
            &updated,
            UpdateEntry {
                name: "Never".to_owned(),
                description: "must not land".to_owned(),
                settings: EntrySettings::default(),
                workdir: "invoke".to_owned(),
                source: Some(b"never".to_vec()),
                expected_source_hash: "sha256:stale".to_owned(),
            },
        )
        .unwrap_err();
    assert!(matches!(error, RepositoryError::SourceChanged { .. }));
    assert_eq!(
        fs::read(root.path().join("scripts/combined/meta.toml")).unwrap(),
        before_meta
    );
    assert_eq!(
        fs::read(root.path().join("scripts/combined/script.py")).unwrap(),
        b"after"
    );
}

#[test]
fn module_typed_copy_edits_ignore_dependency_support_files() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let mut create = request("Module", b"export const value = 1;\n");
    create.kind = EntryKind::parse("js").unwrap();
    create.source = "/original/module.mjs".to_owned();
    create.payload.as_mut().unwrap().stored_name = Some("script.mjs".to_owned());
    let entry = store.create(create).unwrap();
    let directory = root.path().join("scripts/module");
    fs::write(directory.join("package.json"), "{}\n").unwrap();
    fs::write(directory.join("package-lock.json"), "{}\n").unwrap();
    fs::write(directory.join(".skit-deps"), "stamp\n").unwrap();

    let edited = store
        .commit_copy_edit(
            &entry,
            b"export const value = 2;\n",
            &entry.meta.source_hash,
        )
        .unwrap();
    assert_eq!(
        fs::read(directory.join("script.mjs")).unwrap(),
        b"export const value = 2;\n"
    );

    let updated = store
        .update_entry(
            &edited,
            UpdateEntry {
                name: "Renamed module".to_owned(),
                description: "updated".to_owned(),
                settings: EntrySettings::default(),
                workdir: "invoke".to_owned(),
                source: Some(b"export const value = 3;\n".to_vec()),
                expected_source_hash: edited.meta.source_hash.clone(),
            },
        )
        .unwrap();
    assert_eq!(updated.slug, entry.slug);
    assert_eq!(
        fs::read(directory.join("script.mjs")).unwrap(),
        b"export const value = 3;\n"
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

#[test]
fn removal_waits_for_the_dependency_transaction_lock() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let claimed = store
        .claim_identity(&store.create(request("Locked", b"base")).unwrap())
        .unwrap();
    let lock_path = root.path().join(".locks/locked.skit-deps.lock");
    fs::create_dir_all(lock_path.parent().unwrap()).unwrap();
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)
        .unwrap();
    lock.lock().unwrap();
    let (started_tx, started_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let worker = thread::spawn(move || {
        started_tx.send(()).unwrap();
        done_tx.send(store.remove(&claimed)).unwrap();
    });
    started_rx.recv().unwrap();

    assert!(done_rx.recv_timeout(Duration::from_millis(100)).is_err());
    drop(lock);
    assert_eq!(
        done_rx
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .unwrap(),
        "Locked"
    );
    worker.join().unwrap();
}

#[test]
fn a_rename_that_keeps_the_same_slug_reuses_its_own_directory() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let entry = store.create(request("Report", b"print(1)\n")).unwrap();
    assert_eq!(entry.slug.as_str(), "report");

    // The new display name derives the same slug, so the entry keeps its address.
    let renamed = store.rename(&entry, "  Report  ").unwrap();

    assert_eq!(renamed.slug.as_str(), "report");
    assert_eq!(renamed.meta.name, "Report");
    assert!(root.path().join("scripts/report").is_dir());
    assert_eq!(store.scan().unwrap().entries.len(), 1);
}
