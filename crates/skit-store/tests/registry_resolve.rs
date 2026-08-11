use std::fs;

use skit_application::{
    CreateEntry, EntryMutationRepository, EntryPayload, EntryRepository, RepositoryError,
    SourcePermissions,
};
use skit_domain::{EntryKind, EntrySettings, StorageMode};
use skit_store::FileStore;
use tempfile::TempDir;
use toml::{Table, Value};

fn request(name: &str, description: &str) -> CreateEntry {
    CreateEntry {
        name: name.to_owned(),
        kind: EntryKind::parse("future-kind").unwrap(),
        mode: StorageMode::Copy,
        source: format!("/original/{name}.tool"),
        workdir: "invoke".to_owned(),
        description: description.to_owned(),
        payload: Some(EntryPayload {
            bytes: format!("payload for {name}\n").into_bytes(),
            stored_name: Some(format!("{name}.tool")),
            permissions: SourcePermissions::default(),
        }),
        settings: EntrySettings::default(),
    }
}

fn registry(root: &TempDir) -> Table {
    toml::from_str(
        &fs::read_to_string(root.path().join("registry.toml")).expect("registry should exist"),
    )
    .expect("registry should stay valid TOML")
}

fn write_registry(root: &TempDir, document: &Table) {
    fs::write(
        root.path().join("registry.toml"),
        toml::to_string_pretty(document).unwrap(),
    )
    .unwrap();
}

fn row_mut<'a>(document: &'a mut Table, slug: &str) -> &'a mut Table {
    document
        .get_mut("entries")
        .and_then(Value::as_table_mut)
        .and_then(|entries| entries.get_mut(slug))
        .and_then(Value::as_table_mut)
        .expect("entry row should be a table")
}

fn row<'a>(document: &'a Table, slug: &str) -> &'a Table {
    document
        .get("entries")
        .and_then(Value::as_table)
        .and_then(|entries| entries.get(slug))
        .and_then(Value::as_table)
        .expect("entry row should be a table")
}

fn edit_meta(root: &TempDir, slug: &str, key: &str, value: &str) {
    let path = root.path().join("scripts").join(slug).join("meta.toml");
    let mut document = toml::from_str::<Table>(&fs::read_to_string(&path).unwrap()).unwrap();
    document.insert(key.to_owned(), Value::String(value.to_owned()));
    fs::write(path, toml::to_string_pretty(&document).unwrap()).unwrap();
}

fn force_row_stale(document: &mut Table, slug: &str) {
    row_mut(document, slug).insert("mtime_ns".to_owned(), Value::Integer(0));
}

#[test]
fn a_unique_registry_name_hit_propagates_selected_metadata_corruption() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    store.create(request("Broken", "selected by row")).unwrap();
    fs::write(
        root.path().join("scripts/broken/meta.toml"),
        "name = [broken",
    )
    .unwrap();

    let error = store.resolve("Broken").unwrap_err();

    assert!(matches!(
        error,
        RepositoryError::Corrupt { slug, .. } if slug == "broken"
    ));
}

#[test]
fn a_stale_name_hit_sweeps_truth_and_repairs_the_row() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    store.create(request("Before", "rename by hand")).unwrap();
    edit_meta(&root, "before", "name", "After");
    let mut document = registry(&root);
    force_row_stale(&mut document, "before");
    write_registry(&root, &document);
    assert!(matches!(
        store.resolve("Before").unwrap_err(),
        RepositoryError::NotFound { .. }
    ));

    assert_eq!(
        row(&registry(&root), "before")
            .get("name")
            .and_then(Value::as_str),
        Some("After")
    );
    assert_eq!(store.resolve("After").unwrap().slug.as_str(), "before");
}

#[test]
fn a_fast_name_hit_does_not_sweep_or_repair_unrelated_rows() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    store.create(request("Fast", "selected")).unwrap();
    store.create(request("Other", "old row")).unwrap();
    edit_meta(&root, "other", "description", "after hand edit");
    let mut document = registry(&root);
    force_row_stale(&mut document, "other");
    write_registry(&root, &document);

    assert_eq!(store.resolve("Fast").unwrap().slug.as_str(), "fast");

    let untouched = registry(&root);
    assert_eq!(
        row(&untouched, "other")
            .get("description")
            .and_then(Value::as_str),
        Some("old row")
    );
    assert_eq!(
        row(&untouched, "other")
            .get("mtime_ns")
            .and_then(Value::as_integer),
        Some(0)
    );
}

#[test]
fn hand_edited_duplicate_names_are_ambiguous_and_repair_during_the_sweep() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    store.create(request("Alpha", "first")).unwrap();
    store.create(request("Beta", "second")).unwrap();
    edit_meta(&root, "alpha", "name", "Shared");
    edit_meta(&root, "beta", "name", "Shared");
    let mut document = registry(&root);
    force_row_stale(&mut document, "alpha");
    force_row_stale(&mut document, "beta");
    write_registry(&root, &document);
    assert_eq!(
        store.resolve("Shared").unwrap_err(),
        RepositoryError::Ambiguous {
            query: "Shared".to_owned(),
            candidates: vec!["alpha".to_owned(), "beta".to_owned()],
        }
    );

    let repaired = registry(&root);
    assert_eq!(
        row(&repaired, "alpha").get("name").and_then(Value::as_str),
        Some("Shared")
    );
    assert_eq!(
        row(&repaired, "beta").get("name").and_then(Value::as_str),
        Some("Shared")
    );
}
