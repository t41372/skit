use std::{collections::BTreeMap, fs, path::Path};

use skit_store::{ConfigError, FileConfigStore, PromptRunner, expand_user_path};
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
fn user_path_expansion_is_shared_by_every_bash_path_door() {
    let home = expand_user_path(Path::new("~"));
    let child = expand_user_path(Path::new("~/bin/bash"));
    if home == Path::new("~") {
        assert_eq!(child, Path::new("~/bin/bash"));
    } else {
        assert_eq!(child, home.join("bin/bash"));
    }
    assert_eq!(
        expand_user_path(Path::new("relative/~/bash")),
        Path::new("relative/~/bash")
    );
}

#[test]
fn configuration_writes_share_the_v040_cross_process_lock_path() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());

    store.set("form", "plain").unwrap();

    assert!(root.path().join("config.lock").is_file());
    assert!(!root.path().join(".config.lock").exists());
}

#[test]
fn hand_edited_scalar_settings_project_v040_effective_values_without_rewriting() {
    let root = TempDir::new().unwrap();
    let path = root.path().join("config.toml");
    let source = concat!(
        "# This file belongs to the user.\n",
        "language = \"fr-FR\"\n",
        "editor = 9\n",
        "form = \"dialog\"\n",
        "after_run = \"loop\"\n",
        "future = \"keep\"\n",
        "shell = \"hand-written future shape\"\n",
        "js = [\"hand-written\", \"future shape\"]\n",
    );
    fs::write(&path, source).unwrap();
    let store = FileConfigStore::new(root.path());

    let settings = store.settings().unwrap();
    assert_eq!(settings["lang"], "");
    assert_eq!(settings["editor"], "");
    assert_eq!(settings["form"], "tui");
    assert_eq!(settings["after_run"], "exit");
    assert_eq!(settings["shell.bash_path"], "");
    assert_eq!(settings["js.runner"], "");
    assert_eq!(fs::read_to_string(&path).unwrap(), source);
    assert!(!root.path().join("config.toml.bak").exists());
}

#[test]
fn language_values_keep_the_v040_supported_families_and_canonical_spelling() {
    let root = TempDir::new().unwrap();
    let path = root.path().join("config.toml");
    let store = FileConfigStore::new(root.path());

    for (input, stored) in [
        ("zh_tw.UTF-8", "zh-TW"),
        ("EN", "en"),
        ("en-xa", "en-XA"),
        ("zh_hant_hk", "zh-Hant-HK"),
        ("x-PSEUDO", "x-pseudo"),
    ] {
        store.set("lang", input).unwrap();
        assert_eq!(store.get("lang").unwrap(), stored);
        let document = fs::read_to_string(&path)
            .unwrap()
            .parse::<toml::Table>()
            .unwrap();
        assert_eq!(document["language"].as_str(), Some(stored));
    }

    let before = fs::read(&path).unwrap();
    assert!(matches!(
        store.set("lang", "fr-FR"),
        Err(ConfigError::Usage(_))
    ));
    assert_eq!(fs::read(&path).unwrap(), before);

    store.set("lang", "AuTo").unwrap();
    let document = fs::read_to_string(&path)
        .unwrap()
        .parse::<toml::Table>()
        .unwrap();
    assert!(!document.contains_key("language"));
    assert_eq!(store.get("lang").unwrap(), "");

    store.set("lang", "zh-CN").unwrap();
    store.set("lang", "").unwrap();
    let document = fs::read_to_string(&path)
        .unwrap()
        .parse::<toml::Table>()
        .unwrap();
    assert!(!document.contains_key("language"));
    assert_eq!(store.get("lang").unwrap(), "");
}

#[test]
fn hand_edited_supported_language_spelling_is_canonical_only_in_the_projection() {
    let root = TempDir::new().unwrap();
    let path = root.path().join("config.toml");
    let source = "language = \"zh_tw.UTF-8\" # keep bytes\nfuture = 7\n";
    fs::write(&path, source).unwrap();
    let store = FileConfigStore::new(root.path());

    assert_eq!(store.get("lang").unwrap(), "zh-TW");
    assert_eq!(fs::read_to_string(path).unwrap(), source);
}

