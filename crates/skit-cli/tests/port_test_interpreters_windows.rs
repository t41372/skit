#![cfg(windows)]

//! Windows interpreter-resolution ports from Python `tests/test_interpreters.py`.
//!
//! Python can monkeypatch `sys.platform`; Rust makes the platform a compile-time boundary. These
//! exact-name contracts therefore execute on Windows instead of pretending to exercise that branch
//! on POSIX. Missing-config diagnostics intentionally stay red until the Rust behavior matches v0.4.

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

    fn add_shell(&self, name: &str) {
        let source = self.home.path().join(format!("{name}.sh"));
        fs::write(&source, "#!/bin/bash\necho hi\n").unwrap();
        self.command()
            .arg("add")
            .arg(&source)
            .args(["--name", name, "--no-input"])
            .assert()
            .success();
    }

    fn add_ruby(&self, name: &str) {
        let source = self.home.path().join(format!("{name}.rb"));
        fs::write(&source, "puts 'hi'\n").unwrap();
        self.command()
            .arg("add")
            .arg(&source)
            .args(["--kind", "ruby", "--name", name, "--no-input"])
            .assert()
            .success();
    }

    fn set_bash_path(&self, value: &str) {
        FileConfigStore::new(self.config.path())
            .set("shell.bash_path", value)
            .unwrap();
    }
}

fn combined(output: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn test_resolve_bash_on_win32_uses_config_path_when_it_exists() {
    let sandbox = Sandbox::new();
    let bash = sandbox.home.path().join("bash.exe");
    fs::write(&bash, b"").unwrap();
    sandbox.set_bash_path(&bash.display().to_string());
    sandbox.add_shell("d");

    let output = sandbox
        .command()
        .args(["run", "d", "--dry-run", "--no-input"])
        .output()
        .unwrap();
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(0), "{text}");
    assert!(
        String::from_utf8_lossy(&output.stdout).contains(&bash.display().to_string()),
        "configured bash path was not selected: {text}"
    );
}

#[test]
fn test_resolve_bash_on_win32_configured_but_missing_falls_through() {
    let sandbox = Sandbox::new();
    let gone = sandbox.home.path().join("gone.exe");
    sandbox.set_bash_path(&gone.display().to_string());
    sandbox.add_shell("d");

    let output = sandbox
        .command()
        .args(["run", "d", "--no-input"])
        .output()
        .unwrap();
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(126), "{text}");
    assert!(text.contains("Git for Windows"), "{text}");
}

#[test]
fn test_resolve_bash_on_win32_unset_names_both_escape_hatches() {
    let sandbox = Sandbox::new();
    sandbox.add_shell("d");

    let output = sandbox
        .command()
        .args(["run", "d", "--no-input"])
        .output()
        .unwrap();
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(126), "{text}");
    assert!(text.contains("Git for Windows"), "{text}");
    assert!(text.contains("skit config shell.bash_path"), "{text}");
}

#[test]
fn test_resolve_nonbash_on_win32_gets_generic_message() {
    let sandbox = Sandbox::new();
    sandbox.add_ruby("r");

    let output = sandbox
        .command()
        .args(["run", "r", "--no-input"])
        .output()
        .unwrap();
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(126), "{text}");
    assert!(text.contains("ruby"), "{text}");
    assert!(!text.contains("Git for Windows"), "{text}");
}
