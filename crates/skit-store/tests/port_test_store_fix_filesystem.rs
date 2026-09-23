//! Exact filesystem and registry-recovery ports from Python `tests/test_store_fix.py`.

use std::fs;

use skit_application::{
    CreateEntry, EntryMutationRepository as _, EntryPayload, EntryRepository as _,
    SourcePermissions,
};
use skit_domain::{EntryKind, EntrySettings, StorageMode};
use skit_store::FileStore;
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
            permissions: SourcePermissions::default(),
        }),
        settings: EntrySettings::default(),
    }
}

#[test]
fn test_lost_registry_slug_collision_gets_deduped_not_overwritten() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let first = store.create(request("deploy", b"original\n")).unwrap();
    let first_dir = root.path().join("scripts").join(first.slug.as_str());
    let first_source = first_dir.join("script.py");
    let first_meta = first_dir.join("meta.toml");
    let source_before = fs::read(&first_source).unwrap();
    let meta_before = fs::read(&first_meta).unwrap();
    fs::remove_file(root.path().join("registry.toml")).unwrap();

    let second = store.create(request("DEPLOY", b"different\n")).unwrap();

    assert_ne!(second.slug, first.slug);
    assert_eq!(second.slug.as_str(), "deploy-2");
    assert_eq!(fs::read(first_source).unwrap(), source_before);
    assert_eq!(fs::read(first_meta).unwrap(), meta_before);
    assert_eq!(
        fs::read(
            root.path()
                .join("scripts")
                .join(second.slug.as_str())
                .join("script.py")
        )
        .unwrap(),
        b"different\n"
    );
}

#[test]
fn test_fs_truth_skips_unreadable_meta_in_unregistered_orphan_directory() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let good = store.create(request("Good", b"good\n")).unwrap();
    let good_dir = root.path().join("scripts").join(good.slug.as_str());
    let good_meta = good_dir.join("meta.toml");
    let good_source = good_dir.join("script.py");
    let good_meta_before = fs::read(&good_meta).unwrap();
    let good_source_before = fs::read(&good_source).unwrap();
    let orphan = root.path().join("scripts/orphan");
    fs::create_dir(&orphan).unwrap();
    let orphan_meta = b"not valid toml [[[";
    let orphan_source = b"print('orphan')\n";
    fs::write(orphan.join("meta.toml"), orphan_meta).unwrap();
    fs::write(orphan.join("script.py"), orphan_source).unwrap();

    let added = store.create(request("Orphan", b"print('new')\n")).unwrap();

    assert_eq!(added.slug.as_str(), "orphan-2");
    let scan = store.scan().unwrap();
    assert_eq!(
        scan.entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        ["Good", "Orphan"]
    );
    assert!(scan.diagnostics.is_empty());
    assert_eq!(fs::read(good_meta).unwrap(), good_meta_before);
    assert_eq!(fs::read(good_source).unwrap(), good_source_before);
    assert_eq!(fs::read(orphan.join("meta.toml")).unwrap(), orphan_meta);
    assert_eq!(fs::read(orphan.join("script.py")).unwrap(), orphan_source);
    assert_eq!(
        fs::read(root.path().join("scripts/orphan-2/script.py")).unwrap(),
        b"print('new')\n"
    );
}

#[test]
fn test_corrupt_registry_is_backed_up_and_degrades_to_empty() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let entry = store.create(request("Kept", b"keep me\n")).unwrap();
    let entry_dir = root.path().join("scripts").join(entry.slug.as_str());
    let meta = entry_dir.join("meta.toml");
    let source = entry_dir.join("script.py");
    let meta_before = fs::read(&meta).unwrap();
    let source_before = fs::read(&source).unwrap();
    let registry = root.path().join("registry.toml");
    let corrupt = b"not valid toml [[[";
    fs::write(&registry, corrupt).unwrap();

    let first = store.scan().unwrap();

    assert!(first.entries.is_empty());
    assert!(first.diagnostics.is_empty());
    let backup = root.path().join("registry.toml.corrupt");
    assert_eq!(fs::read(&backup).unwrap(), corrupt);
    assert!(!registry.exists());
    assert_eq!(fs::read(&meta).unwrap(), meta_before);
    assert_eq!(fs::read(&source).unwrap(), source_before);

    let second = store.scan().unwrap();
    assert!(second.entries.is_empty());
    assert!(second.diagnostics.is_empty());
    assert_eq!(fs::read(backup).unwrap(), corrupt);
    assert!(!registry.exists());
    assert_eq!(fs::read(meta).unwrap(), meta_before);
    assert_eq!(fs::read(source).unwrap(), source_before);
}