#[test]
fn editor_and_bash_path_trim_on_write_and_whitespace_clears_only_the_target() {
    let root = TempDir::new().unwrap();
    let path = root.path().join("config.toml");
    let bash = root.path().join("bash");
    fs::write(&bash, "").unwrap();
    fs::write(
        &path,
        "future = 7\n[shell]\nother = \"keep\"\n[js]\nother = \"keep too\"\n",
    )
    .unwrap();
    let store = FileConfigStore::new(root.path());

    store.set("editor", "  code --wait  ").unwrap();
    store
        .set("shell.bash_path", &format!("  {}  ", bash.display()))
        .unwrap();
    assert_eq!(store.get("editor").unwrap(), "code --wait");
    assert_eq!(
        store.get("shell.bash_path").unwrap(),
        bash.display().to_string()
    );

    store.set("editor", "   ").unwrap();
    store.set("shell.bash_path", " \t ").unwrap();
    let document = fs::read_to_string(&path)
        .unwrap()
        .parse::<toml::Table>()
        .unwrap();
    assert!(!document.contains_key("editor"));
    assert!(
        !document["shell"]
            .as_table()
            .unwrap()
            .contains_key("bash_path")
    );
    assert_eq!(document["shell"]["other"].as_str(), Some("keep"));
    assert_eq!(document["js"]["other"].as_str(), Some("keep too"));
    assert_eq!(document["future"].as_integer(), Some(7));
}

#[test]
fn clearing_the_last_nested_scalar_removes_only_its_now_empty_section() {
    let root = TempDir::new().unwrap();
    let path = root.path().join("config.toml");
    let bash = root.path().join("bash");
    fs::write(&bash, "").unwrap();
    let store = FileConfigStore::new(root.path());

    store
        .set("shell.bash_path", bash.to_str().unwrap())
        .unwrap();
    store.set("js.runner", "deno").unwrap();
    store.set("shell.bash_path", "").unwrap();
    store.set("js.runner", " \t ").unwrap();

    let document = fs::read_to_string(path)
        .unwrap()
        .parse::<toml::Table>()
        .unwrap();
    assert!(!document.contains_key("shell"));
    assert!(!document.contains_key("js"));
}

#[test]
fn low_level_bash_path_persists_trimmed_values_without_frontend_policy() {
    let root = TempDir::new().unwrap();
    let path = root.path().join("config.toml");
    let hand_edited = root.path().join("hand-edited-missing");
    let source = format!(
        concat!(
            "future = \"keep\" # preserve this comment\n",
            "[shell]\n",
            "other = \"keep too\"\n",
            "bash_path = {:?}\n",
        ),
        hand_edited.display().to_string(),
    );
    fs::write(&path, &source).unwrap();
    let store = FileConfigStore::new(root.path());

    // A read projects the hand-edited string without checking the live filesystem or rewriting.
    assert_eq!(
        store.get("shell.bash_path").unwrap(),
        hand_edited.display().to_string()
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), source);
    assert!(!root.path().join("config.toml.bak").exists());

    let missing = root.path().join("missing");
    let directory = root.path().join("directory");
    fs::create_dir(&directory).unwrap();
    for value in [&missing, &directory] {
        store
            .set("shell.bash_path", &format!("  {}  ", value.display()))
            .unwrap();
        assert_eq!(
            store.get("shell.bash_path").unwrap(),
            value.display().to_string()
        );
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("future = \"keep\" # preserve this comment"));
        assert!(text.contains("other = \"keep too\""));
    }

    store.set("shell.bash_path", " \t ").unwrap();
    assert_eq!(store.get("shell.bash_path").unwrap(), "");
    let text = fs::read_to_string(&path).unwrap();
    let document = text.parse::<toml::Table>().unwrap();
    assert!(
        !document["shell"]
            .as_table()
            .unwrap()
            .contains_key("bash_path")
    );
    assert_eq!(document["shell"]["other"].as_str(), Some("keep too"));
    assert_eq!(document["future"].as_str(), Some("keep"));
    assert!(text.contains("# preserve this comment"));
}

