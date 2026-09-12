use std::{fs, sync::mpsc, thread, time::Duration};

use skit_application::{
    CreateEntry, EntryMutationRepository, EntryPayload, EntryRepository, RepositoryError,
    SourcePermissions,
};
use skit_domain::{EntryKind, EntrySettings, StorageMode};
use skit_store::{FileStore, content_hash};
use tempfile::TempDir;

fn copied_shell(bytes: &[u8]) -> CreateEntry {
    CreateEntry {
        name: "Demo".to_owned(),
        kind: EntryKind::parse("shell").unwrap(),
        mode: StorageMode::Copy,
        source: "/original/demo.sh".to_owned(),
        workdir: "invoke".to_owned(),
        description: String::new(),
        payload: Some(EntryPayload {
            bytes: bytes.to_vec(),
            stored_name: Some("script.sh".to_owned()),
            permissions: SourcePermissions::default(),
        }),
        settings: EntrySettings::default(),
    }
}

#[test]
fn launch_refuses_an_idless_replacement_with_identical_metadata_but_different_bytes() {
    let root = TempDir::new().unwrap();
    let directory = root.path().join("scripts/legacy");
    fs::create_dir_all(&directory).unwrap();
    let metadata = concat!(
        "name = \"Legacy\"\n",
        "kind = \"shell\"\n",
        "mode = \"copy\"\n",
        "source = \"/original/legacy.sh\"\n",
        "workdir = \"invoke\"\n",
    );
    fs::write(directory.join("meta.toml"), metadata).unwrap();
    fs::write(directory.join("script.sh"), b"printf OLD").unwrap();

    let store = FileStore::new(root.path());
    store.rebuild_registry().unwrap();
    let held = store.resolve("legacy").unwrap();
    let expected = content_hash(b"printf OLD");

    fs::remove_dir_all(&directory).unwrap();
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("meta.toml"), metadata).unwrap();
    fs::write(directory.join("script.sh"), b"printf NEW").unwrap();

    assert!(matches!(
        store.prepare_launch(&held, Some(&expected)).unwrap_err(),
        RepositoryError::SourceChanged { .. }
    ));
}

#[test]
fn a_prepared_copy_keeps_its_stored_path_and_allows_source_edits() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let entry = store.create(copied_shell(b"printf OLD")).unwrap();
    let source = store.payload_path(&entry).unwrap();
    let prepared = store
        .prepare_launch(&entry, Some(&entry.meta.source_hash))
        .unwrap();
    assert_eq!(prepared.entry().meta.id, entry.meta.id);
    assert_eq!(prepared.payload_path(), Some(source.as_path()));
    let edited = store
        .commit_copy_edit(&entry, b"printf NEW", &entry.meta.source_hash)
        .unwrap();
    assert_eq!(
        prepared.payload_path(),
        Some(store.payload_path(&edited).unwrap().as_path())
    );
    assert_eq!(fs::read(&source).unwrap(), b"printf NEW");
    drop(prepared);
    assert_eq!(fs::read(&source).unwrap(), b"printf NEW");
}

#[cfg(windows)]
#[test]
fn prepared_copy_allows_readers_that_share_read_access_only() {
    use std::io::Read as _;
    use std::os::windows::fs::OpenOptionsExt as _;
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let entry = store.create(copied_shell(b"printf READER")).unwrap();
    let prepared = store
        .prepare_launch(&entry, Some(&entry.meta.source_hash))
        .unwrap();
    let path = prepared.payload_path().unwrap().to_path_buf();
    let mut reader = fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(&path)
        .unwrap();
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"printf READER");
    drop(reader);
    drop(prepared);
    assert_eq!(fs::read(path).unwrap(), b"printf READER");
}

#[test]
fn launch_refuses_metadata_changed_after_the_form_was_built() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let held = store.create(copied_shell(b"printf OLD")).unwrap();
    store
        .describe(&held, "changed while the form was open")
        .unwrap();

    assert!(matches!(
        store
            .prepare_launch(&held, Some(&held.meta.source_hash))
            .unwrap_err(),
        RepositoryError::StaleEntry { .. }
    ));
}

#[test]
fn a_hand_edited_copy_mode_executable_still_uses_its_recorded_binary() {
    let root = TempDir::new().unwrap();
    let binary = root.path().join("tool");
    fs::write(&binary, b"binary").unwrap();
    let store = FileStore::new(root.path());
    let entry = store
        .create(CreateEntry {
            name: "Binary".to_owned(),
            kind: EntryKind::parse("exe").unwrap(),
            mode: StorageMode::Reference,
            source: binary.display().to_string(),
            workdir: "invoke".to_owned(),
            description: String::new(),
            payload: None,
            settings: EntrySettings::default(),
        })
        .unwrap();
    let metadata = store.entry_dir_path(&entry.slug).join("meta.toml");
    let source = fs::read_to_string(&metadata).unwrap();
    fs::write(
        &metadata,
        source.replace("mode = \"reference\"", "mode = \"copy\""),
    )
    .unwrap();

    let fresh = store.resolve(entry.slug.as_str()).unwrap();
    assert_eq!(store.payload_path(&fresh).unwrap(), binary);
    let prepared = store.prepare_launch(&fresh, None).unwrap();
    assert_eq!(prepared.payload_path(), Some(binary.as_path()));
}

#[test]
fn parallel_launches_share_the_remove_lease() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let entry = store.create(copied_shell(b"printf OLD")).unwrap();
    let first = store
        .prepare_launch(&entry, Some(&entry.meta.source_hash))
        .unwrap();

    let second_store = store.clone();
    let second_entry = entry.clone();
    let expected = entry.meta.source_hash.clone();
    let (prepared, prepared_rx) = mpsc::channel();
    let worker = thread::spawn(move || {
        let second = second_store
            .prepare_launch(&second_entry, Some(&expected))
            .unwrap();
        prepared.send(()).unwrap();
        drop(second);
    });

    let overlapped = prepared_rx.recv_timeout(Duration::from_millis(500)).is_ok();
    drop(first);
    worker.join().unwrap();

    assert!(overlapped, "independent launches were serialized");
}

#[cfg(unix)]
#[test]
fn preparing_and_dropping_a_copy_keeps_its_source_mode_and_bytes() {
    use std::os::unix::fs::PermissionsExt as _;
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let entry = store.create(copied_shell(b"printf STORED")).unwrap();
    let source = store.payload_path(&entry).unwrap();
    fs::set_permissions(&source, fs::Permissions::from_mode(0o640)).unwrap();
    let prepared = store
        .prepare_launch(&entry, Some(&entry.meta.source_hash))
        .unwrap();
    assert_eq!(prepared.payload_path(), Some(source.as_path()));
    drop(prepared);
    assert_eq!(fs::read(&source).unwrap(), b"printf STORED");
    assert_eq!(
        fs::metadata(source).unwrap().permissions().mode() & 0o777,
        0o640
    );
}
