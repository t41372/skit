//! Real-TUI Settings ports from Python `tests/test_effective_uv_metadata.py` at `main@206f9ef`.
//!
//! These are intentionally not reducer-only checks. The entry is created through the real CLI,
//! Settings is opened through an actual `skit tui` PTY, and the final source is read back from the
//! store. A block-only dependency axis therefore has to survive the entire store -> host -> UI ->
//! save path.

use std::{
    fs,
    io::{Read as _, Write as _},
    path::PathBuf,
    thread,
    time::Duration,
};

use assert_cmd::Command;
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use skit_language::read_uv_metadata;
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
            .env_remove("FORCE_COLOR")
            .env_remove("NO_COLOR")
            .env_remove("CLICOLOR")
            .env_remove("CLICOLOR_FORCE")
            .current_dir(self.home.path());
        command
    }

    fn add_block_only(&self) {
        let source = self.home.path().join("x.py");
        fs::write(&source, "print(1)\n").unwrap();
        self.command()
            .arg("add")
            .arg(&source)
            .args([
                "--name",
                "x",
                "--dep",
                "requests",
                "--python",
                ">=3.11",
                "--no-input",
            ])
            .assert()
            .success();

        let meta = fs::read_to_string(self.data.path().join("scripts/x/meta.toml")).unwrap();
        assert!(
            !meta.lines()
                .any(|line| line.trim_start().starts_with("dependencies =")),
            "fixture must keep dependencies block-only: {meta}"
        );
        assert!(
            !meta.lines()
                .any(|line| line.trim_start().starts_with("requires_python =")),
            "fixture must keep the Python constraint block-only: {meta}"
        );
        let effective = read_uv_metadata(&self.stored_source()).expect("fixture PEP 723 block");
        assert_eq!(effective.dependencies, ["requests"]);
        assert_eq!(effective.requires_python, ">=3.11");
    }

    fn stored_source_path(&self) -> PathBuf {
        self.data.path().join("scripts/x/script.py")
    }

    fn stored_source(&self) -> String {
        fs::read_to_string(self.stored_source_path()).unwrap()
    }

    fn run_settings(&self, inputs: &[&[u8]]) -> (u32, String) {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 40,
                cols: 130,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let mut command = CommandBuilder::new(PathBuf::from(env!("CARGO_BIN_EXE_skit")));
        command.arg("tui");
        command.env("TERM", "xterm-256color");
        command.env("SKIT_LANG", "en");
        command.env("SKIT_DATA_DIR", self.data.path());
        command.env("SKIT_STATE_DIR", self.state.path());
        command.env("SKIT_CONFIG_DIR", self.config.path());
        command.env("HOME", self.home.path());
        command.env("USERPROFILE", self.home.path());
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
        writer.write_all(b"p").unwrap();
        writer.flush().unwrap();
        thread::sleep(Duration::from_millis(300));
        for input in inputs {
            let _ = writer.write_all(input);
            let _ = writer.flush();
            thread::sleep(Duration::from_millis(220));
        }
        let _ = writer.write_all(b"\x1b");
        let _ = writer.flush();
        thread::sleep(Duration::from_millis(120));
        let _ = writer.write_all(b"q");
        let _ = writer.flush();

        let status = child.wait().unwrap();
        drop(writer);
        let output = String::from_utf8_lossy(&drain.join().unwrap()).into_owned();
        (status.exit_code(), output)
    }
}

#[test]
fn test_settings_prefills_deps_and_python_from_the_block() {
    let sandbox = Sandbox::new();
    sandbox.add_block_only();

    let (code, output) = sandbox.run_settings(&[]);
    assert_eq!(code, 0, "{output}");
    // This is the terminal output of the real Settings screen. Both values must be present even
    // though meta.toml deliberately carries neither axis.
    assert!(output.contains("requests"), "block-only dependency was not rendered: {output}");
    assert!(
        output.contains(">=3.11"),
        "block-only Python constraint was not rendered: {output}"
    );
}

#[test]
fn test_settings_untouched_save_never_touches_the_deps_axis() {
    let sandbox = Sandbox::new();
    sandbox.add_block_only();
    let before = fs::read(sandbox.stored_source_path()).unwrap();

    let (code, output) = sandbox.run_settings(&[b"\x13"]); // Ctrl+S, no field edits.
    assert_eq!(code, 0, "{output}");

    let after = fs::read(sandbox.stored_source_path()).unwrap();
    assert_eq!(
        after, before,
        "an untouched Settings save rewrote the PEP 723 source block"
    );
    let effective = read_uv_metadata(std::str::from_utf8(&after).unwrap()).expect("PEP 723 block");
    assert_eq!(effective.dependencies, ["requests"]);
    assert_eq!(effective.requires_python, ">=3.11");
}
