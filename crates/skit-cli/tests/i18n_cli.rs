use predicates::prelude::*;
use tempfile::TempDir;

#[test]
fn help_uses_the_requested_traditional_chinese_catalog() {
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
    command
        .env("SKIT_DATA_DIR", data.path())
        .env("SKIT_STATE_DIR", state.path())
        .env("SKIT_CONFIG_DIR", config.path())
        .env("SKIT_LANG", "zh-TW")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("程式、提示詞、執行檔與命令程式庫"))
        .stdout(predicate::str::contains("列出程式庫中的項目"))
        .stdout(predicate::str::contains("選項"));
}

#[test]
fn human_errors_use_the_requested_simplified_chinese_catalog() {
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
    command
        .env("SKIT_DATA_DIR", data.path())
        .env("SKIT_STATE_DIR", state.path())
        .env("SKIT_CONFIG_DIR", config.path())
        .env("SKIT_LANG", "zh-CN")
        .args(["show", "missing"])
        .assert()
        .code(127)
        .stderr(predicate::str::contains("找不到条目"));
}
