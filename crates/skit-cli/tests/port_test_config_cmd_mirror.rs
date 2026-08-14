//! Exact non-interactive mirror CLI contracts from Python v0.4 `tests/test_config_cmd.py`.

#[path = "support/config_cmd.rs"]
mod support;

use std::fs;
use support::{Sandbox, text};

const TSINGHUA: &str = "https://pypi.tuna.tsinghua.edu.cn/simple";
const ALIYUN: &str = "https://mirrors.aliyun.com/pypi/simple";
const USTC: &str = "https://pypi.mirrors.ustc.edu.cn/simple";
const NJU_BASE: &str = "https://mirror.nju.edu.cn/github-release";
const NJU_PYTHON: &str = "https://mirror.nju.edu.cn/github-release/astral-sh/python-build-standalone/";
const NJU_UV: &str = "https://mirror.nju.edu.cn/github-release/astral-sh/uv";
const NPMMIRROR: &str = "https://registry.npmmirror.com";

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

#[test]
fn test_set_mirror_pypi_preset() {
    for (token, expected) in [("tsinghua", TSINGHUA), ("aliyun", ALIYUN), ("ustc", USTC)] {
        let s = Sandbox::new();
        ok(&s, &["config", "mirror.pypi", token]);
        let mirror = s.mirror();
        assert!(mirror.enabled, "preset={token}");
        assert_eq!(mirror.pypi, expected, "preset={token}");
    }
}

#[test]
fn test_pypi_axis_does_not_drag_other_axes() {
    let s = Sandbox::new();
    ok(&s, &["config", "mirror.pypi", "tsinghua"]);
    let mirror = s.mirror();
    assert_eq!(mirror.pypi, TSINGHUA);
    assert_eq!((mirror.npm, mirror.python_install, mirror.uv_binary), (String::new(), String::new(), String::new()));
}

#[test]
fn test_set_mirror_npm_alone() {
    let s = Sandbox::new();
    ok(&s, &["config", "mirror.npm", "npmmirror"]);
    let mirror = s.mirror();
    assert!(mirror.enabled);
    assert_eq!(mirror.npm, NPMMIRROR);
    assert_eq!((mirror.pypi, mirror.python_install, mirror.uv_binary), (String::new(), String::new(), String::new()));
}

#[test]
fn test_set_mirror_github_expands_both_urls() {
    let s = Sandbox::new();
    ok(&s, &["config", "mirror.github", "nju"]);
    let mirror = s.mirror();
    assert!(mirror.enabled);
    assert_eq!(mirror.python_install, NJU_PYTHON);
    assert_eq!(mirror.uv_binary, NJU_UV);
    assert_eq!((mirror.pypi, mirror.npm), (String::new(), String::new()));
}

#[test]
fn test_set_mirror_github_custom_base_expands() {
    let s = Sandbox::new();
    ok(&s, &["config", "mirror.github", "https://my.mirror/gh"]);
    let mirror = s.mirror();
    assert_eq!(mirror.python_install, "https://my.mirror/gh/astral-sh/python-build-standalone/");
    assert_eq!(mirror.uv_binary, "https://my.mirror/gh/astral-sh/uv");
}

#[test]
fn test_set_mirror_github_off_clears_both_urls() {
    let s = Sandbox::new();
    ok(&s, &["config", "mirror.github", "nju"]);
    ok(&s, &["config", "mirror.npm", "npmmirror"]);
    ok(&s, &["config", "mirror.github", "off"]);
    let mirror = s.mirror();
    assert_eq!((mirror.python_install.as_str(), mirror.uv_binary.as_str()), ("", ""));
    assert!(mirror.enabled);
    assert_eq!(mirror.npm, NPMMIRROR);

    let raw = fs::read_to_string(s.path().join("config.toml")).unwrap();
    let doc = raw.parse::<toml::Table>().unwrap();
    let section = doc.get("mirror").and_then(toml::Value::as_table).expect("mirror table");
    assert_eq!(section.get("python_install").and_then(toml::Value::as_str), Some(""));
    assert_eq!(section.get("uv_binary").and_then(toml::Value::as_str), Some(""));
}

