//! Exact public-store ports from `tests/test_rename.py` at Python `main@206f9ef`.
//!
//! These tests intentionally pin the Python behavior at the repository/state boundary. A red
//! assertion is a parity finding for the implementation agent; this test-port branch does not
//! change production code to make the oracle pass.

use std::collections::BTreeMap;

use skit_application::{
    CreateEntry, EntryMutationRepository, EntryPayload, EntryRepository, RepositoryError,
    SourcePermissions, form_state::FormStateRepository,
};
use skit_domain::{EntryKind, EntrySettings, StorageMode};
use skit_store::{FileFormStateStore, FileStore};
use tempfile::TempDir;

fn create_python(store: &FileStore, name: &str) -> skit_domain::Entry {
    store
        .create(CreateEntry {
            name: name.to_owned(),
            kind: EntryKind::parse("python").unwrap(),
            mode: StorageMode::Copy,
            source: format!("/original/{name}.py"),
            workdir: "invoke".to_owned(),
            description: String::new(),
            payload: Some(EntryPayload {
                bytes: b"print(1)\n".to_vec(),
                stored_name: Some("script.py".to_owned()),
                permissions: SourcePermissions::default(),
            }),
            settings: EntrySettings::default(),
        })
        .unwrap()
}

#[test]
fn test_rename_changes_name_and_keeps_slug_dir_and_state() {
    let root = TempDir::new().unwrap();
    let data = root.path().join("data");
    let state = root.path().join("state");
    let store = FileStore::new(&data);
    let entry = create_python(&store, "old");
    let entry_dir = data.join("scripts").join(entry.slug.as_str());
    let form_state = FileFormStateStore::new(&state);
    form_state
        .update(&entry.slug, |stored| {
            stored.values = BTreeMap::from([("X".to_owned(), "1".to_owned())]);
        })
        .unwrap();

    let renamed = store.rename(&entry, "new").unwrap();

    assert_eq!(renamed.meta.name, "new");
    assert_eq!(renamed.slug, entry.slug);
    assert!(
        entry_dir.is_dir(),
        "rename moved or removed the immutable slug directory"
    );
    assert!(
        !data.join("scripts/new").exists(),
        "display-name rename incorrectly created a new slug directory"
    );
    assert_eq!(
        form_state.load(&entry.slug).values,
        BTreeMap::from([("X".to_owned(), "1".to_owned())])
    );
}

#[test]
fn test_rename_updates_resolution_and_listing() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let entry = create_python(&store, "Old Name");
    assert_ne!(entry.slug.as_str(), entry.meta.name);

    store.rename(&entry, "new").unwrap();

    assert_eq!(store.resolve("new").unwrap().meta.name, "new");
    assert!(matches!(
        store.resolve("Old Name").unwrap_err(),
        RepositoryError::NotFound { .. }
    ));
    assert_eq!(store.resolve(entry.slug.as_str()).unwrap().meta.name, "new");
    assert_eq!(
        store
            .scan()
            .unwrap()
            .entries
            .into_iter()
            .map(|entry| entry.name)
            .collect::<Vec<_>>(),
        ["new"]
    );
}

#[test]
fn test_rename_conflict_is_a_clean_error() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let alpha = create_python(&store, "alpha");
    let beta = create_python(&store, "beta");

    let error = store.rename(&beta, "alpha").unwrap_err();

    assert!(matches!(
        error,
        RepositoryError::Conflict { ref name, ref slug }
            if name == "alpha" && slug == alpha.slug.as_str()
    ));
    assert_eq!(store.resolve("beta").unwrap().meta.name, "beta");
    assert_eq!(store.resolve("alpha").unwrap().slug, alpha.slug);
}

#[test]
fn test_rename_to_own_name_is_a_no_op() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let entry = create_python(&store, "same");

    let renamed = store.rename(&entry, "same").unwrap();

    assert_eq!(renamed.meta.name, "same");
    assert_eq!(renamed.slug, entry.slug);
    assert_eq!(store.resolve("same").unwrap().slug, entry.slug);
}

#[test]
fn test_rename_empty_name_rejected() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let entry = create_python(&store, "x");

    assert!(matches!(
        store.rename(&entry, "   ").unwrap_err(),
        RepositoryError::InvalidMutation { .. }
    ));
    assert_eq!(store.resolve("x").unwrap().meta.name, "x");
}

#[test]
fn test_rename_survives_doctor_rebuild() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let entry = create_python(&store, "old");
    store.rename(&entry, "new").unwrap();

    let report = store.rebuild_registry_report().unwrap();

    assert_eq!(report.entry_count, 1);
    assert!(report.problems.is_empty(), "{:?}", report.problems);
    assert_eq!(store.resolve("new").unwrap().meta.name, "new");
}
