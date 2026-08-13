//! Frozen mirror-environment contracts from Python v0.4 `tests/test_config.py`.
use std::collections::BTreeMap;
use skit_store::FileConfigStore;
use tempfile::TempDir;

const PYPI: &str = "https://pypi.tuna.tsinghua.edu.cn/simple";
const PYTHON: &str = "https://mirror.nju.edu.cn/github-release/astral-sh/python-build-standalone/";
const UV: &str = "https://mirror.nju.edu.cn/github-release/astral-sh/uv";
const NPM: &str = "https://registry.npmmirror.com";

fn full() -> (TempDir, FileConfigStore) {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());
    let settings = BTreeMap::from([
        ("mirror.pypi".to_owned(), "tsinghua".to_owned()),
        ("mirror.github".to_owned(), "nju".to_owned()),
        ("mirror.npm".to_owned(), "npmmirror".to_owned()),
    ]);
    store.set_many(&settings).unwrap();
    (root, store)
}

#[test]
fn test_mirror_env_overlays_all_vectors() {
    let (_root, store) = full();
    let env = store.mirror_environment(&BTreeMap::new()).unwrap();
    assert_eq!(env.get("UV_DEFAULT_INDEX").map(String::as_str), Some(PYPI));
    assert_eq!(env.get("UV_PYTHON_INSTALL_MIRROR").map(String::as_str), Some(PYTHON));
    assert_eq!(env.get("NPM_CONFIG_REGISTRY").map(String::as_str), Some(NPM));
    assert_eq!(store.mirror().unwrap().uv_binary, UV);
}

#[test]
fn test_mirror_env_defers_to_user_index() {
    for key in ["UV_DEFAULT_INDEX", "UV_INDEX_URL"] {
        let (_root, store) = full();
        let env = store.mirror_environment(&BTreeMap::from([(key.to_owned(), "https://mine/simple".to_owned())])).unwrap();
        assert!(!env.contains_key("UV_DEFAULT_INDEX"), "{key}");
        assert!(env.contains_key("UV_PYTHON_INSTALL_MIRROR"), "{key}");
    }
}

#[test]
fn rust_additive_mirror_env_defers_to_uv_default_index() {
    let (_root, store) = full();
    assert!(!store.mirror_environment(&BTreeMap::from([("UV_DEFAULT_INDEX".into(), "x".into())])).unwrap().contains_key("UV_DEFAULT_INDEX"));
}
#[test]
fn rust_additive_mirror_env_defers_to_uv_index_url() {
    let (_root, store) = full();
    assert!(!store.mirror_environment(&BTreeMap::from([("UV_INDEX_URL".into(), "x".into())])).unwrap().contains_key("UV_DEFAULT_INDEX"));
}

#[test]
fn test_mirror_env_defers_to_user_python_mirror() {
    let (_root, store) = full();
    let env = store.mirror_environment(&BTreeMap::from([("UV_PYTHON_INSTALL_MIRROR".into(), "https://mine/py/".into())])).unwrap();
    assert!(!env.contains_key("UV_PYTHON_INSTALL_MIRROR"));
    assert!(env.contains_key("UV_DEFAULT_INDEX"));
}

#[test]
fn test_mirror_env_does_not_defer_on_extra_index_url() {
    let (_root, store) = full();
    let env = store.mirror_environment(&BTreeMap::from([("UV_EXTRA_INDEX_URL".into(), "https://x".into())])).unwrap();
    assert_eq!(env.get("UV_DEFAULT_INDEX").map(String::as_str), Some(PYPI));
}

#[test]
fn test_mirror_env_does_not_defer_on_uv_index() {
    let (_root, store) = full();
    let env = store.mirror_environment(&BTreeMap::from([("UV_INDEX".into(), "https://x".into())])).unwrap();
    assert_eq!(env.get("UV_DEFAULT_INDEX").map(String::as_str), Some(PYPI));
}

#[test]
fn test_mirror_env_injects_when_index_env_blank() {
    let (_root, store) = full();
    let env = store.mirror_environment(&BTreeMap::from([("UV_INDEX_URL".into(), String::new())])).unwrap();
    assert_eq!(env.get("UV_DEFAULT_INDEX").map(String::as_str), Some(PYPI));
}

#[test]
fn test_mirror_env_injects_when_python_mirror_blank() {
    let (_root, store) = full();
    let env = store.mirror_environment(&BTreeMap::from([("UV_PYTHON_INSTALL_MIRROR".into(), String::new())])).unwrap();
    assert_eq!(env.get("UV_PYTHON_INSTALL_MIRROR").map(String::as_str), Some(PYTHON));
}

#[test]
fn test_disable_keeps_urls_but_turns_off() {
    let (_root, store) = full();
    store.set("mirror", "off").unwrap();
    let mirror = store.mirror().unwrap();
    assert!(!mirror.enabled);
    assert_eq!(mirror.pypi, PYPI);
    assert_eq!(mirror.uv_binary, UV);
    assert!(store.mirror_environment(&BTreeMap::new()).unwrap().is_empty());
}
