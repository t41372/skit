use std::{collections::BTreeMap, fs};

use skit_store::{ConfigError, FileConfigStore, PromptRunner};
use tempfile::TempDir;

#[test]
fn empty_configuration_keeps_the_v040_public_values() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());
    let settings = store.settings().unwrap();

    assert_eq!(settings["lang"], "");
    assert_eq!(settings["editor"], "");
    assert_eq!(settings["form"], "tui");
    assert_eq!(settings["after_run"], "exit");
    assert_eq!(settings["shell.bash_path"], "");
    assert_eq!(settings["js.runner"], "");
    assert!(store.invalid_runner_rows().unwrap().is_empty());
}

#[test]
fn configuration_updates_preserve_comments_and_key_order() {
    let root = TempDir::new().unwrap();
    let path = root.path().join("config.toml");
    fs::write(
        &path,
        r#"# Keep this file header.
future = 7 # Keep the future note.
language = "en" # Keep the language note.

[prompt]
# Keep the runner-list note.
runners_seeded = true
runners = [
  { name = "mine", argv = ["agent", "{{prompt}}"] }, # Keep the runner note.
]
"#,
    )
    .unwrap();
    let store = FileConfigStore::new(root.path());

    store.set("lang", "zh-CN").unwrap();
    store
        .set_runner(
            PromptRunner {
                name: "other".to_owned(),
                argv: vec!["other-agent".to_owned(), "{{prompt}}".to_owned()],
            },
            false,
        )
        .unwrap();

    let text = fs::read_to_string(path).unwrap();
    for comment in [
        "# Keep this file header.",
        "# Keep the future note.",
        "# Keep the language note.",
        "# Keep the runner-list note.",
        "# Keep the runner note.",
    ] {
        assert!(text.contains(comment), "lost {comment}:\n{text}");
    }
    assert!(
        text.find("future = 7").unwrap() < text.find("language = \"zh-CN\"").unwrap(),
        "{text}"
    );
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

#[test]
fn every_scalar_setting_and_mirror_environment_precedence_is_explicit() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());
    assert_eq!(store.config_dir(), root.path());
    for (key, value) in [
        ("lang", "zh-TW"),
        ("editor", "code --wait"),
        ("form", "plain"),
        ("after_run", "stay"),
        ("shell.bash_path", "/bin/bash"),
        ("js.runner", "node"),
    ] {
        store.set(key, value).unwrap();
        assert_eq!(store.get(key).unwrap(), value);
    }
    assert!(matches!(
        store.set("unknown", "value"),
        Err(ConfigError::Invalid(_))
    ));
    assert!(matches!(
        store.set("form", "dialog"),
        Err(ConfigError::Invalid(_))
    ));
    assert!(matches!(
        store.set("mirror", "on"),
        Err(ConfigError::Invalid(_))
    ));

    store
        .set("mirror.pypi", "http://mirror.example/simple/")
        .unwrap();
    store
        .set("mirror.github", "https://mirror.example/releases")
        .unwrap();
    store
        .set("mirror.npm", "http://mirror.example/npm/")
        .unwrap();
    let overlay = store.mirror_environment(&BTreeMap::new()).unwrap();
    assert!(overlay.contains_key("UV_DEFAULT_INDEX"));
    assert!(overlay.contains_key("UV_PYTHON_INSTALL_MIRROR"));
    assert!(overlay.contains_key("NPM_CONFIG_REGISTRY"));

    for base in [
        BTreeMap::from([("UV_INDEX_URL".to_owned(), "user".to_owned())]),
        BTreeMap::from([("NPM_CONFIG_REGISTRY".to_owned(), "user".to_owned())]),
    ] {
        let overlay = store.mirror_environment(&base).unwrap();
        if base.contains_key("UV_INDEX_URL") {
            assert!(!overlay.contains_key("UV_DEFAULT_INDEX"));
        }
        if base.contains_key("NPM_CONFIG_REGISTRY") {
            assert!(!overlay.contains_key("NPM_CONFIG_REGISTRY"));
        }
    }

    store.set("mirror.pypi", "off").unwrap();
    store.set("mirror.npm", "off").unwrap();
    store.set("mirror.github", "off").unwrap();
    assert_eq!(store.settings().unwrap()["mirror.github"], "off");
}

#[test]
fn batch_settings_validate_every_value_before_one_write() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());
    store.set("lang", "en").unwrap();
    let before = fs::read(root.path().join("config.toml")).unwrap();

    let invalid = BTreeMap::from([
        ("lang".to_owned(), "zh-TW".to_owned()),
        ("form".to_owned(), "not-a-form".to_owned()),
    ]);
    assert!(matches!(
        store.set_many(&invalid),
        Err(ConfigError::Invalid(_))
    ));
    assert_eq!(fs::read(root.path().join("config.toml")).unwrap(), before);

    let valid = BTreeMap::from([
        ("lang".to_owned(), "zh-TW".to_owned()),
        ("mirror".to_owned(), "on".to_owned()),
        ("mirror.pypi".to_owned(), "tsinghua".to_owned()),
    ]);
    store.set_many(&valid).unwrap();
    let settings = store.settings().unwrap();
    assert_eq!(settings["lang"], "zh-TW");
    assert_eq!(settings["mirror"], "on");
    assert_eq!(settings["mirror.pypi"], "tsinghua");
}