#[test]
fn test_paused_github_write_prints_notice_and_clear_does_not() {
    let s = Sandbox::new();
    ok(&s, &["config", "mirror.pypi", "tsinghua"]);
    ok(&s, &["config", "mirror", "off"]);
    let written = ok(&s, &["config", "mirror.github", "nju"]);
    let mirror = s.mirror();
    assert!(!mirror.enabled);
    assert_eq!(mirror.python_install, NJU_PYTHON);
    assert_eq!(mirror.uv_binary, NJU_UV);
    assert_eq!(mirror.pypi, TSINGHUA);
    assert!(String::from_utf8_lossy(&written.stderr).split_whitespace().collect::<Vec<_>>().join(" ").contains("switched off"), "{}", text(&written));

    let cleared = ok(&s, &["config", "mirror.github", "off"]);
    assert!(!text(&cleared).split_whitespace().collect::<Vec<_>>().join(" ").contains("switched off"), "{}", text(&cleared));
    let mirror = s.mirror();
    assert_eq!((mirror.python_install.as_str(), mirror.uv_binary.as_str()), ("", ""));
    assert_eq!(mirror.pypi, TSINGHUA);
}

#[test]
fn test_set_mirror_github_rejects_http_base() {
    let s = Sandbox::new();
    code(&s, &["config", "mirror.github", "http://evil/gh"], 2);
    assert_eq!(s.mirror().uv_binary, "");
}

#[test]
fn test_set_mirror_axis_custom_url() {
    let s = Sandbox::new();
    ok(&s, &["config", "mirror.pypi", "https://my.index/simple"]);
    assert_eq!(s.mirror().pypi, "https://my.index/simple");
}

#[test]
fn test_set_mirror_axis_off_keeps_the_others() {
    let s = Sandbox::new();
    ok(&s, &["config", "mirror.pypi", "tsinghua"]);
    ok(&s, &["config", "mirror.npm", "npmmirror"]);
    ok(&s, &["config", "mirror.pypi", "off"]);
    let mirror = s.mirror();
    assert!(mirror.enabled);
    assert_eq!(mirror.pypi, "");
    assert_eq!(mirror.npm, NPMMIRROR);
}

#[test]
fn test_set_last_axis_off_disables() {
    let s = Sandbox::new();
    ok(&s, &["config", "mirror.npm", "npmmirror"]);
    ok(&s, &["config", "mirror.npm", "off"]);
    assert!(!s.mirror().enabled);
}

#[test]
fn test_unknown_axis_value_exits_2() {
    for key in ["mirror.pypi", "mirror.npm"] {
        let s = Sandbox::new();
        code(&s, &["config", key, "tsnighua"], 2);
        assert!(!s.mirror().enabled, "key={key}");
    }
}

#[test]
fn test_npm_axis_rejects_pypi_vendor_name() {
    let s = Sandbox::new();
    code(&s, &["config", "mirror.npm", "tsinghua"], 2);
    assert_eq!(s.mirror().npm, "");
}

#[test]
fn test_mirror_master_off_preserves_urls_and_on_restores() {
    let s = Sandbox::new();
    ok(&s, &["config", "mirror.pypi", "tsinghua"]);
    ok(&s, &["config", "mirror", "off"]);
    let paused = s.mirror();
    assert!(!paused.enabled);
    assert_eq!(paused.pypi, TSINGHUA);
    ok(&s, &["config", "mirror", "on"]);
    let active = s.mirror();
    assert!(active.enabled);
    assert_eq!(active.pypi, TSINGHUA);
}

#[test]
fn test_mirror_master_on_with_nothing_saved_exits_2() {
    let s = Sandbox::new();
    let output = code(&s, &["config", "mirror", "on"], 2);
    assert!(text(&output).contains("mirror.pypi"), "{}", text(&output));
}

#[test]
fn test_mirror_master_rejects_vendor_names_with_axis_pointer() {
    let s = Sandbox::new();
    let output = code(&s, &["config", "mirror", "tsinghua"], 2);
    assert!(text(&output).contains("mirror.pypi"), "{}", text(&output));
    assert!(!s.mirror().enabled);
}

