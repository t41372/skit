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

#[test]
fn human_success_and_health_output_use_the_requested_catalog_but_json_does_not() {
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    let command = || {
        let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
        command
            .env("SKIT_DATA_DIR", data.path())
            .env("SKIT_STATE_DIR", state.path())
            .env("SKIT_CONFIG_DIR", config.path())
            .env("SKIT_LANG", "zh-TW");
        command
    };

    command()
        .args(["add", "--cmd", "printf ok", "--name", "Library"])
        .assert()
        .success()
        .stdout(predicate::str::contains("已新增：Library"));
    command()
        .args(["config", "after_run", "stay"])
        .assert()
        .success()
        .stdout(predicate::str::contains("已設定：after_run=stay"));
    command()
        .arg("doctor")
        .assert()
        .success()
        .stdout(predicate::str::contains("項目：1"))
        .stdout(predicate::str::contains("程式庫："))
        .stdout(predicate::str::contains("狀態資料："))
        .stdout(predicate::str::contains("組態："));
    command()
        .args(["show", "library", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"name\":\"Library\""))
        .stdout(predicate::str::contains("\"kind\":\"command\""));
}
