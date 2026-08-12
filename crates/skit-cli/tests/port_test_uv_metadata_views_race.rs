//! Real-process port of Python
//! `test_settings_save_diffs_against_compose_time_baseline_not_a_re_read` from
//! `tests/test_uv_metadata_views.py` at `main@206f9ef`.
//!
//! Python simulates the concurrent write by monkeypatching the store after the Settings screen has
//! composed. This Rust test drives the stronger real race: keep an actual `skit tui` Settings screen
//! open on the old effective metadata, mutate both UV axes through a second `skit deps` process, then
//! save the untouched old screen. The stale screen must not overwrite the concurrent values.

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

    fn stored_source(&self) -> String {
        fs::read_to_string(self.data.path().join("scripts/x/script.py")).unwrap()
    }
}

#[test]
fn test_settings_save_diffs_against_compose_time_baseline_not_a_re_read() {
    let sandbox = Sandbox::new();
    let source = sandbox.home.path().join("x.py");
    fs::write(&source, "print(1)\n").unwrap();
    sandbox
        .command()
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

    let before = read_uv_metadata(&sandbox.stored_source()).expect("initial PEP 723 block");
    assert_eq!(before.dependencies, ["requests"]);
    assert_eq!(before.requires_python, ">=3.11");

    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 40,
            cols: 130,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    let mut tui = CommandBuilder::new(PathBuf::from(env!("CARGO_BIN_EXE_skit")));
    tui.arg("tui");
    tui.env("TERM", "xterm-256color");
    tui.env("SKIT_LANG", "en");
    tui.env("SKIT_DATA_DIR", sandbox.data.path());
    tui.env("SKIT_STATE_DIR", sandbox.state.path());
    tui.env("SKIT_CONFIG_DIR", sandbox.config.path());
    tui.env("HOME", sandbox.home.path());
    tui.env("USERPROFILE", sandbox.home.path());
    let mut child = pair.slave.spawn_command(tui).unwrap();
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

    // Library `p` opens Entry settings. At this point the screen owns the requests/>=3.11 baseline.
    writer.write_all(b"p").unwrap();
    writer.flush().unwrap();
    thread::sleep(Duration::from_millis(300));

    // A different skit process moves both axes underneath the already-open screen.
    sandbox
        .command()
        .args([
            "deps",
            "x",
            "--dep",
            "numpy",
            "--python",
            ">=3.13",
        ])
        .assert()
        .success();
    let concurrent = read_uv_metadata(&sandbox.stored_source()).expect("concurrent PEP 723 block");
    assert_eq!(concurrent.dependencies, ["numpy"]);
    assert_eq!(concurrent.requires_python, ">=3.13");

    // Save the old screen without editing either dependency field. If Settings re-reads the new
    // effective metadata at save time and diffs against it, the stale requests/>=3.11 Inputs look
    // like edits and overwrite the concurrent process. Open-time baselines leave both axes alone.
    writer.write_all(b"\x13").unwrap(); // Ctrl+S
    writer.flush().unwrap();
    thread::sleep(Duration::from_millis(350));

    // Close robustly whether save returned to Library or reported a non-dependency warning, then
    // quit the Library. Broken-pipe on the final key only means the prior key already exited.
    let _ = writer.write_all(b"\x1b");
    let _ = writer.flush();
    thread::sleep(Duration::from_millis(120));
    let _ = writer.write_all(b"q");
    let _ = writer.flush();

    let status = child.wait().unwrap();
    drop(writer);
    let output = drain.join().unwrap();
    assert_eq!(
        status.exit_code(),
        0,
        "TUI did not exit cleanly: {}",
        String::from_utf8_lossy(&output)
    );

    let after = read_uv_metadata(&sandbox.stored_source()).expect("final PEP 723 block");
    assert_eq!(
        after.dependencies,
        ["numpy"],
        "untouched stale Settings overwrote the concurrent dependency edit"
    );
    assert_eq!(
        after.requires_python,
        ">=3.13",
        "untouched stale Settings overwrote the concurrent Python constraint"
    );
}
