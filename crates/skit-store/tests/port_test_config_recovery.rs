use std::fs;
use skit_store::FileConfigStore;
use tempfile::TempDir;

#[test]
fn rust_additive_save_editor_recovery_object_preserves_corrupt_bytes() {
    let root = TempDir::new().unwrap();
    let corrupt = "language = \"zh-CN\"\n[mirror]\nenabled = true\npypi = \"https://saved.example/simple\"\nthis is = = not valid toml";
    fs::write(root.path().join("config.toml"), corrupt).unwrap();
    let store = FileConfigStore::new(root.path());
    let recovery = store.set_with_recovery("editor", "vim").unwrap().expect("corrupt edit must report recovery");
    assert_eq!(store.get("editor").unwrap(), "vim");
    assert_eq!(recovery.path, root.path().join("config.toml"));
    assert_eq!(recovery.backup_path, root.path().join("config.toml.bak"));
    assert_eq!(fs::read_to_string(recovery.backup_path).unwrap(), corrupt);
}

#[test]
fn test_save_mirror_backs_up_corrupt_config_instead_of_wiping_it() {
    let root = TempDir::new().unwrap();
    let corrupt = "language = \"zh-CN\"\nthis is = = not valid toml";
    fs::write(root.path().join("config.toml"), corrupt).unwrap();
    let store = FileConfigStore::new(root.path());
    let recovery = store.set_with_recovery("mirror.pypi", "aliyun").unwrap().expect("corrupt mirror edit must report recovery");
    assert_eq!(store.mirror().unwrap().pypi, "https://mirrors.aliyun.com/pypi/simple");
    assert_eq!(fs::read_to_string(recovery.backup_path).unwrap(), corrupt);
}

#[test]
fn test_save_editor_still_preserves_other_keys_when_config_is_valid() {
    let root = TempDir::new().unwrap();
    fs::write(root.path().join("config.toml"), "language = \"zh-CN\"\n").unwrap();
    let store = FileConfigStore::new(root.path());
    store.set("editor", "code --wait").unwrap();
    let text = fs::read_to_string(root.path().join("config.toml")).unwrap();
    let doc: toml::Table = toml::from_str(&text).unwrap();
    assert_eq!(doc["language"].as_str(), Some("zh-CN"));
    assert_eq!(doc["editor"].as_str(), Some("code --wait"));
    assert!(!root.path().join("config.toml.bak").exists());
}
