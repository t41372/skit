//! Real-TUI ports of the four Settings contracts in Python `tests/test_rename.py`.
//!
//! These are intentionally not replaced with store-only tests. The real `skit tui` process opens
//! Entry settings, edits the focused name control, saves through the host transaction, and is then
//! checked at the authoritative store/CLI boundary. Red behavior stays red for the implementation
//! agent to fix.

use std::{
    fs,
    io::{Read as _, Write as _},
    path::PathBuf,
    thread,
    time::Duration,
};

use assert_cmd::Command;
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use skit_application::{
    CreateEntry, EntryMutationRepository, EntryPayload, EntryRepository, RepositoryError,
    SourcePermissions,
};
use skit_domain::{EntryKind, EntrySettings, StorageMode};
use skit_store::FileStore;
use tempfile::TempDir;

const ARGPARSE_WITH_CONST: &[u8] = b"import argparse\nTIMEOUT = 30\nap = argparse.ArgumentParser()\nap.add_argument('--out', required=True)\nap.parse_args()\n";

struct Sandbox {
    _root: TempDir,
    data: PathBuf,
    state: PathBuf,
    config: PathBuf,
    home: PathBuf,
}

impl Sandbox {
    fn new() -> Self {
        let root = TempDir::new().unwrap();
        let data = root.path().join("data");
        let state = root.path().join("state");
        let config = root.path().join("config");
        let home = root.path().join("home");
        for path in [&data, &state, &config, &home] {
            fs::create_dir_all(path).unwrap();
        }
        Self {
            _root: root,
            data,
            state,
            config,
            home,
        }
    }

    fn store(&self) -> FileStore {
        FileStore::new(&self.data)
    }

    fn add_python(&self, name: &str, bytes: &[u8]) -> skit_domain::Entry {
        let source = self.home.join(format!("{name}.py"));
        fs::write(&source, bytes).unwrap();
        self.store()
            .create(CreateEntry {
                name: name.to_owned(),
                kind: EntryKind::parse("python").unwrap(),
                mode: StorageMode::Copy,
                source: source.display().to_string(),
                workdir: "invoke".to_owned(),
                description: String::new(),
                payload: Some(EntryPayload {
                    bytes: bytes.to_vec(),
                    stored_name: Some("script.py".to_owned()),
                    permissions: SourcePermissions::default(),
                }),
                settings: EntrySettings::default(),
            })
            .unwrap()
    }

    fn payload(&self, selector: &str) -> PathBuf {
        let store = self.store();
        let entry = store.resolve(selector).unwrap();
        store.payload_path(&entry).unwrap()
    }

