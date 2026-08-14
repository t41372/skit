//! Public-API strengthening around Python v0.4 mirror-axis persistence and transaction contracts.
//!
//! The three axes are independent. Fresh URL configuration auto-enables; clearing the final URL
//! disables; a paused configuration stays paused while edited so saved siblings are never silently
//! resurrected. Custom URL validation uses one-token http(s), with GitHub release bases https-only.
//!
//! These tests are intentionally `rust_additive_*`: the frozen Python names are owned by the exact
//! canonical files used by `port_test_config_manifest.rs`. Keeping these assertions under invented
//! `test_*` names would falsely make implementation-authored strengthening look like migrated oracles.

use std::collections::BTreeMap;

use skit_store::{ConfigError, FileConfigStore};
use tempfile::TempDir;

const TSINGHUA: &str = "https://pypi.tuna.tsinghua.edu.cn/simple";
const USTC: &str = "https://pypi.mirrors.ustc.edu.cn/simple";
const NPMMIRROR: &str = "https://registry.npmmirror.com";
const NJU_BASE: &str = "https://mirror.nju.edu.cn/github-release";
const NJU_PYTHON: &str =
    "https://mirror.nju.edu.cn/github-release/astral-sh/python-build-standalone/";
const NJU_UV: &str = "https://mirror.nju.edu.cn/github-release/astral-sh/uv";

#[test]
fn rust_additive_fresh_first_axis_auto_enables_and_preset_roundtrips_by_name() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());

    store.set("mirror.pypi", "tsinghua").unwrap();

    assert_eq!(store.get("mirror").unwrap(), "on");
    assert_eq!(store.get("mirror.pypi").unwrap(), "tsinghua");
    let mirror = store.mirror().unwrap();
    assert!(mirror.enabled);
    assert_eq!(mirror.pypi, TSINGHUA);
    assert_eq!(mirror.npm, "");
    assert_eq!(mirror.python_install, "");
    assert_eq!(mirror.uv_binary, "");
}

#[test]
fn rust_additive_axes_are_independent_and_setting_one_preserves_existing_siblings() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());

    store.set("mirror.pypi", "ustc").unwrap();
    store.set("mirror.npm", "npmmirror").unwrap();

    let mirror = store.mirror().unwrap();
    assert!(mirror.enabled);
    assert_eq!(mirror.pypi, USTC);
    assert_eq!(mirror.npm, NPMMIRROR);
    assert_eq!(mirror.python_install, "");
    assert_eq!(mirror.uv_binary, "");
}

#[test]
fn rust_additive_github_preset_expands_one_base_into_both_release_vectors() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());

    store.set("mirror.github", "nju").unwrap();

    assert_eq!(store.get("mirror.github").unwrap(), "nju");
    let mirror = store.mirror().unwrap();
    assert_eq!(mirror.python_install, NJU_PYTHON);
    assert_eq!(mirror.uv_binary, NJU_UV);
}

#[test]
fn rust_additive_custom_github_base_roundtrips_without_trailing_slash() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());
    let base = "https://my.mirror/gh/";

    store.set("mirror.github", base).unwrap();

    assert_eq!(store.get("mirror.github").unwrap(), "https://my.mirror/gh");
    let mirror = store.mirror().unwrap();
    assert_eq!(
        mirror.python_install,
        "https://my.mirror/gh/astral-sh/python-build-standalone/"
    );
    assert_eq!(mirror.uv_binary, "https://my.mirror/gh/astral-sh/uv");
}

#[test]
fn rust_additive_pypi_and_npm_accept_pastable_http_custom_urls_and_trim_one_trailing_slash() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());

    store
        .set("mirror.pypi", "http://corp.internal/simple/")
        .unwrap();
    store
        .set("mirror.npm", "http://corp.internal/npm/")
        .unwrap();

    assert_eq!(store.mirror().unwrap().pypi, "http://corp.internal/simple");
    assert_eq!(store.mirror().unwrap().npm, "http://corp.internal/npm");
}

#[test]
fn rust_additive_github_custom_base_rejects_http_even_though_other_axes_accept_it() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());

    let error = store
        .set("mirror.github", "http://corp.internal/gh")
        .unwrap_err();
    assert!(matches!(error, ConfigError::Usage(_)));
    assert_eq!(store.get("mirror.github").unwrap(), "off");
}