#[test]
fn test_paused_axis_write_preserves_other_axes_and_stays_paused() {
    let s = Sandbox::new();
    ok(&s, &["config", "mirror.pypi", "tsinghua"]);
    ok(&s, &["config", "mirror", "off"]);
    let output = ok(&s, &["config", "mirror.npm", "npmmirror"]);
    let mirror = s.mirror();
    assert!(!mirror.enabled);
    assert_eq!(mirror.pypi, TSINGHUA);
    assert_eq!(mirror.npm, NPMMIRROR);
    let stderr = String::from_utf8_lossy(&output.stderr).split_whitespace().collect::<Vec<_>>().join(" ");
    let stdout = String::from_utf8_lossy(&output.stdout).split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(stderr.contains("switched off"), "{}", text(&output));
    assert!(!stdout.contains("switched off"), "{}", text(&output));
    assert!(stdout.contains("mirror.npm = npmmirror"), "{}", text(&output));
}

#[test]
fn test_paused_axis_clear_leaves_other_axes_and_prints_no_notice() {
    let s = Sandbox::new();
    ok(&s, &["config", "mirror.pypi", "tsinghua"]);
    ok(&s, &["config", "mirror.npm", "npmmirror"]);
    ok(&s, &["config", "mirror", "off"]);
    let output = ok(&s, &["config", "mirror.npm", "off"]);
    let mirror = s.mirror();
    assert!(!mirror.enabled);
    assert_eq!(mirror.pypi, TSINGHUA);
    assert_eq!(mirror.npm, "");
    assert!(!text(&output).split_whitespace().collect::<Vec<_>>().join(" ").contains("switched off"), "{}", text(&output));
}

#[test]
fn test_paused_config_is_fully_visible_in_config_list() {
    let s = Sandbox::new();
    ok(&s, &["config", "mirror.pypi", "aliyun"]);
    ok(&s, &["config", "mirror.npm", "npmmirror"]);
    ok(&s, &["config", "mirror", "off"]);
    let output = ok(&s, &["config"]);
    let flat = String::from_utf8_lossy(&output.stdout).split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(flat.contains("mirror off"), "{flat}");
    assert!(flat.contains("mirror.pypi aliyun"), "{flat}");
    assert!(flat.contains("mirror.github off"), "{flat}");
    assert!(flat.contains("mirror.npm npmmirror"), "{flat}");
}

#[test]
fn test_read_mirror_axis_shows_custom_url() {
    let s = Sandbox::new();
    ok(&s, &["config", "mirror.npm", "https://my.registry"]);
    let output = ok(&s, &["config", "mirror.npm"]);
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "https://my.registry");
}

#[test]
fn test_mirror_github_read_value_round_trips() {
    for value in ["nju", "https://my.mirror/gh"] {
        let s = Sandbox::new();
        ok(&s, &["config", "mirror.github", value]);
        let shown = ok(&s, &["config", "mirror.github"]);
        let token = String::from_utf8(shown.stdout).unwrap().trim().to_owned();
        let before = s.mirror();
        ok(&s, &["config", "mirror.github", &token]);
        assert_eq!(s.mirror(), before, "initial={value:?}, shown={token:?}");
    }
}

#[test]
fn test_mirror_github_rejects_display_strings() {
    for bad in ["https://a b/gh", "https://a·b/gh", "https://x/py/ + https://x/uv"] {
        let s = Sandbox::new();
        ok(&s, &["config", "mirror.github", "nju"]);
        let before = fs::read(s.path().join("config.toml")).unwrap();
        code(&s, &["config", "mirror.github", bad], 2);
        assert_eq!(fs::read(s.path().join("config.toml")).unwrap(), before, "bad={bad:?}");
    }
}

#[test]
fn test_mirror_axis_rejects_whitespace_url() {
    let s = Sandbox::new();
    code(&s, &["config", "mirror.pypi", "https://a b/simple"], 2);
    assert_eq!(s.mirror().pypi, "");
}
