use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command as ProcessCommand, Output},
};

use assert_cmd::Command;
use tempfile::TempDir;

struct Sandbox {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    home: TempDir,
    tools: TempDir,
}

impl Sandbox {
    fn new() -> Self {
        Self {
            data: TempDir::new().unwrap(),
            state: TempDir::new().unwrap(),
            config: TempDir::new().unwrap(),
            home: TempDir::new().unwrap(),
            tools: TempDir::new().unwrap(),
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
}

fn combined(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn compile_recorder(root: &Path) -> (PathBuf, PathBuf) {
    let source = root.join("rec.rs");
    let capture = root.join("spawned");
    fs::write(
        &source,
        r#"
use std::{env, fs};
fn main() {
    fs::write(env::var_os("SKIT_UNREADABLE_CAPTURE").expect("capture"), b"spawned").unwrap();
}
"#,
    )
    .unwrap();
    let executable = root.join(if cfg!(windows) { "rec.exe" } else { "rec" });
    assert!(
        ProcessCommand::new("rustc")
            .arg(source)
            .arg("-o")
            .arg(&executable)
            .status()
            .unwrap()
            .success()
    );
    (executable, capture)
}

#[test]
fn test_build_unreadable_body_is_a_clean_launch_error() {
    let sandbox = Sandbox::new();
    let (recorder, capture) = compile_recorder(sandbox.tools.path());
    let added_runner = sandbox.run(&[
        "runner",
        "add",
        "rec",
        "--",
        recorder.to_str().unwrap(),
        "{{prompt}}",
    ]);
    assert_eq!(added_runner.status.code(), Some(0), "{}", combined(&added_runner));

    let source = sandbox.home.path().join("p.prompt.md");
    fs::write(&source, "Do {{a}}\n").unwrap();
    let added = sandbox.run(&[
        "add",
        source.to_str().unwrap(),
        "--name",
        "p",
        "--runner",
        "rec",
        "--no-input",
    ]);
    assert_eq!(added.status.code(), Some(0), "{}", combined(&added));

    let body = sandbox.data.path().join("scripts/p/prompt.md");
    fs::remove_file(&body).unwrap();
    fs::create_dir(&body).unwrap();

    let output = sandbox
        .command()
        .env("SKIT_UNREADABLE_CAPTURE", &capture)
        .args(["run", "p", "--set", "a=1", "--no-input"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(125), "{}", combined(&output));
    let shown = combined(&output);
    assert!(shown.contains("Can't read"), "frozen clean read-error wording drifted: {shown}");
    assert!(shown.contains("prompt.md"), "read failure did not identify the prompt body: {shown}");
    assert!(!shown.contains("panicked at") && !shown.contains("stack backtrace"), "{shown}");
    assert!(!capture.exists(), "child spawned despite unreadable prompt body");
}
