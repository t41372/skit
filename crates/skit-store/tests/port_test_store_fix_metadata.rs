//! Exact metadata-corruption ports from Python `tests/test_store_fix.py` at `main@206f9ef`.
//!
//! Python exposed a raw-dictionary `ScriptMeta` constructor. Rust decodes authoritative metadata
//! at the stronger public `FileStore` boundary. These owners therefore drive the real scan,
//! resolve, and rebuild paths. A bad entry must become typed corruption without hiding a valid
//! sibling or changing the metadata bytes during a read.

use std::{collections::BTreeMap, fs, path::PathBuf};

use skit_application::{DiagnosticCode, EntryRepository as _, RepositoryError};
use skit_i18n::Locale;
use skit_store::{FileStore, RegistryRebuildProblem};
use tempfile::TempDir;
use toml::{Table, Value};

fn meta_path(root: &TempDir, slug: &str) -> PathBuf {
    root.path().join("scripts").join(slug).join("meta.toml")
}

fn write_meta(root: &TempDir, slug: &str, body: &str) -> PathBuf {
    let path = meta_path(root, slug);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, body).unwrap();
    path
}

fn write_registry_row(root: &TempDir, slug: &str, name: &str) {
    let path = root.path().join("registry.toml");
    let mut document = if path.exists() {
        toml::from_str::<Table>(&fs::read_to_string(&path).unwrap()).unwrap()
    } else {
        Table::new()
    };
    let entries = document
        .entry("entries")
        .or_insert_with(|| Value::Table(Table::new()))
        .as_table_mut()
        .unwrap();
    entries.insert(
        slug.to_owned(),
        Value::Table(Table::from_iter([
            ("name".to_owned(), Value::String(name.to_owned())),
            ("kind".to_owned(), Value::String("future-kind".to_owned())),
            ("mode".to_owned(), Value::String("copy".to_owned())),
            ("description".to_owned(), Value::String(String::new())),
        ])),
    );
    fs::write(path, toml::to_string_pretty(&document).unwrap()).unwrap();
}

fn write_good(root: &TempDir) -> PathBuf {
    let path = write_meta(
        root,
        "good",
        concat!(
            "schema = 1\n",
            "name = \"Good\"\n",
            "kind = \"future-kind\"\n",
            "mode = \"copy\"\n",
            "workdir = \"invoke\"\n",
            "description = \"valid sibling\"\n",
        ),
    );
    write_registry_row(root, "good", "Good");
    path
}

fn diagnostic_for(root: &TempDir, slug: &str) -> skit_application::Diagnostic {
    let scan = FileStore::new(root.path()).scan().unwrap();
    assert!(
        scan.entries.is_empty(),
        "corrupt metadata leaked into healthy entries"
    );
    scan.diagnostics
        .into_iter()
        .find(|diagnostic| diagnostic.slug.as_deref() == Some(slug))
        .unwrap_or_else(|| panic!("no corruption diagnostic for {slug}"))
}

fn assert_localized_corruption(diagnostic: &skit_application::Diagnostic, field: &str) {
    for (locale, prefix) in [
        (Locale::En, "entry \"bad\" has corrupt metadata:"),
        (Locale::ZhCn, "条目 \"bad\" 的元数据已损坏："),
        (Locale::ZhTw, "項目 \"bad\" 的中繼資料已損毀："),
    ] {
        let text = diagnostic.localize(locale);
        assert!(text.contains(prefix), "locale={locale:?}: {text}");
        assert!(text.contains(field), "locale={locale:?}: {text}");
    }
}

#[test]
fn test_from_toml_dict_missing_name_raises_scriptmetaerror_not_keyerror() {
    let root = TempDir::new().unwrap();
    write_meta(&root, "bad", "schema = 1\nkind = \"future-kind\"\n");
    write_registry_row(&root, "bad", "bad");

    let diagnostic = diagnostic_for(&root, "bad");
    assert_eq!(diagnostic.code, DiagnosticCode::CorruptMetadata);
    assert!(
        diagnostic.message.contains("name"),
        "{}",
        diagnostic.message
    );
}

