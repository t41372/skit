use std::{fs, path::{Path, PathBuf}, process::{Command as ProcessCommand, Output}};

use assert_cmd::Command;
use tempfile::TempDir;

struct Sandbox {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    home: TempDir,
    tools: TempDir,
    capture: PathBuf,
    probe: PathBuf,
}

impl Sandbox {
    fn new() -> Self {
        let tools = TempDir::new().unwrap();
        let capture = tools.path().join("runner-argv.txt");
        let probe = compile_probe(tools.path());
        Self {
            data: TempDir::new().unwrap(),
            state: TempDir::new().unwrap(),
            config: TempDir::new().unwrap(),
            home: TempDir::new().unwrap(),
            tools,
            capture,
            probe,
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
            .env("SKIT_PROMPT_RUNNER_CAPTURE", &self.capture)
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

    fn configure_probe(&self, name: &str) {
        let output = self.run(&[
            "runner",
            "add",
            name,
            "--force",
            "--",
            self.probe.to_str().unwrap(),
            "--",
            "{{prompt}}",
        ]);
        assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    }

    fn add_prompt(&self, name: &str, body: &str, pin: Option<&str>) {
        let source = self.source(&format!("{name}.prompt.md"), body);
        let mut args = vec![
            "add",
            source.to_str().unwrap(),
            "--name",
            name,
            "--no-input",
        ];
        if let Some(pin) = pin {
            args.extend(["--runner", pin]);
        }
        let output = self.run(&args);
        assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    }

    fn captured(&self) -> Vec<String> {
        fs::read_to_string(&self.capture)
            .expect("runner capture")
            .split('\u{001e}')
            .map(str::to_owned)
            .collect()
    }

    fn last_runner(&self) -> String {
        let path = self.state.path().join("prompt.toml");
        let Ok(text) = fs::read_to_string(path) else {
            return String::new();
        };
        text.lines()
            .find_map(|line| line.strip_prefix("last_runner = \"").and_then(|v| v.strip_suffix('"')))
            .unwrap_or_default()
            .to_owned()
    }
}

fn compile_probe(root: &Path) -> PathBuf {
    let source = root.join("prompt_runner_probe.rs");
    fs::write(
        &source,
        r#"
use std::{env, fs};
fn main() {
    let capture = env::var_os("SKIT_PROMPT_RUNNER_CAPTURE").expect("capture");
    let args = env::args_os()
        .skip(1)
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    fs::write(capture, args.join("\u{001e}")).expect("write capture");
}
"#,
    )
    .unwrap();
    let executable = root.join(if cfg!(windows) { "prompt-runner-probe.exe" } else { "prompt-runner-probe" });
    let status = ProcessCommand::new("rustc")
        .arg(&source)
        .arg("-o")
        .arg(&executable)
        .status()
        .unwrap();
    assert!(status.success(), "failed to compile prompt runner probe");
    executable
}

fn combined(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn test_run_prompt_runner_flag_threads_through() {
    let sandbox = Sandbox::new();
    sandbox.configure_probe("claude");
    sandbox.add_prompt("p", "Do {{a}}\n", None);
    let output = sandbox.run(&[
        "run", "p", "--runner", " claude ", "--set", "a=1", "--no-input",
    ]);
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    let args = sandbox.captured();
    assert_eq!(args, ["--", "Do 1\n"]);
    assert_eq!(sandbox.last_runner(), "claude", "an explicit --runner is a picker choice");
}

#[test]
fn test_run_prompt_unicode_placeholder_threads_through_set() {
    let sandbox = Sandbox::new();
    sandbox.configure_probe("claude");
    sandbox.add_prompt("p", "审查 {{目标}}\n", None);
    let output = sandbox.run(&[
        "run",
        "p",
        "--runner",
        "claude",
        "--set",
        "目标=src/app.py",
        "--no-input",
    ]);
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    assert_eq!(sandbox.captured(), ["--", "审查 src/app.py\n"]);
}

#[test]
fn test_run_prompt_pin_resolves_without_touching_last_picked() {
    let sandbox = Sandbox::new();
    sandbox.configure_probe("codex");
    sandbox.add_prompt("p", "Do {{a}}\n", Some("codex"));
    assert_eq!(sandbox.last_runner(), "", "adding a pin must not count as a run-time pick");
    let output = sandbox.run(&["run", "p", "--set", "a=1", "--no-input"]);
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    assert_eq!(sandbox.captured(), ["--", "Do 1\n"]);
    assert_eq!(sandbox.last_runner(), "", "using the stored pin must not rewrite picker state");
}

#[test]
fn test_run_prompt_extra_args_pass_through_after_dashes() {
    let sandbox = Sandbox::new();
    sandbox.configure_probe("claude");
    sandbox.add_prompt("p", "Do {{a}}\n", Some("claude"));
    let output = sandbox.run(&[
        "run", "p", "--set", "a=1", "--no-input", "--", "--model", "opus",
    ]);
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    assert_eq!(
        sandbox.captured(),
        ["--model", "opus", "--", "Do 1\n"],
        "agent flags must be inserted before the runner's -- separator"
    );
}

#[test]
fn test_run_prompt_reuses_last_extra_agent_args() {
    let sandbox = Sandbox::new();
    sandbox.configure_probe("claude");
    sandbox.add_prompt("p", "Do {{a}}\n", Some("claude"));

    let first = sandbox.run(&[
        "run", "p", "--set", "a=1", "--no-input", "--", "--model", "opus",
    ]);
    assert_eq!(first.status.code(), Some(0), "{}", combined(&first));
    assert_eq!(sandbox.captured(), ["--model", "opus", "--", "Do 1\n"]);

    let second = sandbox.run(&["run", "p", "--set", "a=1", "--no-input"]);
    assert_eq!(second.status.code(), Some(0), "{}", combined(&second));
    assert_eq!(
        sandbox.captured(),
        ["--model", "opus", "--", "Do 1\n"],
        "omitting a tail must replay the remembered agent flags"
    );

    let third = sandbox.run(&[
        "run", "p", "--set", "a=1", "--no-input", "--", "--model", "sonnet",
    ]);
    assert_eq!(third.status.code(), Some(0), "{}", combined(&third));
    assert_eq!(sandbox.captured(), ["--model", "sonnet", "--", "Do 1\n"]);
}

#[test]
fn test_normal_prompt_transparency_omits_body_but_keeps_agent_flags() {
    let sandbox = Sandbox::new();
    sandbox.configure_probe("claude");
    let body = format!("PRIVATE-DOCUMENT-START\n{}{{{{a}}}}\n", "detail ".repeat(2_000));
    sandbox.add_prompt("p", &body, Some("claude"));
    let output = sandbox.run(&[
        "run", "p", "--set", "a=done", "--no-input", "--", "--model", "opus",
    ]);
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    let shown = combined(&output);
    assert!(!shown.contains("PRIVATE-DOCUMENT-START"), "prompt body leaked into normal transparency: {shown}");
    assert!(shown.contains("rendered prompt omitted"), "{shown}");
    assert!(shown.contains("--model") && shown.contains("opus"), "{shown}");
    let captured = sandbox.captured();
    assert_eq!(&captured[..3], ["--model", "opus", "--"]);
    assert!(captured[3].starts_with("PRIVATE-DOCUMENT-START\n"));
    assert!(captured[3].ends_with("done\n"));
}