    fn show_json(&self, selector: &str) -> serde_json::Value {
        let output = self
            .cli()
            .args(["show", selector, "--json"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "show failed: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }

    fn cli(&self) -> Command {
        let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
        command
            .env("SKIT_DATA_DIR", &self.data)
            .env("SKIT_STATE_DIR", &self.state)
            .env("SKIT_CONFIG_DIR", &self.config)
            .env("SKIT_LANG", "en")
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("XDG_CONFIG_HOME", self.home.join("xdg-config"))
            .env("XDG_DATA_HOME", self.home.join("xdg-data"))
            .env("XDG_STATE_HOME", self.home.join("xdg-state"))
            .current_dir(&self.home);
        command
    }
}

fn run_tui(sandbox: &Sandbox, after_boot: impl FnOnce(), steps: &[(&[u8], u64)]) -> (u32, Vec<u8>) {
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
    command.env("SKIT_DATA_DIR", &sandbox.data);
    command.env("SKIT_STATE_DIR", &sandbox.state);
    command.env("SKIT_CONFIG_DIR", &sandbox.config);
    command.env("HOME", &sandbox.home);
    command.env("USERPROFILE", &sandbox.home);
    command.env("XDG_CONFIG_HOME", sandbox.home.join("xdg-config"));
    command.env("XDG_DATA_HOME", sandbox.home.join("xdg-data"));
    command.env("XDG_STATE_HOME", sandbox.home.join("xdg-state"));

    let mut child = pair.slave.spawn_command(command).unwrap();
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().unwrap();
    let drain = thread::spawn(move || {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).unwrap();
        bytes
    });
    let mut writer = pair.master.take_writer().unwrap();

    // Crossterm asks for cursor position while entering raw mode. Answer that protocol request
    // before any user key, then let the first Library frame settle.
    thread::sleep(Duration::from_millis(60));
    writer.write_all(b"\x1b[1;1R").unwrap();
    writer.flush().unwrap();
    thread::sleep(Duration::from_millis(180));
    after_boot();

    for (bytes, delay_ms) in steps {
        writer.write_all(bytes).unwrap();
        writer.flush().unwrap();
        thread::sleep(Duration::from_millis(*delay_ms));
    }

    let status = child.wait().unwrap();
    drop(writer);
    (status.exit_code(), drain.join().unwrap())
}

#[test]
fn test_settings_screen_renames_on_save() {
    let sandbox = Sandbox::new();
    let old = sandbox.add_python("old", b"print(1)\n");

    let (code, _) = run_tui(
        &sandbox,
        || {},
        &[
            (b"p", 220),
            (b"\x7f\x7f\x7f", 40),
            (b"shiny", 60),
            (b"\x13", 320), // Ctrl+S
            (b"q", 120),
        ],
    );

    assert_eq!(code, 0);
    let store = sandbox.store();
    let renamed = store.resolve("shiny").unwrap();
    assert_eq!(renamed.meta.name, "shiny");
    assert_eq!(renamed.slug, old.slug);
    assert!(matches!(
        store.resolve("old").unwrap_err(),
        RepositoryError::NotFound { .. }
    ));
}

#[test]
fn test_settings_screen_rename_conflict_stays_open() {
    let sandbox = Sandbox::new();
    let beta = sandbox.add_python("beta", b"print('beta')\n");

    // Boot with one row so beta is unambiguously selected. Add the conflicting name after the
    // Library frame exists; the save transaction must still re-read current store truth.
    let (code, _) = run_tui(
        &sandbox,
        || {
            sandbox.add_python("alpha", b"print('alpha')\n");
        },
        &[
            (b"p", 220),
            (b"\x7f\x7f\x7f\x7f", 40),
            (b"alpha", 60),
            (b"\x13", 280), // first save must conflict and keep Settings editable
            (b"\x7f\x7f\x7f\x7f\x7f", 40),
            (b"gamma", 60),
            (b"\x13", 320), // succeeds only if the failed save stayed on the same screen
            (b"q", 120),
        ],
    );

    assert_eq!(code, 0);
    let store = sandbox.store();
    assert_eq!(store.resolve("alpha").unwrap().meta.name, "alpha");
    let gamma = store.resolve("gamma").unwrap();
    assert_eq!(gamma.slug, beta.slug);
    assert!(matches!(
        store.resolve("beta").unwrap_err(),
        RepositoryError::NotFound { .. }
    ));
}

#[test]
fn test_settings_hides_manage_checkboxes_for_argparse_script() {
    let sandbox = Sandbox::new();
    sandbox.add_python("ap", ARGPARSE_WITH_CONST);
    assert_eq!(sandbox.show_json("ap")["param_source"], "argparse");

    let (code, bytes) = run_tui(&sandbox, || {}, &[(b"p", 320), (b"\x1b", 120), (b"q", 120)]);

    assert_eq!(code, 0);
    let output = String::from_utf8_lossy(&bytes);
    assert!(
        output.contains("comes from its own command-line arguments"),
        "reader-driven explanation never appeared in Settings:\n{output}"
    );
    assert!(
        !output.contains("Detected but not yet managed"),
        "Settings offered source-management candidates for an argparse-owned form:\n{output}"
    );
    assert!(
        !output.contains("TIMEOUT"),
        "the argparse constant leaked into a manage checkbox:\n{output}"
    );
}

#[test]
fn test_settings_save_keeps_argparse_source() {
    let sandbox = Sandbox::new();
    sandbox.add_python("ap2", ARGPARSE_WITH_CONST);
    let payload = sandbox.payload("ap2");
    let before = fs::read(&payload).unwrap();
    assert_eq!(sandbox.show_json("ap2")["param_source"], "argparse");

    let (code, _) = run_tui(&sandbox, || {}, &[(b"p", 220), (b"\x13", 320), (b"q", 120)]);

    assert_eq!(code, 0);
    assert_eq!(fs::read(&payload).unwrap(), before);
    assert!(
        !String::from_utf8_lossy(&before).contains("[tool.skit]"),
        "fixture unexpectedly started with a managed block"
    );
    assert_eq!(sandbox.show_json("ap2")["param_source"], "argparse");
}
