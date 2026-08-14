//! Exact raw JSON and scalar configuration contracts from Python v0.4 `tests/test_config_cmd.py`.

#[path = "support/config_cmd.rs"]
mod support;

use std::fs;
use serde_json::Value as JsonValue;
use support::{Sandbox, text};

fn ok(s: &Sandbox, args: &[&str]) -> std::process::Output {
    let output = s.run(args);
    assert_eq!(output.status.code(), Some(0), "{}", text(&output));
    output
}

fn code(s: &Sandbox, args: &[&str], expected: i32) -> std::process::Output {
    let output = s.run(args);
    assert_eq!(output.status.code(), Some(expected), "{}", text(&output));
    output
}

fn json(s: &Sandbox, args: &[&str]) -> JsonValue {
    let output = ok(s, args);
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| panic!("{error}: {}", text(&output)))
}

fn raw(s: &Sandbox) -> toml::Table {
    fs::read_to_string(s.path().join("config.toml"))
        .unwrap()
        .parse::<toml::Table>()
        .unwrap()
}

#[test]
fn test_config_json_emits_raw_values_never_a_localized_sentinel() {
    let s = Sandbox::new();
    let mut whole = s.command();
    let output = whole.env("SKIT_LANG", "zh-TW").args(["config", "--json"]).output().unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", text(&output));
    let doc: JsonValue = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(doc["editor"], "");
    assert_eq!(doc["js.runner"], "");
    assert_eq!(doc["lang"], "");
    assert_eq!(doc["mirror"], "off");
    assert_eq!(doc["shell.bash_path"], "");
    let machine = String::from_utf8_lossy(&output.stdout);
    assert!(!machine.contains("自動（deno > bun > node）"));

    let mut editor = s.command();
    let editor = editor.env("SKIT_LANG", "zh-TW").args(["config", "editor", "--json"]).output().unwrap();
    assert_eq!(serde_json::from_slice::<JsonValue>(&editor.stdout).unwrap(), serde_json::json!({"editor":""}));
    let mut js = s.command();
    let js = js.env("SKIT_LANG", "zh-TW").args(["config", "js.runner", "--json"]).output().unwrap();
    assert_eq!(serde_json::from_slice::<JsonValue>(&js.stdout).unwrap(), serde_json::json!({"js.runner":""}));

    let mut write = s.command();
    let write = write.env("SKIT_LANG", "zh-TW").args(["config", "mirror.pypi", "tsinghua", "--json"]).output().unwrap();
    assert_eq!(write.status.code(), Some(0), "{}", text(&write));
    assert_eq!(serde_json::from_slice::<JsonValue>(&write.stdout).unwrap()["mirror.pypi"], "tsinghua");

    let mut human = s.command();
    let human = human.env("SKIT_LANG", "zh-TW").arg("config").output().unwrap();
    assert_eq!(human.status.code(), Some(0), "{}", text(&human));
    let human = String::from_utf8_lossy(&human.stdout);
    assert!(human.contains("自動（deno > bun > node）"), "frozen zh-TW JS sentinel disappeared:\n{human}");
    assert!(human.contains("$VISUAL / $EDITOR"), "frozen editor-default sentinel disappeared:\n{human}");
}

#[test]
fn test_config_json_single_key_is_raw_master_token() {
    let s = Sandbox::new();
    ok(&s, &["config", "mirror.pypi", "tsinghua"]);
    assert_eq!(json(&s, &["config", "mirror", "--json"]), serde_json::json!({"mirror":"on"}));
}

#[test]
fn test_config_json_single_key_lang_is_raw_override_tag() {
    let s = Sandbox::new();
    ok(&s, &["config", "lang", "zh-CN"]);
    assert_eq!(json(&s, &["config", "lang", "--json"]), serde_json::json!({"lang":"zh-CN"}));
}

#[test]
fn test_config_json_lang_unset_is_empty_string() {
    let s = Sandbox::new();
    assert_eq!(json(&s, &["config", "lang", "--json"]), serde_json::json!({"lang":""}));
}

#[test]
fn test_lang_override_non_string_reads_as_empty() {
    let s = Sandbox::new();
    fs::create_dir_all(s.path()).unwrap();
    fs::write(s.path().join("config.toml"), "language = 123\n").unwrap();
    assert_eq!(json(&s, &["config", "lang", "--json"]), serde_json::json!({"lang":""}));
}

#[test]
fn test_config_json_mirror_github_raw_token() {
    for (token, expected) in [("nju", "nju"), ("https://my/gh", "https://my/gh")] {
        let s = Sandbox::new();
        ok(&s, &["config", "mirror.github", token]);
        assert_eq!(json(&s, &["config", "mirror.github", "--json"]), serde_json::json!({"mirror.github":expected}), "token={token}");
    }
}

