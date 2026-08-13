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
        .stdout(predicate::str::contains("列出工具庫中的項目"))
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
        // Management commands report a missing name with exit 1. Only the run path uses
        // 127 (src/skit/cli.py:2483 `raise _fail(str(exc), 1)` against cli.py:3008).
        .assert()
        .code(1)
        .stderr(predicate::str::contains("找不到条目"));
}

#[test]
fn add_flag_refusals_use_the_v040_catalog_in_every_locale() {
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    let source = data.path().join("refusal.sh");
    fs::write(&source, "#!/bin/sh\nprintf ok\n").unwrap();
    let command = |locale: &str| {
        let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
        command
            .env("SKIT_DATA_DIR", data.path())
            .env("SKIT_STATE_DIR", state.path())
            .env("SKIT_CONFIG_DIR", config.path())
            .env("SKIT_LANG", locale);
        command
    };

    for (locale, dependency_refusal, stdin_refusal) in [
        (
            "en",
            "shell entries don't take package dependencies — drop --dep.",
            "--ref can't apply here — stdin authors a brand-new copy, and --ref/--exe need an existing file (nothing was added).",
        ),
        (
            "zh-CN",
            "shell 条目不接受依赖包——去掉 --dep。",
            "--ref 在这里无法应用——stdin 会撰写一份全新副本，而 --ref/--exe 需要现成的文件(未添加任何内容)。",
        ),
        (
            "zh-TW",
            "shell 條目不接受依賴套件——拿掉 --dep。",
            "--ref 在這裡無法套用——stdin 會撰寫一份全新副本，而 --ref/--exe 需要現成的檔案(未加入任何內容)。",
        ),
    ] {
        command(locale)
            .arg("add")
            .arg(&source)
            .args(["--dep", "requests", "--no-input"])
            .assert()
            .code(2)
            .stderr(predicate::str::contains(dependency_refusal));
        command(locale)
            .args(["add", "-", "--name", "clip", "--ref"])
            .write_stdin("print('hi')\n")
            .assert()
            .code(2)
            .stderr(predicate::str::contains(stdin_refusal));
    }
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
    // The v0.4 write confirmation is `key = value` with no localizable words; the
    // localized human surface for config is the unset display sentinel on a read.
    command()
        .args(["config", "after_run", "stay"])
        .assert()
        .success()
        .stdout(predicate::str::contains("after_run = stay"));
    command()
        .args(["config", "js.runner"])
        .assert()
        .success()
        .stdout(predicate::str::contains("自動（deno > bun > node）"));
    command()
        .arg("doctor")
        .assert()
        .success()
        .stdout(predicate::str::contains("項目：1"))
        .stdout(predicate::str::contains("工具庫："))
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

    // These are whole catalog rows, so an exact lookup must translate each one. The row set
    // and its order come from `_print_show_human` (`src/skit/cli.py:2366-2427`): identity,
    // description, source, work directory, the prompt runner, the field table, then the run
    // hint. Version 0.4 prints no "missing" and no "drift" row for a healthy entry — the
    // missing marker appears only when `launcher.missing_marker` returns one
    // (`src/skit/cli.py:2382-2384`), and the interpolation row appears only when insertion is
    // off (`src/skit/cli.py:2412-2413`).
    command("zh-CN")
        .args(["show", "report"])
        .assert()
        .success()
        .stdout(predicate::str::contains("(提示词 · copy)"))
        .stdout(predicate::str::contains("来源:"))
        .stdout(predicate::str::contains("工作目录:invoke"))
        .stdout(predicate::str::contains("执行器:(运行时询问)"))
        .stdout(predicate::str::contains("运行:skit run Report"))
        .stdout(predicate::str::contains("缺失").not())
        .stdout(predicate::str::contains("漂移").not());

    command("zh-TW")
        .args(["show", "report"])
        .assert()
        .success()
        .stdout(predicate::str::contains("(提示詞 · copy)"))
        .stdout(predicate::str::contains("來源:"))
        .stdout(predicate::str::contains("工作目錄:invoke"))
        .stdout(predicate::str::contains("執行器:(執行時詢問)"))
        .stdout(predicate::str::contains("執行:skit run Report"));

    // The pinned runner and the insertion switch are the two conditional prompt rows.
    command("en")
        .args(["add", "--prompt", "--name", "Pinned", "--no-input"])
        .args(["--runner", "claude", "--no-interpolate"])
        .write_stdin("Body {{subject}}\n")
        .assert()
        .success();
    command("zh-CN")
        .args(["show", "pinned"])
        .assert()
        .success()
        .stdout(predicate::str::contains("执行器:claude"))
        .stdout(predicate::str::contains("变量插入:关闭(正文原样送达)"));

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

    // Version 0.4 prints Row/Runner/Command/Status (`src/skit/cli.py:3306-3319`), the raw index
    // is zero-based (`src/skit/config.py:687` `enumerate`), and the Status column carries the
    // closed human wording of `prompt_runner_row_reason` (`src/skit/config.py:592-624`) — never
    // the English machine code that `--json` reports.
    command()
        .args(["runner", "list", "--all"])
        .assert()
        .success()
        .stdout(predicate::str::contains("行"))
        .stdout(predicate::str::contains("执行器"))
        .stdout(predicate::str::contains("状态"))
        .stdout(predicate::str::contains("│ 0 "))
        .stdout(predicate::str::contains("│ 1 "))
        .stdout(predicate::str::contains(
            "命令必须恰好包含一个 {{prompt}} 槽位——渲染后的提示词会放在那里。",
        ))
        .stdout(predicate::str::contains("必须提供名称。"))
        .stdout(predicate::str::contains("prompt-slot-count").not())
        .stdout(predicate::str::contains("A name is required").not());

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
    // Version 0.4 resolves the row before it asks anything, so an absent row refuses with
    // exit 1 and the same wording with or without a confirmation flag
    // (`src/skit/cli.py:3471-3477`).
    for extra in ["--no-input", "--yes"] {
        command()
            .args(["runner", "remove", "--row", "99", extra])
            .assert()
            .code(1)
            .stderr(predicate::str::contains(
                "未知执行器行：99。请用 skit runner list --all 检查",
            ))
            .stderr(predicate::str::contains("Unknown runner row").not());
    }
    // An unreadable add source is a store failure, so it exits 1 — 125 belongs to the run
    // path (`src/skit/cli.py:418-422` raises `StoreError`, and every add lane turns that into
    // `_fail(str(exc), 1)`).
    command()
        .arg("add")
        .arg(data.path().join("does-not-exist.sh"))
        .args(["--name", "Missing"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("无法解析"))
        .stderr(predicate::str::contains("无法resolve").not());
    let meta_path = data.path().join("scripts/future/meta.toml");
    let meta = fs::read_to_string(&meta_path).unwrap();
    let meta = meta.replace("kind = \"shell\"", "kind = \"future-kind\"");
    assert!(meta.contains("kind = \"future-kind\""), "{meta}");
    fs::write(&meta_path, meta).unwrap();

    // An open-ended kind stays visible and is not an issue: `doctor` skips it without a word
    // (`src/skit/healthcheck.py:89` `spec_for(entry.meta.kind) is None` → continue), and the
    // library keeps listing it. Only the malformed runner rows are reported, in Chinese.
    command()
        .arg("list")
        .assert()
        .success()
        .stdout(predicate::str::contains("future-kind"));
    command()
        .arg("doctor")
        .assert()
        .success()
        .stdout(predicate::str::contains("{'future': 1}"))
        .stdout(predicate::str::contains("broken"))
        .stdout(predicate::str::contains("malformed").not())
        .stdout(predicate::str::contains("unknown entry kind").not());
}
