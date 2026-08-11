//! User-facing state-shape regression from Python v0.4 `tests/test_argstate_mut.py`.
//!
//! A malformed last-run section in one hand-edited values file must not take down the entire
//! machine-readable library listing. This is a real CLI/storage assertion, not a repository-only
//! smoke test.

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

    fn ok(&self, args: &[&str]) -> std::process::Output {
        let output = self.command().args(args).output().unwrap();
        assert!(
            output.status.success(),
            "args={args:?}\nstdout={}\nstderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }
}

#[test]
fn test_a_scalar_last_run_still_lists_through_the_cli() {
    let sandbox = Sandbox::new();
    sandbox.ok(&[
        "add",
        "--cmd",
        "echo hi",
        "--name",
        "chores",
        "--no-input",
    ]);

    let values_dir = sandbox.state.path().join("values");
    fs::create_dir_all(&values_dir).unwrap();
    fs::write(values_dir.join("chores.toml"), "last_run = \"garbage\"\n").unwrap();

    let output = sandbox.ok(&["list", "--json"]);
    let rows: Vec<Value> = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "stdout must be exactly JSON: {error}\nstdout={}\nstderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    let [row] = rows.as_slice() else {
        panic!("expected exactly one library row: {rows:?}");
    };
    assert_eq!(row["name"], "chores");
    assert_eq!(row["last_run_at"], Value::Null);
    assert_eq!(row["last_exit"], Value::Null);
}
