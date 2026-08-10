//! Public-API ports of Python v0.4 scalar `skit config` storage contracts.

use std::{collections::BTreeMap, fs};

use skit_store::{ConfigError, FileConfigStore};
use tempfile::TempDir;
use toml::Value;

fn document(root: &TempDir) -> toml::Table {
    toml::from_str(&fs::read_to_string(root.path().join("config.toml")).unwrap()).unwrap()
}

#[test]
fn test_scalar_defaults_match_the_machine_config_contract() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());
    let settings = store.settings().unwrap();

    for (key, expected) in [
        ("lang", ""),
        ("editor", ""),
        ("mirror", "off"),
        ("mirror.pypi", "off"),
        ("mirror.github", "off"),
        ("mirror.npm", "off"),
        ("form", "tui"),
        ("after_run", "exit"),
        ("shell.bash_path", ""),
        ("js.runner", ""),
    ] {
        assert_eq!(settings.get(key).map(String::as_str), Some(expected), "{key}");
    }
}

#[test]
fn test_set_lang_writes_language_key_and_auto_clears_it() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());

    store.set("lang", "zh-CN").unwrap();
    assert_eq!(store.get("lang").unwrap(), "zh-CN");
    assert_eq!(
        document(&root).get("language").and_then(Value::as_str),
        Some("zh-CN")
    );

    store.set("lang", "auto").unwrap();
    assert_eq!(store.get("lang").unwrap(), "");
    assert!(!document(&root).contains_key("language"));
}

#[test]
fn test_unknown_language_is_usage_error_and_does_not_write_it() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());

    let error = store.set("lang", "xx-YY").unwrap_err();
    assert!(matches!(error, ConfigError::Usage(_)));
    assert_eq!(store.get("lang").unwrap(), "");
}

#[test]
fn test_form_accepts_plain_and_tui_but_refuses_unknown_style() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());

    store.set("form", "plain").unwrap();
    assert_eq!(store.get("form").unwrap(), "plain");
    store.set("form", "tui").unwrap();
    assert_eq!(store.get("form").unwrap(), "tui");

    let before = fs::read(root.path().join("config.toml")).unwrap();
    let error = store.set("form", "fancy").unwrap_err();
    assert!(matches!(error, ConfigError::Usage(_)));
    assert_eq!(fs::read(root.path().join("config.toml")).unwrap(), before);
}

#[test]
fn test_after_run_accepts_stay_and_exit_but_refuses_unknown_mode() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());

    assert_eq!(store.get("after_run").unwrap(), "exit");
    store.set("after_run", "stay").unwrap();
    assert_eq!(store.get("after_run").unwrap(), "stay");
    store.set("after_run", "exit").unwrap();
    assert_eq!(store.get("after_run").unwrap(), "exit");

    let error = store.set("after_run", "loop").unwrap_err();
    assert!(matches!(error, ConfigError::Usage(_)));
}

#[test]
fn test_after_run_garbage_in_hand_edited_config_normalizes_to_exit() {
    let root = TempDir::new().unwrap();
    fs::write(root.path().join("config.toml"), "after_run = \"never\"\n").unwrap();
    let store = FileConfigStore::new(root.path());

    assert_eq!(store.get("after_run").unwrap(), "exit");
}

#[test]
fn test_js_runner_accepts_deno_bun_node_and_empty_but_refuses_unknown() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());

    for name in ["deno", "bun", "node"] {
        store.set("js.runner", name).unwrap();
        assert_eq!(store.get("js.runner").unwrap(), name);
    }
    store.set("js.runner", "").unwrap();
    assert_eq!(store.get("js.runner").unwrap(), "");

    let error = store.set("js.runner", "carrier-pigeon").unwrap_err();
    assert!(matches!(error, ConfigError::Usage(_)));
    assert_eq!(store.get("js.runner").unwrap(), "");
}

#[test]
fn test_unknown_config_key_is_usage_error() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());

    assert!(matches!(store.get("theme"), Err(ConfigError::Usage(_))));
    assert!(matches!(
        store.set("theme", "dark"),
        Err(ConfigError::Usage(_))
    ));
}

#[test]
fn test_scalar_write_preserves_other_sections_and_unknown_fields() {
    let root = TempDir::new().unwrap();
    fs::write(
        root.path().join("config.toml"),
        concat!(
            "future = 7\n",
            "[prompt]\n",
            "other = 1\n",
            "[mirror]\n",
            "enabled = false\n",
            "pypi = \"https://example/simple\"\n",
        ),
    )
    .unwrap();
    let store = FileConfigStore::new(root.path());

    store.set("editor", "code --wait").unwrap();
    let doc = document(&root);
    assert_eq!(doc.get("future").and_then(Value::as_integer), Some(7));
    assert_eq!(
        doc.get("prompt")
            .and_then(Value::as_table)
            .and_then(|prompt| prompt.get("other"))
            .and_then(Value::as_integer),
        Some(1)
    );
    assert_eq!(
        doc.get("mirror")
            .and_then(Value::as_table)
            .and_then(|mirror| mirror.get("pypi"))
            .and_then(Value::as_str),
        Some("https://example/simple")
    );
    assert_eq!(doc.get("editor").and_then(Value::as_str), Some("code --wait"));
}

#[test]
fn test_set_many_validates_every_value_before_mutating_the_file() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());
    store.set("editor", "vim").unwrap();
    let before = fs::read(root.path().join("config.toml")).unwrap();

    let batch = BTreeMap::from([
        ("editor".to_owned(), "code".to_owned()),
        ("form".to_owned(), "fancy".to_owned()),
    ]);
    let error = store.set_many(&batch).unwrap_err();
    assert!(matches!(error, ConfigError::Usage(_)));
    assert_eq!(fs::read(root.path().join("config.toml")).unwrap(), before);
}
