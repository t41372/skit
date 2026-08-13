use std::{collections::BTreeMap, fs};
use skit_store::{FileConfigStore, MirrorSettings};
use tempfile::TempDir;

#[test]
fn test_mirror_env_skips_empty_urls() {
    let root = TempDir::new().unwrap();
    fs::write(root.path().join("config.toml"), "[mirror]\nenabled = true\n").unwrap();
    let store = FileConfigStore::new(root.path());
    assert!(store.mirror().unwrap().enabled);
    assert!(store.mirror_environment(&BTreeMap::new()).unwrap().is_empty());
}

#[test]
fn test_load_mirror_ignores_malformed_section() {
    let root = TempDir::new().unwrap();
    fs::write(root.path().join("config.toml"), "mirror = \"not-a-table\"\n").unwrap();
    assert_eq!(FileConfigStore::new(root.path()).mirror().unwrap(), MirrorSettings::default());
}

#[test]
fn test_load_mirror_rejects_string_enabled() {
    let root = TempDir::new().unwrap();
    fs::write(root.path().join("config.toml"), "[mirror]\nenabled = \"false\"\npypi = \"https://x/simple\"\n").unwrap();
    assert!(!FileConfigStore::new(root.path()).mirror().unwrap().enabled);
}

#[test]
fn test_load_mirror_ignores_non_str_url() {
    let root = TempDir::new().unwrap();
    fs::write(root.path().join("config.toml"), "[mirror]\nenabled = true\npypi = 123\n").unwrap();
    let mirror = FileConfigStore::new(root.path()).mirror().unwrap();
    assert!(mirror.enabled);
    assert_eq!(mirror.pypi, "");
}
