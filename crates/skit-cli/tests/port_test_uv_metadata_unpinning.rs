//! Direct CLI ports of the non-Textual contracts in Python `tests/test_uv_metadata_unpinning.py`
//! from `main@206f9ef`.

use std::fs;

use assert_cmd::Command;
use skit_application::EntryRepository as _;
use skit_domain::EntrySettings;
use skit_language::read_uv_metadata;
use skit_store::FileStore;
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

    fn source(&self) -> std::path::PathBuf {
        let path = self.home.path().join("s.py");
        fs::write(&path, "print(1)\n").unwrap();
        path
    }

    fn add(&self, name: &str, requires_python: Option<&str>) {
        let source = self.source();
        let mut command = self.command();
        command.arg("add").arg(source).args(["--name", name]);
        if let Some(value) = requires_python {
            command.args(["--python", value]);
        }
        command.arg("--no-input").assert().success();
    }

    fn stored_source(&self, slug: &str) -> String {
        fs::read_to_string(
            self.data
                .path()
                .join("scripts")
                .join(slug)
                .join("script.py"),
        )
        .unwrap()
    }

    fn uv_metadata(&self, slug: &str) -> skit_language::UvMetadata {
        read_uv_metadata(&self.stored_source(slug)).expect("stored copy must have a PEP 723 block")
    }

    fn meta_settings(&self, selector: &str) -> EntrySettings {
        let entry = FileStore::new(self.data.path()).resolve(selector).unwrap();
        EntrySettings::from_meta(&entry.meta)
    }

    fn dry_run(&self, selector: &str) -> String {
        let output = self
            .command()
            .args(["run", selector, "--dry-run", "--no-input"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "dry run failed: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
    }
}

#[test]
fn test_pin_unpin_repin_block_line_tracks_the_constraint_end_to_end() {
    let sandbox = Sandbox::new();
    sandbox.add("a", None);

    sandbox
        .command()
        .args(["deps", "a", "--python", ">=3.12"])
        .assert()
        .success();
    assert_eq!(sandbox.uv_metadata("a").requires_python, ">=3.12");
    assert!(sandbox.dry_run("a").contains("--python"));

    sandbox
        .command()
        .args(["deps", "a", "--python", "-"])
        .assert()
        .success();
    assert_eq!(sandbox.uv_metadata("a").requires_python, "");
    assert!(!sandbox.stored_source("a").contains("requires-python"));
    assert!(!sandbox.dry_run("a").contains("--python"));
    let json = sandbox
        .command()
        .args(["deps", "a", "--json"])
        .output()
        .unwrap();
    assert!(json.status.success());
    let payload: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(payload["requires_python"], "");

    sandbox
        .command()
        .args(["deps", "a", "--python", ">=3.13"])
        .assert()
        .success();
    assert_eq!(sandbox.uv_metadata("a").requires_python, ">=3.13");
    assert!(sandbox.dry_run("a").contains("--python"));
    assert_eq!(sandbox.meta_settings("a").requires_python, ">=3.13");
}

#[test]
fn test_deps_only_edit_preserves_a_pin_that_lives_only_in_the_block() {
    let sandbox = Sandbox::new();
    sandbox.add("a", Some(">=3.11"));

    assert_eq!(sandbox.meta_settings("a").requires_python, "");
    assert_eq!(sandbox.uv_metadata("a").requires_python, ">=3.11");

    sandbox
        .command()
        .args(["deps", "a", "--dep", "requests"])
        .assert()
        .success();

    let metadata = sandbox.uv_metadata("a");
    assert_eq!(metadata.requires_python, ">=3.11");
    assert_eq!(metadata.dependencies, ["requests"]);
}

#[test]
fn test_deps_only_edit_preserves_a_pin_that_lives_in_meta() {
    let sandbox = Sandbox::new();
    sandbox.add("a", None);
    sandbox
        .command()
        .args(["deps", "a", "--python", ">=3.10"])
        .assert()
        .success();
    assert_eq!(sandbox.meta_settings("a").requires_python, ">=3.10");

    sandbox
        .command()
        .args(["deps", "a", "--dep", "requests"])
        .assert()
        .success();

    assert_eq!(sandbox.meta_settings("a").requires_python, ">=3.10");
    let metadata = sandbox.uv_metadata("a");
    assert_eq!(metadata.requires_python, ">=3.10");
    assert_eq!(metadata.dependencies, ["requests"]);
}
