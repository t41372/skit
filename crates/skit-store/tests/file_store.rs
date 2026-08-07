use std::fs;

use skit_application::{DiagnosticCode, EntryRepository, RepositoryError};
use skit_store::FileStore;
use tempfile::TempDir;

fn write_meta(root: &TempDir, slug: &str, body: &str) {
    let dir = root.path().join("scripts").join(slug);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("meta.toml"), body).unwrap();
}

#[test]
fn scans_current_and_legacy_metadata_without_requiring_a_registry() {
    let root = TempDir::new().unwrap();
    write_meta(
        &root,
        "current",
        r#"
schema = 1
name = "Current"
kind = "python"
mode = "copy"
source = "/tmp/current.py"
source_hash = "sha256:abc"
added_at = "2026-08-07T00:00:00+00:00"
id = "0123456789abcdef0123456789abcdef"
workdir = "origin"
description = "current metadata"
requires_python = ">=3.12"
"#,
    );
    write_meta(
        &root,
        "legacy",
        r#"
schema = 1
name = "Legacy"
kind = "future-kind"
mode = "reference"
source = "/tmp/legacy.tool"
source_hash = ""
added_at = "2025-01-01T00:00:00+00:00"
workdir = "invoke"
description = "no id yet"
"#,
    );

    let scan = FileStore::new(root.path()).scan().unwrap();

    assert_eq!(scan.entries.len(), 2);
    let current = FileStore::new(root.path()).resolve("current").unwrap();
    assert_eq!(
        current.meta.id.unwrap().as_str(),
        "0123456789abcdef0123456789abcdef"
    );
    assert_eq!(
        current.meta.extra.get("requires_python").unwrap(),
        &serde_json::json!(">=3.12")
    );
    let legacy = FileStore::new(root.path()).resolve("legacy").unwrap();
    assert!(legacy.meta.id.is_none());
    assert_eq!(legacy.meta.kind.as_str(), "future-kind");
}

#[test]
fn one_corrupt_entry_becomes_a_diagnostic_without_hiding_valid_entries() {
    let root = TempDir::new().unwrap();
    write_meta(
        &root,
        "good",
        r#"name = "Good"
kind = "command"
mode = "copy"
"#,
    );
    write_meta(&root, "broken", "name = [not valid TOML");

    let scan = FileStore::new(root.path()).scan().unwrap();

    assert_eq!(scan.entries.len(), 1);
    assert_eq!(scan.entries[0].slug.as_str(), "good");
    assert_eq!(scan.diagnostics.len(), 1);
    assert_eq!(scan.diagnostics[0].code, DiagnosticCode::CorruptMetadata);
    assert_eq!(scan.diagnostics[0].slug.as_deref(), Some("broken"));
}

#[test]
fn resolve_prefers_an_exact_slug_then_refuses_ambiguous_names() {
    let root = TempDir::new().unwrap();
    write_meta(
        &root,
        "same",
        r#"name = "First"
kind = "command"
mode = "copy"
"#,
    );
    write_meta(
        &root,
        "other",
        r#"name = "same"
kind = "prompt"
mode = "copy"
"#,
    );
    write_meta(
        &root,
        "third",
        r#"name = "same"
kind = "shell"
mode = "copy"
"#,
    );
    let store = FileStore::new(root.path());

    assert_eq!(store.resolve("same").unwrap().slug.as_str(), "same");

    fs::remove_dir_all(root.path().join("scripts").join("same")).unwrap();
    let error = store.resolve("same").unwrap_err();
    assert!(matches!(error, RepositoryError::Ambiguous { .. }));
}

#[test]
fn a_missing_selector_is_classified_as_not_found() {
    let root = TempDir::new().unwrap();
    let error = FileStore::new(root.path()).resolve("missing").unwrap_err();
    assert!(matches!(error, RepositoryError::NotFound { .. }));
}
