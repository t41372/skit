use std::{fs, path::Path, time::UNIX_EPOCH};

use skit_application::{
    CreateEntry, EntryMutationRepository, EntryPayload, EntryRepository, LibraryService,
    RepositoryError, SourcePermissions,
};
use skit_domain::{EntryKind, EntrySettings, StorageMode};
use skit_store::FileStore;
use tempfile::TempDir;
use toml::{Table, Value};

fn request(
    name: &str,
    kind: &str,
    mode: StorageMode,
    description: &str,
    stored_name: &str,
    bytes: &[u8],
) -> CreateEntry {
    CreateEntry {
        name: name.to_owned(),
        kind: EntryKind::parse(kind).unwrap(),
        mode,
        source: format!("/original/{stored_name}"),
        workdir: if mode == StorageMode::Reference {
            "origin"
        } else {
            "invoke"
        }
        .to_owned(),
        description: description.to_owned(),
        payload: Some(EntryPayload {
            bytes: bytes.to_vec(),
            stored_name: Some(stored_name.to_owned()),
            permissions: SourcePermissions::default(),
        }),
        settings: EntrySettings::default(),
    }
}

fn registry(root: &TempDir) -> Table {
    toml::from_str(
        &fs::read_to_string(root.path().join("registry.toml")).expect("registry.toml should exist"),
    )
    .expect("registry.toml should be valid TOML")
}

fn entries(document: &Table) -> &Table {
    document
        .get("entries")
        .and_then(Value::as_table)
        .expect("registry should carry an entries table")
}

fn row<'a>(document: &'a Table, slug: &str) -> &'a Table {
    entries(document)
        .get(slug)
        .and_then(Value::as_table)
        .expect("entry row should be a table")
}

fn mtime_ns(path: &Path) -> i64 {
    let nanos = fs::metadata(path)
        .unwrap()
        .modified()
        .unwrap()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    i64::try_from(nanos).unwrap()
}

fn assert_python_row(
    root: &TempDir,
    slug: &str,
    name: &str,
    kind: &str,
    mode: &str,
    description: &str,
    target: Option<&str>,
) {
    let document = registry(root);
    let row = row(&document, slug);
    assert_eq!(row.get("name").and_then(Value::as_str), Some(name));
    assert_eq!(row.get("kind").and_then(Value::as_str), Some(kind));
    assert_eq!(row.get("mode").and_then(Value::as_str), Some(mode));
    assert_eq!(
        row.get("description").and_then(Value::as_str),
        Some(description)
    );
    assert_eq!(
        row.get("mtime_ns").and_then(Value::as_integer),
        Some(mtime_ns(
            &root.path().join("scripts").join(slug).join("meta.toml")
        ))
    );
    match target {
        Some(target) => assert_eq!(row.get("target").and_then(Value::as_str), Some(target)),
        None => assert!(!row.contains_key("target")),
    }
}

fn seed_registry(root: &TempDir) -> Table {
    let document = toml::from_str::<Table>(
        r#"
format_note = "preserve unknown top-level fields"

[entries.external]
name = "External"
kind = "future-kind"
mode = "copy"
description = "owned by another writer"
mtime_ns = 123
custom = "keep this field"
"#,
    )
    .unwrap();
    fs::write(
        root.path().join("registry.toml"),
        toml::to_string_pretty(&document).unwrap(),
    )
    .unwrap();
    document
}

fn poison_registry(root: &TempDir) {
    let path = root.path().join("registry.toml");
    if path.is_file() {
        fs::remove_file(&path).unwrap();
    }
    fs::create_dir(&path).unwrap();
    let backup = root.path().join("registry.toml.corrupt");
    fs::create_dir(&backup).unwrap();
    fs::write(
        backup.join("occupied"),
        b"do not replace a non-empty directory",
    )
    .unwrap();
}

#[test]
fn create_projects_python_compatible_copy_and_reference_rows() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());

    store
        .create(request(
            "Copy",
            "python",
            StorageMode::Copy,
            "copied entry",
            "script.py",
            b"print('copy')\n",
        ))
        .unwrap();
    store
        .create(request(
            "Linked",
            "shell",
            StorageMode::Reference,
            "reference entry",
            "linked.sh",
            b"#!/bin/sh\necho linked\n",
        ))
        .unwrap();

    assert_python_row(
        &root,
        "copy",
        "Copy",
        "python",
        "copy",
        "copied entry",
        None,
    );
    assert_python_row(
        &root,
        "linked",
        "Linked",
        "shell",
        "reference",
        "reference entry",
        Some("/original/linked.sh"),
    );
    assert!(root.path().join("registry.native.lock").is_file());
}

