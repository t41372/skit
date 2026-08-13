use std::{collections::BTreeMap, fs};
use skit_store::{FileConfigStore, MirrorSettings};
use tempfile::TempDir;

#[test]
fn test_defaults_when_no_config() {
    let root=TempDir::new().unwrap(); let store=FileConfigStore::new(root.path());
    assert!(!root.path().join("config.toml").exists());
    assert_eq!(store.mirror().unwrap(),MirrorSettings::default());
    assert!(store.mirror_environment(&BTreeMap::new()).unwrap().is_empty());
    assert!(!store.mirror_configured().unwrap());
}

#[test]
fn test_save_mirror_preserves_other_keys() {
    let root=TempDir::new().unwrap(); fs::write(root.path().join("config.toml"),"language = \"zh-CN\"\n").unwrap();
    let store=FileConfigStore::new(root.path()); store.set("mirror.pypi","ustc").unwrap();
    let text=fs::read_to_string(root.path().join("config.toml")).unwrap(); let doc:toml::Table=toml::from_str(&text).unwrap();
    assert_eq!(doc["language"].as_str(),Some("zh-CN"));
    assert_eq!(doc["mirror"]["pypi"].as_str(),Some("https://pypi.mirrors.ustc.edu.cn/simple"));
}

#[test]
fn test_load_config_tolerates_corrupt_toml() {
    let root=TempDir::new().unwrap(); fs::write(root.path().join("config.toml"),"this is = = not [valid toml").unwrap();
    let store=FileConfigStore::new(root.path());
    assert_eq!(store.mirror().unwrap(),MirrorSettings::default());
    let settings=store.settings().unwrap();
    assert_eq!(settings["lang"],""); assert_eq!(settings["editor"],"");
}
