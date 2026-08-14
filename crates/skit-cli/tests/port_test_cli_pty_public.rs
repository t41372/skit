use std::{
    fs,
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
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

    fn add_command(&self, name: &str, template: &str) {
        let output = self.run(&["add", "--cmd", template, "--name", name]);
        assert_eq!(
            output.status.code(),
            Some(0),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn add_hostile_python_param(&self) {
        let source = self.home.path().join("hostile.py");
        fs::write(
            &source,
            concat!(
                "# /// script\n",
                "# dependencies = []\n",
                "#\n",
                "# [tool.skit]\n",
                "# schema = 1\n",
                "#\n",
                "# [[tool.skit.params]]\n",
                "# name = \"[red]msg[/red]\"\n",
                "# kind = \"const\"\n",
                "# type = \"str\"\n",
                "# ///\n",
                "print(1)\n",
            ),
        )
        .expect("hostile managed source");
        let output = self.run(&[
            "add",
            source.to_str().expect("utf8 source"),
            "--name",
            "e",
            "--no-input",
        ]);
        assert_eq!(
            output.status.code(),
            Some(0),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn entry_exists(&self, slug: &str) -> bool {
        self.data.path().join("scripts").join(slug).is_dir()
    }

    fn state_text(&self, slug: &str) -> String {
        fs::read_to_string(self.state.path().join("values").join(format!("{slug}.toml")))
            .unwrap_or_default()
    }
}

fn run_pty(sandbox: &Sandbox, args: &[&str], input: &[&[u8]]) -> (u32, String) {
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 24,
            cols: 100,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("open pty");
    let mut command = CommandBuilder::new(PathBuf::from(env!("CARGO_BIN_EXE_skit")));
    command.args(args);
    command.env("TERM", "xterm-256color");
    command.env("SKIT_LANG", "en");
    command.env("SKIT_DATA_DIR", sandbox.data.path());
    command.env("SKIT_STATE_DIR", sandbox.state.path());
    command.env("SKIT_CONFIG_DIR", sandbox.config.path());
    command.env("HOME", sandbox.home.path());
    command.env("USERPROFILE", sandbox.home.path());
    command.cwd(sandbox.home.path());

    let mut child = pair.slave.spawn_command(command).expect("spawn skit in pty");
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().expect("pty reader");
    let drain = thread::spawn(move || {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).expect("read pty");
        bytes
    });
    let mut writer = pair.master.take_writer().expect("pty writer");
    thread::sleep(Duration::from_millis(80));
    // Crossterm/dialoguer may ask for the current cursor position before accepting input.
    let _ = writer.write_all(b"\x1b[1;1R");
    let _ = writer.flush();
    for chunk in input {
        thread::sleep(Duration::from_millis(140));
        if writer.write_all(chunk).is_err() {
            break;
        }
        let _ = writer.flush();
    }
    let status = child.wait().expect("wait skit");
    drop(writer);
    let output = String::from_utf8_lossy(&drain.join().expect("join pty reader")).into_owned();
    (status.exit_code(), output)
}

fn assert_state_contains(text: &str, needles: &[&str]) {
    for needle in needles {
        assert!(text.contains(needle), "missing {needle:?} in state: {text}");
    }
}

#[test]
fn test_remove_confirm_abort() {
    let sandbox = Sandbox::new();
    sandbox.add_command("a", "echo hi");
    let (code, output) = run_pty(&sandbox, &["remove", "a"], &[b"n\n"]);
    assert_ne!(code, 0, "abort unexpectedly succeeded: {output}");
    assert!(sandbox.entry_exists("a"), "abort removed the entry: {output}");
}

#[test]
fn test_preset_save_command_with_params() {
    let sandbox = Sandbox::new();
    sandbox.add_command("e", "echo {msg}");
    let (code, output) = run_pty(
        &sandbox,
        &["preset", "save", "e", "prod"],
        &[b"hello\r"],
    );
    assert_eq!(code, 0, "{output}");
    let state = sandbox.state_text("e");
    assert_state_contains(&state, &["prod", "msg", "hello"]);
}

#[test]
fn test_preset_save_command_escapes_markup_in_preset_name_and_entry_name() {
    let sandbox = Sandbox::new();
    sandbox.add_command("[blue]e[/blue]", "echo {msg}");
    let (code, output) = run_pty(
        &sandbox,
        &[
            "preset",
            "save",
            "[blue]e[/blue]",
            "[green]p[/green]",
        ],
        &[b"hi\r"],
    );
    assert_eq!(code, 0, "{output}");
    assert!(output.contains("[green]p[/green]"), "{output}");
    assert!(output.contains("[blue]e[/blue]"), "{output}");
}

#[test]
fn test_preset_save_prompt_escapes_markup_in_placeholder_name() {
    let sandbox = Sandbox::new();
    // The frozen test injects a hostile declaration directly because normal command placeholders
    // are identifier-constrained. Use real hand-edited managed metadata here for the same reason:
    // reach the real preset-save prompt without first making parser acceptance the thing under test.
    sandbox.add_hostile_python_param();
    let (code, output) = run_pty(
        &sandbox,
        &["preset", "save", "e", "p"],
        &[b"x\r"],
    );
    assert_eq!(code, 0, "{output}");
    assert!(
        output.contains("[red]msg[/red]"),
        "the interactive preset prompt must render the hostile parameter name literally: {output}"
    );
    assert_state_contains(&sandbox.state_text("e"), &["[red]msg[/red]", "x"]);
}