#[test]
fn test_from_toml_dict_missing_kind_raises_scriptmetaerror_not_keyerror() {
    let root = TempDir::new().unwrap();
    write_meta(&root, "bad", "schema = 1\nname = \"bad\"\n");
    write_registry_row(&root, "bad", "bad");

    let diagnostic = diagnostic_for(&root, "bad");
    assert_eq!(diagnostic.code, DiagnosticCode::CorruptMetadata);
    assert!(
        diagnostic.message.contains("kind"),
        "{}",
        diagnostic.message
    );
}

#[test]
fn test_list_entries_skips_valid_toml_missing_name_key() {
    let root = TempDir::new().unwrap();
    let good = write_good(&root);
    let bad = write_meta(&root, "bad-slug", "schema = 1\nkind = \"future-kind\"\n");
    write_registry_row(&root, "bad-slug", "bad");
    let good_before = fs::read(&good).unwrap();
    let bad_before = fs::read(&bad).unwrap();

    let scan = FileStore::new(root.path()).scan().unwrap();

    assert_eq!(
        scan.entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        ["Good"]
    );
    assert!(scan.diagnostics.iter().any(|item| {
        item.code == DiagnosticCode::CorruptMetadata && item.slug.as_deref() == Some("bad-slug")
    }));
    assert_eq!(fs::read(good).unwrap(), good_before);
    assert_eq!(fs::read(bad).unwrap(), bad_before);
}

#[test]
fn test_doctor_rebuild_reports_missing_key_instead_of_crashing() {
    let root = TempDir::new().unwrap();
    write_good(&root);
    write_meta(&root, "bad-slug", "schema = 1\nkind = \"future-kind\"\n");

    let report = FileStore::new(root.path())
        .rebuild_registry_report()
        .unwrap();

    assert_eq!(report.entry_count, 1);
    assert!(report.problems.iter().any(|problem| matches!(
        problem,
        RegistryRebuildProblem::CorruptMetadata { slug, reason }
            if slug == "bad-slug" && reason.contains("name")
    )));
}

#[test]
fn test_resolve_corrupt_missing_key_meta_raises_notfounderror_not_keyerror() {
    let root = TempDir::new().unwrap();
    write_meta(&root, "bad-slug", "schema = 1\nkind = \"future-kind\"\n");
    write_registry_row(&root, "bad-slug", "bad");

    assert!(matches!(
        FileStore::new(root.path()).resolve("bad-slug"),
        Err(RepositoryError::NotFound { .. })
    ));
}

#[test]
fn test_from_toml_dict_scalar_dependencies_raises_scriptmetaerror_not_typeerror() {
    let root = TempDir::new().unwrap();
    write_meta(
        &root,
        "bad",
        "schema = 1\nname = \"bad\"\nkind = \"future-kind\"\ndependencies = 5\n",
    );
    write_registry_row(&root, "bad", "bad");

    let diagnostic = diagnostic_for(&root, "bad");
    assert_eq!(diagnostic.code, DiagnosticCode::CorruptMetadata);
    assert!(
        diagnostic.message.contains("dependencies"),
        "{}",
        diagnostic.message
    );
}

#[test]
fn test_from_toml_dict_scalar_params_raises_scriptmetaerror_not_typeerror() {
    let root = TempDir::new().unwrap();
    write_meta(
        &root,
        "bad",
        "schema = 1\nname = \"bad\"\nkind = \"future-kind\"\nparams = 5\n",
    );
    write_registry_row(&root, "bad", "bad");

    let diagnostic = diagnostic_for(&root, "bad");
    assert_eq!(diagnostic.code, DiagnosticCode::CorruptMetadata);
    assert!(
        diagnostic.message.contains("params"),
        "{}",
        diagnostic.message
    );
}

