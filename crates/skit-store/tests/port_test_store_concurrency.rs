//! Public-API behavioral ports of registry locking contracts from
//! `origin/main@206f9ef:tests/test_store_fix.py`.

use std::{collections::BTreeSet, fs, sync::Arc, thread};

use skit_application::{
    CreateEntry, EntryMutationRepository, EntryPayload, EntryRepository, SourcePermissions,
};
use skit_domain::{EntryKind, EntrySettings, StorageMode};
use skit_store::FileStore;
use tempfile::TempDir;

fn request(index: usize) -> CreateEntry {
    let name = format!("script-{index}");
    CreateEntry {
        name: name.clone(),
        kind: EntryKind::parse("future-kind").unwrap(),
        mode: StorageMode::Copy,
        source: format!("/original/{name}.tool"),
        workdir: "invoke".to_owned(),
        description: String::new(),
        payload: Some(EntryPayload {
            bytes: format!("payload {index}\n").into_bytes(),
            stored_name: Some("script.tool".to_owned()),
            permissions: SourcePermissions::default(),
        }),
        settings: EntrySettings::default(),
    }
}

#[test]
fn test_concurrent_add_python_both_succeed_with_distinct_slugs() {
    let root = TempDir::new().unwrap();
    let store = Arc::new(FileStore::new(root.path()));

    let handles = (0..8)
        .map(|index| {
            let store = Arc::clone(&store);
            thread::spawn(move || store.create(request(index)))
        })
        .collect::<Vec<_>>();

    let entries = handles
        .into_iter()
        .map(|handle| handle.join().unwrap().unwrap())
        .collect::<Vec<_>>();

    assert_eq!(entries.len(), 8);
    assert_eq!(
        entries
            .iter()
            .map(|entry| entry.slug.as_str())
            .collect::<BTreeSet<_>>()
            .len(),
        8
    );
    let scan = store.scan().unwrap();
    assert_eq!(scan.entries.len(), 8);
    assert_eq!(
        scan.entries
            .iter()
            .map(|entry| entry.slug.as_str())
            .collect::<BTreeSet<_>>()
            .len(),
        8
    );
}

#[test]
fn test_registry_lock_uses_a_versioned_persistent_native_inode() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let native = root.path().join("registry.native.lock");
    let old_protocol = root.path().join("registry.lock");

    store.create(request(1)).unwrap();
    assert!(native.is_file());
    assert!(!old_protocol.exists());

    #[cfg(unix)]
    let first_identity = {
        use std::os::unix::fs::MetadataExt as _;
        fs::metadata(&native).unwrap().ino()
    };

    // Stable Rust does not expose the Windows file index without an experimental API. Instead pin
    // an opaque payload into the persistent lock file. A path-unlink/recreate lease protocol loses
    // these bytes, while a kernel lock on the same persistent file leaves them untouched.
    #[cfg(windows)]
    let sentinel = b"skit-native-lock-persistent-sentinel-20260812";
    #[cfg(windows)]
    fs::write(&native, sentinel).unwrap();

    store.create(request(2)).unwrap();
    assert!(native.is_file());
    assert!(!old_protocol.exists());

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        assert_eq!(
            fs::metadata(&native).unwrap().ino(),
            first_identity,
            "the native registry lock was deleted/recreated between holders"
        );
    }
    #[cfg(windows)]
    assert_eq!(
        fs::read(&native).unwrap(),
        sentinel,
        "the native registry lock path was replaced instead of preserving the persistent lock file"
    );
}
