use std::{fs, path::{Path, PathBuf}, process::{Command as ProcessCommand, Output}};

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
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("source parent");
        }
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
        let output = self.run(&["add", "--cmd", template, "--name", name]);
        assert_success(&output);
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
            r#"use std::{env, fs::OpenOptions, io::Write as _};
fn main() {
    let args = env::args().skip(1).collect::<Vec<_>>();
    let capture = env::var("SKIT_FAKE_CAPTURE").unwrap();
    let mut file = OpenOptions::new().create(true).append(true).open(capture).unwrap();
    writeln!(file, "{}", args.join("\u{1e}")).unwrap();
}
"#,
        )
        .expect("fake uv source");
        let uv = self.bin.path().join(if cfg!(windows) { "uv.exe" } else { "uv" });
        let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
        let status = ProcessCommand::new(rustc)
            .arg(&source)
            .arg("-o")
            .arg(&uv)
            .status()
            .expect("compile fake uv");
        assert!(status.success(), "fake uv compile failed: {status}");
        uv
    }

    fn run_python_with_capture(&self, args: &[&str], capture: &Path) -> Output {
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

#[test]
fn test_params_table_escapes_markup_in_name_and_default() {
    let sandbox = Sandbox::new();
    sandbox.add_python("a", "print(1)\n");
    let stored = sandbox.data.path().join("scripts/a/script.py");
    fs::write(
        &stored,
        concat!(
            "# /// script\n",
            "# dependencies = []\n",
            "#\n",
            "# [tool.skit]\n",
            "# schema = 1\n",
            "#\n",
            "# [[tool.skit.params]]\n",
            "# name = \"[red]NAME[/red]\"\n",
            "# kind = \"const\"\n",
            "# type = \"str\"\n",
            "# default = \"[blue]hi[/blue]\"\n",
            "# ///\n",
            "print(1)\n",
        ),
    )
    .expect("hand-edit managed metadata");
    let output = sandbox.run(&["params", "a"]);
    assert_success(&output);
    let shown = combined(&output);
    assert!(shown.contains("[red]NAME[/red]"), "{shown}");
    assert!(shown.contains("[blue]hi[/blue]"), "{shown}");
}

#[test]
fn test_doctor_uv_path_escapes_markup() {
    let sandbox = Sandbox::new();
    let bin = sandbox.home.path().join("usr/[red]bin[/red]");
    fs::create_dir_all(&bin).expect("markup bin");
    let uv = bin.join(if cfg!(windows) { "uv.exe" } else { "uv" });
    fs::copy(env!("CARGO_BIN_EXE_skit"), &uv).expect("copy fake uv");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mut permissions = fs::metadata(&uv).expect("uv metadata").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&uv, permissions).expect("uv mode");
    }
    let output = sandbox.command().env("PATH", &bin).arg("doctor").output().expect("doctor");
    assert_success(&output);
    assert!(combined(&output).contains("[red]bin[/red]"), "{}", combined(&output));
}

#[test]
fn test_edit_params_updated_summary_escapes_markup_in_name() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("job.py", "X = 1\nprint(X)\n");
    let output = sandbox.run(&[
        "add",
        source.to_str().unwrap(),
        "--name",
        "[blue]a[/blue]",
        "--no-input",
    ]);
    assert_success(&output);
    let output = sandbox.run(&["params", "[blue]a[/blue]", "--resync"]);
    assert_success(&output);
    assert!(combined(&output).contains("[blue]a[/blue]"), "{}", combined(&output));
}

#[test]
fn test_edit_params_malformed_prompt_escapes_markup() {
    let sandbox = Sandbox::new();
    sandbox.add_python("a", "X = 1\nprint(X)\n");
    let output = sandbox.run(&["params", "a", "--prompt", "[red]bad[/red]"]);
    assert_success(&output);
    assert!(combined(&output).contains("[red]bad[/red]"), "{}", combined(&output));
}

#[test]
fn test_run_reusing_last_arguments_escapes_markup() {
    let sandbox = Sandbox::new();
    sandbox.add_command("j", "echo ready");
    sandbox.write_state("j", "extra_args = [\"[red]arg[/red]\"]\n");
    let output = sandbox.run(&["run", "j", "--no-input"]);
    assert_success(&output);
    assert!(combined(&output).contains("[red]arg[/red]"), "{}", combined(&output));
}

#[test]
fn test_run_raw_passes_argv_genuinely_raw() {
    let sandbox = Sandbox::new();
    fs::write(sandbox.home.path().join("match.txt"), b"x").expect("match");
    sandbox.add_python("rawr", "print(1)\n");
    let capture = sandbox.home.path().join("capture.log");
    let output = sandbox.run_python_with_capture(
        &["run", "rawr", "--raw", "--no-input", "--", "{env:UNSET}", "*.txt"],
        &capture,
    );
    assert_success(&output);
    let argv = fs::read_to_string(capture).expect("capture");
    assert!(argv.contains("{env:UNSET}"), "{argv}");
    assert!(argv.contains("*.txt"), "{argv}");
    assert!(!argv.contains("match.txt"), "raw argv must not glob: {argv}");
}

#[test]
fn test_run_cli_argv_not_reexpanded() {
    let sandbox = Sandbox::new();
    fs::write(sandbox.home.path().join("match.txt"), b"x").expect("match");
    sandbox.add_python("noglob", "print(1)\n");
    let capture = sandbox.home.path().join("capture.log");
    let output = sandbox.run_python_with_capture(
        &["run", "noglob", "--no-input", "--", "*.txt"],
        &capture,
    );
    assert_success(&output);
    let argv = fs::read_to_string(capture).expect("capture");
    assert!(argv.contains("*.txt"), "{argv}");
    assert!(!argv.contains("match.txt"), "explicit shell argv must not be expanded again: {argv}");
}
