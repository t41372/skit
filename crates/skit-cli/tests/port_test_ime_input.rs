//! Architecture-equivalent port of Python `tests/test_ime_input.py` from `main@206f9ef`.
//!
//! Python needs three import-order/environment tests because Textual may push kitty keyboard
//! enhancement mode (`ESC [>25u`) unless its opt-out is visible before Textual imports. The Rust
//! frontend has no Textual import or `TEXTUAL_DISABLE_KITTY_KEY` seam, so reproducing those env-var
//! assertions would test invented behavior. The user-visible contract is stronger and simpler here:
//! starting the real TUI on a PTY must not emit a kitty keyboard *push* or *set* sequence at all.
//! This directly guards the iTerm2/macOS CJK-IME regression the Python tests were written for.

use std::{
    fs,
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use tempfile::TempDir;

fn run_tui_and_capture_raw(data: &Path, state: &Path, config: &Path) -> (u32, Vec<u8>) {
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 24,
            cols: 100,
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
    let mut child = pair.slave.spawn_command(command).unwrap();
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().unwrap();
    let drain = thread::spawn(move || {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).unwrap();
        bytes
    });
    let mut writer = pair.master.take_writer().unwrap();

    // Crossterm may ask for the cursor position during setup. Answer that query before sending the
    // ordinary Library quit key so this test observes a normal initialized TUI, not an aborted one.
    thread::sleep(Duration::from_millis(60));
    writer.write_all(b"\x1b[1;1R").unwrap();
    writer.flush().unwrap();
    thread::sleep(Duration::from_millis(180));
    writer.write_all(b"q").unwrap();
    writer.flush().unwrap();

    let status = child.wait().unwrap();
    drop(writer);
    (status.exit_code(), drain.join().unwrap())
}

fn kitty_keyboard_enable_sequences(output: &[u8]) -> Vec<Vec<u8>> {
    let mut found = Vec::new();
    let mut offset = 0;
    while offset + 3 <= output.len() {
        let is_enable_prefix = output[offset..].starts_with(b"\x1b[>")
            || output[offset..].starts_with(b"\x1b[=");
        if !is_enable_prefix {
            offset += 1;
            continue;
        }

        // Kitty keyboard push/set payloads are numeric flag lists terminated by `u`. Restricting
        // the payload shape avoids mistaking unrelated CSI `>` terminal-identification traffic for
        // keyboard enhancement.
        let payload = &output[offset + 3..];
        let end = payload
            .iter()
            .take(32)
            .position(|byte| *byte == b'u');
        if let Some(end) = end {
            let flags = &payload[..end];
            if !flags.is_empty()
                && flags
                    .iter()
                    .all(|byte| byte.is_ascii_digit() || *byte == b';')
            {
                found.push(output[offset..offset + 3 + end + 1].to_vec());
                offset += 3 + end + 1;
                continue;
            }
        }
        offset += 1;
    }
    found
}

#[test]
fn test_kitty_protocol_opt_out_is_effective_before_tui_input_starts() {
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    fs::create_dir_all(data.path()).unwrap();

    let (code, output) = run_tui_and_capture_raw(data.path(), state.path(), config.path());
    assert_eq!(
        code,
        0,
        "TUI did not complete its normal startup/quit round trip; raw output: {:?}",
        String::from_utf8_lossy(&output)
    );

    // Pin the exact incident sequence as a readable regression receipt, then reject every numeric
    // kitty push/set variant so changing flags cannot silently reintroduce the same IME failure.
    assert!(
        !output.windows(b"\x1b[>25u".len()).any(|window| window == b"\x1b[>25u"),
        "the exact Textual/iTerm2 regression sequence was emitted"
    );
    let enables = kitty_keyboard_enable_sequences(&output);
    assert!(
        enables.is_empty(),
        "the Rust TUI enabled kitty keyboard enhancements: {:?}",
        enables
            .iter()
            .map(|sequence| String::from_utf8_lossy(sequence).into_owned())
            .collect::<Vec<_>>()
    );
}
