use std::{fs, path::PathBuf, process::Output};

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
            .current_dir(self.home.path());
        command
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command().args(args).output().unwrap()
    }

    fn source(&self, name: &str, body: &str) -> PathBuf {
        let path = self.home.path().join(name);
        fs::write(&path, body).unwrap();
        path
    }

    fn add_prompt(&self, name: &str, body: &str) {
        let source = self.source(&format!("{name}.prompt.md"), body);
        let output = self.run(&["add", source.to_str().unwrap(), "--name", name, "--no-input"]);
        assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    }

    fn json(&self, args: &[&str]) -> Value {
        let output = self.run(args);
        assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
        serde_json::from_slice(&output.stdout).unwrap()
    }
}

fn combined(output: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr))
}

#[test]
fn test_params_schema_edits_refused_while_insertion_is_off() {
    let sandbox = Sandbox::new();
    sandbox.add_prompt("p", "{{a}} {{b}}\n");
    let off = sandbox.run(&["params", "p", "--no-interpolate"]);
    assert_eq!(off.status.code(), Some(0), "{}", combined(&off));
    let before = sandbox.json(&["params", "p", "--json"]);
    for flags in [
        vec!["--add", "b"],
        vec!["--rm", "a"],
        vec!["--deliver", "a=placeholder"],
    ] {
        let mut args = vec!["params", "p"];
        args.extend(flags.iter().copied());
        let output = sandbox.run(&args);
        assert_eq!(output.status.code(), Some(1), "args={args:?}\n{}", combined(&output));
        assert!(combined(&output).contains("Variable insertion is off"), "args={args:?}\n{}", combined(&output));
        assert_eq!(sandbox.json(&["params", "p", "--json"]), before, "refused edit mutated state: {args:?}");
    }
    let on = sandbox.run(&["params", "p", "--interpolate"]);
    assert_eq!(on.status.code(), Some(0), "{}", combined(&on));
    let rm = sandbox.run(&["params", "p", "--rm", "b"]);
    assert_eq!(rm.status.code(), Some(0), "{}", combined(&rm));
}
