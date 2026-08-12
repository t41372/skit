//! High-value CLI ports from Python `tests/test_effective_uv_metadata.py` and
//! `tests/test_uv_metadata_views.py` at `main@206f9ef`.
//!
//! These tests keep the original observable boundaries: the stored PEP 723 block, human/JSON read
//! surfaces, and the dry-run launch command. They do not substitute the lower-level edit-plan tests
//! that already exist in `skit-language`.

use std::fs;

use assert_cmd::Command;
use serde_json::Value as JsonValue;
use skit_language::read_uv_metadata;
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

    fn python(&self) -> std::path::PathBuf {
        let path = self.home.path().join("x.py");
        fs::write(&path, "print(1)\n").unwrap();
        path
    }

    fn add_block_only(&self, requires_python: Option<&str>) {
        let source = self.python();
        let mut command = self.command();
        command.arg("add").arg(&source).args(["--dep", "requests"]);
        if let Some(requires_python) = requires_python {
            command.args(["--python", requires_python]);
        }
        command.arg("--no-input").assert().success();
    }

    fn stored_source(&self) -> String {
        fs::read_to_string(self.data.path().join("scripts/x/script.py")).unwrap()
    }

    fn meta_text(&self) -> String {
        fs::read_to_string(self.data.path().join("scripts/x/meta.toml")).unwrap()
    }

    fn json(&self, args: &[&str]) -> JsonValue {
        let output = self.command().args(args).output().unwrap();
        assert!(
            output.status.success(),
            "command failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }

    fn human(&self, args: &[&str]) -> String {
        let output = self.command().args(args).output().unwrap();
        assert!(
            output.status.success(),
            "command failed: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    }
}

#[test]
fn test_add_dep_then_python_pin_keeps_block_deps_end_to_end() {
    let sandbox = Sandbox::new();
    sandbox.add_block_only(None);

    let before = sandbox.stored_source();
    let before_metadata = read_uv_metadata(&before).expect("add --dep must create a PEP 723 block");
    assert_eq!(before_metadata.dependencies, ["requests"]);
    assert_eq!(before_metadata.requires_python, "");
    assert!(
        !sandbox.meta_text().lines().any(|line| line.trim_start().starts_with("dependencies =")),
        "add-time Python dependencies must live in the stored copy's PEP 723 block, not meta.toml"
    );

    sandbox
        .command()
        .args(["deps", "x", "--python", ">=3.12"])
        .assert()
        .success();

    let after = sandbox.stored_source();
    let metadata = read_uv_metadata(&after).expect("python-only edit must keep a readable block");
    assert_eq!(metadata.dependencies, ["requests"]);
    assert_eq!(metadata.requires_python, ">=3.12");

    let payload = sandbox.json(&["deps", "x", "--json"]);
    assert_eq!(payload["dependencies"], serde_json::json!(["requests"]));
    assert_eq!(payload["requires_python"], ">=3.12");
}

#[test]
fn test_add_dep_then_python_pin_run_command_carries_both() {
    let sandbox = Sandbox::new();
    sandbox.add_block_only(None);
    sandbox
        .command()
        .args(["deps", "x", "--python", ">=3.12"])
        .assert()
        .success();

    let output = sandbox
        .command()
        .args(["run", "x", "--dry-run", "--no-input"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "dry run failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(flat.contains("--python"), "{flat}");
    assert!(flat.contains(">=3.12"), "{flat}");
    assert!(flat.contains("--script"), "{flat}");
    assert!(!flat.contains("--with requests"), "block-owned deps must not be duplicated: {flat}");
}

#[test]
fn test_deps_read_human_reports_effective_block_only() {
    let sandbox = Sandbox::new();
    sandbox.add_block_only(Some(">=3.11"));

    let text = sandbox.human(&["deps", "x"]);
    assert!(text.contains("Dependencies of x: requests"), "{text}");
    assert!(text.contains("Python constraint: >=3.11"), "{text}");
}

#[test]
fn test_deps_read_json_reports_effective_block_only() {
    let sandbox = Sandbox::new();
    sandbox.add_block_only(Some(">=3.11"));

    let payload = sandbox.json(&["deps", "x", "--json"]);
    assert_eq!(payload["dependencies"], serde_json::json!(["requests"]));
    assert_eq!(payload["requires_python"], ">=3.11");
}

#[test]
fn test_show_json_reports_effective_deps_for_block_only() {
    let sandbox = Sandbox::new();
    sandbox.add_block_only(Some(">=3.11"));

    let payload = sandbox.json(&["show", "x", "--json"]);
    assert_eq!(payload["dependencies"], serde_json::json!(["requests"]));
    assert_eq!(payload["requires_python"], ">=3.11");
}

#[test]
fn test_show_human_block_only_prints_effective_deps_and_constraint() {
    let sandbox = Sandbox::new();
    sandbox.add_block_only(Some(">=3.11"));

    let text = sandbox.human(&["show", "x"]);
    assert!(text.contains("Dependencies: requests"), "{text}");
    assert!(text.contains("Python constraint: >=3.11"), "{text}");
}

#[test]
fn test_show_human_meta_carried_deps_unchanged() {
    let sandbox = Sandbox::new();
    let source = sandbox.python();
    sandbox
        .command()
        .arg("add")
        .arg(source)
        .args(["--name", "x", "--ref", "--dep", "rich", "--no-input"])
        .assert()
        .success();

    let text = sandbox.human(&["show", "x"]);
    assert!(text.contains("Dependencies: rich"), "{text}");
}

#[test]
fn test_show_human_no_uv_metadata_prints_neither_line() {
    let sandbox = Sandbox::new();
    let source = sandbox.python();
    sandbox
        .command()
        .arg("add")
        .arg(source)
        .args(["--name", "x", "--no-input"])
        .assert()
        .success();

    let text = sandbox.human(&["show", "x"]);
    assert!(!text.contains("Dependencies:"), "{text}");
    assert!(!text.contains("Python constraint:"), "{text}");
}
