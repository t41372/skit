//! Behavioral ports from `origin/main@206f9ef:tests/test_store_fix.py`.
//!
//! This is intentionally a test-only oracle. The Rust implementation is not changed in this
//! branch when one of these Python contracts fails.

use std::fs;

use skit_application::{
    CreateEntry, EntryMutationRepository, EntryPayload, EntryRepository, RepositoryError,
    SourcePermissions,
};
use skit_domain::{EntryKind, EntrySettings, StorageMode};
use skit_store::FileStore;
use tempfile::TempDir;
use toml::{Table, Value};

fn request(name: &str, bytes: &[u8]) -> CreateEntry {
    CreateEntry {
        name: name.to_owned(),
        kind: EntryKind::parse("future-kind").unwrap(),
        mode: StorageMode::Copy,
        source: format!("/original/{name}.tool"),
        workdir: "invoke".to_owned(),
        description: String::new(),
        payload: Some(EntryPayload {
            bytes: bytes.to_vec(),
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

fn add_registry_row(root: &TempDir, slug: &str, name: &str) {
    let mut document = if root.path().join("registry.toml").exists() {
        registry(root)
    } else {
        Table::from_iter([("entries".to_owned(), Value::Table(Table::new()))])
    };
    entries_mut(&mut document).insert(
        slug.to_owned(),
        Value::Table(Table::from_iter([
            ("name".to_owned(), Value::String(name.to_owned())),
            ("kind".to_owned(), Value::String("future-kind".to_owned())),
            ("description".to_owned(), Value::String(String::new())),
        ])),
    );
    write_registry(root, &document);
}

#[test]
fn test_list_entries_skips_valid_toml_missing_name_key() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    store.create(request("good", b"good\n")).unwrap();

    let bad = root.path().join("scripts").join("bad-slug");
    fs::create_dir_all(&bad).unwrap();
    fs::write(bad.join("meta.toml"), "schema = 1\nkind = \"future-kind\"\n").unwrap();
    add_registry_row(&root, "bad-slug", "bad");

    let names = store
        .scan()
        .unwrap()
        .entries
        .into_iter()
        .map(|entry| entry.name)
        .collect::<Vec<_>>();
    assert_eq!(names, ["good"]);
}

#[test]
fn test_doctor_rebuild_reports_missing_key_instead_of_crashing() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    store.create(request("good", b"good\n")).unwrap();

    let bad = root.path().join("scripts").join("bad-slug");
    fs::create_dir_all(&bad).unwrap();
    fs::write(bad.join("meta.toml"), "schema = 1\nkind = \"future-kind\"\n").unwrap();

    assert_eq!(store.rebuild_registry().unwrap(), 1);
}

#[test]
fn test_resolve_corrupt_missing_key_meta_raises_not_found_not_decode_panic() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let bad = root.path().join("scripts").join("bad-slug");
    fs::create_dir_all(&bad).unwrap();
    fs::write(bad.join("meta.toml"), "schema = 1\nkind = \"future-kind\"\n").unwrap();
    add_registry_row(&root, "bad-slug", "bad");

    assert!(matches!(
        store.resolve("bad-slug").unwrap_err(),
        RepositoryError::NotFound { .. }
    ));
}

#[test]
fn test_list_entries_skips_scalar_dependencies_meta() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    store.create(request("good", b"good\n")).unwrap();

    let bad = root.path().join("scripts").join("bad-type-slug");
    fs::create_dir_all(&bad).unwrap();
    fs::write(
        bad.join("meta.toml"),
        "schema = 1\nname = \"bad\"\nkind = \"future-kind\"\ndependencies = 5\n",
    )
    .unwrap();
    add_registry_row(&root, "bad-type-slug", "bad");

    let names = store
        .scan()
        .unwrap()
        .entries
        .into_iter()
        .map(|entry| entry.name)
        .collect::<Vec<_>>();
    assert_eq!(names, ["good"]);
}

#[test]
fn test_resolve_scalar_dependencies_meta_raises_not_found_not_type_error() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let bad = root.path().join("scripts").join("bad-type-slug");
    fs::create_dir_all(&bad).unwrap();
    fs::write(
        bad.join("meta.toml"),
        "schema = 1\nname = \"bad\"\nkind = \"future-kind\"\ndependencies = 5\n",
    )
    .unwrap();
    add_registry_row(&root, "bad-type-slug", "bad");

    assert!(matches!(
        store.resolve("bad-type-slug").unwrap_err(),
        RepositoryError::NotFound { .. }
    ));
}

#[test]
fn test_lost_registry_name_collision_does_not_clobber_existing_script() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let entry = store.create(request("Deploy", b"original\n")).unwrap();
    let stored = root
        .path()
        .join("scripts")
        .join(entry.slug.as_str())
        .join("script.tool");
    let original = fs::read(&stored).unwrap();
    fs::remove_file(root.path().join("registry.toml")).unwrap();

    assert!(matches!(
        store.create(request("Deploy", b"different\n")).unwrap_err(),
        RepositoryError::Conflict { .. }
    ));
    assert_eq!(fs::read(stored).unwrap(), original);
}

#[test]
fn test_lost_registry_slug_collision_gets_deduped_not_overwritten() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let first = store.create(request("deploy", b"original\n")).unwrap();
    let stored = root
        .path()
        .join("scripts")
        .join(first.slug.as_str())
        .join("script.tool");
    let original = fs::read(&stored).unwrap();
    fs::remove_file(root.path().join("registry.toml")).unwrap();

    let second = store.create(request("DEPLOY", b"different\n")).unwrap();

    assert_ne!(second.slug, first.slug);
    assert_eq!(fs::read(stored).unwrap(), original);
}

#[test]
fn test_add_entry_still_reuses_preexisting_empty_slug_dir() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    fs::create_dir_all(root.path().join("scripts").join("myname")).unwrap();

    let entry = store.create(request("myname", b"payload\n")).unwrap();

    assert_eq!(entry.slug.as_str(), "myname");
}

#[test]
fn test_fs_truth_ignores_stray_non_directory_entries_in_scripts_dir() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    fs::create_dir_all(root.path().join("scripts")).unwrap();
    fs::write(root.path().join("scripts").join("stray-file.txt"), "not an entry").unwrap();

    let entry = store.create(request("ok", b"payload\n")).unwrap();

    assert_eq!(entry.meta.name, "ok");
}

#[test]
fn test_fs_truth_skips_unreadable_meta_in_unregistered_orphan_directory() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let orphan = root.path().join("scripts").join("orphan");
    fs::create_dir_all(&orphan).unwrap();
    fs::write(orphan.join("meta.toml"), "not valid toml [[[").unwrap();
    fs::write(orphan.join("script.tool"), "orphan\n").unwrap();

    let entry = store.create(request("ok", b"payload\n")).unwrap();

    assert_eq!(entry.meta.name, "ok");
    assert_ne!(entry.slug.as_str(), "orphan");
}
