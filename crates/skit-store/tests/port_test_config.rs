//! Public-surface behavioral ports from `origin/main@206f9ef:tests/test_config.py`.
//!
//! These tests intentionally exercise only `FileConfigStore`. A red assertion is a Python/Rust
//! parity finding; this branch does not patch configuration production code to make it green.

use std::{
    collections::BTreeMap,
    fs,
    sync::{Arc, Barrier},
    thread,
};

use skit_store::{FileConfigStore, MirrorSettings};
use tempfile::TempDir;
use toml::{Table, Value};

const PYPI: &str = "https://pypi.tuna.tsinghua.edu.cn/simple";
const PYTHON_INSTALL: &str =
    "https://mirror.nju.edu.cn/github-release/astral-sh/python-build-standalone/";
const UV_BINARY: &str = "https://mirror.nju.edu.cn/github-release/astral-sh/uv";
const NPM: &str = "https://registry.npmmirror.com";

fn write_config(root: &TempDir, body: &str) {
    fs::write(root.path().join("config.toml"), body).unwrap();
}

fn full_mirror_body() -> String {
    format!(
        "[mirror]\nenabled = true\npypi = {PYPI:?}\npython_install = {PYTHON_INSTALL:?}\nuv_binary = {UV_BINARY:?}\nnpm = {NPM:?}\n"
    )
}

#[test]
fn test_defaults_when_no_config() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());

    assert_eq!(store.mirror().unwrap(), MirrorSettings::default());
    assert!(!store.mirror_configured().unwrap());
    assert_eq!(store.get("editor").unwrap(), "");
    assert_eq!(store.get("form").unwrap(), "tui");
    assert_eq!(store.get("after_run").unwrap(), "exit");
    assert!(
        store
            .mirror_environment(&BTreeMap::new())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn test_load_mirror_ignores_malformed_section() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());
    write_config(&root, "mirror = \"not-a-table\"\n");

    assert_eq!(store.mirror().unwrap(), MirrorSettings::default());
}

#[test]
fn test_load_mirror_type_hardens_enabled_urls_and_uv_binary_scheme() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());
    write_config(
        &root,
        "[mirror]\nenabled = \"false\"\npypi = 123\nuv_binary = \"http://evil/uv\"\n",
    );

    let mirror = store.mirror().unwrap();
    assert!(!mirror.enabled);
    assert_eq!(mirror.pypi, "");
    assert_eq!(mirror.uv_binary, "");
}

#[test]
fn test_load_mirror_preserves_https_uv_binary() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());
    write_config(
        &root,
        "[mirror]\nenabled = true\nuv_binary = \"https://ok/uv\"\n",
    );

    let mirror = store.mirror().unwrap();
    assert!(mirror.enabled);
    assert_eq!(mirror.uv_binary, "https://ok/uv");
}

#[test]
fn test_load_config_tolerates_corrupt_toml() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());
    write_config(&root, "this is = = not [valid toml");

    assert_eq!(store.mirror().unwrap(), MirrorSettings::default());
    assert_eq!(store.get("editor").unwrap(), "");
    assert_eq!(store.get("form").unwrap(), "tui");
}

#[test]
fn test_save_editor_backs_up_corrupt_config_instead_of_wiping_it() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());
    let corrupt = b"language = \"zh-CN\"\n[mirror]\nenabled = true\npypi = \"https://tsinghua\"\nthis is = = not valid toml";
    fs::write(root.path().join("config.toml"), corrupt).unwrap();

    let recovery = store
        .set_with_recovery("editor", "vim")
        .unwrap()
        .expect("a corrupt source must report recovery");

    assert_eq!(store.get("editor").unwrap(), "vim");
    assert_eq!(recovery.path, root.path().join("config.toml"));
    assert_eq!(recovery.backup_path, root.path().join("config.toml.bak"));
    assert_eq!(fs::read(&recovery.backup_path).unwrap(), corrupt);
}

