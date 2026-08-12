//! Remaining public CLI/store boundary ports from Python `tests/test_effective_uv_metadata.py`
//! at `main@206f9ef`.
//!
//! These cases deliberately exercise the real `skit deps` front door. In particular, the npm
//! pair observes filesystem cleanup, because preserving metadata while accidentally deleting the
//! private support tree would still violate the Python contract.

use std::fs;

use assert_cmd::Command;
use serde_json::Value as JsonValue;
use tempfile::TempDir;

struct Sandbox {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    home: TempDir,
}

impl Sandbox {
    fn new() -> Self {
        Self {
            data: TempDir::new().unwrap(),
            state: TempDir::new().unwrap(),
            config: TempDir::new().unwrap(),
            home: TempDir::new().unwrap(),
        }
    }

    fn command(&self) -> Command {
        let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
        command
            .env("SKIT_DATA_DIR", self.data.path())
            .env("SKIT_STATE_DIR", self.state.path())
            .env("SKIT_CONFIG_DIR", self.config.path())
            .env("SKIT_LANG", "en")
            .env("HOME", self.home.path())
            .env("USERPROFILE", self.home.path())
            .env("XDG_CONFIG_HOME", self.home.path().join("xdg-config"))
            .env("XDG_DATA_HOME", self.home.path().join("xdg-data"))
            .env("XDG_STATE_HOME", self.home.path().join("xdg-state"))
            .env_remove("FORCE_COLOR")
            .env_remove("NO_COLOR")
            .env_remove("CLICOLOR")
            .env_remove("CLICOLOR_FORCE")
            .current_dir(self.home.path());
        command
    }

    fn source(&self, name: &str, body: &str) -> std::path::PathBuf {
        let path = self.home.path().join(name);
        fs::write(&path, body).unwrap();
        path
    }

    fn json(&self, args: &[&str]) -> JsonValue {
        let output = self.command().args(args).output().unwrap();
        assert!(
            output.status.success(),
            "command failed: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }

    fn add_js_with_dependency(&self) -> std::path::PathBuf {
        let source = self.source("a.js", "console.log(1);\n");
        self.command()
            .arg("add")
            .arg(&source)
            .args(["--name", "j", "--kind", "js", "--no-input"])
            .assert()
            .success();
        self.command()
            .args(["deps", "j", "--dep", "chalk@^5"])
            .assert()
            .success();
        self.data.path().join("scripts/j")
    }
}

#[test]
fn test_deps_read_meta_carried_entry_is_unchanged() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("x.py", "print(1)\n");
    sandbox
        .command()
        .arg("add")
        .arg(&source)
        .args([
            "--name",
            "x",
            "--ref",
            "--dep",
            "requests",
            "--no-input",
        ])
        .assert()
        .success();

    let payload = sandbox.json(&["deps", "x", "--json"]);
    assert_eq!(payload["dependencies"], serde_json::json!(["requests"]));
    assert_eq!(payload["requires_python"], "");
}

#[test]
fn test_deps_read_js_entry_falls_through_to_meta() {
    let sandbox = Sandbox::new();
    sandbox.add_js_with_dependency();

    let payload = sandbox.json(&["deps", "j", "--json"]);
    assert_eq!(payload["dependencies"], serde_json::json!(["chalk@^5"]));
    assert_eq!(payload["requires_python"], "");
}

#[test]
fn test_update_dependencies_npm_none_does_not_sweep_node_modules() {
    let sandbox = Sandbox::new();
    let entry = sandbox.add_js_with_dependency();
    fs::write(
        entry.join("package.json"),
        "{\n  \"name\": \"skit-private-entry\",\n  \"private\": true,\n  \"dependencies\": {\n    \"chalk\": \"^5\"\n  }\n}\n",
    )
    .unwrap();
    fs::write(entry.join(".skit-deps"), "v1\nnode\n0000000000000000\n").unwrap();
    fs::create_dir(entry.join("node_modules")).unwrap();
    fs::write(entry.join("node_modules/sentinel"), "must survive an untouched read\n").unwrap();

    // No --dep and no --clear is the public spelling of the Python `dependencies=None` branch.
    let payload = sandbox.json(&["deps", "j", "--json"]);
    assert_eq!(payload["dependencies"], serde_json::json!(["chalk@^5"]));
    assert!(entry.join("package.json").is_file());
    assert!(entry.join(".skit-deps").is_file());
    assert!(entry.join("node_modules/sentinel").is_file());
}

#[test]
fn test_update_dependencies_npm_clear_does_sweep_node_modules() {
    let sandbox = Sandbox::new();
    let entry = sandbox.add_js_with_dependency();
    fs::write(
        entry.join("package.json"),
        "{\n  \"name\": \"skit-private-entry\",\n  \"private\": true,\n  \"dependencies\": {\n    \"chalk\": \"^5\"\n  }\n}\n",
    )
    .unwrap();
    fs::write(entry.join(".skit-deps"), "v1\nnode\n0000000000000000\n").unwrap();
    fs::create_dir(entry.join("node_modules")).unwrap();
    fs::write(entry.join("node_modules/sentinel"), "must be swept on explicit clear\n").unwrap();

    sandbox.command().args(["deps", "j", "--clear"]).assert().success();

    let payload = sandbox.json(&["deps", "j", "--json"]);
    assert_eq!(payload["dependencies"], serde_json::json!([]));
    assert_eq!(payload["requires_python"], "");
    assert!(!entry.join("node_modules").exists());
    assert!(!entry.join("package.json").exists());
    assert!(!entry.join(".skit-deps").exists());
}