#[test]
fn mutations_refresh_rows_move_keys_and_preserve_unrelated_registry_content() {
    let root = TempDir::new().unwrap();
    let seeded = seed_registry(&root);
    let external = row(&seeded, "external").clone();
    let store = FileStore::new(root.path());
    let entry = store
        .create(request(
            "Alpha",
            "python",
            StorageMode::Copy,
            "before",
            "script.py",
            b"alpha",
        ))
        .unwrap();
    let claimed = store.claim_identity(&entry).unwrap();
    let described = store.describe(&claimed, "after").unwrap();

    assert_python_row(&root, "alpha", "Alpha", "python", "copy", "after", None);
    let renamed = store.rename(&described, "Renamed Tool").unwrap();
    let document = registry(&root);
    assert!(entries(&document).contains_key("alpha"));
    assert_eq!(row(&document, "external"), &external);
    assert_eq!(
        document.get("format_note").and_then(Value::as_str),
        Some("preserve unknown top-level fields")
    );
    assert_python_row(
        &root,
        "alpha",
        "Renamed Tool",
        "python",
        "copy",
        "after",
        None,
    );

    assert_eq!(store.remove(&renamed).unwrap(), "Renamed Tool");
    let document = registry(&root);
    assert!(!entries(&document).contains_key("alpha"));
    assert_eq!(row(&document, "external"), &external);
}

#[test]
fn row_projection_preserves_unknown_fields_on_the_same_entry() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let entry = store
        .create(request(
            "Extensible",
            "python",
            StorageMode::Copy,
            "before",
            "script.py",
            b"print('ok')\n",
        ))
        .unwrap();
    let mut document = registry(&root);
    document
        .get_mut("entries")
        .and_then(Value::as_table_mut)
        .and_then(|entries| entries.get_mut("extensible"))
        .and_then(Value::as_table_mut)
        .unwrap()
        .insert(
            "vendor_projection_note".to_owned(),
            Value::String("preserve me".to_owned()),
        );
    fs::write(
        root.path().join("registry.toml"),
        toml::to_string_pretty(&document).unwrap(),
    )
    .unwrap();

    store.describe(&entry, "after").unwrap();

    assert_eq!(
        row(&registry(&root), "extensible")
            .get("vendor_projection_note")
            .and_then(Value::as_str),
        Some("preserve me")
    );
}

#[test]
fn an_unregistered_entry_stays_hidden_until_an_explicit_rebuild() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    store
        .create(request(
            "Hidden",
            "python",
            StorageMode::Copy,
            "still on disk",
            "script.py",
            b"print('hidden')\n",
        ))
        .unwrap();
    let mut document = registry(&root);
    document
        .get_mut("entries")
        .and_then(Value::as_table_mut)
        .unwrap()
        .remove("hidden");
    fs::write(
        root.path().join("registry.toml"),
        toml::to_string_pretty(&document).unwrap(),
    )
    .unwrap();

    assert!(store.scan().unwrap().entries.is_empty());
    assert!(matches!(
        store.resolve("hidden").unwrap_err(),
        RepositoryError::NotFound { .. }
    ));
    assert!(matches!(
        store.resolve("Hidden").unwrap_err(),
        RepositoryError::NotFound { .. }
    ));

    assert_eq!(store.rebuild_registry().unwrap(), 1);
    assert_eq!(store.scan().unwrap().entries[0].slug.as_str(), "hidden");
}

#[test]
fn a_metadata_mutator_does_not_resurrect_a_missing_registry_row() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let held = store
        .create(request(
            "Subject",
            "python",
            StorageMode::Copy,
            "before",
            "script.py",
            b"print('subject')\n",
        ))
        .unwrap();
    let mut document = registry(&root);
    document
        .get_mut("entries")
        .and_then(Value::as_table_mut)
        .unwrap()
        .remove("subject");
    fs::write(
        root.path().join("registry.toml"),
        toml::to_string_pretty(&document).unwrap(),
    )
    .unwrap();

    let updated = LibraryService::new(store.clone())
        .describe(&held, "after")
        .unwrap();

    assert_eq!(updated.meta.description, "after");
    let document = registry(&root);
    assert!(!entries(&document).contains_key("subject"));
    let metadata = fs::read_to_string(root.path().join("scripts/subject/meta.toml")).unwrap();
    assert!(metadata.contains("description = \"after\""));
}

#[test]
fn legacy_identity_claim_reprojects_the_new_metadata_stamp() {
    let root = TempDir::new().unwrap();
    let directory = root.path().join("scripts/legacy");
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join("meta.toml"),
        r#"name = "Legacy"
kind = "python"
mode = "copy"
source = "/old.py"
source_hash = "sha256:old"
description = "legacy"
"#,
    )
    .unwrap();
    fs::write(directory.join("script.py"), b"legacy").unwrap();
    fs::write(
        root.path().join("registry.toml"),
        format!(
            r#"[entries.legacy]
name = "Legacy"
kind = "python"
mode = "copy"
description = "legacy"
mtime_ns = {}
"#,
            mtime_ns(&directory.join("meta.toml"))
        ),
    )
    .unwrap();

    let store = FileStore::new(root.path());
    let held = store.resolve("legacy").unwrap();
    let claimed = store.claim_identity(&held).unwrap();

    assert!(claimed.meta.id.is_some());
    assert_python_row(&root, "legacy", "Legacy", "python", "copy", "legacy", None);
}

