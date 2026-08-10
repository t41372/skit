//! Public-surface metadata-shape ports from Python v0.4 corruption tests.
//!
//! The test file mutates authoritative `meta.toml` after creating a valid entry, then uses only
//! public `FileStore` reads/rebuilds. Red assertions are parity findings; production parsing is not
//! changed in this branch.

use std::fs;

use skit_application::{
    CreateEntry, EntryMutationRepository, EntryPayload, EntryRepository, RepositoryError,
    SourcePermissions,
};
use skit_domain::{EntryKind, EntrySettings, StorageMode};
use skit_store::FileStore;
use tempfile::TempDir;
use toml::{Table, Value};

fn request(name: &str, kind: &str) -> CreateEntry {
    CreateEntry {
        name: name.to_owned(),
        kind: EntryKind::parse(kind).unwrap(),
        mode: StorageMode::Copy,
        source: format!("/original/{name}.tool"),
        workdir: "invoke".to_owned(),
        description: String::new(),
        payload: Some(EntryPayload {
            bytes: b"payload\n".to_vec(),
            stored_name: Some("script.tool".to_owned()),
            permissions: SourcePermissions::default(),
        }),
        settings: EntrySettings::default(),
    }
}

fn meta_path(root: &TempDir, slug: &str) -> std::path::PathBuf {
    root.path().join("scripts").join(slug).join("meta.toml")
}

fn mutate_meta(root: &TempDir, slug: &str, edit: impl FnOnce(&mut Table)) {
    let path = meta_path(root, slug);
    let mut document = toml::from_str::<Table>(&fs::read_to_string(&path).unwrap()).unwrap();
    edit(&mut document);
    fs::write(path, toml::to_string_pretty(&document).unwrap()).unwrap();
}

fn force_registry_fallback(root: &TempDir, slug: &str) {
    let path = root.path().join("registry.toml");
    let mut document = toml::from_str::<Table>(&fs::read_to_string(&path).unwrap()).unwrap();
    document
        .get_mut("entries")
        .and_then(Value::as_table_mut)
        .and_then(|entries| entries.get_mut(slug))
        .and_then(Value::as_table_mut)
        .unwrap()
        .remove("mtime_ns");
    fs::write(path, toml::to_string_pretty(&document).unwrap()).unwrap();
}

fn listed_names(store: &FileStore) -> Vec<String> {
    store
        .scan()
        .unwrap()
        .entries
        .into_iter()
        .map(|entry| entry.name)
        .collect()
}

#[test]
fn test_scalar_params_meta_is_corruption_not_an_empty_parameter_list() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let bad = store.create(request("bad-params", "command")).unwrap();
    store.create(request("good", "command")).unwrap();
    mutate_meta(&root, bad.slug.as_str(), |meta| {
        meta.insert("params".to_owned(), Value::Integer(5));
    });
    force_registry_fallback(&root, bad.slug.as_str());

    assert_eq!(listed_names(&store), ["good"]);
    assert_eq!(store.rebuild_registry().unwrap(), 1);
    assert!(matches!(
        store.resolve(bad.slug.as_str()).unwrap_err(),
        RepositoryError::NotFound { .. }
    ));
}

#[test]
fn test_prompt_runner_wrong_type_is_metadata_corruption() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let bad = store.create(request("bad-runner", "prompt")).unwrap();
    store.create(request("good", "prompt")).unwrap();
    mutate_meta(&root, bad.slug.as_str(), |meta| {
        meta.insert("runner".to_owned(), Value::Integer(123));
    });
    force_registry_fallback(&root, bad.slug.as_str());

    assert_eq!(listed_names(&store), ["good"]);
    assert_eq!(store.rebuild_registry().unwrap(), 1);
    assert!(matches!(
        store.resolve(bad.slug.as_str()).unwrap_err(),
        RepositoryError::NotFound { .. }
    ));
}

#[test]
fn test_missing_kind_in_valid_toml_is_skipped_not_a_raw_decode_failure() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let bad = store.create(request("missing-kind", "command")).unwrap();
    store.create(request("good", "command")).unwrap();
    mutate_meta(&root, bad.slug.as_str(), |meta| {
        meta.remove("kind");
    });
    force_registry_fallback(&root, bad.slug.as_str());

    assert_eq!(listed_names(&store), ["good"]);
    assert_eq!(store.rebuild_registry().unwrap(), 1);
    assert!(matches!(
        store.resolve(bad.slug.as_str()).unwrap_err(),
        RepositoryError::NotFound { .. }
    ));
}

#[test]
fn test_missing_name_in_valid_toml_is_skipped_not_a_raw_decode_failure() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let bad = store.create(request("missing-name", "command")).unwrap();
    store.create(request("good", "command")).unwrap();
    mutate_meta(&root, bad.slug.as_str(), |meta| {
        meta.remove("name");
    });
    force_registry_fallback(&root, bad.slug.as_str());

    assert_eq!(listed_names(&store), ["good"]);
    assert_eq!(store.rebuild_registry().unwrap(), 1);
    assert!(matches!(
        store.resolve(bad.slug.as_str()).unwrap_err(),
        RepositoryError::NotFound { .. }
    ));
}

#[test]
fn test_non_bool_interpolate_keeps_the_default_enabled_semantics() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let prompt = store.create(request("prompt", "prompt")).unwrap();
    mutate_meta(&root, prompt.slug.as_str(), |meta| {
        meta.insert("interpolate".to_owned(), Value::String("no".to_owned()));
    });

    let resolved = store.resolve(prompt.slug.as_str()).unwrap();
    assert!(EntrySettings::from_meta(&resolved.meta).interpolate);
}
