//! Membership/name-ownership ports from Python v0.4 `tests/test_store.py`.
//!
//! Registry TOML is deliberately hand-edited in these fixtures. All behavior is observed through
//! the public `FileStore` repository boundary; no Rust private registry helper is called.

use std::fs;

use skit_application::{
    CreateEntry, EntryMutationRepository as _, EntryPayload, EntryRepository as _, RepositoryError,
    SourcePermissions,
};
use skit_domain::{EntryKind, EntrySettings, StorageMode};
use skit_store::FileStore;
use tempfile::TempDir;
use toml::{Table, Value};

fn request(name: &str, kind: &str) -> CreateEntry {
    let stored_name = match kind {
        "python" => Some("script.py"),
        "shell" => Some("script.sh"),
        _ => None,
    };
    CreateEntry {
        name: name.to_owned(),
        kind: EntryKind::parse(kind).unwrap(),
        mode: if kind == "command" { StorageMode::Reference } else { StorageMode::Copy },
        source: String::new(),
        workdir: "invoke".to_owned(),
        description: String::new(),
        payload: stored_name.map(|stored_name| EntryPayload {
            bytes: b"print('x')\n".to_vec(),
            stored_name: Some(stored_name.to_owned()),
            permissions: SourcePermissions::default(),
        }),
        settings: if kind == "command" {
            EntrySettings {
                template: "echo hi".to_owned(),
                ..EntrySettings::default()
            }
        } else {
            EntrySettings::default()
        },
    }
}

fn load_registry(root: &TempDir) -> Table {
    toml::from_str(&fs::read_to_string(root.path().join("registry.toml")).unwrap()).unwrap()
}

fn save_registry(root: &TempDir, document: &Table) {
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
        .expect("registry entries table")
}

#[test]
fn test_an_index_whose_entries_key_is_not_a_table_reads_empty() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    store.create(request("real", "python")).unwrap();
    fs::write(root.path().join("registry.toml"), "entries = 5\n").unwrap();

    assert!(store.scan().unwrap().entries.is_empty());
    assert_eq!(store.rebuild_registry().unwrap(), 1);
    assert_eq!(
        store
            .scan()
            .unwrap()
            .entries
            .iter()
            .map(|summary| summary.name.as_str())
            .collect::<Vec<_>>(),
        ["real"]
    );
}

#[test]
fn test_add_survives_a_hand_broken_row_that_can_claim_no_name() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let good = store.create(request("real", "python")).unwrap();
    let mut registry = load_registry(&root);
    entries_mut(&mut registry).insert("bad".to_owned(), Value::String("garbage".to_owned()));
    entries_mut(&mut registry).insert(
        "numeric".to_owned(),
        Value::Table(Table::from_iter([
            ("name".to_owned(), Value::Integer(7)),
            ("kind".to_owned(), Value::String("python".to_owned())),
            ("description".to_owned(), Value::String(String::new())),
        ])),
    );
    save_registry(&root, &registry);

    let added = store.create(request("newcmd", "command")).unwrap();
    assert_eq!(store.resolve("newcmd").unwrap().slug, added.slug);
    assert_eq!(store.resolve("real").unwrap().slug, good.slug);
    let collision = store.create(request("real", "command")).unwrap_err();
    assert!(matches!(collision, RepositoryError::Conflict { .. }));
}

#[test]
fn test_an_entry_whose_row_was_mangled_still_defends_its_name() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let good = store.create(request("Guarded Name", "python")).unwrap();
    let mut registry = load_registry(&root);
    entries_mut(&mut registry).insert(good.slug.as_str().to_owned(), Value::String("garbage".to_owned()));
    save_registry(&root, &registry);

    let collision = store.create(request("Guarded Name", "command")).unwrap_err();
    assert!(matches!(collision, RepositoryError::Conflict { .. }));
    assert_eq!(store.resolve("Guarded Name").unwrap().slug, good.slug);
}
