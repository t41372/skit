use std::{
    fs,
    io::{Read as _, Write as _},
    path::PathBuf,
    process::Output,
    thread,
    time::Duration,
};

use assert_cmd::Command;
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
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
        fs::write(&path, body).expect("source");
        path
    }

    fn set_form(&self, value: &str) {
        let output = self.run(&["config", "form", value]);
        assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    }

    fn mark_first_run_done(&self) {
        let output = self.run(&["config", "mirror", "off"]);
        assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    }

    fn entry_exists(&self, slug: &str) -> bool {
        self.data.path().join("scripts").join(slug).is_dir()
    }
}

fn combined(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn flat(text: &str) -> String {
    text.replace("\r\n", "\n")
        .replace('\r', "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn run_pty(
    sandbox: &Sandbox,
    args: &[&str],
    term: &str,
    chunks: &[&[u8]],
) -> (u32, String) {
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 30,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("open pty");
    let mut command = CommandBuilder::new(PathBuf::from(env!("CARGO_BIN_EXE_skit")));
    command.args(args);
    command.cwd(sandbox.home.path());
    command.env("TERM", term);
    command.env("SKIT_DATA_DIR", sandbox.data.path());
    command.env("SKIT_STATE_DIR", sandbox.state.path());
    command.env("SKIT_CONFIG_DIR", sandbox.config.path());
    command.env("SKIT_LANG", "en");
    command.env("HOME", sandbox.home.path());
    command.env("USERPROFILE", sandbox.home.path());
    command.env("XDG_CONFIG_HOME", sandbox.home.path().join("xdg-config"));
    command.env("XDG_DATA_HOME", sandbox.home.path().join("xdg-data"));
    command.env("XDG_STATE_HOME", sandbox.home.path().join("xdg-state"));

    let mut child = pair.slave.spawn_command(command).expect("spawn pty child");
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().expect("pty reader");
    let drain = thread::spawn(move || {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).expect("read pty");
        bytes
    });
    let mut writer = pair.master.take_writer().expect("pty writer");
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
    let status = child.wait().expect("wait pty child");
    drop(writer);
    let output = String::from_utf8_lossy(&drain.join().expect("join reader"))
        .replace("\r\n", "\n")
        .replace('\r', "");
    (status.exit_code(), output)
}

#[test]
fn test_no_subcommand_dispatches_to_tui() {
    let sandbox = Sandbox::new();
    sandbox.mark_first_run_done();
    let (code, output) = run_pty(&sandbox, &[], "xterm-256color", &[b"\x03", b"\x03"]);
    assert_eq!(code, 0, "{output}");
    assert!(
        output.contains("\x1b[?1049h") || output.contains("\x1b[?1049l") || output.contains("skit"),
        "bare invocation never entered the terminal workbench: {output:?}"
    );
}

#[test]
fn test_add_interactive_panel_cancel_exits_130() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("job.py", "print(1)\n");
    let (code, output) = run_pty(
        &sandbox,
        &["add", source.to_str().unwrap(), "--name", "cancel-me"],
        "xterm-256color",
        &[b"\x1b"],
    );
    assert_eq!(code, 130, "{output}");
    assert!(!sandbox.entry_exists("cancel-me"), "cancelled review created an entry: {output}");
}

#[test]
fn test_add_interactive_plain_form_keeps_line_prompts() {
    let sandbox = Sandbox::new();
    sandbox.set_form("plain");
    let source = sandbox.source("plainly.py", "print(1)\n");
    let (code, output) = run_pty(
        &sandbox,
        &["add", source.to_str().unwrap(), "--name", "plainly"],
        "xterm-256color",
        &[b"\r", b"\r", b"\r"],
    );
    assert_eq!(code, 0, "{output}");
    let shown = flat(&output);
    assert!(shown.contains("Description (optional)"), "plain form skipped the line prompt: {shown}");
    assert!(sandbox.entry_exists("plainly"), "plain form did not create the entry: {shown}");
}

#[test]
fn test_add_term_dumb_keeps_line_prompts() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("dumb.py", "print(1)\n");
    let (code, output) = run_pty(
        &sandbox,
        &["add", source.to_str().unwrap(), "--name", "dumb"],
        "dumb",
        &[b"\r", b"\r", b"\r"],
    );
    assert_eq!(code, 0, "{output}");
    let shown = flat(&output);
    assert!(shown.contains("Description (optional)"), "TERM=dumb must use line prompts: {shown}");
    assert!(sandbox.entry_exists("dumb"), "TERM=dumb add did not commit: {shown}");
}

#[test]
fn test_add_exe_interactive_line_asks_name_and_description() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("tool", "#!/bin/sh\necho hi\n");
    let (code, output) = run_pty(
        &sandbox,
        &["add", source.to_str().unwrap(), "--exe"],
        "xterm-256color",
        &[b"nightly\r", b"runs the backup\r", b"\x1b"],
    );
    assert_eq!(code, 0, "{output}");
    let shown = flat(&output);
    assert!(shown.contains("Name in skit"), "{shown}");
    assert!(shown.contains("Description (optional)"), "{shown}");
    assert!(sandbox.entry_exists("nightly"), "interactive executable identity was not stored: {shown}");
}

#[test]
fn test_add_exe_interactive_skips_asks_when_name_and_description_given() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("tool2", "#!/bin/sh\necho hi\n");
    let (code, output) = run_pty(
        &sandbox,
        &[
            "add",
            source.to_str().unwrap(),
            "--exe",
            "--name",
            "tool",
            "--description",
            "does things",
        ],
        "xterm-256color",
        &[b"\r", b"\r", b"\x1b"],
    );
    assert_eq!(code, 0, "{output}");
    let shown = flat(&output);
    assert!(!shown.contains("Name in skit"), "explicit name still triggered a prompt: {shown}");
    assert!(!shown.contains("Description (optional)"), "explicit description still triggered a prompt: {shown}");
    assert!(sandbox.entry_exists("tool"), "explicit executable identity was not stored: {shown}");
}