#[test]
fn scalar_nested_sections_can_be_repaired_without_losing_unrelated_data() {
    let root = TempDir::new().unwrap();
    let path = root.path().join("config.toml");
    let bash = root.path().join("bash");
    fs::write(&bash, "").unwrap();
    fs::write(
        &path,
        concat!(
            "future = 7 # keep root data\n",
            "shell = \"future shell shape\"\n",
            "js = 42\n",
        ),
    )
    .unwrap();
    let store = FileConfigStore::new(root.path());

    store
        .set("shell.bash_path", bash.to_str().unwrap())
        .unwrap();
    store.set("js.runner", "bun").unwrap();

    let text = fs::read_to_string(&path).unwrap();
    let document = text.parse::<toml::Table>().unwrap();
    assert_eq!(document["future"].as_integer(), Some(7));
    assert_eq!(document["shell"]["bash_path"].as_str(), bash.to_str());
    assert_eq!(document["js"]["runner"].as_str(), Some("bun"));
    assert!(text.contains("# keep root data"));
}

#[test]
fn a_scalar_mirror_section_is_repaired_without_losing_root_extensions() {
    let root = TempDir::new().unwrap();
    let path = root.path().join("config.toml");
    fs::write(
        &path,
        "future = \"keep\" # keep extension\nmirror = \"old future shape\"\n",
    )
    .unwrap();
    let store = FileConfigStore::new(root.path());

    store.set("mirror.pypi", "tsinghua").unwrap();

    let text = fs::read_to_string(path).unwrap();
    let document = text.parse::<toml::Table>().unwrap();
    assert_eq!(document["future"].as_str(), Some("keep"));
    assert_eq!(document["mirror"]["enabled"].as_bool(), Some(true));
    assert_eq!(
        document["mirror"]["pypi"].as_str(),
        Some("https://pypi.tuna.tsinghua.edu.cn/simple")
    );
    assert!(text.contains("# keep extension"));
}

#[test]
fn malformed_toml_reads_defaults_without_rewriting_or_creating_a_backup() {
    let root = TempDir::new().unwrap();
    let path = root.path().join("config.toml");
    let corrupt = b"language = \"zh-TW\"\nthis = [is not valid";
    fs::write(&path, corrupt).unwrap();
    let store = FileConfigStore::new(root.path());

    assert_eq!(store.get("lang").unwrap(), "");
    assert_eq!(store.get("form").unwrap(), "tui");
    assert_eq!(store.get("after_run").unwrap(), "exit");
    assert_eq!(fs::read(&path).unwrap(), corrupt);
    assert!(!root.path().join("config.toml.bak").exists());
}

#[test]
fn an_unreadable_config_path_is_a_v040_default_on_read_but_never_overwritten() {
    let root = TempDir::new().unwrap();
    let path = root.path().join("config.toml");
    fs::create_dir(&path).unwrap();
    fs::write(path.join("owned"), "keep").unwrap();
    let store = FileConfigStore::new(root.path());

    assert_eq!(store.get("form").unwrap(), "tui");
    assert_eq!(store.get("lang").unwrap(), "");
    assert!(path.is_dir());

    assert!(matches!(
        store.set("editor", "vim"),
        Err(ConfigError::Io { .. })
    ));
    assert_eq!(fs::read_to_string(path.join("owned")).unwrap(), "keep");
}

#[test]
fn the_first_write_after_malformed_toml_preserves_an_exact_backup_then_repairs() {
    let root = TempDir::new().unwrap();
    let path = root.path().join("config.toml");
    let backup = root.path().join("config.toml.bak");
    let corrupt = "language = \"zh-TW\"\nthis = [is not valid\n使用者資料".as_bytes();
    fs::write(&path, corrupt).unwrap();
    fs::write(&backup, b"older backup").unwrap();
    let store = FileConfigStore::new(root.path());

    let recovery = store
        .set_with_recovery("editor", "vim")
        .unwrap()
        .expect("a malformed write must report its byte-exact backup");

    assert_eq!(fs::read(&backup).unwrap(), corrupt);
    assert_eq!(recovery.path, path);
    assert_eq!(recovery.backup_path.as_deref(), Some(backup.as_path()));
    assert_eq!(store.get("editor").unwrap(), "vim");
    assert_eq!(store.get("lang").unwrap(), "");
}

