//! Direct CLI ports from the audited compatibility tail of Python
//! `tests/test_langs.py` (`origin/main@206f9ef`). Each Rust test keeps the Python test name and
//! its WHY comment. The Python implementation is the behavioral oracle.

use std::fs;

use predicates::prelude::*;
use serde_json::Value;
use tempfile::TempDir;

struct Sandbox {
    data: TempDir,
    state: TempDir,
    config: TempDir,
}

impl Sandbox {
    fn new() -> Self {
        Self {
            data: TempDir::new().unwrap(),
            state: TempDir::new().unwrap(),
            config: TempDir::new().unwrap(),
        }
    }

    fn command(&self) -> assert_cmd::Command {
        let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
        command
            .env("SKIT_DATA_DIR", self.data.path())
            .env("SKIT_STATE_DIR", self.state.path())
            .env("SKIT_CONFIG_DIR", self.config.path())
            .env("SKIT_LANG", "en")
            .env("PATH", self.data.path().join("empty-path"));
        command
    }

    fn register(&self, slug: &str) {
        fs::write(
            self.data.path().join("registry.toml"),
            format!("[entries.{slug}]\n"),
        )
        .unwrap();
    }

    fn write_executable_entry(&self) {
        let source = self.data.path().join(if cfg!(windows) {
            "tool.exe"
        } else {
            "tool"
        });
        fs::write(&source, "not executed\n").unwrap();
        let entry = self.data.path().join("scripts/prog");
        fs::create_dir_all(&entry).unwrap();
        fs::write(
            entry.join("meta.toml"),
            format!(
                concat!(
                    "schema = 1\n",
                    "name = \"prog\"\n",
                    "kind = \"exe\"\n",
                    "mode = \"reference\"\n",
                    "source = {:?}\n",
                    "source_hash = \"\"\n",
                    "added_at = \"2026-08-10T00:00:00Z\"\n",
                    "id = \"0123456789abcdef0123456789abcdef\"\n",
                    "workdir = \"origin\"\n",
                    "description = \"\"\n",
                ),
                source.display().to_string()
            ),
        )
        .unwrap();
        self.register("prog");
    }

    fn write_python_entry(&self) {
        let entry = self.data.path().join("scripts/a");
        fs::create_dir_all(&entry).unwrap();
        fs::write(entry.join("script.py"), "print(1)\n").unwrap();
        fs::write(
            entry.join("meta.toml"),
            concat!(
                "schema = 1\n",
                "name = \"a\"\n",
                "kind = \"python\"\n",
                "mode = \"copy\"\n",
                "source = \"/old/a.py\"\n",
                "source_hash = \"\"\n",
                "added_at = \"2026-08-10T00:00:00Z\"\n",
                "id = \"1123456789abcdef0123456789abcdef\"\n",
                "workdir = \"invoke\"\n",
                "description = \"\"\n",
            ),
        )
        .unwrap();
        self.register("a");
    }
}

#[test]
fn test_params_exe_prints_plain_message_without_manage_dead_end() {
    // `--manage` hard-errors for kinds without an analyzer, so the empty-params message
    // must not send exe users down that dead end (it used to suggest --manage).
    let sandbox = Sandbox::new();
    sandbox.write_executable_entry();
    sandbox
        .command()
        .args(["params", "prog"])
        .assert()
        .success()
        .stdout(predicate::str::contains("has no managed parameters"))
        .stdout(predicate::str::contains("--manage").not());
}

#[test]
fn test_doctor_missing_uv_pure_exe_library_exits_zero() {
    // A library with no python entries runs fine without uv — exit 1 there sent
    // automation chasing a phantom problem. The uv line still prints.
    let sandbox = Sandbox::new();
    sandbox.write_executable_entry();
    sandbox
        .command()
        .args(["doctor"])
        .assert()
        .success()
        .stdout(predicate::str::contains("uv"));
}

#[test]
fn test_doctor_missing_uv_with_python_entry_exits_one() {
    let sandbox = Sandbox::new();
    sandbox.write_python_entry();
    sandbox
        .command()
        .args(["doctor"])
        .assert()
        .code(1);
}

#[test]
fn test_doctor_json_missing_uv_pure_exe_library_exits_zero() {
    let sandbox = Sandbox::new();
    sandbox.write_executable_entry();
    let output = sandbox.command().args(["doctor", "--json"]).output().unwrap();
    assert!(
        output.status.success(),
        "doctor failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["uv"], Value::Null);
}
