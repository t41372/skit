//! Exact CLI/storage ports of the public resync contracts in Python v0.4
//! `tests/test_source_default_semantics.py`.
//!
//! The Python suite is authoritative. These tests assert both the machine readback and the actual
//! stored `[tool.skit]` bytes so a projection-only implementation cannot satisfy the oracle.

use std::fs;

use assert_cmd::Command;
use serde_json::Value;
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
            .env_remove("PSModulePath")
            .current_dir(self.home.path());
        command
    }

    fn add_python(&self, name: &str, source: &str) {
        let path = self.home.path().join(format!("{name}.py"));
        fs::write(&path, source).unwrap();
        let output = self
            .command()
            .args(["add", path.to_str().unwrap(), "--name", name, "--no-input"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "stdout={}\nstderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn output(&self, args: &[&str]) -> std::process::Output {
        self.command().args(args).output().unwrap()
    }

    fn params(&self, slug: &str) -> Value {
        let output = self.output(&["params", slug, "--json"]);
        assert!(
            output.status.success(),
            "stdout={}\nstderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
            panic!(
                "stdout must be exactly JSON: {error}\nstdout={}\nstderr={}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
        })
    }

    fn stored(&self, slug: &str) -> String {
        fs::read_to_string(self.data.path().join("scripts").join(slug).join("script.py")).unwrap()
    }
}

fn parameter<'a>(document: &'a Value, name: &str) -> &'a Value {
    document["parameters"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["name"] == name)
        .unwrap_or_else(|| panic!("missing parameter {name}: {document}"))
}

#[test]
fn test_resync_writes_source_default_into_ok_and_type_changed_specs() {
    let sandbox = Sandbox::new();
    sandbox.add_python(
        "defaults",
        r#"# /// script
# dependencies = []
#
# [tool.skit]
# schema = 1
#
# [[tool.skit.params]]
# name = "CITY"
# kind = "const"
# type = "str"
# default = "old-city"
#
# [[tool.skit.params]]
# name = "RETRIES"
# kind = "const"
# type = "int"
# default = 3
# ///
CITY = "Taipei"
RETRIES = "three"
print(CITY, RETRIES)
"#,
    );

    let resync = sandbox.output(&["params", "defaults", "--resync"]);
    assert!(
        resync.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&resync.stdout),
        String::from_utf8_lossy(&resync.stderr)
    );

    let document = sandbox.params("defaults");
    let city = parameter(&document, "CITY");
    let retries = parameter(&document, "RETRIES");
    assert_eq!((city["type"].as_str(), city["default"].as_str()), (Some("str"), Some("Taipei")));
    assert_eq!((retries["type"].as_str(), retries["default"].as_str()), (Some("str"), Some("three")));

    let stored = sandbox.stored("defaults");
    assert!(stored.contains("default = \"Taipei\""), "{stored}");
    assert!(stored.contains("default = \"three\""), "{stored}");
    assert!(!stored.contains("default = \"old-city\""), "{stored}");
    assert!(!stored.contains("default = 3\n"), "{stored}");
}

#[test]
fn test_resync_current_default_and_rebind_and_untouched_input_share_one_pass() {
    let sandbox = Sandbox::new();
    sandbox.add_python(
        "mixed",
        r#"# /// script
# dependencies = []
#
# [tool.skit]
# schema = 1
#
# [[tool.skit.params]]
# name = "CITY"
# kind = "const"
# type = "str"
# default = "old"
#
# [[tool.skit.params]]
# name = "input-1"
# kind = "input"
# type = "str"
# order = 0
# prompt = "Name: "
#
# [[tool.skit.params]]
# name = "input-2"
# kind = "input"
# type = "str"
# order = 1
# prompt = "Old label: "
# ///
CITY = "Taipei"
who = input("Name: ")
pw = input("New label: ")
print(CITY, who, pw)
"#,
    );

    let resync = sandbox.output(&["params", "mixed", "--resync"]);
    assert!(
        resync.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&resync.stdout),
        String::from_utf8_lossy(&resync.stderr)
    );
    let stderr = String::from_utf8(resync.stderr).unwrap();
    assert!(
        stderr.lines().any(|line| {
            line == "input-2: re-anchored to its current position after its prompt stopped matching uniquely; double-check the prompt/secret assignment is still correct."
        }),
        "{stderr}"
    );

    let document = sandbox.params("mixed");
    let city = parameter(&document, "CITY");
    let first = parameter(&document, "input-1");
    let second = parameter(&document, "input-2");
    assert_eq!(city["default"], "Taipei");
    assert_eq!((first["order"].as_i64(), first["prompt"].as_str()), (Some(0), Some("Name: ")));
    assert_eq!((second["order"].as_i64(), second["prompt"].as_str()), (Some(1), Some("New label: ")));

    let stored = sandbox.stored("mixed");
    assert!(stored.contains("default = \"Taipei\""), "{stored}");
    assert!(stored.contains("prompt = \"Name: \""), "{stored}");
    assert!(stored.contains("prompt = \"New label: \""), "{stored}");
    assert!(!stored.contains("prompt = \"Old label: \""), "{stored}");
}
