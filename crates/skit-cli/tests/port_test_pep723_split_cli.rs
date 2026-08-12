//! Direct CLI ports of the dependency-call-site regressions in Python `tests/test_pep723_split.py`
//! at `main@206f9ef`.

use std::fs;

use assert_cmd::Command;
use serde_json::{Value as JsonValue, json};
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

    fn source(&self, name: &str, body: &str) -> std::path::PathBuf {
        let path = self.home.path().join(name);
        fs::write(&path, body).unwrap();
        path
    }

    fn assert_effective_dependencies(&self, selector: &str, expected: &[&str]) {
        let output = self
            .command()
            .args(["show", selector, "--json"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "show failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let payload: JsonValue = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(payload["dependencies"], json!(expected));
    }

    fn assert_stored_pep723_dependencies(&self, slug: &str, expected: &[&str]) {
        let source = fs::read_to_string(self.data.path().join("scripts").join(slug).join("script.py"))
            .unwrap();
        let metadata = read_uv_metadata(&source).expect("stored Python copy must own a PEP 723 block");
        let expected = expected.iter().map(|value| (*value).to_owned()).collect::<Vec<_>>();
        assert_eq!(metadata.dependencies, expected);
    }
}

#[test]
fn test_add_dep_flags_carry_specifier_commas() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("s.py", "import requests\nprint(requests)\n");

    sandbox
        .command()
        .arg("add")
        .arg(&source)
        .args([
            "--name",
            "r",
            "--dep",
            "requests>=2,<3",
            "--dep",
            "rich",
            "--no-input",
        ])
        .assert()
        .success();

    sandbox.assert_effective_dependencies("r", &["requests>=2,<3", "rich"]);
    sandbox.assert_stored_pep723_dependencies("r", &["requests>=2,<3", "rich"]);
}

#[test]
fn test_deps_dep_flags_carry_specifier_commas() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("s.py", "print(1)\n");

    sandbox
        .command()
        .arg("add")
        .arg(&source)
        .args(["--name", "a", "--no-input"])
        .assert()
        .success();

    sandbox
        .command()
        .args([
            "deps",
            "a",
            "--dep",
            "requests>=2,<3",
            "--dep",
            "rich",
        ])
        .assert()
        .success();

    sandbox.assert_effective_dependencies("a", &["requests>=2,<3", "rich"]);
    sandbox.assert_stored_pep723_dependencies("a", &["requests>=2,<3", "rich"]);
}
