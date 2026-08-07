use std::fs;

use skit_application::{DiagnosticCode, EntryRepository, RepositoryError};
use skit_domain::StorageMode;
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

    let store = FileStore::new(root.path());
    let scan = store.scan().unwrap();

    assert_eq!(scan.entries.len(), 2);
    assert_eq!(
        scan.entries
            .iter()
            .find(|entry| entry.slug.as_str() == "legacy")
            .unwrap()
            .target
            .as_deref(),
        Some("/tmp/legacy.tool")
    );
    let current = store.resolve("current").unwrap();
    assert_eq!(
        current.meta.id.unwrap().as_str(),
        "0123456789abcdef0123456789abcdef"
    );
    assert_eq!(
        current.meta.extra.get("requires_python").unwrap(),
        &serde_json::json!(">=3.12")
    );
    let legacy = store.resolve("legacy").unwrap();
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
    assert_eq!(
        error,
        RepositoryError::Ambiguous {
            query: "same".to_owned(),
            candidates: vec!["other".to_owned(), "third".to_owned()],
        }
    );
}

#[test]
fn exact_display_names_resolve_without_case_folding_or_guessing() {
    let root = TempDir::new().unwrap();
    write_meta(
        &root,
        "alpha",
        r#"name = "Display Name"
kind = "command"
mode = "copy"
"#,
    );
    write_meta(&root, "broken", "not = [toml");
    let store = FileStore::new(root.path());

    assert_eq!(
        store.resolve("Display Name").unwrap().slug.as_str(),
        "alpha"
    );
    assert!(matches!(
        store.resolve("display name").unwrap_err(),
        RepositoryError::NotFound { .. }
    ));
}

#[test]
fn a_missing_selector_is_classified_as_not_found() {
    let root = TempDir::new().unwrap();
    let error = FileStore::new(root.path()).resolve("missing").unwrap_err();
    assert!(matches!(error, RepositoryError::NotFound { .. }));
}

#[test]
fn a_missing_library_is_an_empty_scan_and_the_root_is_observable() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());

    assert_eq!(store.data_dir(), root.path());
    assert_eq!(store.scan().unwrap(), Default::default());
}

#[test]
fn invalid_slug_directories_are_diagnostics_and_regular_files_are_ignored() {
    let root = TempDir::new().unwrap();
    fs::create_dir_all(root.path().join("scripts").join("Upper")).unwrap();
    fs::write(root.path().join("scripts").join("README"), "not an entry").unwrap();
    write_meta(
        &root,
        "valid",
        r#"name = "Valid"
kind = "command"
"#,
    );

    let scan = FileStore::new(root.path()).scan().unwrap();

    assert_eq!(scan.entries.len(), 1);
    assert_eq!(scan.diagnostics.len(), 1);
    assert_eq!(scan.diagnostics[0].code, DiagnosticCode::InvalidSlug);
    assert_eq!(scan.diagnostics[0].slug.as_deref(), Some("Upper"));
}

#[test]
fn malformed_kind_and_id_are_isolated_as_corrupt_metadata() {
    let root = TempDir::new().unwrap();
    write_meta(
        &root,
        "blank-kind",
        r#"name = "Blank"
kind = "  "
"#,
    );
    write_meta(
        &root,
        "bad-id",
        r#"name = "Bad id"
kind = "command"
id = "not-a-uuid"
"#,
    );

    let scan = FileStore::new(root.path()).scan().unwrap();

    assert!(scan.entries.is_empty());
    assert_eq!(scan.diagnostics.len(), 2);
    assert!(
        scan.diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code == DiagnosticCode::CorruptMetadata)
    );
    assert!(matches!(
        FileStore::new(root.path()).resolve("bad-id").unwrap_err(),
        RepositoryError::Corrupt { .. }
    ));
}

#[test]
fn metadata_defaults_match_legacy_files() {
    let root = TempDir::new().unwrap();
    write_meta(
        &root,
        "minimal",
        r#"name = "Minimal"
kind = "command"
"#,
    );

    let entry = FileStore::new(root.path()).resolve("minimal").unwrap();

    assert_eq!(entry.meta.schema, 1);
    assert_eq!(entry.meta.mode, StorageMode::Copy);
    assert_eq!(entry.meta.workdir, "origin");
    assert!(entry.meta.source.is_empty());
    assert!(entry.meta.source_hash.is_empty());
    assert!(entry.meta.added_at.is_empty());
    assert!(entry.meta.description.is_empty());
}

#[test]
fn a_non_directory_scripts_path_is_a_repository_io_error() {
    let root = TempDir::new().unwrap();
    fs::write(root.path().join("scripts"), "not a directory").unwrap();

    let error = FileStore::new(root.path()).scan().unwrap_err();

    assert!(matches!(
        error,
        RepositoryError::Io {
            operation: "scan",
            ..
        }
    ));
}

#[test]
fn an_exact_slug_with_missing_metadata_reports_io_instead_of_falling_back() {
    let root = TempDir::new().unwrap();
    fs::create_dir_all(root.path().join("scripts").join("empty")).unwrap();

    let error = FileStore::new(root.path()).resolve("empty").unwrap_err();

    assert!(matches!(
        error,
        RepositoryError::Io {
            operation: "read",
            ..
        }
    ));
}