#[test]
fn stale_registry_rows_reserve_names_and_slugs_like_the_python_writer() {
    let root = TempDir::new().unwrap();
    fs::write(
        root.path().join("registry.toml"),
        r#"[entries.reserved]
name = "Reserved Name"
kind = "command"
mode = "reference"
description = "stale but still reserved"
mtime_ns = 1
target = ""
"#,
    )
    .unwrap();
    let store = FileStore::new(root.path());

    assert!(
        store
            .create(request(
                "Reserved Name",
                "python",
                StorageMode::Copy,
                "conflict",
                "script.py",
                b"conflict",
            ))
            .is_err()
    );
    let suffixed = store
        .create(request(
            "Reserved!",
            "python",
            StorageMode::Copy,
            "slug collision",
            "script.py",
            b"suffixed",
        ))
        .unwrap();
    assert_eq!(suffixed.slug.as_str(), "reserved-2");
}

#[test]
fn corrupt_registry_is_backed_up_before_a_fresh_projection_is_written() {
    let root = TempDir::new().unwrap();
    let corrupt = b"entries = [this is not valid TOML";
    fs::write(root.path().join("registry.toml"), corrupt).unwrap();
    let store = FileStore::new(root.path());

    store
        .create(request(
            "Recovered",
            "python",
            StorageMode::Copy,
            "after corruption",
            "script.py",
            b"recovered",
        ))
        .unwrap();

    assert_eq!(
        fs::read(root.path().join("registry.toml.corrupt")).unwrap(),
        corrupt
    );
    assert_python_row(
        &root,
        "recovered",
        "Recovered",
        "python",
        "copy",
        "after corruption",
        None,
    );
}

#[test]
fn registry_write_failures_roll_back_every_entry_mutation() {
    let create_root = TempDir::new().unwrap();
    poison_registry(&create_root);
    let create_store = FileStore::new(create_root.path());
    assert!(
        create_store
            .create(request(
                "Create",
                "python",
                StorageMode::Copy,
                "never committed",
                "script.py",
                b"create",
            ))
            .is_err()
    );
    assert!(!create_root.path().join("scripts/create").exists());

    let describe_root = TempDir::new().unwrap();
    let describe_store = FileStore::new(describe_root.path());
    let described = describe_store
        .create(request(
            "Describe",
            "python",
            StorageMode::Copy,
            "before",
            "script.py",
            b"describe",
        ))
        .unwrap();
    poison_registry(&describe_root);
    assert!(describe_store.describe(&described, "after").is_err());
    assert_eq!(
        toml::from_str::<Table>(
            &fs::read_to_string(describe_root.path().join("scripts/describe/meta.toml")).unwrap()
        )
        .unwrap()["description"]
            .as_str(),
        Some("before")
    );

    let rename_root = TempDir::new().unwrap();
    let rename_store = FileStore::new(rename_root.path());
    let renamed = rename_store
        .create(request(
            "Rename",
            "python",
            StorageMode::Copy,
            "before",
            "script.py",
            b"rename",
        ))
        .unwrap();
    poison_registry(&rename_root);
    assert!(rename_store.rename(&renamed, "After Rename").is_err());
    assert!(rename_root.path().join("scripts/rename").is_dir());
    assert!(!rename_root.path().join("scripts/after-rename").exists());
    assert_eq!(
        toml::from_str::<Table>(
            &fs::read_to_string(rename_root.path().join("scripts/rename/meta.toml")).unwrap()
        )
        .unwrap()["name"]
            .as_str(),
        Some("Rename")
    );

    let remove_root = TempDir::new().unwrap();
    let remove_store = FileStore::new(remove_root.path());
    let removed = remove_store
        .create(request(
            "Remove",
            "python",
            StorageMode::Copy,
            "before",
            "script.py",
            b"remove",
        ))
        .unwrap();
    poison_registry(&remove_root);
    assert!(remove_store.remove(&removed).is_err());
    assert!(
        remove_root
            .path()
            .join("scripts/remove/meta.toml")
            .is_file()
    );

    let edit_root = TempDir::new().unwrap();
    let edit_store = FileStore::new(edit_root.path());
    let edited = edit_store
        .create(request(
            "Edit",
            "python",
            StorageMode::Copy,
            "before",
            "script.py",
            b"base",
        ))
        .unwrap();
    poison_registry(&edit_root);
    assert!(
        edit_store
            .commit_copy_edit(&edited, b"next", &edited.meta.source_hash)
            .is_err()
    );
    assert_eq!(
        fs::read(edit_root.path().join("scripts/edit/script.py")).unwrap(),
        b"base"
    );
    assert_eq!(
        toml::from_str::<Table>(
            &fs::read_to_string(edit_root.path().join("scripts/edit/meta.toml")).unwrap()
        )
        .unwrap()["source_hash"]
            .as_str(),
        Some(edited.meta.source_hash.as_str())
    );
}
