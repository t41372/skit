//! Public-API behavioral ports of rename/remove ownership contracts from
//! `origin/main@206f9ef:tests/test_store_mut.py`.
//!
//! The Python suite remains the oracle. This file does not justify production changes in the
//! test-port branch when a Rust behavior differs.

use std::{
    fs,
    sync::{Arc, Barrier},
    thread,
};

use skit_application::{
    CreateEntry, EntryMutationRepository, EntryPayload, EntryRepository, RepositoryError,
    SourcePermissions,
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

fn registry(root: &TempDir) -> Table {
    toml::from_str(&fs::read_to_string(root.path().join("registry.toml")).unwrap()).unwrap()
}

#[test]
fn test_rename_to_taken_name_is_refused() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    store.create(request("alpha", "a\n")).unwrap();
    let beta = store.create(request("beta", "b\n")).unwrap();

    assert!(matches!(
        store.rename(&beta, "alpha").unwrap_err(),
        RepositoryError::Conflict { .. }
    ));
}

#[test]
fn test_rename_to_another_entrys_slug_string_is_taken() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let alpha = store.create(request("Alpha Name", "a\n")).unwrap();
    let beta = store.create(request("beta", "b\n")).unwrap();
    assert_ne!(alpha.slug.as_str(), alpha.meta.name);

    assert!(matches!(
        store.rename(&beta, alpha.slug.as_str()).unwrap_err(),
        RepositoryError::Conflict { .. }
    ));
}

#[test]
fn test_rename_to_its_own_slug_string_is_allowed() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let entry = store.create(request("Some Name", "body\n")).unwrap();
    assert_ne!(entry.slug.as_str(), entry.meta.name);

    let renamed = store.rename(&entry, entry.slug.as_str()).unwrap();

    assert_eq!(renamed.slug, entry.slug);
    assert_eq!(renamed.meta.name, entry.slug.as_str());
    assert_eq!(
        store.resolve(entry.slug.as_str()).unwrap().meta.name,
        entry.slug.as_str()
    );
}

#[test]
fn test_rename_updates_meta_and_registry_while_preserving_slug_directory() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let entry = store.create(request("before", "body\n")).unwrap();
    let slug = entry.slug.clone();
    let entry_dir = root.path().join("scripts").join(slug.as_str());

    let renamed = store.rename(&entry, "after").unwrap();

    assert_eq!(renamed.slug, slug);
    assert_eq!(renamed.meta.name, "after");
    assert!(entry_dir.is_dir());
    let meta = toml::from_str::<Table>(&fs::read_to_string(entry_dir.join("meta.toml")).unwrap())
        .unwrap();
    assert_eq!(meta.get("name").and_then(Value::as_str), Some("after"));
    assert_eq!(
        registry(&root)
            .get("entries")
            .and_then(Value::as_table)
            .and_then(|entries| entries.get(slug.as_str()))
            .and_then(Value::as_table)
            .and_then(|row| row.get("name"))
            .and_then(Value::as_str),
        Some("after")
    );
}

#[test]
fn test_rename_race_exactly_one_of_two_concurrent_claims_wins() {
    let root = TempDir::new().unwrap();
    let store = Arc::new(FileStore::new(root.path()));
    let alpha = store.create(request("aaa", "a\n")).unwrap();
    let beta = store.create(request("bbb", "b\n")).unwrap();
    let barrier = Arc::new(Barrier::new(2));

    let handles = [alpha, beta]
        .into_iter()
        .map(|entry| {
            let store = Arc::clone(&store);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                store.rename(&entry, "shared-name")
            })
        })
        .collect::<Vec<_>>();

    let results = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, Err(RepositoryError::Conflict { .. })))
            .count(),
        1
    );
    assert_eq!(
        store
            .scan()
            .unwrap()
            .entries
            .iter()
            .filter(|entry| entry.name == "shared-name")
            .count(),
        1
    );
}

#[test]
fn test_registered_entry_name_for_create_comes_from_registry_not_drifted_meta() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let alpha = store.create(request("alpha", "a\n")).unwrap();
    let meta_path = root
        .path()
        .join("scripts")
        .join(alpha.slug.as_str())
        .join("meta.toml");
    let mut meta = toml::from_str::<Table>(&fs::read_to_string(&meta_path).unwrap()).unwrap();
    meta.insert("name".to_owned(), Value::String("beta".to_owned()));
    fs::write(&meta_path, toml::to_string_pretty(&meta).unwrap()).unwrap();

    let beta = store.create(request("beta", "b\n")).unwrap();

    assert_eq!(beta.meta.name, "beta");
    assert_ne!(beta.slug, alpha.slug);
}

#[test]
fn test_remove_real_delete_removes_directory_and_returns_name() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let entry = store.create(request("gone", "body\n")).unwrap();
    let entry_dir = root.path().join("scripts").join(entry.slug.as_str());

    let name = store.remove(&entry).unwrap();

    assert_eq!(name, "gone");
    assert!(!entry_dir.exists());
    assert!(matches!(
        store.resolve(entry.slug.as_str()).unwrap_err(),
        RepositoryError::NotFound { .. }
    ));
}
