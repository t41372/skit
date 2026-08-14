//! Exact redirected-output npm suggestion port from Python v0.4 `tests/test_js_deps.py`.

use std::fs;

use assert_cmd::Command;
use serde_json::Value as JsonValue;
use tempfile::TempDir;

#[test]
fn test_resolve_npm_dependencies_does_not_prompt_when_stdout_is_piped() {
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();
    let source = home.path().join("t.mjs");
    fs::write(&source, "import chalk from \"chalk\";\n").unwrap();

    let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
    let output = command
        .env("SKIT_DATA_DIR", data.path())
        .env("SKIT_STATE_DIR", state.path())
        .env("SKIT_CONFIG_DIR", config.path())
        .env("SKIT_LANG", "en")
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env("XDG_CONFIG_HOME", home.path().join("xdg-config"))
        .env("XDG_DATA_HOME", home.path().join("xdg-data"))
        .env("XDG_STATE_HOME", home.path().join("xdg-state"))
        .current_dir(home.path())
        .arg("add")
        .arg(&source)
        .args(["--name", "t"])
        .output()
        .unwrap();
    let rendered = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.status.code(), Some(0), "{rendered}");
    assert!(
        !rendered.contains("Dependencies"),
        "a redirected add asked an invisible dependency question:\n{rendered}"
    );

    let mut show = assert_cmd::cargo::cargo_bin_cmd!("skit");
    let shown = show
        .env("SKIT_DATA_DIR", data.path())
        .env("SKIT_STATE_DIR", state.path())
        .env("SKIT_CONFIG_DIR", config.path())
        .env("SKIT_LANG", "en")
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env("XDG_CONFIG_HOME", home.path().join("xdg-config"))
        .env("XDG_DATA_HOME", home.path().join("xdg-data"))
        .env("XDG_STATE_HOME", home.path().join("xdg-state"))
        .current_dir(home.path())
        .args(["deps", "t", "--json"])
        .output()
        .unwrap();
    assert_eq!(shown.status.code(), Some(0), "{}", String::from_utf8_lossy(&shown.stderr));
    let payload: JsonValue = serde_json::from_slice(&shown.stdout).unwrap();
    assert_eq!(payload["dependencies"], serde_json::json!(["chalk"]));
}