#[test]
fn test_config_json_mirror_github_underivable_pair_is_literal_custom() {
    let s = Sandbox::new();
    fs::create_dir_all(s.path()).unwrap();
    fs::write(
        s.path().join("config.toml"),
        "[mirror]\nenabled = true\npython_install = \"https://my/py/\"\nuv_binary = \"https://my/uv\"\npypi = \"\"\nnpm = \"\"\n",
    )
    .unwrap();
    assert_eq!(json(&s, &["config", "mirror.github", "--json"]), serde_json::json!({"mirror.github":"custom"}));
}

#[test]
fn test_set_editor() {
    let s = Sandbox::new();
    let output = ok(&s, &["config", "editor", "code --wait"]);
    assert_eq!(raw(&s).get("editor").and_then(toml::Value::as_str), Some("code --wait"));
    assert!(String::from_utf8_lossy(&output.stdout).contains("code --wait"));
}

#[test]
fn test_clear_editor_with_empty_value() {
    let s = Sandbox::new();
    ok(&s, &["config", "editor", "nano"]);
    ok(&s, &["config", "editor", ""]);
    assert_eq!(raw(&s).get("editor").and_then(toml::Value::as_str), Some(""));
}

#[test]
fn test_read_editor_default_line() {
    let s = Sandbox::new();
    let output = ok(&s, &["config", "editor"]);
    assert!(String::from_utf8_lossy(&output.stdout).contains("$VISUAL / $EDITOR"), "{}", text(&output));
}

#[test]
fn test_form_defaults_to_tui() {
    let s = Sandbox::new();
    let output = ok(&s, &["config", "form"]);
    assert!(String::from_utf8_lossy(&output.stdout).contains("tui"), "{}", text(&output));
}

#[test]
fn test_set_form_plain_and_back() {
    let s = Sandbox::new();
    ok(&s, &["config", "form", "plain"]);
    assert_eq!(raw(&s).get("form").and_then(toml::Value::as_str), Some("plain"));
    ok(&s, &["config", "form", "tui"]);
    assert_eq!(raw(&s).get("form").and_then(toml::Value::as_str), Some("tui"));
}

#[test]
fn test_unknown_form_style_exits_2() {
    let s = Sandbox::new();
    code(&s, &["config", "form", "fancy"], 2);
}

#[test]
fn test_read_after_run_default() {
    let s = Sandbox::new();
    let output = ok(&s, &["config", "after_run"]);
    assert!(String::from_utf8_lossy(&output.stdout).contains("exit"), "{}", text(&output));
}

#[test]
fn test_set_after_run_stay_and_back() {
    let s = Sandbox::new();
    ok(&s, &["config", "after_run", "stay"]);
    assert_eq!(raw(&s).get("after_run").and_then(toml::Value::as_str), Some("stay"));
    ok(&s, &["config", "after_run", "exit"]);
    assert_eq!(raw(&s).get("after_run").and_then(toml::Value::as_str), Some("exit"));
}

#[test]
fn test_unknown_after_run_exits_2() {
    let s = Sandbox::new();
    code(&s, &["config", "after_run", "loop"], 2);
}

#[test]
fn test_after_run_garbage_in_config_file_normalizes_to_exit() {
    let s = Sandbox::new();
    fs::create_dir_all(s.path()).unwrap();
    fs::write(s.path().join("config.toml"), "after_run = \"never\"\n").unwrap();
    let output = ok(&s, &["config", "after_run"]);
    assert!(String::from_utf8_lossy(&output.stdout).contains("exit"), "{}", text(&output));
}

#[test]
fn test_mirror_write_preserves_language() {
    let s = Sandbox::new();
    ok(&s, &["config", "lang", "zh-CN"]);
    ok(&s, &["config", "mirror", "off"]);
    assert_eq!(raw(&s).get("language").and_then(toml::Value::as_str), Some("zh-CN"));
}

#[test]
fn test_lang_clear_preserves_mirror() {
    let s = Sandbox::new();
    ok(&s, &["config", "mirror.pypi", "tsinghua"]);
    ok(&s, &["config", "lang", "auto"]);
    assert_eq!(s.mirror().pypi, "https://pypi.tuna.tsinghua.edu.cn/simple");
}

#[test]
fn test_form_write_preserves_mirror_and_language() {
    let s = Sandbox::new();
    ok(&s, &["config", "lang", "zh-CN"]);
    ok(&s, &["config", "mirror.pypi", "tsinghua"]);
    ok(&s, &["config", "form", "plain"]);
    let doc = raw(&s);
    assert_eq!(doc.get("language").and_then(toml::Value::as_str), Some("zh-CN"));
    assert_eq!(doc.get("form").and_then(toml::Value::as_str), Some("plain"));
    assert!(s.mirror().enabled);
    assert_eq!(s.mirror().pypi, "https://pypi.tuna.tsinghua.edu.cn/simple");
}
