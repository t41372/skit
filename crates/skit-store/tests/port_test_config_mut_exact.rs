//! Public-surface ports of the executable contracts in Python v0.4 `tests/test_config_mut.py`.
//!
//! The five tests below use FileConfigStore's real nested scalar RMW boundary. The two Python
//! corrupt-config tests assert exact frontend stderr warning sentences; FileConfigStore exposes the
//! typed recovery fact but not that frontend wording, so those contracts remain explicitly blocked
//! in the companion manifest instead of being replaced by weaker `.bak` existence checks.

use std::fs;

use skit_store::FileConfigStore;
use tempfile::TempDir;
use toml::{Table, Value};

fn write_config(root: &TempDir, body: &str) {
    fs::write(root.path().join("config.toml"), body).unwrap();
}

fn document(root: &TempDir) -> Table {
    toml::from_str(&fs::read_to_string(root.path().join("config.toml")).unwrap()).unwrap()
}

fn section<'a>(document: &'a Table, name: &str) -> &'a Table {
    document
        .get(name)
        .and_then(Value::as_table)
        .unwrap_or_else(|| panic!("missing [{name}] in {document:?}"))
}

#[test]
fn test_save_bash_path_clear_tolerates_missing_bash_path_key() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());
    write_config(&root, "[shell]\nother = \"keep\"\n");

    store.set("shell.bash_path", "").unwrap();

    let doc = document(&root);
    assert_eq!(section(&doc, "shell").len(), 1);
    assert_eq!(
        section(&doc, "shell").get("other").and_then(Value::as_str),
        Some("keep")
    );
    assert!(!section(&doc, "shell").contains_key("bash_path"));
}

#[test]
fn test_save_bash_path_clear_tolerates_missing_shell_section() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());
    write_config(&root, "language = \"en\"\n");

    store.set("shell.bash_path", "").unwrap();

    let doc = document(&root);
    assert_eq!(doc.len(), 1);
    assert_eq!(doc.get("language").and_then(Value::as_str), Some("en"));
    assert!(!doc.contains_key("shell"));
}

#[test]
fn test_save_js_runner_clear_tolerates_missing_runner_key() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());
    write_config(&root, "[js]\nother = \"keep\"\n");

    store.set("js.runner", "").unwrap();

    let doc = document(&root);
    assert_eq!(section(&doc, "js").len(), 1);
    assert_eq!(
        section(&doc, "js").get("other").and_then(Value::as_str),
        Some("keep")
    );
    assert!(!section(&doc, "js").contains_key("runner"));
}

#[test]
fn test_save_js_runner_clear_tolerates_missing_js_section() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());
    write_config(&root, "language = \"en\"\n");

    store.set("js.runner", "").unwrap();

    let doc = document(&root);
    assert_eq!(doc.len(), 1);
    assert_eq!(doc.get("language").and_then(Value::as_str), Some("en"));
    assert!(!doc.contains_key("js"));
}

#[test]
fn test_save_js_runner_preserves_sibling_js_keys() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());
    write_config(&root, "[js]\nother = \"keep\"\n");

    store.set("js.runner", "deno").unwrap();

    let doc = document(&root);
    assert_eq!(section(&doc, "js").len(), 2);
    assert_eq!(
        section(&doc, "js").get("runner").and_then(Value::as_str),
        Some("deno")
    );
    assert_eq!(
        section(&doc, "js").get("other").and_then(Value::as_str),
        Some("keep")
    );
}