#[cfg(unix)]
#[test]
fn a_corrupt_backup_keeps_the_v040_source_permissions() {
    use std::os::unix::fs::PermissionsExt as _;

    let root = TempDir::new().unwrap();
    let path = root.path().join("config.toml");
    let backup = root.path().join("config.toml.bak");
    fs::write(&path, "invalid = [toml").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    let store = FileConfigStore::new(root.path());

    store.set("editor", "vim").unwrap();

    assert_eq!(
        fs::metadata(&backup).unwrap().permissions().mode() & 0o777,
        0o640
    );
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o640
    );
}

#[test]
fn non_utf8_toml_is_also_a_read_only_default_and_a_byte_exact_recoverable_write() {
    let root = TempDir::new().unwrap();
    let path = root.path().join("config.toml");
    let backup = root.path().join("config.toml.bak");
    let corrupt = b"language = \"zh-TW\"\ninvalid = \"\xff\"\n";
    fs::write(&path, corrupt).unwrap();
    let store = FileConfigStore::new(root.path());

    assert_eq!(store.get("lang").unwrap(), "");
    assert_eq!(fs::read(&path).unwrap(), corrupt);
    assert!(!backup.exists());

    store.set("form", "plain").unwrap();
    assert_eq!(fs::read(&backup).unwrap(), corrupt);
    assert_eq!(store.get("form").unwrap(), "plain");
}

#[test]
fn a_failed_corrupt_backup_still_applies_the_v040_update() {
    let root = TempDir::new().unwrap();
    let path = root.path().join("config.toml");
    let backup = root.path().join("config.toml.bak");
    let corrupt = b"this = [is not valid";
    fs::write(&path, corrupt).unwrap();
    fs::create_dir(&backup).unwrap();
    let blocker = backup.join("config.toml");
    fs::create_dir(&blocker).unwrap();
    fs::write(blocker.join("owned"), "keep").unwrap();
    let store = FileConfigStore::new(root.path());

    let recovery = store
        .set_with_recovery("editor", "vim")
        .unwrap()
        .expect("a malformed write reports recovery even when its backup fails");

    assert_eq!(recovery.backup_path, None);
    assert_eq!(store.get("editor").unwrap(), "vim");
    assert_eq!(fs::read_to_string(blocker.join("owned")).unwrap(), "keep");
}

#[test]
fn a_backup_directory_preserves_the_corrupt_config_inside_it() {
    let root = TempDir::new().unwrap();
    let path = root.path().join("config.toml");
    let backup = root.path().join("config.toml.bak");
    let corrupt = b"this = [is not valid";
    fs::write(&path, corrupt).unwrap();
    fs::create_dir(&backup).unwrap();
    fs::write(backup.join("owned"), "keep").unwrap();
    let store = FileConfigStore::new(root.path());

    let recovery = store
        .set_with_recovery("editor", "vim")
        .unwrap()
        .expect("a malformed write must report its backup");

    assert_eq!(recovery.backup_path.as_deref(), Some(backup.as_path()));
    assert_eq!(fs::read(backup.join("config.toml")).unwrap(), corrupt);
    assert_eq!(fs::read_to_string(backup.join("owned")).unwrap(), "keep");
    assert_eq!(store.get("editor").unwrap(), "vim");
}

#[cfg(unix)]
#[test]
fn a_backup_directory_symlink_never_writes_outside_the_config_directory() {
    use std::os::unix::fs::symlink;

    let root = TempDir::new().unwrap();
    let config = root.path().join("config");
    let outside = root.path().join("outside");
    fs::create_dir(&config).unwrap();
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("owned"), "keep").unwrap();
    let path = config.join("config.toml");
    fs::write(&path, "this = [is not valid").unwrap();
    symlink(&outside, config.join("config.toml.bak")).unwrap();
    let store = FileConfigStore::new(&config);

    let recovery = store
        .set_with_recovery("editor", "vim")
        .unwrap()
        .expect("a malformed write must report recovery");

    assert_eq!(recovery.backup_path, None);
    assert_eq!(store.get("editor").unwrap(), "vim");
    assert_eq!(fs::read_to_string(outside.join("owned")).unwrap(), "keep");
    assert!(!outside.join("config.toml").exists());
}