#[test]
fn rust_additive_custom_axis_gate_rejects_non_urls_whitespace_and_display_prose_without_writing() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());

    for bad in [
        "ftp://x",
        "https://a b/x",
        "https://a\tb",
        "https://a\nb",
        "https://a·b",
        "pypi=tsinghua · npm=off",
    ] {
        let error = store.set("mirror.pypi", bad).unwrap_err();
        assert!(matches!(error, ConfigError::Usage(_)), "{bad:?}: {error}");
        assert_eq!(store.get("mirror.pypi").unwrap(), "off");
    }
}

#[test]
fn rust_additive_clearing_one_of_several_axes_keeps_master_enabled() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());
    store.set("mirror.pypi", "tsinghua").unwrap();
    store.set("mirror.npm", "npmmirror").unwrap();

    store.set("mirror.pypi", "off").unwrap();

    let mirror = store.mirror().unwrap();
    assert!(mirror.enabled);
    assert_eq!(mirror.pypi, "");
    assert_eq!(mirror.npm, NPMMIRROR);
}

#[test]
fn rust_additive_clearing_the_last_axis_disables_the_master() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());
    store.set("mirror.npm", "npmmirror").unwrap();

    store.set("mirror.npm", "off").unwrap();

    let mirror = store.mirror().unwrap();
    assert!(!mirror.enabled);
    assert_eq!(mirror.npm, "");
    assert_eq!(store.get("mirror").unwrap(), "off");
}

#[test]
fn rust_additive_paused_axis_edit_stays_paused_and_preserves_other_saved_urls() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());
    store.set("mirror.pypi", "tsinghua").unwrap();
    store.set("mirror.github", "nju").unwrap();
    store.set("mirror.npm", "npmmirror").unwrap();
    store.set("mirror", "off").unwrap();

    store.set("mirror.npm", "https://new/npm").unwrap();

    let mirror = store.mirror().unwrap();
    assert!(!mirror.enabled);
    assert_eq!(mirror.pypi, TSINGHUA);
    assert_eq!(mirror.python_install, NJU_PYTHON);
    assert_eq!(mirror.uv_binary, NJU_UV);
    assert_eq!(mirror.npm, "https://new/npm");
    assert_eq!(store.get("mirror").unwrap(), "off");
}

#[test]
fn rust_additive_master_enable_refuses_when_no_urls_are_saved() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());

    let error = store.set("mirror", "on").unwrap_err();
    assert!(matches!(error, ConfigError::Usage(_)));
    assert_eq!(store.get("mirror").unwrap(), "off");
}

#[test]
fn rust_additive_master_reenable_restores_saved_urls_without_rewriting_axes() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());
    store.set("mirror.pypi", "tsinghua").unwrap();
    store.set("mirror.github", "nju").unwrap();
    store.set("mirror", "off").unwrap();
    let saved = store.mirror().unwrap();

    store.set("mirror", "on").unwrap();

    let enabled = store.mirror().unwrap();
    assert!(enabled.enabled);
    assert_eq!(enabled.pypi, saved.pypi);
    assert_eq!(enabled.python_install, saved.python_install);
    assert_eq!(enabled.uv_binary, saved.uv_binary);
}

#[test]
fn rust_additive_each_axis_can_supply_environment_independently() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());

    store.set("mirror.pypi", "tsinghua").unwrap();
    assert_eq!(
        store.mirror_environment(&BTreeMap::new()).unwrap(),
        BTreeMap::from([("UV_DEFAULT_INDEX".to_owned(), TSINGHUA.to_owned())])
    );

    store.set("mirror.pypi", "off").unwrap();
    store.set("mirror.npm", "npmmirror").unwrap();
    assert_eq!(
        store.mirror_environment(&BTreeMap::new()).unwrap(),
        BTreeMap::from([("NPM_CONFIG_REGISTRY".to_owned(), NPMMIRROR.to_owned(),)])
    );
}

#[test]
fn rust_additive_github_preset_base_constant_remains_exact() {
    assert_eq!(NJU_BASE, "https://mirror.nju.edu.cn/github-release");
}
