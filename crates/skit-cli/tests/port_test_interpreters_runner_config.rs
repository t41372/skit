//! JavaScript runtime configuration port from Python `tests/test_interpreters.py`.

use std::fs;

use assert_cmd::Command;
use skit_store::FileConfigStore;
use tempfile::TempDir;

struct Sandbox {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    home: TempDir,
    empty_path: TempDir,
}

impl Sandbox {
    fn new() -> Self {
        let sandbox = Self {
            data: TempDir::new().unwrap(),
            state: TempDir::new().unwrap(),
            config: TempDir::new().unwrap(),
            home: TempDir::new().unwrap(),
            empty_path: TempDir::new().unwrap(),
        };
        fs::write(
            sandbox.config.path().join("config.toml"),
            "[mirror]\nenabled = false\n",
        )
        .unwrap();
        sandbox
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
            .env("PATH", self.empty_path.path())
            .current_dir(self.home.path());
        command
    }
}

#[test]
fn test_runner_config_override() {
    let sandbox = Sandbox::new();
    FileConfigStore::new(sandbox.config.path())
        .set("js.runner", "node")
        .unwrap();
    let source = sandbox.home.path().join("d.js");
    fs::write(&source, "console.log(1)\n").unwrap();
    sandbox
        .command()
        .arg("add")
        .arg(&source)
        .args(["--name", "d", "--no-input"])
        .assert()
        .success();

    let output = sandbox
        .command()
        .args(["run", "d", "--dry-run", "--no-input"])
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let script = sandbox.data.path().join("scripts/d/script.js");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        format!("node {}", script.display())
    );
}
