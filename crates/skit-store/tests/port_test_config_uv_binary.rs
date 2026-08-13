use std::fs;
use skit_store::FileConfigStore;
use tempfile::TempDir;

#[test]
fn test_load_mirror_blanks_non_https_uv_binary() {
    let root = TempDir::new().unwrap();
    fs::write(root.path().join("config.toml"), "[mirror]\nenabled = true\nuv_binary = \"http://plain.example/uv\"\n").unwrap();
    assert_eq!(FileConfigStore::new(root.path()).mirror().unwrap().uv_binary, "");
}

#[test]
fn test_load_mirror_preserves_https_uv_binary() {
    let root = TempDir::new().unwrap();
    fs::write(root.path().join("config.toml"), "[mirror]\nenabled = true\nuv_binary = \"https://ok/uv\"\n").unwrap();
    assert_eq!(FileConfigStore::new(root.path()).mirror().unwrap().uv_binary, "https://ok/uv");
}

#[test]
fn test_nju_preset_uv_binary_stays_https() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());
    store.set("mirror.github", "nju").unwrap();
    let value = store.mirror().unwrap().uv_binary;
    assert_eq!(value, "https://mirror.nju.edu.cn/github-release/astral-sh/uv");
    assert!(value.starts_with("https://"));
}
