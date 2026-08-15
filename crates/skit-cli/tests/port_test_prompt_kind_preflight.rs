use std::{
    fs,
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
    process::{Command as ProcessCommand, Output},
    thread,
    time::Duration,
};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
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

    fn command(&self) -> assert_cmd::Command {
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

    fn set_plain_form(&self) {
        let output = self.run(&["config", "form", "plain"]);
        assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    }

    fn add_runner(&self, name: &str, argv: &[&str]) {
        let mut args = vec!["runner", "add", name, "--force", "--"];
        args.extend(argv.iter().copied());
        let output = self.run(&args);
        assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    }

    fn add_prompt(&self, name: &str, body: &str, runner: Option<&str>) {
        let source = self.source(&format!("{name}.prompt.md"), body);
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

fn compile_recorder(root: &Path, name: &str) -> (PathBuf, PathBuf) {
    let source = root.join(format!("{name}.rs"));
    let capture = root.join(format!("{name}.capture"));
    fs::write(
        &source,
        r#"
use std::{env, fs};
fn main() {
    let capture = env::var_os("SKIT_PREFLIGHT_CAPTURE").expect("capture");
    let args = env::args_os()
        .skip(1)
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    fs::write(capture, args.join("\u{001e}")).unwrap();
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

fn run_pty(sandbox: &Sandbox, args: &[&str], chunks: &[&[u8]], path: Option<&Path>) -> (u32, String) {
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 30,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    let mut command = CommandBuilder::new(PathBuf::from(env!("CARGO_BIN_EXE_skit")));
    command
        .args(args)
        .cwd(sandbox.home.path())
        .env("TERM", "xterm-256color")
        .env("SKIT_DATA_DIR", sandbox.data.path())
        .env("SKIT_STATE_DIR", sandbox.state.path())
        .env("SKIT_CONFIG_DIR", sandbox.config.path())
        .env("SKIT_LANG", "en")
        .env("HOME", sandbox.home.path())
        .env("USERPROFILE", sandbox.home.path())
        .env("XDG_CONFIG_HOME", sandbox.home.path().join("xdg-config"))
        .env("XDG_DATA_HOME", sandbox.home.path().join("xdg-data"))
        .env("XDG_STATE_HOME", sandbox.home.path().join("xdg-state"));
    if let Some(path) = path {
        command.env("PATH", path);
    }

    let mut child = pair.slave.spawn_command(command).unwrap();
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().unwrap();
    let drain = thread::spawn(move || {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).unwrap();
        bytes
    });
    let mut writer = pair.master.take_writer().unwrap();
    thread::sleep(Duration::from_millis(120));
    let _ = writer.write_all(b"\x1b[1;1R");
    let _ = writer.flush();
    for chunk in chunks {
        thread::sleep(Duration::from_millis(180));
        if writer.write_all(chunk).is_err() {
            break;
        }
        let _ = writer.flush();
    }
    let status = child.wait().unwrap();
    drop(writer);
    let output = String::from_utf8_lossy(&drain.join().unwrap())
        .replace("\r\n", "\n")
        .replace('\r', "");
    (status.exit_code(), output)
}

#[test]
fn test_preflight_checks_the_pin_only() {
    let sandbox = Sandbox::new();
    sandbox.set_plain_form();
    sandbox.add_prompt("unpinned", "Do {{a}}\n", None);

    // An unpinned prompt has no runner to preflight. In a real terminal it must reach the
    // run form's runner chooser instead of being rejected as non-executable up front. Ctrl+D only
    // terminates the test after that boundary has been observed; its eventual cancel code is not
    // part of this frozen preflight oracle.
    let (code, output) = run_pty(
        &sandbox,
        &["run", "unpinned", "--set", "a=1", "--plain"],
        &[b"\x04"],
        None,
    );
    assert!(output.contains("Prompt runner"), "unpinned prompt never reached the runner chooser: {output}");
    assert_ne!(code, 126, "unpinned prompt was incorrectly preflighted as if it had a pin: {output}");

    sandbox.add_runner("missing", &["definitely-not-installed", "{{prompt}}"]);
    sandbox.add_prompt("pinned", "Do {{a}}\n", Some("missing"));
    let (code, output) = run_pty(
        &sandbox,
        &["run", "pinned", "--set", "a=1", "--plain"],
        &[b"\x04"],
        Some(sandbox.tools.path()),
    );
    assert_eq!(code, 126, "pinned missing runner was not refused by preflight: {output}");
    assert!(output.contains("definitely-not-installed"), "{output}");
    assert!(!output.contains("Prompt runner"), "run form opened before a pinned missing runner was refused: {output}");
}

#[test]
fn test_preflight_explicit_runner_overrides_a_stale_pin() {
    let sandbox = Sandbox::new();
    let (working, capture) = compile_recorder(sandbox.tools.path(), "working");
    sandbox.add_runner("removed", &[working.to_str().unwrap(), "--removed", "{{prompt}}"]);
    sandbox.add_runner("working", &[working.to_str().unwrap(), "--working", "{{prompt}}"]);
    sandbox.add_prompt("p", "body", Some("removed"));
    let removed = sandbox.run(&["runner", "remove", "removed", "--yes"]);
    assert_eq!(removed.status.code(), Some(0), "{}", combined(&removed));

    let output = sandbox
        .command()
        .env("SKIT_PREFLIGHT_CAPTURE", &capture)
        .args(["run", "p", "--runner", "working", "--no-input"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    assert_eq!(
        fs::read_to_string(&capture).unwrap().split('\u{001e}').collect::<Vec<_>>(),
        ["--working", "body"],
        "stale stored pin vetoed or contaminated the explicit configured runner"
    );
}

#[test]
fn test_preflight_missing_body() {
    let sandbox = Sandbox::new();
    sandbox.set_plain_form();
    sandbox.add_prompt("p", "Do {{a}}\n", None);
    fs::remove_file(sandbox.data.path().join("scripts/p/prompt.md")).unwrap();

    let (code, output) = run_pty(
        &sandbox,
        &["run", "p", "--set", "a=1", "--plain"],
        &[b"\x04"],
        None,
    );
    assert_eq!(code, 127, "missing prompt body did not fail at preflight: {output}");
    assert!(!output.contains("Prompt runner"), "run form opened before the missing prompt body was refused: {output}");
}