#[test]
fn runner_validation_duplicate_policy_and_configuration_failures_are_typed() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());
    for runner in [
        PromptRunner {
            name: String::new(),
            argv: vec!["agent".to_owned(), "{{prompt}}".to_owned()],
        },
        PromptRunner {
            name: "empty".to_owned(),
            argv: Vec::new(),
        },
        PromptRunner {
            name: "program-slot".to_owned(),
            argv: vec!["{{prompt}}".to_owned()],
        },
        PromptRunner {
            name: "twice".to_owned(),
            argv: vec!["agent".to_owned(), "{{prompt}}{{prompt}}".to_owned()],
        },
    ] {
        assert!(matches!(
            store.set_runner(runner, false),
            Err(ConfigError::Invalid(_))
        ));
    }

    let runner = PromptRunner {
        name: "custom".to_owned(),
        argv: vec!["agent".to_owned(), "{{prompt}}".to_owned()],
    };
    store.set_runner(runner.clone(), false).unwrap();
    assert!(matches!(
        store.set_runner(runner.clone(), false),
        Err(ConfigError::Invalid(_))
    ));
    store.set_runner(runner, true).unwrap();

    fs::write(root.path().join("config.toml"), "not = [valid").unwrap();
    assert!(matches!(store.settings(), Err(ConfigError::Parse { .. })));

    let file_root = root.path().join("file-root");
    fs::write(&file_root, "file").unwrap();
    assert!(matches!(
        FileConfigStore::new(&file_root).settings(),
        Err(ConfigError::Io { .. })
    ));
}

#[test]
fn runner_mutations_preserve_malformed_and_future_rows_until_row_repair() {
    let root = TempDir::new().unwrap();
    let path = root.path().join("config.toml");
    fs::write(
        &path,
        r#"[prompt]
runners_seeded = true
runners = [
  { name = "valid", argv = ["old", "{{prompt}}"], future = 7 },
  { name = "broken", argv = ["no marker"], keep = "yes" },
  "future-shape",
]
"#,
    )
    .unwrap();
    let store = FileConfigStore::new(root.path());

    store
        .set_runner(
            PromptRunner {
                name: "valid".to_owned(),
                argv: vec!["new".to_owned(), "{{prompt}}".to_owned()],
            },
            true,
        )
        .unwrap();
    let document = fs::read_to_string(&path)
        .unwrap()
        .parse::<toml::Table>()
        .unwrap();
    let rows = document["prompt"]["runners"].as_array().unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0]["future"].as_integer(), Some(7));
    assert_eq!(rows[1]["keep"].as_str(), Some("yes"));
    assert_eq!(rows[2].as_str(), Some("future-shape"));

    assert!(store.remove_runner("valid").unwrap());
    assert_eq!(store.runner_rows().unwrap().len(), 2);
    assert!(store.remove_runner_row(1).unwrap());
    let rows = store.runner_rows().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].descriptor, "future-shape");
}

#[test]
fn wrong_table_shapes_are_refused_without_losing_the_existing_file() {
    let root = TempDir::new().unwrap();
    let path = root.path().join("config.toml");
    fs::write(&path, "shell = \"not a table\"\nfuture = 3\n").unwrap();
    let store = FileConfigStore::new(root.path());
    assert!(matches!(
        store.set("shell.bash_path", "/bin/bash"),
        Err(ConfigError::Invalid(_))
    ));
    assert_eq!(
        fs::read_to_string(path).unwrap(),
        "shell = \"not a table\"\nfuture = 3\n"
    );
}

#[test]
fn partial_and_custom_mirror_rows_have_stable_public_tokens() {
    let root = TempDir::new().unwrap();
    let path = root.path().join("config.toml");
    let store = FileConfigStore::new(root.path());

    fs::write(
        &path,
        "[mirror]\nenabled = true\nnpm = \"https://npm.example\"\n",
    )
    .unwrap();
    let settings = store.settings().unwrap();
    assert_eq!(settings["mirror"], "on");
    assert_eq!(settings["mirror.github"], "off");

    fs::write(
        &path,
        concat!(
            "[mirror]\n",
            "enabled = true\n",
            "python_install = \"https://custom.example/astral-sh/python-build-standalone/\"\n",
            "uv_binary = \"https://custom.example/astral-sh/uv\"\n",
        ),
    )
    .unwrap();
    assert_eq!(
        store.settings().unwrap()["mirror.github"],
        "https://custom.example"
    );

    fs::write(
        &path,
        concat!(
            "[mirror]\n",
            "enabled = true\n",
            "python_install = \"https://custom.example/python/\"\n",
            "uv_binary = \"https://different.example/uv\"\n",
        ),
    )
    .unwrap();
    assert_eq!(store.settings().unwrap()["mirror.github"], "custom");
}

