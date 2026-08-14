//! Exact shell/runtime selector CLI contracts from Python v0.4 `tests/test_config_cmd.py`.

#[path = "support/config_cmd.rs"]
mod support;

use std::fs;
use support::{Sandbox, text};
use tempfile::TempDir;

fn ok(s: &Sandbox, args: &[&str]) -> std::process::Output {
    let output = s.run(args);
    assert_eq!(output.status.code(), Some(0), "{}", text(&output));
    output
}

#[test]
fn test_read_bash_path_default_line() {
    let s = Sandbox::new();
    let output = ok(&s, &["config", "shell.bash_path"]);
    assert!(String::from_utf8_lossy(&output.stdout).contains("auto"), "{}", text(&output));
}

#[test]
fn test_set_bash_path_to_existing_file() {
    let s = Sandbox::new();
    let root = TempDir::new().unwrap();
    let bash = root.path().join("bash");
    fs::write(&bash, b"").unwrap();
    let path = bash.to_str().unwrap();
    let output = ok(&s, &["config", "shell.bash_path", path]);
    assert!(String::from_utf8_lossy(&output.stdout).replace('\n', "").contains(path), "{}", text(&output));
    let read = ok(&s, &["config", "shell.bash_path", "--json"]);
    assert_eq!(serde_json::from_slice::<serde_json::Value>(&read.stdout).unwrap(), serde_json::json!({"shell.bash_path":path}));
}

#[test]
fn test_set_bash_path_to_missing_file_is_usage_error() {
    let s = Sandbox::new();
    let root = TempDir::new().unwrap();
    let ghost = root.path().join("nope");
    let output = s.run(&["config", "shell.bash_path", ghost.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(2), "{}", text(&output));
    let read = ok(&s, &["config", "shell.bash_path", "--json"]);
    assert_eq!(serde_json::from_slice::<serde_json::Value>(&read.stdout).unwrap(), serde_json::json!({"shell.bash_path":""}));
}

#[test]
fn test_clear_bash_path_with_empty_value() {
    let s = Sandbox::new();
    let root = TempDir::new().unwrap();
    let bash = root.path().join("bash");
    fs::write(&bash, b"").unwrap();
    ok(&s, &["config", "shell.bash_path", bash.to_str().unwrap()]);
    ok(&s, &["config", "shell.bash_path", ""]);
    let read = ok(&s, &["config", "shell.bash_path", "--json"]);
    assert_eq!(serde_json::from_slice::<serde_json::Value>(&read.stdout).unwrap(), serde_json::json!({"shell.bash_path":""}));
}

#[test]
fn test_bare_config_lists_dotted_keys() {
    let s = Sandbox::new();
    let output = ok(&s, &["config"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("shell.bash_path"), "{stdout}");
    assert!(stdout.contains("js.runner"), "{stdout}");
}

#[test]
fn test_read_js_runner_default_line() {
    let s = Sandbox::new();
    let output = ok(&s, &["config", "js.runner"]);
    assert!(String::from_utf8_lossy(&output.stdout).contains("auto"), "{}", text(&output));
}

#[test]
fn test_set_js_runner() {
    for runner in ["deno", "bun", "node"] {
        let s = Sandbox::new();
        let output = ok(&s, &["config", "js.runner", runner]);
        assert!(String::from_utf8_lossy(&output.stdout).contains(runner), "runner={runner}: {}", text(&output));
        let read = ok(&s, &["config", "js.runner", "--json"]);
        assert_eq!(serde_json::from_slice::<serde_json::Value>(&read.stdout).unwrap(), serde_json::json!({"js.runner":runner}), "runner={runner}");
    }
}

#[test]
fn test_set_js_runner_unknown_is_usage_error() {
    let s = Sandbox::new();
    let output = s.run(&["config", "js.runner", "carrier-pigeon"]);
    assert_eq!(output.status.code(), Some(2), "{}", text(&output));
    assert!(text(&output).contains("deno"), "the error must name a real recovery choice: {}", text(&output));
    let read = ok(&s, &["config", "js.runner", "--json"]);
    assert_eq!(serde_json::from_slice::<serde_json::Value>(&read.stdout).unwrap(), serde_json::json!({"js.runner":""}));
}

#[test]
fn test_clear_js_runner_with_empty_value() {
    let s = Sandbox::new();
    ok(&s, &["config", "js.runner", "bun"]);
    ok(&s, &["config", "js.runner", ""]);
    let read = ok(&s, &["config", "js.runner", "--json"]);
    assert_eq!(serde_json::from_slice::<serde_json::Value>(&read.stdout).unwrap(), serde_json::json!({"js.runner":""}));
}
