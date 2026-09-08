use std::{fs, io, sync::mpsc, thread, time::Duration};

use skit_application::{
    CreateEntry, EntryMutationRepository, EntryPayload, EntryRepository, RepositoryError,
    SourcePermissions,
};
use skit_domain::{EntryKind, EntrySettings, StorageMode};
use skit_store::{
    FileStore, LaunchSnapshotAllocator, LaunchSnapshotRequest, LaunchSnapshotStem, content_hash,
};
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

#[derive(Debug)]
struct FixedLaunchSnapshotAllocator(LaunchSnapshotStem);

impl LaunchSnapshotAllocator for FixedLaunchSnapshotAllocator {
    fn next_stem(&self, _request: LaunchSnapshotRequest<'_>) -> io::Result<LaunchSnapshotStem> {
        Ok(self.0)
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
fn a_prepared_copy_keeps_the_verified_bytes_when_the_library_source_changes() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let entry = store.create(copied_shell(b"printf OLD")).unwrap();
    let prepared = store
        .prepare_launch(&entry, Some(&entry.meta.source_hash))
        .unwrap();
    assert_eq!(prepared.entry().meta.id, entry.meta.id);
    let snapshot = prepared.payload_path().unwrap().to_path_buf();

    let edited = store
        .commit_copy_edit(&entry, b"printf NEW", &entry.meta.source_hash)
        .unwrap();

    assert_eq!(
        fs::read(prepared.payload_path().unwrap()).unwrap(),
        b"printf OLD"
    );
    assert_eq!(
        fs::read(store.payload_path(&edited).unwrap()).unwrap(),
        b"printf NEW"
    );
    drop(prepared);
    assert!(!snapshot.exists());
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
    assert!(!path.exists());
}

#[cfg(windows)]
#[test]
fn prepared_launch_drop_clears_readonly_through_its_owned_handle() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let entry = store.create(copied_shell(b"printf READONLY")).unwrap();
    let source = store.payload_path(&entry).unwrap();
    let mut permissions = fs::metadata(&source).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(&source, permissions).unwrap();
    let prepared = store
        .prepare_launch(&entry, Some(&entry.meta.source_hash))
        .unwrap();
    let snapshot = prepared.payload_path().unwrap().to_path_buf();
    assert!(fs::metadata(&snapshot).unwrap().permissions().readonly());

    drop(prepared);

    assert!(!snapshot.exists());

    let linked = store
        .prepare_launch(&entry, Some(&entry.meta.source_hash))
        .unwrap();
    let linked_snapshot = linked.payload_path().unwrap().to_path_buf();
    let sibling = linked_snapshot.with_file_name("launch-snapshot-hard-link");
    fs::hard_link(&linked_snapshot, &sibling).unwrap();
    assert!(
        fs::metadata(&linked_snapshot)
            .unwrap()
            .permissions()
            .readonly()
    );

    drop(linked);

    assert!(linked_snapshot.exists());
    assert!(sibling.exists());
    assert!(
        fs::metadata(&linked_snapshot)
            .unwrap()
            .permissions()
            .readonly()
    );
    let mut cleanup_permissions = fs::metadata(&linked_snapshot).unwrap().permissions();
    cleanup_permissions.set_readonly(false);
    fs::set_permissions(&linked_snapshot, cleanup_permissions).unwrap();
    fs::remove_file(linked_snapshot).unwrap();
    fs::remove_file(sibling).unwrap();
}

#[cfg(unix)]
#[test]
fn an_allocated_launch_snapshot_keeps_store_owned_bytes_mode_and_drop_cleanup() {
    use std::os::unix::fs::PermissionsExt as _;

    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let entry = store.create(copied_shell(b"printf CHECKED")).unwrap();
    let source = store.payload_path(&entry).unwrap();
    fs::set_permissions(&source, fs::Permissions::from_mode(0o640)).unwrap();

    let prepared = store
        .prepare_launch_with_snapshot_allocator(
            &entry,
            Some(&entry.meta.source_hash),
            &FixedLaunchSnapshotAllocator(LaunchSnapshotStem::new(0)),
        )
        .unwrap();
    let snapshot = prepared.payload_path().unwrap().to_path_buf();

    assert_eq!(
        snapshot.file_name().unwrap(),
        ".run-00000000000000000000000000000000.sh"
    );
    assert_eq!(fs::read(&snapshot).unwrap(), b"printf CHECKED");
    assert_eq!(
        fs::metadata(&snapshot).unwrap().permissions().mode() & 0o777,
        0o640
    );
    drop(prepared);
    assert!(!snapshot.exists());
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
