//! Exact npm-mirror ports from Python v0.4 `tests/test_js_deps.py`.

use std::{collections::BTreeMap, fs};

use skit_store::FileConfigStore;
use tempfile::TempDir;

#[test]
fn test_npm_axis_is_independent_of_the_pypi_axis() {
    let root = TempDir::new().unwrap();
    let config = FileConfigStore::new(root.path());

    config.set("mirror.pypi", "tsinghua").unwrap();
    config.set("mirror", "on").unwrap();
    assert_eq!(config.mirror().unwrap().npm, "");

    config.set("mirror.pypi", "off").unwrap();
    config.set("mirror.npm", "npmmirror").unwrap();
    config.set("mirror", "on").unwrap();
    let mirror = config.mirror().unwrap();
    assert!(mirror.enabled);
    assert_eq!(mirror.npm, "https://registry.npmmirror.com");
    assert_eq!(mirror.pypi, "");
}

#[test]
fn test_mirror_npm_round_trips_through_save_and_load() {
    let root = TempDir::new().unwrap();
    let config = FileConfigStore::new(root.path());
    config.set("mirror.npm", "https://my.registry").unwrap();
    config.set("mirror", "on").unwrap();

    let reloaded = FileConfigStore::new(root.path()).mirror().unwrap();
    assert!(reloaded.enabled);
    assert_eq!(reloaded.npm, "https://my.registry");
}

#[test]
fn test_mirror_env_sets_npm_registry_and_defers_to_the_user() {
    let root = TempDir::new().unwrap();
    let config = FileConfigStore::new(root.path());
    config.set("mirror.npm", "npmmirror").unwrap();
    config.set("mirror", "on").unwrap();

    assert_eq!(
        config.mirror_environment(&BTreeMap::new()).unwrap()["NPM_CONFIG_REGISTRY"],
        "https://registry.npmmirror.com"
    );
    for variable in ["NPM_CONFIG_REGISTRY", "npm_config_registry"] {
        let overlay = config
            .mirror_environment(&BTreeMap::from([(
                variable.to_owned(),
                "https://user.registry".to_owned(),
            )]))
            .unwrap();
        assert!(
            !overlay.contains_key("NPM_CONFIG_REGISTRY"),
            "a truthy user {variable} value must win: {overlay:?}"
        );
    }

    let overlay = config
        .mirror_environment(&BTreeMap::from([(
            "NPM_CONFIG_REGISTRY".to_owned(),
            String::new(),
        )]))
        .unwrap();
    assert_eq!(
        overlay["NPM_CONFIG_REGISTRY"],
        "https://registry.npmmirror.com",
        "an empty user value means unset, so the configured npm mirror must still apply"
    );
}

#[test]
fn test_mirror_env_without_npm_url_sets_nothing_npm() {
    let root = TempDir::new().unwrap();
    let config = FileConfigStore::new(root.path());
    config.set("mirror.pypi", "https://p").unwrap();
    config.set("mirror", "on").unwrap();
    assert!(
        !config
            .mirror_environment(&BTreeMap::new())
            .unwrap()
            .contains_key("NPM_CONFIG_REGISTRY")
    );
}

#[test]
fn test_load_mirror_type_hardens_a_hand_edited_npm_value() {
    let root = TempDir::new().unwrap();
    fs::create_dir_all(root.path()).unwrap();
    fs::write(
        root.path().join("config.toml"),
        "[mirror]\nenabled = true\nnpm = 123\n",
    )
    .unwrap();
    let config = FileConfigStore::new(root.path());
    assert_eq!(config.mirror().unwrap().npm, "");
}