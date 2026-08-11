//! Public-API storage consequence ports from Python v0.4 store tests.
//!
//! These cases pin only externally observable create behavior. Red assertions remain parity
//! findings; this branch does not modify `FileStore` production code.

use std::fs;

use sha2::{Digest as _, Sha256};
use skit_application::{CreateEntry, EntryMutationRepository, EntryPayload, SourcePermissions};
use skit_domain::{EntryKind, EntrySettings, StorageMode};
use skit_store::FileStore;
use tempfile::TempDir;
use toml::{Table, Value};

fn hash(bytes: &[u8]) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
}

fn request(name: &str, mode: StorageMode, bytes: &[u8]) -> CreateEntry {
    CreateEntry {
        name: name.to_owned(),
        kind: EntryKind::parse("future-kind").unwrap(),
        mode,
        source: format!("/original/{name}.tool"),
        workdir: if mode == StorageMode::Reference {
            "origin"
        } else {
            "invoke"
        }
        .to_owned(),
        description: "description".to_owned(),
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

fn row<'a>(document: &'a Table, slug: &str) -> &'a Table {
    document
        .get("entries")
        .and_then(Value::as_table)
        .and_then(|entries| entries.get(slug))
        .and_then(Value::as_table)
        .unwrap()
}

#[test]
fn test_copy_create_hashes_the_original_bytes_and_stores_them_byte_exact() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let bytes = b"\x00hello\r\n\xfftail\n";

    let entry = store
        .create(request("copy-bytes", StorageMode::Copy, bytes))
        .unwrap();

    assert_eq!(entry.meta.source_hash, hash(bytes));
    assert_eq!(
        fs::read(
            root.path()
                .join("scripts")
                .join(entry.slug.as_str())
                .join("script.tool"),
        )
        .unwrap(),
        bytes
    );
}

#[test]
fn test_reference_create_hashes_the_snapshot_but_never_stores_a_copy() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let bytes = b"reference source bytes\n";

    let entry = store
        .create(request("linked", StorageMode::Reference, bytes))
        .unwrap();

    assert_eq!(entry.meta.source_hash, hash(bytes));
    let directory = root.path().join("scripts").join(entry.slug.as_str());
    assert!(directory.join("meta.toml").is_file());
    assert!(!directory.join("script.tool").exists());
    assert_eq!(fs::read_dir(directory).unwrap().count(), 1);
}

#[test]
fn test_create_records_a_parseable_utc_rfc3339_timestamp_without_fractional_seconds() {
    use time::{OffsetDateTime, format_description::well_known::Rfc3339};

    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let entry = store
        .create(request("timestamp", StorageMode::Copy, b"body\n"))
        .unwrap();

    let parsed = OffsetDateTime::parse(&entry.meta.added_at, &Rfc3339).unwrap();
    assert_eq!(parsed.offset().whole_seconds(), 0);
    assert!(!entry.meta.added_at.contains('.'));
}

#[test]
fn test_copy_registry_row_is_a_listing_projection_not_full_metadata() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let entry = store
        .create(request("projected", StorageMode::Copy, b"body\n"))
        .unwrap();
    let document = registry(&root);
    let row = row(&document, entry.slug.as_str());

    assert_eq!(row.get("name").and_then(Value::as_str), Some("projected"));
    assert_eq!(row.get("kind").and_then(Value::as_str), Some("future-kind"));
    assert_eq!(row.get("mode").and_then(Value::as_str), Some("copy"));
    assert_eq!(
        row.get("description").and_then(Value::as_str),
        Some("description")
    );
    assert!(row.get("mtime_ns").and_then(Value::as_integer).is_some());
    for forbidden in [
        "source",
        "source_hash",
        "added_at",
        "id",
        "workdir",
        "target",
    ] {
        assert!(
            !row.contains_key(forbidden),
            "unexpected projection field: {forbidden}"
        );
    }
}

#[test]
fn test_reference_registry_row_carries_only_the_launch_target_extra_field() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let entry = store
        .create(request("linked-row", StorageMode::Reference, b"body\n"))
        .unwrap();
    let document = registry(&root);
    let row = row(&document, entry.slug.as_str());

    assert_eq!(row.get("mode").and_then(Value::as_str), Some("reference"));
    assert_eq!(
        row.get("target").and_then(Value::as_str),
        Some("/original/linked-row.tool")
    );
    for forbidden in ["source", "source_hash", "added_at", "id", "workdir"] {
        assert!(
            !row.contains_key(forbidden),
            "unexpected projection field: {forbidden}"
        );
    }
}

#[test]
fn test_same_payload_bytes_produce_the_same_source_hash_across_storage_modes() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let bytes = b"same bytes\n";

    let copied = store
        .create(request("copied", StorageMode::Copy, bytes))
        .unwrap();
    let linked = store
        .create(request("linked-hash", StorageMode::Reference, bytes))
        .unwrap();

    assert_eq!(copied.meta.source_hash, linked.meta.source_hash);
    assert_eq!(copied.meta.source_hash, hash(bytes));
}
