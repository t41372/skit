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

    fn source_bytes(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.home.path().join(name);
        fs::write(&path, bytes).unwrap();
        path
    }

    fn add_runner(&self, name: &str, program: &Path) {
        let output = self.run(&[
            "runner",
            "add",
            name,
            "--",
            program.to_str().unwrap(),
            "{{prompt}}",
        ]);
        assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    }

    fn add_prompt(&self, source: &Path, name: &str, runner: Option<&str>) {
        let mut args = vec![
            "add",
            source.to_str().unwrap(),
            "--name",
            name,
            "--no-input",
        ];
        if let Some(runner) = runner {
            args.extend(["--runner", runner]);
        }
        let output = self.run(&args);
        assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    }
}

fn combined(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("skit-cli lives at <repo>/crates/skit-cli")
        .to_path_buf()
}

fn compile_recorder(root: &Path, name: &str) -> (PathBuf, PathBuf) {
    let source = root.join(format!("{name}.rs"));
    let capture = root.join(format!("{name}.capture"));
    fs::write(
        &source,
        r#"
use std::{env, fs};
fn main() {
    let capture = env::var_os("SKIT_PROMPT_KIND_CAPTURE").expect("capture");
    let prompt = env::args().nth(1).expect("one prompt argv element");
    fs::write(capture, prompt.as_bytes()).unwrap();
}
"#,
    )
    .unwrap();
    let executable = root.join(if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    });
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
fn test_target_is_the_prompt_body() {
    let sandbox = Sandbox::new();
    let source = sandbox.source_bytes("outside.prompt.md", b"body\n");
    sandbox.add_prompt(&source, "p", None);

    let target = sandbox.data.path().join("scripts/p/prompt.md");
    assert!(target.is_file(), "prompt target was not the stored prompt body: {}", target.display());
    assert_eq!(fs::read(&target).unwrap(), b"body\n");
    assert_ne!(target, source, "copy-mode prompt target incorrectly remained the original source");
}

#[test]
fn test_run_entry_preserves_crlf_bodies_byte_for_byte() {
    let sandbox = Sandbox::new();
    let (recorder, capture) = compile_recorder(sandbox.tools.path(), "crlf-recorder");
    sandbox.add_runner("rec", &recorder);

    let raw = fs::read(repo_root().join("tests/corpus/prompt/02_crlf.prompt.md")).unwrap();
    assert!(raw.windows(2).any(|pair| pair == b"\r\n"), "frozen corpus lost CRLF");
    let source = sandbox.source_bytes("crlf.prompt.md", &raw);
    sandbox.add_prompt(&source, "crlf", Some("rec"));

    let output = sandbox
        .command()
        .env("SKIT_PROMPT_KIND_CAPTURE", &capture)
        .args([
            "run",
            "crlf",
            "--set",
            "task=T",
            "--set",
            "repo=R",
            "--no-input",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));

    let expected = String::from_utf8(raw)
        .unwrap()
        .replace("{{task}}", "T")
        .replace("{{repo}}", "R")
        .into_bytes();
    let captured = fs::read(&capture).expect("real runner did not capture the prompt argv");
    assert_eq!(captured, expected);
    assert!(captured.windows(2).any(|pair| pair == b"\r\n"));
}

#[test]
fn test_run_entry_executes_the_recorder_end_to_end() {
    let sandbox = Sandbox::new();
    let (recorder, capture) = compile_recorder(sandbox.tools.path(), "injection-recorder");
    sandbox.add_runner("rec", &recorder);

    let raw = fs::read(repo_root().join("tests/corpus/prompt/04_injection.prompt.md")).unwrap();
    let text = String::from_utf8(raw.clone()).unwrap();
    let source = sandbox.source_bytes("inject.prompt.md", &raw);
    sandbox.add_prompt(&source, "inject", Some("rec"));

    let output = sandbox
        .command()
        .env("SKIT_PROMPT_KIND_CAPTURE", &capture)
        .args([
            "run",
            "inject",
            "--set",
            "path=src/x.py",
            "--no-input",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));

    let captured = fs::read_to_string(&capture).expect("real runner did not capture the prompt argv");
    assert_eq!(captured, text.replace("{{path}}", "src/x.py"));
    assert!(!sandbox.home.path().join("pwned").exists(), "prompt text was executed by a shell");
}