#[test]
fn test_save_editor_still_preserves_other_keys_when_config_is_valid() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());
    write_config(&root, "language = \"zh-CN\"\nunknown = \"keep\"\n");

    assert!(
        store
            .set_with_recovery("editor", "code --wait")
            .unwrap()
            .is_none()
    );

    let document =
        toml::from_str::<Table>(&fs::read_to_string(root.path().join("config.toml")).unwrap())
            .unwrap();
    assert_eq!(
        document.get("language").and_then(Value::as_str),
        Some("zh-CN")
    );
    assert_eq!(
        document.get("unknown").and_then(Value::as_str),
        Some("keep")
    );
    assert_eq!(
        document.get("editor").and_then(Value::as_str),
        Some("code --wait")
    );
    assert!(!root.path().join("config.toml.bak").exists());
}

#[test]
fn test_mirror_env_overlays_all_vectors() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());
    write_config(&root, &full_mirror_body());

    let env = store.mirror_environment(&BTreeMap::new()).unwrap();
    assert_eq!(env.get("UV_DEFAULT_INDEX").map(String::as_str), Some(PYPI));
    assert_eq!(
        env.get("UV_PYTHON_INSTALL_MIRROR").map(String::as_str),
        Some(PYTHON_INSTALL)
    );
    assert_eq!(
        env.get("NPM_CONFIG_REGISTRY").map(String::as_str),
        Some(NPM)
    );
}

#[test]
fn test_mirror_env_defers_to_nonempty_user_index() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());
    write_config(&root, &full_mirror_body());
    let base = BTreeMap::from([("UV_INDEX_URL".to_owned(), "https://mine/simple".to_owned())]);

    let env = store.mirror_environment(&base).unwrap();
    assert!(!env.contains_key("UV_DEFAULT_INDEX"));
    assert!(env.contains_key("UV_PYTHON_INSTALL_MIRROR"));
}

#[test]
fn test_mirror_env_does_not_defer_on_additive_index_vars() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());
    write_config(&root, &full_mirror_body());

    for key in ["UV_EXTRA_INDEX_URL", "UV_INDEX"] {
        let base = BTreeMap::from([(key.to_owned(), "https://x".to_owned())]);
        let env = store.mirror_environment(&base).unwrap();
        assert_eq!(env.get("UV_DEFAULT_INDEX").map(String::as_str), Some(PYPI));
    }
}

#[test]
fn test_mirror_env_injects_when_index_env_blank() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());
    write_config(&root, &full_mirror_body());
    let base = BTreeMap::from([("UV_INDEX_URL".to_owned(), String::new())]);

    let env = store.mirror_environment(&base).unwrap();
    assert_eq!(env.get("UV_DEFAULT_INDEX").map(String::as_str), Some(PYPI));
}

#[test]
fn test_mirror_env_injects_when_python_mirror_blank() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());
    write_config(&root, &full_mirror_body());
    let base = BTreeMap::from([("UV_PYTHON_INSTALL_MIRROR".to_owned(), String::new())]);

    let env = store.mirror_environment(&base).unwrap();
    assert_eq!(
        env.get("UV_PYTHON_INSTALL_MIRROR").map(String::as_str),
        Some(PYTHON_INSTALL)
    );
}

#[test]
fn test_disable_keeps_urls_but_turns_off_environment_overlay() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());
    write_config(&root, &full_mirror_body());

    store.set("mirror", "off").unwrap();

    let mirror = store.mirror().unwrap();
    assert!(!mirror.enabled);
    assert_eq!(mirror.pypi, PYPI);
    assert_eq!(mirror.uv_binary, UV_BINARY);
    assert!(
        store
            .mirror_environment(&BTreeMap::new())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn test_config_transactions_from_many_threads_preserve_independent_keys() {
    let root = TempDir::new().unwrap();
    let store = Arc::new(FileConfigStore::new(root.path()));
    let barrier = Arc::new(Barrier::new(4));
    let writes = [
        ("editor", "vim"),
        ("lang", "zh-TW"),
        ("form", "plain"),
        ("after_run", "stay"),
    ];

    let handles = writes
        .into_iter()
        .map(|(key, value)| {
            let store = Arc::clone(&store);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                store.set(key, value).unwrap();
            })
        })
        .collect::<Vec<_>>();

    for handle in handles {
        handle.join().unwrap();
    }

    assert_eq!(store.get("editor").unwrap(), "vim");
    assert_eq!(store.get("lang").unwrap(), "zh-TW");
    assert_eq!(store.get("form").unwrap(), "plain");
    assert_eq!(store.get("after_run").unwrap(), "stay");
    assert!(root.path().join("config.lock").is_file());
}
