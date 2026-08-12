//! Exact corruption-facing ports from Python `tests/test_store_fix.py` at `main@206f9ef`.
//!
//! Python's first helper tests call `ScriptMeta.from_toml_dict` directly. Rust does not expose a
//! comparable raw-dict model constructor: authoritative metadata is decoded inside `FileStore`.
//! Those helper contracts therefore map to the stronger public boundary below — the same malformed
//! TOML must become typed corruption/diagnostics, never a panic or silent healthy entry.

use std::fs;

use skit_application::{DiagnosticCode, EntryRepository as _, RepositoryError};
use skit_i18n::Locale;
use skit_store::{FileStore, RegistryRebuildProblem};
use tempfile::TempDir;

fn write_meta(root: &TempDir, slug: &str, body: &str) {
    let directory = root.path().join("scripts").join(slug);
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("meta.toml"), body).unwrap();
}

fn write_registry_row(root: &TempDir, slug: &str, name: &str) {
    fs::create_dir_all(root.path()).unwrap();
    fs::write(
        root.path().join("registry.toml"),
        format!(
            concat!(
                "[entries.{slug}]\n",
                "name = {name:?}\n",
                "kind = \"python\"\n",
                "mode = \"copy\"\n",
                "description = \"\"\n",
            ),
            slug = slug,
            name = name,
        ),
    )
    .unwrap();
}

fn diagnostic_for(root: &TempDir, slug: &str) -> skit_application::Diagnostic {
    let store = FileStore::new(root.path());
    let scan = store.scan().unwrap();
    assert!(scan.entries.is_empty(), "corrupt metadata leaked into healthy entries");
    scan.diagnostics
        .into_iter()
        .find(|diagnostic| diagnostic.slug.as_deref() == Some(slug))
        .unwrap_or_else(|| panic!("no corruption diagnostic for {slug}"))
}

#[test]
fn test_from_toml_dict_missing_name_raises_scriptmetaerror_not_keyerror() {
    let root = TempDir::new().unwrap();
    write_meta(&root, "bad", "schema = 1\nkind = \"python\"\n");
    write_registry_row(&root, "bad", "bad");

    let diagnostic = diagnostic_for(&root, "bad");
    assert_eq!(diagnostic.code, DiagnosticCode::CorruptMetadata);
    assert!(diagnostic.message.contains("name"), "{}", diagnostic.message);
}

#[test]
fn test_from_toml_dict_missing_kind_raises_scriptmetaerror_not_keyerror() {
    let root = TempDir::new().unwrap();
    write_meta(&root, "bad", "schema = 1\nname = \"bad\"\n");
    write_registry_row(&root, "bad", "bad");

    let diagnostic = diagnostic_for(&root, "bad");
    assert_eq!(diagnostic.code, DiagnosticCode::CorruptMetadata);
    assert!(diagnostic.message.contains("kind"), "{}", diagnostic.message);
}

#[test]
fn test_resolve_corrupt_missing_key_meta_raises_notfounderror_not_keyerror() {
    let root = TempDir::new().unwrap();
    write_meta(&root, "bad-slug", "schema = 1\nkind = \"python\"\n");
    write_registry_row(&root, "bad-slug", "bad");

    let error = FileStore::new(root.path()).resolve("bad-slug").unwrap_err();
    assert!(
        matches!(error, RepositoryError::Corrupt { .. } | RepositoryError::NotFound { .. }),
        "corrupt authoritative meta escaped as the wrong failure class: {error}"
    );
    assert!(!matches!(error, RepositoryError::Io { .. }));
}

#[test]
fn test_from_toml_dict_scalar_dependencies_raises_scriptmetaerror_not_typeerror() {
    let root = TempDir::new().unwrap();
    write_meta(
        &root,
        "bad",
        "schema = 1\nname = \"bad\"\nkind = \"python\"\ndependencies = 5\n",
    );
    write_registry_row(&root, "bad", "bad");

    let diagnostic = diagnostic_for(&root, "bad");
    assert_eq!(diagnostic.code, DiagnosticCode::CorruptMetadata);
    assert!(diagnostic.message.contains("dependencies"), "{}", diagnostic.message);
}

#[test]
fn test_from_toml_dict_scalar_params_raises_scriptmetaerror_not_typeerror() {
    let root = TempDir::new().unwrap();
    write_meta(
        &root,
        "bad",
        "schema = 1\nname = \"bad\"\nkind = \"command\"\nparams = 5\n",
    );
    write_registry_row(&root, "bad", "bad");

    let diagnostic = diagnostic_for(&root, "bad");
    assert_eq!(diagnostic.code, DiagnosticCode::CorruptMetadata);
    assert!(diagnostic.message.contains("params"), "{}", diagnostic.message);
}

#[test]
fn test_doctor_rebuild_reports_scalar_params_instead_of_crashing() {
    let root = TempDir::new().unwrap();
    write_meta(
        &root,
        "bad-type-slug",
        "schema = 1\nname = \"bad\"\nkind = \"command\"\nparams = 5\n",
    );
    write_meta(
        &root,
        "good",
        "schema = 1\nname = \"good\"\nkind = \"command\"\nmode = \"copy\"\n",
    );

    let report = FileStore::new(root.path()).rebuild_registry_report().unwrap();
    assert_eq!(report.entry_count, 1);
    assert!(
        report.problems.iter().any(|problem| matches!(
            problem,
            RegistryRebuildProblem::CorruptMetadata { slug, .. } if slug == "bad-type-slug"
        )),
        "doctor/rebuild did not report the scalar params corruption: {:?}",
        report.problems
    );
}

#[test]
fn test_resolve_scalar_dependencies_meta_raises_notfounderror_not_typeerror() {
    let root = TempDir::new().unwrap();
    write_meta(
        &root,
        "bad-type-slug",
        "schema = 1\nname = \"bad\"\nkind = \"python\"\ndependencies = 5\n",
    );
    write_registry_row(&root, "bad-type-slug", "bad");

    let error = FileStore::new(root.path())
        .resolve("bad-type-slug")
        .unwrap_err();
    assert!(
        matches!(error, RepositoryError::Corrupt { .. } | RepositoryError::NotFound { .. }),
        "wrong-typed metadata escaped the corruption boundary: {error}"
    );
}

#[test]
fn test_from_toml_dict_missing_key_message_is_gettext_wrapped() {
    let root = TempDir::new().unwrap();
    write_meta(&root, "bad", "schema = 1\nkind = \"python\"\n");
    write_registry_row(&root, "bad", "bad");
    let diagnostic = diagnostic_for(&root, "bad");

    let zh = diagnostic.localize(Locale::ZhTw);
    assert!(zh.contains("bad"), "{zh}");
    assert!(zh.contains("name"), "{zh}");
    assert_ne!(zh, diagnostic.message, "corruption message bypassed localization");
}

#[test]
fn test_from_toml_dict_invalid_type_message_is_gettext_wrapped() {
    let root = TempDir::new().unwrap();
    write_meta(
        &root,
        "bad",
        "schema = 1\nname = \"bad\"\nkind = \"python\"\ndependencies = 5\n",
    );
    write_registry_row(&root, "bad", "bad");
    let diagnostic = diagnostic_for(&root, "bad");

    let zh = diagnostic.localize(Locale::ZhTw);
    assert!(zh.contains("bad"), "{zh}");
    assert!(zh.contains("dependencies"), "{zh}");
    assert_ne!(zh, diagnostic.message, "invalid-type message bypassed localization");
}