#[test]
fn test_list_entries_skips_scalar_dependencies_meta() {
    let root = TempDir::new().unwrap();
    let good = write_good(&root);
    let bad = write_meta(
        &root,
        "bad-type-slug",
        "schema = 1\nname = \"bad\"\nkind = \"future-kind\"\ndependencies = 5\n",
    );
    write_registry_row(&root, "bad-type-slug", "bad");
    let good_before = fs::read(&good).unwrap();
    let bad_before = fs::read(&bad).unwrap();

    let scan = FileStore::new(root.path()).scan().unwrap();

    assert_eq!(
        scan.entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        ["Good"]
    );
    assert!(scan.diagnostics.iter().any(|item| {
        item.code == DiagnosticCode::CorruptMetadata
            && item.slug.as_deref() == Some("bad-type-slug")
            && item.message.contains("dependencies")
    }));
    assert_eq!(fs::read(good).unwrap(), good_before);
    assert_eq!(fs::read(bad).unwrap(), bad_before);
}

#[test]
fn test_doctor_rebuild_reports_scalar_params_instead_of_crashing() {
    let root = TempDir::new().unwrap();
    write_good(&root);
    write_meta(
        &root,
        "bad-type-slug",
        "schema = 1\nname = \"bad\"\nkind = \"future-kind\"\nparams = 5\n",
    );

    let report = FileStore::new(root.path())
        .rebuild_registry_report()
        .unwrap();

    assert_eq!(report.entry_count, 1);
    assert!(report.problems.iter().any(|problem| matches!(
        problem,
        RegistryRebuildProblem::CorruptMetadata { slug, reason }
            if slug == "bad-type-slug" && reason.contains("params")
    )));
}

#[test]
fn test_resolve_scalar_dependencies_meta_raises_notfounderror_not_typeerror() {
    let root = TempDir::new().unwrap();
    write_meta(
        &root,
        "bad-type-slug",
        "schema = 1\nname = \"bad\"\nkind = \"future-kind\"\ndependencies = 5\n",
    );
    write_registry_row(&root, "bad-type-slug", "bad");

    assert!(matches!(
        FileStore::new(root.path()).resolve("bad-type-slug"),
        Err(RepositoryError::NotFound { .. })
    ));
}

#[test]
fn test_from_toml_dict_missing_key_message_is_gettext_wrapped() {
    let root = TempDir::new().unwrap();
    write_meta(&root, "bad", "schema = 1\nkind = \"future-kind\"\n");
    write_registry_row(&root, "bad", "bad");

    assert_localized_corruption(&diagnostic_for(&root, "bad"), "name");
}

#[test]
fn test_from_toml_dict_invalid_type_message_is_gettext_wrapped() {
    let root = TempDir::new().unwrap();
    write_meta(
        &root,
        "bad",
        "schema = 1\nname = \"bad\"\nkind = \"future-kind\"\ndependencies = 5\n",
    );
    write_registry_row(&root, "bad", "bad");

    assert_localized_corruption(&diagnostic_for(&root, "bad"), "dependencies");
}

#[test]
fn valid_known_lists_keep_unknown_toml_open_kinds_and_source_bytes() {
    let root = TempDir::new().unwrap();
    let path = write_meta(
        &root,
        "open",
        concat!(
            "schema = 1\n",
            "name = \"Open\"\n",
            "kind = \"future-kind\"\n",
            "dependencies = [\"one\", \"two\"]\n",
            "params = [\"CITY\"]\n",
            "vendor_scalar = 7\n",
            "[vendor_table]\n",
            "enabled = true\n",
        ),
    );
    write_registry_row(&root, "open", "Open");
    let before = fs::read(&path).unwrap();
    let store = FileStore::new(root.path());

    let scan = store.scan().unwrap();
    let entry = store.resolve("open").unwrap();

    assert_eq!(scan.entries.len(), 1);
    assert!(scan.diagnostics.is_empty());
    assert_eq!(entry.meta.kind.as_str(), "future-kind");
    assert_eq!(
        entry.meta.extra["dependencies"],
        serde_json::json!(["one", "two"])
    );
    assert_eq!(entry.meta.extra["params"], serde_json::json!(["CITY"]));
    assert_eq!(entry.meta.extra["vendor_scalar"], serde_json::json!(7));
    assert_eq!(
        entry.meta.extra["vendor_table"],
        serde_json::json!(BTreeMap::from([("enabled", true)]))
    );
    assert_eq!(fs::read(path).unwrap(), before);
}
