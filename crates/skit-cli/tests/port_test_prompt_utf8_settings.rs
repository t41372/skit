//! Companion for Python
//! `test_tui_review_rescan_and_settings_reject_invalid_prompt_without_replacement_character`.
//!
//! The exact Python-named test lives in `skit-tui` and covers review rescan rendering. This real
//! process test covers the second half of that same contract: opening Settings on a now-invalid
//! stored prompt must surface the strict UTF-8 error instead of opening a replacement-decoded form.

use std::{
    fs,
    io::{Read as _, Write as _},
    path::PathBuf,
    thread,
    time::Duration,
};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use skit_application::{CreateEntry, EntryMutationRepository, EntryPayload, SourcePermissions};
use skit_domain::{EntryKind, EntrySettings, StorageMode};
use skit_store::FileStore;
use tempfile::TempDir;

#[test]
fn prompt_utf8_settings_surface_rejects_invalid_bytes_without_replacement_character() {
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();
    let store = FileStore::new(data.path());
    let source = home.path().join("settings.prompt.md");
    fs::write(&source, b"hello {{name}}\n").unwrap();
    let entry = store
        .create(CreateEntry {
            name: "settings-bad".to_owned(),
            kind: EntryKind::parse("prompt").unwrap(),
            mode: StorageMode::Copy,
            source: source.display().to_string(),
            workdir: "invoke".to_owned(),
            description: String::new(),
            payload: Some(EntryPayload {
                bytes: b"hello {{name}}\n".to_vec(),
                stored_name: Some("prompt.md".to_owned()),
                permissions: SourcePermissions::default(),
            }),
            settings: EntrySettings::default(),
        })
        .unwrap();
    let payload = store.payload_path(&entry).unwrap();
    fs::write(&payload, b"hello \xff changed\n").unwrap();

    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 32,
            cols: 132,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    let mut command = CommandBuilder::new(PathBuf::from(env!("CARGO_BIN_EXE_skit")));
    command.arg("tui");
    command.cwd(home.path());
    command.env("TERM", "xterm-256color");
    command.env("SKIT_DATA_DIR", data.path());
    command.env("SKIT_STATE_DIR", state.path());
    command.env("SKIT_CONFIG_DIR", config.path());
    command.env("SKIT_LANG", "en");
    command.env("HOME", home.path());
    command.env("USERPROFILE", home.path());
    command.env("XDG_CONFIG_HOME", home.path().join("xdg-config"));
    command.env("XDG_DATA_HOME", home.path().join("xdg-data"));
    command.env("XDG_STATE_HOME", home.path().join("xdg-state"));

    let mut child = pair.slave.spawn_command(command).unwrap();
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().unwrap();
    let drain = thread::spawn(move || {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).unwrap();
        bytes
    });
    let mut writer = pair.master.take_writer().unwrap();

    // Crossterm may query the cursor position while entering the alternate screen.
    writer.write_all(b"\x1b[1;1R").unwrap();
    writer.flush().unwrap();
    thread::sleep(Duration::from_millis(180));
    // `p` is the Rust Library's entry-settings door used by the existing real-TUI Settings tests.
    // If Settings incorrectly opens, Escape returns to the library before `q`; if the strict reader
    // refuses, Escape is harmless and `q` still exits.
    writer.write_all(b"p").unwrap();
    writer.flush().unwrap();
    thread::sleep(Duration::from_millis(180));
    // Keep Escape and q as separate terminal events: one read containing ESC+q may decode as Alt-q.
    writer.write_all(b"\x1b").unwrap();
    writer.flush().unwrap();
    thread::sleep(Duration::from_millis(90));
    writer.write_all(b"q").unwrap();
    writer.flush().unwrap();

    let status = child.wait().unwrap();
    drop(writer);
    let transcript = String::from_utf8_lossy(&drain.join().unwrap())
        .replace("\r\n", "\n")
        .replace('\r', "");

    assert_eq!(status.exit_code(), 0, "{transcript}");
    assert!(transcript.contains("isn't valid UTF-8"), "{transcript}");
    assert!(
        !transcript.contains('\u{fffd}'),
        "Settings replacement-decoded the prompt: {transcript}"
    );
    assert_eq!(fs::read(&payload).unwrap(), b"hello \xff changed\n");
}
