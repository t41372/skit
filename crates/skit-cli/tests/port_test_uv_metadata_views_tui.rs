//! PTY ports of the library-detail effective-dependency contracts from Python
//! `tests/test_uv_metadata_views.py` at `main@206f9ef`.

use std::{
    fs,
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
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
            data: TempDir::new().unwrap(),
            state: TempDir::new().unwrap(),
            config: TempDir::new().unwrap(),
            home: TempDir::new().unwrap(),
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

    fn add_python(&self, with_dependency: bool) {
        let source = self.home.path().join("x.py");
        fs::write(&source, "print(1)\n").unwrap();
        let mut command = self.command();
        command.arg("add").arg(source).args(["--name", "x"]);
        if with_dependency {
            command.args(["--dep", "requests"]);
        }
        command.arg("--no-input").assert().success();
    }

    fn tui_output(&self) -> String {
        run_tui(
            self.data.path(),
            self.state.path(),
            self.config.path(),
            self.home.path(),
        )
    }
}

fn run_tui(data: &Path, state: &Path, config: &Path, home: &Path) -> String {
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 30,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    let mut command = CommandBuilder::new(PathBuf::from(env!("CARGO_BIN_EXE_skit")));
    command.arg("tui");
    command.env("TERM", "xterm-256color");
    command.env("SKIT_LANG", "en");
    command.env("SKIT_DATA_DIR", data);
    command.env("SKIT_STATE_DIR", state);
    command.env("SKIT_CONFIG_DIR", config);
    command.env("HOME", home);
    command.env("USERPROFILE", home);
    let mut child = pair.slave.spawn_command(command).unwrap();
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().unwrap();
    let drain = thread::spawn(move || {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).unwrap();
        bytes
    });
    let mut writer = pair.master.take_writer().unwrap();
    thread::sleep(Duration::from_millis(60));
    writer.write_all(b"\x1b[1;1R").unwrap();
    writer.flush().unwrap();
    thread::sleep(Duration::from_millis(220));
    writer.write_all(b"q").unwrap();
    writer.flush().unwrap();

    let status = child.wait().unwrap();
    drop(writer);
    let bytes = drain.join().unwrap();
    assert_eq!(
        status.exit_code(),
        0,
        "TUI failed: {}",
        String::from_utf8_lossy(&bytes)
    );
    String::from_utf8_lossy(&bytes).into_owned()
}

#[test]
fn test_detail_pane_block_only_shows_effective_depends_on() {
    let sandbox = Sandbox::new();
    sandbox.add_python(true);

    // Prove this is the block-only branch rather than a test that accidentally succeeds from raw
    // meta. Python's regression specifically requires the add-time dependency to live only in the
    // stored PEP 723 source block.
    let meta = fs::read_to_string(sandbox.data.path().join("scripts/x/meta.toml")).unwrap();
    assert!(
        !meta.lines().any(|line| line.trim_start().starts_with("dependencies =")),
        "dependency unexpectedly lives in meta.toml"
    );

    let output = sandbox.tui_output();
    assert!(output.contains("Depends on"), "{output}");
    assert!(output.contains("requests"), "{output}");
}

#[test]
fn test_detail_pane_no_deps_omits_the_depends_on_line() {
    let sandbox = Sandbox::new();
    sandbox.add_python(false);

    let output = sandbox.tui_output();
    assert!(!output.contains("Depends on"), "{output}");
}
