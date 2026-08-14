use std::{env, fs, path::{Path, PathBuf}, process::{Command as ProcessCommand, Output}};

use assert_cmd::Command;
use tempfile::TempDir;

struct Sandbox {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    home: TempDir,
    bin: TempDir,
}

impl Sandbox {
    fn new() -> Self {
        Self {
            data: TempDir::new().expect("data"),
            state: TempDir::new().expect("state"),
            config: TempDir::new().expect("config"),
            home: TempDir::new().expect("home"),
            bin: TempDir::new().expect("bin"),
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
        fs::write(&path, body).expect("source");
        path
    }

    fn add_python(&self, name: &str, body: &str) {
        let source = self.source(&format!("{name}.py"), body);
        let output = self.run(&[
            "add",
            source.to_str().expect("utf8 source"),
            "--name",
            name,
            "--no-input",
        ]);
        assert_success(&output);
    }

    fn add_command(&self, name: &str, template: &str) {
        assert_success(&self.run(&["add", "--cmd", template, "--name", name]));
    }

    fn write_state(&self, slug: &str, body: &str) {
        let root = self.state.path().join("values");
        fs::create_dir_all(&root).expect("values dir");
        fs::write(root.join(format!("{slug}.toml")), body).expect("state");
    }

    fn fake_uv(&self) -> PathBuf {
        let source = self.bin.path().join("fake_uv.rs");
        fs::write(
            &source,
            r#"use std::{env, fs::{self, OpenOptions}, io::Write as _, path::PathBuf};
fn main() {
    let args = env::args().skip(1).collect::<Vec<_>>();
    if let Some(capture) = env::var_os("SKIT_FAKE_CAPTURE") {
        let capture = PathBuf::from(capture);
        let mut file = OpenOptions::new().create(true).append(true).open(&capture).unwrap();
        writeln!(file, "CALL\t{}", args.join("\u{1e}")).unwrap();
        if let Some(index) = args.iter().position(|arg| arg == "--script") {
            if let Some(path) = args.get(index + 1) {
                writeln!(file, "SCRIPT\t{path}").unwrap();
                if let Ok(bytes) = fs::read(path) {
                    fs::write(capture.with_extension("script"), bytes).unwrap();
                }
            }
        }
    }
    let code = env::var("SKIT_FAKE_EXIT").ok().and_then(|value| value.parse::<i32>().ok()).unwrap_or(0);
    std::process::exit(code);
}
"#,
        )
        .expect("fake uv source");
        let uv = self.bin.path().join(if cfg!(windows) { "uv.exe" } else { "uv" });
        let rustc = env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
        let status = ProcessCommand::new(rustc)
            .arg(&source)
            .arg("-o")
            .arg(&uv)
            .status()
            .expect("compile fake uv");
        assert!(status.success(), "fake uv compiler failed: {status}");
        uv
    }

    fn run_with_fake_uv(&self, args: &[&str], capture: &Path) -> Output {
        self.fake_uv();
        self.command()
            .env("PATH", self.bin.path())
            .env("SKIT_FAKE_CAPTURE", capture)
            .args(args)
            .output()
            .expect("run with fake uv")
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

fn managed_param(name: &str, ty: &str, default: &str) -> String {
    format!(
        concat!(
            "# /// script\n",
            "# dependencies = []\n",
            "#\n",
            "# [tool.skit]\n",
            "# schema = 1\n",
            "#\n",
            "# [[tool.skit.params]]\n",
            "# name = {name:?}\n",
            "# kind = \"const\"\n",
            "# type = {ty:?}\n",
            "# ///\n",
            "{name} = {default}\n",
            "print({name})\n",
        ),
        name = name,
        ty = ty,
        default = default,
    )
}

fn capture_text(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|error| panic!("missing capture {}: {error}", path.display()))
}

fn captured_script_path(path: &Path) -> PathBuf {
    capture_text(path)
        .lines()
        .rev()
        .find_map(|line| line.strip_prefix("SCRIPT\t"))
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("no SCRIPT row in {}", capture_text(path)))
}

#[test]
fn test_run_python_with_params_injects() {
    let sandbox = Sandbox::new();
    sandbox.add_python("j", &managed_param("CITY", "str", "\"Taipei\""));
    sandbox.write_state("j", "[values]\nCITY = \"Kaohsiung\"\n");
    let capture = sandbox.home.path().join("capture.log");
    let output = sandbox.run_with_fake_uv(&["run", "j", "--no-input"], &capture);
    assert_success(&output);

    let stored = sandbox.data.path().join("scripts/j/script.py");
    let launched = captured_script_path(&capture);
    assert_ne!(launched, stored, "a managed override must launch an injected artifact");
    let bytes = fs::read_to_string(capture.with_extension("script")).expect("captured script");
    assert!(bytes.contains("Kaohsiung"), "injected value missing: {bytes}");
    assert!(!bytes.contains("CITY = \"Taipei\""), "definition default survived injection: {bytes}");
}

#[test]
fn test_run_extra_args_bypass_required_field_validation() {
    let sandbox = Sandbox::new();
    sandbox.add_python(
        "ar",
        concat!(
            "import argparse\n",
            "ap = argparse.ArgumentParser()\n",
            "ap.add_argument('-o', '--output', required=True)\n",
            "ap.parse_args()\n",
        ),
    );
    let capture = sandbox.home.path().join("capture.log");
    let output = sandbox.run_with_fake_uv(
        &["run", "ar", "--no-input", "--", "-o", "x.png"],
        &capture,
    );
    assert_success(&output);
    let calls = capture_text(&capture);
    assert!(calls.contains("-o"), "{calls}");
    assert!(calls.contains("x.png"), "{calls}");
}

#[test]
fn test_run_required_field_missing_without_extra_args_exits_125() {
    let sandbox = Sandbox::new();
    sandbox.add_python(
        "ar2",
        concat!(
            "import argparse\n",
            "ap = argparse.ArgumentParser()\n",
            "ap.add_argument('-o', '--output', required=True)\n",
            "ap.parse_args()\n",
        ),
    );
    let capture = sandbox.home.path().join("capture.log");
    sandbox.fake_uv();
    let output = sandbox
        .command()
        .env("PATH", sandbox.bin.path())
        .env("SKIT_FAKE_CAPTURE", &capture)
        .args(["run", "ar2", "--no-input"])
        .output()
        .expect("run required");
    assert_code(&output, 125);
    assert!(combined(&output).contains("output"), "{}", combined(&output));
    assert!(!capture.exists(), "required validation must fail before the runner is spawned");
}

#[test]
fn test_run_raw_skips_form() {
    let sandbox = Sandbox::new();
    let original = managed_param("CITY", "str", "\"Taipei\"");
    sandbox.add_python("j", &original);
    sandbox.write_state("j", "[values]\nCITY = \"Kaohsiung\"\n");
    let capture = sandbox.home.path().join("capture.log");
    let output = sandbox.run_with_fake_uv(&["run", "j", "--raw", "--no-input"], &capture);
    assert_success(&output);

    let launched = captured_script_path(&capture);
    assert_eq!(
        launched,
        sandbox.data.path().join("scripts/j/script.py"),
        "--raw must launch the stored source, not an injected artifact"
    );
    assert_eq!(
        fs::read_to_string(capture.with_extension("script")).expect("captured script"),
        original
    );
}

#[test]
fn test_run_passes_and_remembers_extra_args() {
    let sandbox = Sandbox::new();
    sandbox.add_python("j", "print(1)\n");
    let capture = sandbox.home.path().join("capture.log");
    let first = sandbox.run_with_fake_uv(
        &["run", "j", "--no-input", "--", "--flag", "v"],
        &capture,
    );
    assert_success(&first);
    let first_call = capture_text(&capture);
    assert!(first_call.contains("--flag"), "{first_call}");
    assert!(first_call.contains("v"), "{first_call}");

    fs::remove_file(&capture).expect("reset capture");
    let second = sandbox.run_with_fake_uv(&["run", "j", "--no-input"], &capture);
    assert_success(&second);
    let second_call = capture_text(&capture);
    assert!(second_call.contains("--flag"), "remembered flag missing: {second_call}");
    assert!(second_call.contains("v"), "remembered value missing: {second_call}");
}

#[test]
fn test_run_bad_typed_value_caught_at_validation() {
    let sandbox = Sandbox::new();
    sandbox.add_python("j", &managed_param("RETRIES", "int", "3"));
    sandbox.write_state("j", "[values]\nRETRIES = \"not-a-number\"\n");
    let capture = sandbox.home.path().join("capture.log");
    sandbox.fake_uv();
    let output = sandbox
        .command()
        .env("PATH", sandbox.bin.path())
        .env("SKIT_FAKE_CAPTURE", &capture)
        .args(["run", "j", "--no-input"])
        .output()
        .expect("run invalid typed value");
    assert_code(&output, 125);
    let shown = combined(&output);
    assert!(shown.contains("not-a-number"), "{shown}");
    assert!(shown.contains("whole number"), "{shown}");
    assert!(!shown.to_lowercase().contains("resync"), "{shown}");
    assert!(!capture.exists(), "bad typed input must fail before spawn");
}

#[test]
fn test_run_command_entry_collects_values() {
    let sandbox = Sandbox::new();
    sandbox.add_command("e", "echo {msg}");
    sandbox.write_state("e", "[values]\nmsg = \"hi\"\n");
    let output = sandbox.run(&["run", "e", "--no-input"]);
    assert_success(&output);
    assert!(combined(&output).contains("hi"), "{}", combined(&output));
}
