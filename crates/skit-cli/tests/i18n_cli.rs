use std::fs;

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
fn clap_errors_translate_framework_text_without_rewriting_user_arguments() {
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    let command = |argument: &str| {
        let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
        command
            .env("SKIT_DATA_DIR", data.path())
            .env("SKIT_STATE_DIR", state.path())
            .env("SKIT_CONFIG_DIR", config.path())
            .env("SKIT_LANG", "zh-CN")
            .arg(argument);
        command
    };

    for argument in ["Print help", "Entry added"] {
        command(argument)
            .assert()
            .code(2)
            .stderr(predicate::str::contains("错误：无法识别子命令"))
            .stderr(predicate::str::contains(argument))
            .stderr(predicate::str::contains("用法：skit"))
            .stderr(predicate::str::contains("如需更多信息"));
    }

    command("--halp")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("错误：发现意外参数"))
        .stderr(predicate::str::contains("提示："))
        .stderr(predicate::str::contains("--help"));
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

#[test]
fn scalar_report_labels_translate_in_every_supported_locale() {
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    let command = |locale: &str| {
        let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
        command
            .env("SKIT_DATA_DIR", data.path())
            .env("SKIT_STATE_DIR", state.path())
            .env("SKIT_CONFIG_DIR", config.path())
            .env("SKIT_LANG", locale);
        command
    };
    command("en")
        .args(["add", "--prompt", "--name", "Report", "--no-input"])
        .write_stdin("Body {{subject}}\n")
        .assert()
        .success();

    // These are whole catalog rows, so an exact lookup must translate each one.
    command("zh-CN")
        .args(["show", "report"])
        .assert()
        .success()
        .stdout(predicate::str::contains("缺失：否"))
        .stdout(predicate::str::contains("漂移：否"))
        .stdout(predicate::str::contains("提示词运行器：未设置"))
        .stdout(predicate::str::contains("插值：开启"));

    // Hong Kong, Macau, and Singapore resolve to a Chinese catalog, not to English.
    for (locale, expected) in [
        ("zh-HK", "程式、提示詞、執行檔與命令程式庫"),
        ("zh-MO", "程式、提示詞、執行檔與命令程式庫"),
        ("zh-SG", "脚本、提示词、程序与命令库"),
    ] {
        command(locale)
            .arg("--help")
            .assert()
            .success()
            .stdout(predicate::str::contains(expected));
    }
}

#[test]
fn runner_rows_and_doctor_reasons_translate_nested_skit_text() {
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    fs::write(
        config.path().join("config.toml"),
        r#"[prompt]
runners = [
  { name = "broken", argv = ["agent"] },
  { future = 1 },
]
"#,
    )
    .unwrap();
    let command = || {
        let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
        command
            .env("SKIT_DATA_DIR", data.path())
            .env("SKIT_STATE_DIR", state.path())
            .env("SKIT_CONFIG_DIR", config.path())
            .env("SKIT_LANG", "zh-CN");
        command
    };

    command()
        .args(["runner", "list", "--all"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "提示词运行器命令必须在程序之后正好包含一次 {{prompt}}",
        ))
        .stdout(predicate::str::contains("第 2 行"))
        .stdout(predicate::str::contains(
            "运行器行需要名称和字符串 argv 数组",
        ))
        .stdout(predicate::str::contains("runner row needs").not());

    let source = data.path().join("future.sh");
    fs::write(&source, "printf ok\n").unwrap();
    command()
        .arg("add")
        .arg(&source)
        .args(["--name", "Future", "--kind", "shell"])
        .assert()
        .success();
    command()
        .args(["add", "--cmd", "echo {name}", "--name", "Fields"])
        .assert()
        .success();
    command()
        .args(["params", "fields", "--env-target", "broken"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("环境目标需要 NAME=VALUE"))
        .stderr(predicate::str::contains("environment target").not());
    command()
        .args(["runner", "remove", "--row", "99", "--no-input"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "移除提示词运行器需要确认；请传入 --yes",
        ));
    command()
        .args(["runner", "remove", "--row", "99", "--yes"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("未知的提示词运行器：第 99 行"))
        .stderr(predicate::str::contains("row 99").not());
    command()
        .arg("add")
        .arg(data.path().join("does-not-exist.sh"))
        .args(["--name", "Missing"])
        .assert()
        .code(125)
        .stderr(predicate::str::contains("无法解析"))
        .stderr(predicate::str::contains("无法resolve").not());
    let meta_path = data.path().join("scripts/future/meta.toml");
    let meta = fs::read_to_string(&meta_path).unwrap();
    let meta = meta.replace("kind = \"shell\"", "kind = \"future-kind\"");
    assert!(meta.contains("kind = \"future-kind\""), "{meta}");
    fs::write(&meta_path, meta).unwrap();

    command()
        .arg("doctor")
        .assert()
        .success()
        .stdout(predicate::str::contains("未知的条目类型：future-kind"))
        .stdout(predicate::str::contains("unknown entry kind").not());
}
