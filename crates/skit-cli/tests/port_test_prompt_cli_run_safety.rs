use std::{fs, path::{Path, PathBuf}, process::{Command as ProcessCommand, Output}};

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

    fn source(&self, name: &str, body: &str) -> PathBuf {
        let path = self.home.path().join(name);
        fs::write(&path, body).unwrap();
        path
    }

    fn add_prompt(&self, name: &str, body: &str, pin: Option<&str>) {
        let source = self.source(&format!("{name}.prompt.md"), body);
        let mut args = vec!["add", source.to_str().unwrap(), "--name", name, "--no-input"];
        if let Some(pin) = pin {
            args.extend(["--runner", pin]);
        }
        let output = self.run(&args);
        assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    }

    fn configure_probe(&self, name: &str, executable: &Path) {
        let output = self.run(&[
            "runner", "add", name, "--force", "--", executable.to_str().unwrap(), "{{prompt}}",
        ]);
        assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    }
}

fn combined(output: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr))
}

fn compile_capture(root: &Path, name: &str) -> PathBuf {
    let source = root.join(format!("{name}.rs"));
    fs::write(
        &source,
        r#"
use std::{env, fs};
fn main() {
    let args = env::args_os().skip(1).map(|v| v.to_string_lossy().into_owned()).collect::<Vec<_>>();
    if let Some(path) = env::var_os("SKIT_RUNNER_CAPTURE") {
        fs::write(path, args.join("\u{001e}")).unwrap();
    }
}
"#,
    ).unwrap();
    let executable = root.join(if cfg!(windows) { format!("{name}.exe") } else { name.to_owned() });
    let status = ProcessCommand::new("rustc").arg(source).arg("-o").arg(&executable).status().unwrap();
    assert!(status.success());
    executable
}

#[test]
fn test_extra_argv_does_not_hide_a_filled_flag_type_error() {
    let sandbox = Sandbox::new();
    let source = sandbox.source(
        "count.py",
        concat!(
            "import argparse\n",
            "p = argparse.ArgumentParser()\n",
            "p.add_argument('--count', type=int, required=True)\n",
            "p.parse_args()\n",
        ),
    );
    let added = sandbox.run(&["add", source.to_str().unwrap(), "--name", "count", "--no-input"]);
    assert_eq!(added.status.code(), Some(0), "{}", combined(&added));
    let output = sandbox.run(&[
        "run", "count", "--set", "count=nope", "--no-input", "--", "--count", "2",
    ]);
    assert_eq!(output.status.code(), Some(125), "{}", combined(&output));
    assert!(combined(&output).contains("whole number"), "{}", combined(&output));
}

#[test]
fn test_real_prompt_run_warns_before_sending_a_nonempty_secret() {
    let sandbox = Sandbox::new();
    let probe = compile_capture(sandbox.tools.path(), "secret-agent");
    sandbox.configure_probe("secret-agent", &probe);
    sandbox.add_prompt("sec", "Use {{api_key}}\n", Some("secret-agent"));
    let capture = sandbox.tools.path().join("secret-capture.txt");
    let output = sandbox
        .command()
        .env("SKIT_RUNNER_CAPTURE", &capture)
        .args(["run", "sec", "--set", "api_key=hunter2", "--no-input"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    let shown = combined(&output);
    assert!(shown.contains("never saved by skit"), "{shown}");
    assert!(shown.contains("selected agent as plaintext"), "{shown}");
    assert!(shown.contains("may log or sync"), "{shown}");
    assert!(!shown.contains("hunter2"), "secret leaked into transparency: {shown}");
    assert_eq!(fs::read_to_string(capture).unwrap(), "Use hunter2\n");
}

#[test]
fn test_noninteractive_pi_run_warns_and_uses_lossy_fallback() {
    for text in ["--help\nsecond line", "@README.md", "install", "config"] {
        let sandbox = Sandbox::new();
        let pi = compile_capture(sandbox.tools.path(), "pi");
        let capture = sandbox.tools.path().join("pi-capture.txt");
        sandbox.add_prompt("p", text, Some("pi"));
        let output = sandbox
            .command()
            .env("PATH", sandbox.tools.path())
            .env("SKIT_RUNNER_CAPTURE", &capture)
            .args(["run", "p", "--no-input"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0), "text={text:?}\n{}", combined(&output));
        let shown = combined(&output);
        assert!(shown.contains("Warning: Pi would interpret"), "{shown}");
        assert!(shown.contains("prepended one newline"), "{shown}");
        assert_eq!(fs::read_to_string(capture).unwrap(), format!("\n{text}"));
        assert!(pi.is_file());
    }
}

#[test]
fn test_noninteractive_pi_dry_run_warns_and_shows_fallback() {
    let sandbox = Sandbox::new();
    sandbox.add_prompt("p", "--help", Some("pi"));
    let output = sandbox.run(&["run", "p", "--dry-run", "--no-input"]);
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    let shown = combined(&output);
    assert!(shown.contains("Warning: Pi would interpret"), "{shown}");
    assert!(shown.contains("one character longer"), "{shown}");
    assert!(shown.contains("\n--help"), "{shown}");
}

#[test]
fn test_missing_runner_binary_refuses_before_any_delivery_output() {
    let sandbox = Sandbox::new();
    let configured = sandbox.run(&[
        "runner", "add", "missing", "--force", "--", "definitely-not-installed", "{{prompt}}",
    ]);
    assert_eq!(configured.status.code(), Some(0), "{}", combined(&configured));
    sandbox.add_prompt("sec", "Use {{api_key}}\n", Some("missing"));
    let output = sandbox
        .command()
        .env("PATH", "")
        .args(["run", "sec", "--set", "api_key=hunter2", "--no-input"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(126), "{}", combined(&output));
    let shown = combined(&output);
    assert!(shown.contains("definitely-not-installed"), "{shown}");
    assert!(!shown.contains("selected agent as plaintext"), "{shown}");
    assert!(!shown.contains('→'), "{shown}");
    assert!(!shown.contains("hunter2"), "{shown}");
}
