//! Store/config/state ports from Python `tests/test_review_fixes.py` at `main@206f9ef`.

use std::fs;

use skit_application::{
    CreateEntry, EntryMutationRepository as _,
    form_state::FormStateRepository as _,
};
use skit_domain::{EntryKind, EntrySettings, Slug, StorageMode};
use skit_i18n::requested_locale;
use skit_store::{FileConfigStore, FileFormStateStore, FileStore};
use tempfile::TempDir;

#[test]
fn test_is_supported_rejects_junk() {
    // Nonempty supported spellings exercise the same normalization/validation used by the persisted
    // language setting. Canonical spelling is asserted only where Rust publicly exposes it; the
    // Python oracle's core contract is acceptance/rejection, not collapsing en-US to bare en.
    for (input, canonical) in [
        ("zh-TW", "zh-TW"),
        ("zh_TW.UTF-8", "zh-TW"),
        ("en-US", "en-US"),
        ("x-pseudo", "x-pseudo"),
    ] {
        let root = TempDir::new().unwrap();
        let config = FileConfigStore::new(root.path());
        config
            .set("lang", input)
            .unwrap_or_else(|error| panic!("supported locale {input:?} was refused: {error}"));
        assert_eq!(config.get("lang").unwrap(), canonical, "input={input:?}");
    }

    for input in ["ent", "english", "fr"] {
        let root = TempDir::new().unwrap();
        let config = FileConfigStore::new(root.path());
        assert!(
            config.set("lang", input).is_err(),
            "unsupported locale {input:?} was accepted"
        );
    }

    // An empty string is not a supported language tag in the Python helper. Rust's config command
    // intentionally gives an empty language a different meaning (`auto`), so test the public locale
    // request boundary instead of incorrectly requiring the config setter to reject its own syntax.
    assert_eq!(requested_locale(Some("")), None);
}

#[test]
fn test_argstate_corrupt_file_fallback() {
    let root = TempDir::new().unwrap();
    let values = root.path().join("values");
    fs::create_dir_all(&values).unwrap();
    let path = values.join("myscript.toml");
    fs::write(&path, "[[[invalid").unwrap();
    let before = fs::read(&path).unwrap();
    let store = FileFormStateStore::new(root.path());
    let slug = Slug::parse("myscript").unwrap();

    let state = store.load(&slug);

    assert!(state.values.is_empty());
    assert!(state.presets.is_empty());
    assert!(state.extra_args.is_empty());
    assert_eq!(
        fs::read(&path).unwrap(),
        before,
        "a read repaired the corrupt state file"
    );
}

#[test]
fn test_config_language_corrupt_file() {
    let root = TempDir::new().unwrap();
    let path = root.path().join("config.toml");
    fs::write(&path, "[[[bad").unwrap();
    let before = fs::read(&path).unwrap();
    let config = FileConfigStore::new(root.path());

    assert_eq!(config.get("lang").unwrap(), "");
    assert_eq!(
        fs::read(&path).unwrap(),
        before,
        "a config read silently rewrote corruption"
    );
}

#[test]
fn test_set_language_with_existing_corrupt_config() {
    let root = TempDir::new().unwrap();
    let path = root.path().join("config.toml");
    let corrupt = b"[[[bad";
    fs::write(&path, corrupt).unwrap();
    let config = FileConfigStore::new(root.path());

    let recovery = config
        .set_with_recovery("lang", "en-US")
        .expect("setting language should recover a malformed config")
        .expect("malformed config recovery must be reported");

    assert_eq!(recovery.path, path);
    assert_eq!(fs::read(&recovery.backup_path).unwrap(), corrupt);
    assert_eq!(config.get("lang").unwrap(), "en-US");
    let rewritten = fs::read_to_string(&path).unwrap();
    let _: toml::Table = toml::from_str(&rewritten).expect("recovered config must be valid TOML");
}

#[test]
fn test_normalize_four_char_subtag() {
    let root = TempDir::new().unwrap();
    let config = FileConfigStore::new(root.path());

    config.set("lang", "zh-hant-tw").unwrap();

    assert_eq!(config.get("lang").unwrap(), "zh-Hant-TW");
    let stored = fs::read_to_string(root.path().join("config.toml")).unwrap();
    assert!(
        stored.contains("zh-Hant-TW"),
        "the canonical four-character script subtag did not reach persisted config: {stored}"
    );
}

fn command_entry(name: &str) -> CreateEntry {
    CreateEntry {
        name: name.to_owned(),
        kind: EntryKind::parse("command").unwrap(),
        mode: StorageMode::Reference,
        source: String::new(),
        workdir: "invoke".to_owned(),
        description: String::new(),
        payload: None,
        settings: EntrySettings {
            template: "echo ok".to_owned(),
            ..EntrySettings::default()
        },
    }
}

#[test]
fn test_unique_slug_multiple_collisions() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());

    let first = store.create(command_entry("hello")).unwrap();
    let second = store.create(command_entry("hello!")).unwrap();
    let third = store.create(command_entry("hello?")).unwrap();

    assert_eq!(first.slug.as_str(), "hello");
    assert_eq!(second.slug.as_str(), "hello-2");
    assert_eq!(third.slug.as_str(), "hello-3");
}