#[test]
fn an_invalid_requested_value_does_not_repair_or_back_up_malformed_toml() {
    let root = TempDir::new().unwrap();
    let path = root.path().join("config.toml");
    let corrupt = b"this = [is not valid";
    fs::write(&path, corrupt).unwrap();
    let store = FileConfigStore::new(root.path());

    assert!(matches!(
        store.set("form", "dialog"),
        Err(ConfigError::Usage(_))
    ));
    assert_eq!(fs::read(&path).unwrap(), corrupt);
    assert!(!root.path().join("config.toml.bak").exists());
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
        assert!(matches!(store.set(key, value), Err(ConfigError::Usage(_))));
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
fn mirror_environment_treats_empty_values_as_unset_but_keeps_nonempty_precedence() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());
    store.set("mirror.pypi", "tsinghua").unwrap();
    store.set("mirror.github", "nju").unwrap();
    store.set("mirror.npm", "npmmirror").unwrap();

    let empty = BTreeMap::from([
        ("UV_DEFAULT_INDEX".to_owned(), String::new()),
        ("UV_INDEX_URL".to_owned(), String::new()),
        ("UV_PYTHON_INSTALL_MIRROR".to_owned(), String::new()),
        ("NPM_CONFIG_REGISTRY".to_owned(), String::new()),
        ("npm_config_registry".to_owned(), String::new()),
    ]);
    let overlay = store.mirror_environment(&empty).unwrap();
    assert!(overlay.contains_key("UV_DEFAULT_INDEX"));
    assert!(overlay.contains_key("UV_PYTHON_INSTALL_MIRROR"));
    assert!(overlay.contains_key("NPM_CONFIG_REGISTRY"));

    let mixed = BTreeMap::from([
        ("UV_DEFAULT_INDEX".to_owned(), String::new()),
        ("UV_INDEX_URL".to_owned(), "https://user.example".to_owned()),
        ("NPM_CONFIG_REGISTRY".to_owned(), String::new()),
        (
            "npm_config_registry".to_owned(),
            "https://user.example".to_owned(),
        ),
    ]);
    let overlay = store.mirror_environment(&mixed).unwrap();
    assert!(!overlay.contains_key("UV_DEFAULT_INDEX"));
    assert!(!overlay.contains_key("NPM_CONFIG_REGISTRY"));
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
    assert_eq!(
        store.invalid_runner_rows().unwrap(),
        ["broken", "not-a-table"]
    );
    let rows = store.runner_rows().unwrap();
    assert_eq!(rows[0].index, Some(0));
    assert_eq!(rows[1].reason.as_deref(), Some("prompt-slot-count"));
    assert_eq!(rows[2].reason.as_deref(), Some("row-not-table"));
}

#[test]
fn hand_written_runner_rows_are_authoritative_without_a_seed_marker() {
    let root = TempDir::new().unwrap();
    let path = root.path().join("config.toml");
    let source = concat!(
        "# user-owned runner list\n",
        "[prompt]\n",
        "future = 7\n",
        "[[prompt.runners]]\n",
        "name = \"mine\"\n",
        "argv = [\"mytool\", \"{{prompt}}\"]\n",
    );
    fs::write(&path, source).unwrap();
    let store = FileConfigStore::new(root.path());

    assert_eq!(store.runners().unwrap()[0].name, "mine");
    store.ensure_runners_seeded().unwrap();

    assert_eq!(fs::read_to_string(path).unwrap(), source);
    assert_eq!(store.runner_rows().unwrap()[0].index, Some(0));
}

#[test]
fn malformed_runner_containers_are_visible_and_management_reads_do_not_rewrite_them() {
    for (source, reason, descriptor) in [
        (
            "language = \"zh-TW\"\nprompt = \"garbage\"\n",
            "prompt-section-not-table",
            "prompt",
        ),
        (
            "[prompt]\nrunners = \"garbage\"\n",
            "runners-not-list",
            "prompt.runners",
        ),
    ] {
        let root = TempDir::new().unwrap();
        let path = root.path().join("config.toml");
        fs::write(&path, source).unwrap();
        let store = FileConfigStore::new(root.path());

        assert!(store.runners().unwrap().is_empty());
        let row = store.runner_rows().unwrap().pop().unwrap();
        assert_eq!(row.index, None);
        assert_eq!(row.reason.as_deref(), Some(reason));
        assert_eq!(row.descriptor, descriptor);
        store.ensure_runners_seeded().unwrap();
        assert_eq!(fs::read_to_string(path).unwrap(), source);
    }
}

