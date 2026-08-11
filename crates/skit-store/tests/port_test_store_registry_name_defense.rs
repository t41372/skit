//! Behavioral ports of registry name-ownership defenses from
//! `origin/main@206f9ef:tests/test_store.py`.
//!
//! The tests intentionally use only public repository APIs. A failure is a Python/Rust parity
//! finding; this branch does not patch the store implementation to make the oracle green.

use std::fs;

use skit_application::{
    CreateEntry, EntryMutationRepository, EntryPayload, EntryRepository, RepositoryError,
    SourcePermissions,
};
use skit_domain::{EntryKind, EntrySettings, StorageMode};
use skit_store::FileStore;
use tempfile::TempDir;
use toml::{Table, Value};

fn request(name: &str) -> CreateEntry {
    CreateEntry {
        name: name.to_owned(),
        kind: EntryKind::parse("future-kind").unwrap(),
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

fn registry(root: &TempDir) -> Table {
    toml::from_str(&fs::read_to_string(root.path().join("registry.toml")).unwrap()).unwrap()
}

fn write_registry(root: &TempDir, document: &Table) {
    fs::write(
        root.path().join("registry.toml"),
        toml::to_string_pretty(document).unwrap(),
    )
    .unwrap();
}

fn entries_mut(document: &mut Table) -> &mut Table {
    document
        .get_mut("entries")
        .and_then(Value::as_table_mut)
        .unwrap()
}

#[test]
fn test_add_survives_a_hand_broken_row_that_can_claim_no_name() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let good = store.create(request("real")).unwrap();

    let mut document = registry(&root);
    entries_mut(&mut document).insert("bad".to_owned(), Value::String("garbage".to_owned()));
    entries_mut(&mut document).insert(
        "numeric".to_owned(),
        Value::Table(Table::from_iter([
            ("name".to_owned(), Value::Integer(7)),
            ("kind".to_owned(), Value::String("future-kind".to_owned())),
            ("description".to_owned(), Value::String(String::new())),
        ])),
    );
    write_registry(&root, &document);

    let added = store.create(request("newcmd")).unwrap();

    assert_eq!(store.resolve("newcmd").unwrap().slug, added.slug);
    assert_eq!(store.resolve("real").unwrap().slug, good.slug);
    assert!(matches!(
        store.create(request("real")).unwrap_err(),
        RepositoryError::Conflict { .. }
    ));
}

#[test]
fn test_an_entry_whose_row_was_mangled_still_defends_its_name() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let good = store.create(request("Guarded Name")).unwrap();

    let mut document = registry(&root);
    entries_mut(&mut document).insert(
        good.slug.as_str().to_owned(),
        Value::String("garbage".to_owned()),
    );
    write_registry(&root, &document);

    assert!(matches!(
        store.create(request("Guarded Name")).unwrap_err(),
        RepositoryError::Conflict { .. }
    ));
}
