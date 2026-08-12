//! Exact config-storage ports from Python `tests/test_editor.py` at `main@206f9ef`.
//!
//! These are kept at the store boundary instead of routing them through a UI. The Python tests are
//! specifically about editor persistence, clearing, type tolerance, and preserving unrelated keys.

use std::fs;

use skit_store::FileConfigStore;
use tempfile::TempDir;

fn config_text(root: &TempDir) -> String {
    fs::read_to_string(root.path().join("config.toml")).unwrap_or_default()
}

#[test]
fn test_config_editor_roundtrip_and_clear() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());

    assert_eq!(store.get("editor").unwrap(), "");
    store.set("editor", "code --wait").unwrap();
    assert_eq!(store.get("editor").unwrap(), "code --wait");
    store.set("editor", "").unwrap();
    assert_eq!(store.get("editor").unwrap(), "");
    assert!(
        !config_text(&root).lines().any(|line| line.trim_start().starts_with("editor =")),
        "clearing editor must remove the key, not persist an empty replacement"
    );
}

#[test]
fn test_save_editor_preserves_other_keys() {
    let root = TempDir::new().unwrap();
    fs::write(root.path().join("config.toml"), "language = \"zh-TW\"\n").unwrap();
    let store = FileConfigStore::new(root.path());

    store.set("editor", "nano").unwrap();

    let text = config_text(&root);
    assert!(text.contains("language = \"zh-TW\""), "unrelated key was lost: {text}");
    assert!(text.contains("editor = \"nano\""), "editor write missing: {text}");
}

#[test]
fn test_load_editor_non_string_value_is_blank() {
    let root = TempDir::new().unwrap();
    fs::write(root.path().join("config.toml"), "editor = 123\n").unwrap();
    let store = FileConfigStore::new(root.path());

    assert_eq!(store.get("editor").unwrap(), "");
}

#[test]
fn test_save_editor_clear_when_absent_does_not_raise() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());

    store.set("editor", "").unwrap();
    assert_eq!(store.get("editor").unwrap(), "");
    assert!(
        !config_text(&root).lines().any(|line| line.trim_start().starts_with("editor =")),
        "clearing an absent editor must not fabricate an editor key"
    );
}