#[test]
fn runner_validation_keeps_v040_reason_codes_and_duplicate_semantics() {
    let root = TempDir::new().unwrap();
    fs::write(
        root.path().join("config.toml"),
        r#"[prompt]
runners = [
  { name = "good", argv = ["agent", "{{prompt}}"] },
  { name = " same ", argv = ["first", "{{prompt}}"] },
  { name = "same", argv = ["second", "{{prompt}}"] },
  { name = "", argv = ["agent", "{{prompt}}"] },
  { name = "argv", argv = "not-a-list" },
  { name = "empty", argv = ["agent", ""] },
  { name = "binary", argv = ["{{prompt}}"] },
  { name = "slots", argv = ["agent"] },
  { name = "hole", argv = ["agent", "{{other}}", "{{prompt}}"] },
]
"#,
    )
    .unwrap();
    let store = FileConfigStore::new(root.path());

    assert_eq!(
        store
            .runners()
            .unwrap()
            .into_iter()
            .map(|runner| runner.name)
            .collect::<Vec<_>>(),
        ["good", "same"]
    );
    assert_eq!(
        store
            .runner_rows()
            .unwrap()
            .into_iter()
            .map(|row| row.reason)
            .collect::<Vec<_>>(),
        [
            None,
            None,
            Some("duplicate".to_owned()),
            Some("name".to_owned()),
            Some("argv-type".to_owned()),
            Some("empty".to_owned()),
            Some("prompt-in-binary".to_owned()),
            Some("prompt-slot-count".to_owned()),
            Some("stray-hole".to_owned()),
        ]
    );
}

#[test]
fn blank_runner_name_stays_visible_in_the_raw_row_but_not_the_valid_runner_list() {
    let root = TempDir::new().unwrap();
    fs::write(
        root.path().join("config.toml"),
        concat!(
            "[prompt]\n",
            "runners_seeded = true\n",
            "runners = [{ name = \"\", argv = [\"agent\", \"{{prompt}}\"] }]\n",
        ),
    )
    .unwrap();
    let store = FileConfigStore::new(root.path());

    assert!(store.runners().unwrap().is_empty());
    let row = store.runner_rows().unwrap().pop().unwrap();
    assert_eq!(row.name.as_deref(), Some(""));
    assert_eq!(row.reason.as_deref(), Some("name"));
    assert_eq!(
        row.argv.as_deref(),
        Some(["agent".to_owned(), "{{prompt}}".to_owned()].as_slice())
    );
    assert!(row.descriptor.starts_with('{'), "{}", row.descriptor);
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
        Err(ConfigError::Usage(_))
    ));
    assert!(matches!(
        store.set("form", "dialog"),
        Err(ConfigError::Usage(_))
    ));
    assert!(matches!(
        store.set("mirror", "on"),
        Err(ConfigError::Usage(_))
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
        Err(ConfigError::Usage(_))
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
            Err(ConfigError::Usage(_))
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

    let malformed = "not = [valid";
    fs::write(root.path().join("config.toml"), malformed).unwrap();
    assert_eq!(store.settings().unwrap()["form"], "tui");
    assert_eq!(
        fs::read_to_string(root.path().join("config.toml")).unwrap(),
        malformed
    );

    let file_root = root.path().join("file-root");
    fs::write(&file_root, "file").unwrap();
    let blocked = FileConfigStore::new(&file_root);
    assert_eq!(blocked.settings().unwrap()["form"], "tui");
    assert!(matches!(
        blocked.set("editor", "vim"),
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
    assert!(store.remove_runner_row(0).unwrap());
    let rows = store.runner_rows().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].descriptor, "future-shape");
}

#[test]
fn clearing_a_wrong_scalar_section_repairs_only_that_section() {
    let root = TempDir::new().unwrap();
    let path = root.path().join("config.toml");
    fs::write(&path, "shell = \"not a table\"\nfuture = 3\n").unwrap();
    let store = FileConfigStore::new(root.path());
    store.set("shell.bash_path", "").unwrap();
    let document = fs::read_to_string(path)
        .unwrap()
        .parse::<toml::Table>()
        .unwrap();
    assert!(!document.contains_key("shell"));
    assert_eq!(document["future"].as_integer(), Some(3));
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

    assert!(matches!(error, ConfigError::Usage(_)));
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
    assert_eq!(store.invalid_runner_rows().unwrap(), ["{'bogus': 1}"]);

    // An index past the end addresses no stored row.
    assert!(!store.remove_runner_row(2).unwrap());
    assert_eq!(store.runner_rows().unwrap().len(), 2);

    assert!(store.remove_runner_row(1).unwrap());

    let rows = store.runner_rows().unwrap();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].descriptor.contains("mine"), "{:?}", rows[0]);
    assert!(store.invalid_runner_rows().unwrap().is_empty());
}

