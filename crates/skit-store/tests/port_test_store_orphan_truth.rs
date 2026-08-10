//! Filesystem-truth ownership oracles for entries that fell out of `registry.toml`.
//!
//! These are public-API consequences of Python v0.4's `_fs_truth` defense. The registry is only an
//! index: losing one row must not make the corresponding live entry's name or slug available for
//! overwrite. Production code is intentionally untouched if these tests fail.

use std::fs;

use skit_application::{
    CreateEntry, EntryMutationRepository, EntryPayload, RepositoryError, SourcePermissions,
};
use skit_domain::{EntryKind, EntrySettings, StorageMode};
use skit_store::FileStore;
use tempfile::TempDir;
use toml::{Table, Value};

fn request(name: &str, body: &str) -> CreateEntry {
    CreateEntry {
        name: name.to_owned(),
        kind: EntryKind::parse("future-kind").unwrap(),
        mode: StorageMode::Copy,
        source: format!("/original/{name}.tool"),
        workdir: "invoke".to_owned(),
        description: String::new(),
        payload: Some(EntryPayload {
            bytes: body.as_bytes().to_vec(),
            stored_name: Some("script.tool".to_owned()),
            permissions: SourcePermissions::default(),
        }),
        settings: EntrySettings::default(),
    }
}

fn drop_registry_row(root: &TempDir, slug: &str) {
    let path = root.path().join("registry.toml");
    let mut document = toml::from_str::<Table>(&fs::read_to_string(&path).unwrap()).unwrap();
    document
        .get_mut("entries")
        .and_then(Value::as_table_mut)
        .unwrap()
        .remove(slug);
    fs::write(path, toml::to_string_pretty(&document).unwrap()).unwrap();
}

#[test]
fn test_a_live_entry_missing_only_its_registry_row_still_defends_its_display_name() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let hidden = store.create(request("Guarded Name", "original\n")).unwrap();
    store.create(request("visible", "visible\n")).unwrap();
    let payload = root
        .path()
        .join("scripts")
        .join(hidden.slug.as_str())
        .join("script.tool");
    let original = fs::read(&payload).unwrap();
    drop_registry_row(&root, hidden.slug.as_str());

    assert!(matches!(
        store
            .create(request("Guarded Name", "replacement\n"))
            .unwrap_err(),
        RepositoryError::Conflict { .. }
    ));
    assert_eq!(fs::read(payload).unwrap(), original);
}

#[test]
fn test_a_live_entry_missing_only_its_registry_row_still_defends_its_slug() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let hidden = store.create(request("deploy", "original\n")).unwrap();
    store.create(request("visible", "visible\n")).unwrap();
    let payload = root
        .path()
        .join("scripts")
        .join(hidden.slug.as_str())
        .join("script.tool");
    let original = fs::read(&payload).unwrap();
    drop_registry_row(&root, hidden.slug.as_str());

    let new_entry = store.create(request("DEPLOY", "replacement\n")).unwrap();

    assert_ne!(new_entry.slug, hidden.slug);
    assert_eq!(fs::read(payload).unwrap(), original);
}

#[test]
fn test_a_nonempty_unregistered_corrupt_directory_protects_its_slug_from_reuse() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let orphan = root.path().join("scripts").join("orphan");
    fs::create_dir_all(&orphan).unwrap();
    fs::write(orphan.join("meta.toml"), "not valid toml [[[\n").unwrap();
    fs::write(orphan.join("script.tool"), "do not clobber\n").unwrap();
    let original_meta = fs::read(orphan.join("meta.toml")).unwrap();
    let original_payload = fs::read(orphan.join("script.tool")).unwrap();

    let entry = store.create(request("orphan", "new payload\n")).unwrap();

    assert_ne!(entry.slug.as_str(), "orphan");
    assert_eq!(fs::read(orphan.join("meta.toml")).unwrap(), original_meta);
    assert_eq!(
        fs::read(orphan.join("script.tool")).unwrap(),
        original_payload
    );
}
