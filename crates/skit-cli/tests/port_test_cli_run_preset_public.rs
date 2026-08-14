use std::{fs, path::PathBuf, process::Output};

use assert_cmd::Command;
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
            data: TempDir::new().expect("data"),
            state: TempDir::new().expect("state"),
            config: TempDir::new().expect("config"),
            home: TempDir::new().expect("home"),
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
        self.command().args(args).output().expect("run skit")
    }

    fn source(&self, name: &str, body: &str) -> PathBuf {
        let path = self.home.path().join(name);
        fs::write(&path, body).expect("write source");
        path
    }

    fn add_python(&self, name: &str) {
        let source = self.source(&format!("{name}.py"), "print(1)\n");
        let output = self.run(&[
            "add",
            source.to_str().expect("utf8 source"),
            "--name",
            name,
            "--no-input",
        ]);
        assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    }

    fn add_command(&self, name: &str, template: &str) {
        let output = self.run(&["add", "--cmd", template, "--name", name]);
        assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    }

    fn write_state(&self, slug: &str, body: &str) {
        let dir = self.state.path().join("values");
        fs::create_dir_all(&dir).expect("values dir");
        fs::write(dir.join(format!("{slug}.toml")), body).expect("state file");
    }
}

fn combined(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn assert_code(output: &Output, code: i32) {
    assert_eq!(output.status.code(), Some(code), "{}", combined(output));
}

fn assert_success(output: &Output) {
    assert_code(output, 0);
}

#[test]
fn test_run_not_found_exits_127() {
    let output = Sandbox::new().run(&["run", "ghost"]);
    assert_code(&output, 127);
}

#[test]
fn test_run_unknown_preset_rejected() {
    let sandbox = Sandbox::new();
    sandbox.add_command("j", "echo ready");
    let output = sandbox.run(&["run", "j", "--preset", "nope", "--no-input"]);
    assert_code(&output, 2);
}

#[test]
fn test_run_command_reuses_last_extra_args() {
    let sandbox = Sandbox::new();
    sandbox.add_command("cmd", "echo ready");

    let first = sandbox.run(&["run", "cmd", "--no-input", "--", "--loud"]);
    assert_success(&first);
    assert!(combined(&first).contains("--loud"), "{}", combined(&first));

    let second = sandbox.run(&["run", "cmd", "--no-input"]);
    assert_success(&second);
    assert!(
        combined(&second).contains("--loud"),
        "a no-tail rerun must replay the stored tail: {}",
        combined(&second)
    );

    let third = sandbox.run(&["run", "cmd", "--no-input", "--", "--quiet"]);
    assert_success(&third);
    let shown = combined(&third);
    assert!(shown.contains("--quiet"), "{shown}");
    assert!(!shown.contains("--loud"), "an explicit tail replaces the remembered one: {shown}");
}

#[test]
fn test_run_nonzero_exit_propagates() {
    let sandbox = Sandbox::new();
    let template = if cfg!(windows) {
        "cmd /C exit 3"
    } else {
        "sh -c 'exit 3'"
    };
    sandbox.add_command("j", template);
    let output = sandbox.run(&["run", "j", "--no-input"]);
    assert_code(&output, 3);
}

#[test]
fn test_preset_list_none() {
    let sandbox = Sandbox::new();
    sandbox.add_python("a");
    assert_success(&sandbox.run(&["preset", "list", "a"]));
}

#[test]
fn test_preset_list_shows() {
    let sandbox = Sandbox::new();
    sandbox.add_command("a", "echo {CITY}");
    sandbox.write_state("a", "[presets.prod]\nCITY = \"Taipei\"\n");
    let output = sandbox.run(&["preset", "list", "a"]);
    assert_success(&output);
    let shown = combined(&output);
    assert!(shown.contains("prod"), "{shown}");
    assert!(shown.contains("Taipei"), "{shown}");
}

#[test]
fn test_preset_list_not_found() {
    let output = Sandbox::new().run(&["preset", "list", "ghost"]);
    assert_code(&output, 1);
}

#[test]
fn test_preset_delete() {
    let sandbox = Sandbox::new();
    sandbox.add_command("a", "echo {CITY}");
    sandbox.write_state("a", "[presets.prod]\nCITY = \"Taipei\"\n");
    let output = sandbox.run(&["preset", "delete", "a", "prod"]);
    assert_success(&output);
    let listed = sandbox.run(&["preset", "list", "a"]);
    assert_success(&listed);
    assert!(!combined(&listed).contains("prod"), "{}", combined(&listed));
    let raw = fs::read_to_string(sandbox.state.path().join("values/a.toml")).expect("state");
    assert!(!raw.contains("prod"), "deleted presets must disappear from disk: {raw}");
}

#[test]
fn test_preset_delete_unknown() {
    let sandbox = Sandbox::new();
    sandbox.add_python("a");
    let output = sandbox.run(&["preset", "delete", "a", "nope"]);
    assert_code(&output, 1);
}

#[test]
fn test_preset_delete_not_found() {
    let output = Sandbox::new().run(&["preset", "delete", "ghost", "p"]);
    assert_code(&output, 1);
}

#[test]
fn test_preset_save_not_found() {
    let output = Sandbox::new().run(&["preset", "save", "ghost", "p"]);
    assert_code(&output, 1);
}

#[test]
fn test_preset_save_python_no_params() {
    let sandbox = Sandbox::new();
    sandbox.add_python("a");
    let output = sandbox.run(&["preset", "save", "a", "p"]);
    assert_code(&output, 2);
}

#[test]
fn test_preset_save_command_no_params() {
    let sandbox = Sandbox::new();
    sandbox.add_command("e", "echo hi");
    let output = sandbox.run(&["preset", "save", "e", "p"]);
    assert_code(&output, 2);
}