#[test]
fn a_raw_row_index_outside_the_stored_list_changes_nothing() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());
    // A fresh file projects the visible defaults, but no raw rows exist yet.
    assert!(!store.runner_rows().unwrap().is_empty());

    assert!(!store.remove_runner_row(0).unwrap());

    assert!(!store.runner_rows().unwrap().is_empty());
    assert!(
        !root.path().join("config.toml").exists() || {
            let text = fs::read_to_string(root.path().join("config.toml")).unwrap();
            !text.contains("runners_seeded")
        }
    );
}

#[test]
fn raw_runner_removal_compares_the_complete_management_snapshot() {
    let root = TempDir::new().unwrap();
    let path = root.path().join("config.toml");
    fs::write(
        &path,
        r#"[prompt]
runners = [
  { name = "good", argv = ["good", "{{prompt}}"] },
  { name = "target", argv = ["target"], future = 7 },
]
"#,
    )
    .unwrap();
    let store = FileConfigStore::new(root.path());
    let expected = store.runner_rows().unwrap().remove(1);

    fs::write(
        &path,
        r#"[prompt]
runners = [
  { name = "inserted", argv = ["inserted", "{{prompt}}"] },
  { name = "good", argv = ["good", "{{prompt}}"] },
  { name = "target", argv = ["target"], future = 7 },
]
"#,
    )
    .unwrap();

    assert!(!store.remove_runner_row_if_unchanged(&expected).unwrap());
    assert_eq!(store.runner_rows().unwrap().len(), 3);
    assert_eq!(
        store.runner_rows().unwrap()[1].name.as_deref(),
        Some("good")
    );
}

#[test]
fn stable_runner_removal_refuses_a_replacement_after_confirmation() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());
    store
        .set_runner(
            PromptRunner {
                name: "victim".to_owned(),
                argv: vec!["old".to_owned(), "{{prompt}}".to_owned()],
            },
            false,
        )
        .unwrap();
    let expected = store
        .runner_rows()
        .unwrap()
        .into_iter()
        .filter(|row| row.name.as_deref() == Some("victim"))
        .collect::<Vec<_>>();
    store
        .set_runner(
            PromptRunner {
                name: "victim".to_owned(),
                argv: vec!["new".to_owned(), "{{prompt}}".to_owned()],
            },
            true,
        )
        .unwrap();

    assert!(
        !store
            .remove_runner_if_unchanged("victim", &expected)
            .unwrap()
    );
    assert_eq!(
        store
            .runners()
            .unwrap()
            .into_iter()
            .find(|runner| runner.name == "victim")
            .unwrap()
            .argv[0],
        "new"
    );
}

#[test]
fn malformed_runner_container_repair_is_targeted_and_snapshot_checked() {
    let root = TempDir::new().unwrap();
    let path = root.path().join("config.toml");
    fs::write(&path, "language = \"zh-TW\"\nprompt = \"garbage\"\n").unwrap();
    let store = FileConfigStore::new(root.path());
    let stale = store.runner_rows().unwrap().pop().unwrap();

    fs::write(&path, "language = \"zh-TW\"\nprompt = \"newer\"\n").unwrap();
    assert!(!store.remove_runner_row_if_unchanged(&stale).unwrap());
    assert_eq!(
        store.runner_rows().unwrap()[0].reason.as_deref(),
        Some("prompt-section-not-table")
    );

    let current = store.runner_rows().unwrap().pop().unwrap();
    assert!(store.remove_runner_row_if_unchanged(&current).unwrap());
    let document = fs::read_to_string(path).unwrap();
    assert!(document.contains("language = \"zh-TW\""), "{document}");
    assert!(document.contains("runners_seeded = true"), "{document}");
    assert!(document.contains("runners = []"), "{document}");
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
