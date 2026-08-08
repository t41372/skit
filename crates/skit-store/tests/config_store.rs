use std::{collections::BTreeMap, fs};

use skit_store::{ConfigError, FileConfigStore};
use tempfile::TempDir;

#[test]
fn empty_configuration_keeps_the_v040_public_values() {
    let root = TempDir::new().unwrap();
    let settings = FileConfigStore::new(root.path()).settings().unwrap();

    assert_eq!(settings["lang"], "");
    assert_eq!(settings["editor"], "");
    assert_eq!(settings["form"], "tui");
    assert_eq!(settings["after_run"], "exit");
    assert_eq!(settings["shell.bash_path"], "");
    assert_eq!(settings["js.runner"], "");
}

#[test]
fn mirror_axes_round_trip_as_stable_tokens_and_preserve_paused_urls() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());
    let defaults = store.settings().unwrap();
    assert_eq!(defaults["mirror"], "off");
    assert_eq!(defaults["mirror.pypi"], "off");
    assert_eq!(defaults["mirror.github"], "off");
    assert_eq!(defaults["mirror.npm"], "off");

    store.set("mirror.pypi", "tsinghua").unwrap();
    store.set("mirror.github", "nju").unwrap();
    store.set("mirror.npm", "npmmirror").unwrap();
    let settings = store.settings().unwrap();
    assert_eq!(settings["mirror"], "on");
    assert_eq!(settings["mirror.pypi"], "tsinghua");
    assert_eq!(settings["mirror.github"], "nju");
    assert_eq!(settings["mirror.npm"], "npmmirror");

    store.set("mirror", "off").unwrap();
    let mirror = store.mirror().unwrap();
    assert!(!mirror.enabled);
    assert!(!mirror.pypi.is_empty());
    assert!(!mirror.python_install.is_empty());
    assert!(!mirror.uv_binary.is_empty());
    assert!(!mirror.npm.is_empty());
    assert_eq!(store.settings().unwrap()["mirror"], "off");
    store.set("mirror", "on").unwrap();
    assert!(store.mirror().unwrap().enabled);
}

#[test]
fn custom_mirror_urls_are_validated_and_github_expands_to_both_release_axes() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());
    store
        .set("mirror.github", "https://mirror.example/releases/")
        .unwrap();
    let mirror = store.mirror().unwrap();
    assert_eq!(
        mirror.python_install,
        "https://mirror.example/releases/astral-sh/python-build-standalone/"
    );
    assert_eq!(
        mirror.uv_binary,
        "https://mirror.example/releases/astral-sh/uv"
    );
    assert_eq!(
        store.settings().unwrap()["mirror.github"],
        "https://mirror.example/releases"
    );

    for (key, value) in [
        ("mirror.pypi", "not-a-url"),
        ("mirror.github", "http://unsafe.example"),
        ("mirror.npm", "https://space.example/a b"),
    ] {
        assert!(matches!(
            store.set(key, value),
            Err(ConfigError::Invalid(_))
        ));
    }
}

#[test]
fn mirror_environment_is_a_child_only_overlay_and_defers_to_user_choices() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());
    store.set("mirror.pypi", "tsinghua").unwrap();
    store.set("mirror.github", "nju").unwrap();
    store.set("mirror.npm", "npmmirror").unwrap();
    let base = BTreeMap::from([
        (
            "UV_DEFAULT_INDEX".to_owned(),
            "https://user.example/simple".to_owned(),
        ),
        (
            "npm_config_registry".to_owned(),
            "https://user.example/npm".to_owned(),
        ),
    ]);
    let overlay = store.mirror_environment(&base).unwrap();
    assert!(!overlay.contains_key("UV_DEFAULT_INDEX"));
    assert!(!overlay.contains_key("NPM_CONFIG_REGISTRY"));
    assert_eq!(
        overlay["UV_PYTHON_INSTALL_MIRROR"],
        "https://mirror.nju.edu.cn/github-release/astral-sh/python-build-standalone/"
    );
    assert_eq!(base["UV_DEFAULT_INDEX"], "https://user.example/simple");

    store.set("mirror", "off").unwrap();
    assert!(
        store
            .mirror_environment(&BTreeMap::new())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn mirror_updates_keep_unknown_configuration_fields() {
    let root = TempDir::new().unwrap();
    fs::write(
        root.path().join("config.toml"),
        "future = 3\n[mirror]\nfuture_axis = \"keep\"\n",
    )
    .unwrap();
    let store = FileConfigStore::new(root.path());
    store.set("mirror.npm", "npmmirror").unwrap();
    let text = fs::read_to_string(root.path().join("config.toml")).unwrap();
    assert!(text.contains("future = 3"));
    assert!(text.contains("future_axis = \"keep\""));
}

#[test]
fn malformed_runner_rows_are_reported_but_do_not_hide_valid_siblings() {
    let root = TempDir::new().unwrap();
    fs::write(
        root.path().join("config.toml"),
        r#"
[prompt]
runners_seeded = true
runners = [
  { name = "valid", argv = ["agent", "{{prompt}}"] },
  { name = "broken", argv = ["agent"] },
  "not-a-table",
]
"#,
    )
    .unwrap();
    let store = FileConfigStore::new(root.path());
    assert_eq!(store.runners().unwrap()[0].name, "valid");
    assert_eq!(store.invalid_runner_rows().unwrap(), ["broken", "row 3"]);
}