#[test]
fn reading_an_unknown_key_is_a_typed_refusal() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());

    let error = store.get("colour").unwrap_err();

    assert!(matches!(error, ConfigError::Invalid(_)));
    assert!(error.to_string().contains("colour"));
    assert_eq!(store.get("after_run").unwrap(), "exit");
}

#[test]
fn a_raw_row_index_addresses_the_rows_the_reader_reported() {
    let root = TempDir::new().unwrap();
    // A hand-written list with one malformed row. `--row` must address exactly these.
    fs::write(
        root.path().join("config.toml"),
        concat!(
            "[prompt]\n",
            "[[prompt.runners]]\n",
            "name = \"mine\"\n",
            "argv = [\"mytool\", \"{{prompt}}\"]\n",
            "[[prompt.runners]]\n",
            "bogus = 1\n",
        ),
    )
    .unwrap();
    let store = FileConfigStore::new(root.path());
    assert_eq!(store.runner_rows().unwrap().len(), 2);
    assert_eq!(store.invalid_runner_rows().unwrap(), ["row 2"]);

    // An index past the end, and index zero, address no stored row.
    assert!(!store.remove_runner_row(3).unwrap());
    assert!(!store.remove_runner_row(0).unwrap());
    assert_eq!(store.runner_rows().unwrap().len(), 2);

    assert!(store.remove_runner_row(2).unwrap());

    let rows = store.runner_rows().unwrap();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].descriptor.contains("mine"), "{:?}", rows[0]);
    assert!(store.invalid_runner_rows().unwrap().is_empty());
}

#[test]
fn a_raw_row_index_outside_the_stored_list_changes_nothing() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());
    // A fresh file declares no rows, so no index addresses anything.
    assert!(store.runner_rows().unwrap().is_empty());

    assert!(!store.remove_runner_row(1).unwrap());
    assert!(!store.remove_runner_row(0).unwrap());

    assert!(store.runner_rows().unwrap().is_empty());
    assert!(
        !root.path().join("config.toml").exists() || {
            let text = fs::read_to_string(root.path().join("config.toml")).unwrap();
            !text.contains("runners_seeded")
        }
    );
}

#[test]
fn seeding_keeps_prompt_runners_the_user_already_wrote() {
    let root = TempDir::new().unwrap();
    fs::write(
        root.path().join("config.toml"),
        concat!(
            "[prompt]\n",
            "keep = \"unknown\"\n",
            "[[prompt.runners]]\n",
            "name = \"mine\"\n",
            "argv = [\"mytool\", \"{{prompt}}\"]\n",
        ),
    )
    .unwrap();
    let store = FileConfigStore::new(root.path());

    store
        .set_runner(
            PromptRunner {
                name: "other".to_owned(),
                argv: vec!["othertool".to_owned(), "{{prompt}}".to_owned()],
            },
            false,
        )
        .unwrap();

    let names = store
        .runners()
        .unwrap()
        .into_iter()
        .map(|runner| runner.name)
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        ["mine", "other"],
        "the seeds must not replace user rows"
    );
    let text = fs::read_to_string(root.path().join("config.toml")).unwrap();
    assert!(text.contains("keep = \"unknown\""), "{text}");
}

#[test]
fn a_future_prompt_shape_is_refused_instead_of_replaced() {
    let root = TempDir::new().unwrap();
    fs::write(
        root.path().join("config.toml"),
        "prompt = \"a future scalar shape\"\n",
    )
    .unwrap();
    let store = FileConfigStore::new(root.path());

    let error = store
        .set_runner(
            PromptRunner {
                name: "mine".to_owned(),
                argv: vec!["mytool".to_owned(), "{{prompt}}".to_owned()],
            },
            false,
        )
        .unwrap_err();

    assert!(matches!(error, ConfigError::Invalid(_)));
    assert_eq!(
        fs::read_to_string(root.path().join("config.toml")).unwrap(),
        "prompt = \"a future scalar shape\"\n"
    );
}

#[test]
fn a_prompt_table_without_runners_gains_an_empty_list_before_a_write() {
    let root = TempDir::new().unwrap();
    // A hand-edited file can claim the seed already ran while the list is absent.
    fs::write(
        root.path().join("config.toml"),
        "[prompt]\nkeep = \"unknown\"\nrunners_seeded = true\n",
    )
    .unwrap();
    let store = FileConfigStore::new(root.path());

    store
        .set_runner(
            PromptRunner {
                name: "demo".to_owned(),
                argv: vec!["demo".to_owned(), "{{prompt}}".to_owned()],
            },
            false,
        )
        .unwrap();

    let text = fs::read_to_string(root.path().join("config.toml")).unwrap();
    assert!(text.contains("keep = \"unknown\""));
    assert!(
        store
            .runner_rows()
            .unwrap()
            .iter()
            .any(|row| { row.descriptor.contains("demo") })
    );
}
