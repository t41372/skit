//! Behavioral ports of the public `tests/test_tui_edit.py` contracts from Python `main@206f9ef`.
//!
//! Five of the six Python tests have observable Rust seams here. Source resolution is asserted at
//! the store boundary, while the editor round trip crosses the real `skit tui` PTY boundary. The
//! sixth Python test reaches Textual's private `_drift_cache` tuple directly. Rust has no equivalent
//! public cache object; that contract is intentionally not replaced with a weaker assertion here.
//! A future port must prove the post-editor Library projection re-derives drift, not merely that a
//! source file changed.

use std::{
    fs,
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
    process::Command as StdCommand,
    thread,
    time::Duration,
};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use skit_application::EntryRepository as _;
use skit_store::FileStore;
use tempfile::TempDir;

fn write_entry(
    data: &Path,
    slug: &str,
    name: &str,
    kind: &str,
    mode: &str,
    source: &Path,
    payload: Option<(&str, &[u8])>,
) {
    let directory = data.join("scripts").join(slug);
    fs::create_dir_all(&directory).unwrap();
    if let Some((filename, bytes)) = payload {
        fs::write(directory.join(filename), bytes).unwrap();
    }
    let template = (kind == "command")
        .then_some("template = \"echo hi\"\n")
        .unwrap_or("");
    fs::write(
        directory.join("meta.toml"),
        format!(
            concat!(
                "schema = 1\n",
                "name = {name:?}\n",
                "kind = {kind:?}\n",
                "mode = {mode:?}\n",
                "source = {source:?}\n",
                "source_hash = \"\"\n",
                "added_at = \"2026-08-12T00:00:00Z\"\n",
                "id = \"0123456789abcdef0123456789abcdef\"\n",
                "workdir = \"origin\"\n",
                "description = \"\"\n",
                "{template}",
            ),
            name = name,
            kind = kind,
            mode = mode,
            source = source.display().to_string(),
            template = template,
        ),
    )
    .unwrap();
    FileStore::new(data).rebuild_registry().unwrap();
}

fn compile_editor(root: &Path) -> PathBuf {
    let source = root.join("editor_probe.rs");
    fs::write(
        &source,
        r#"
use std::{env, fs, io::Write as _, path::PathBuf};
fn main() {
    let target = PathBuf::from(env::args_os().nth(1).expect("editor target"));
    if let Some(capture) = env::var_os("SKIT_EDIT_CAPTURE") {
        fs::write(capture, target.to_string_lossy().as_bytes()).expect("capture editor target");
    }
    if env::var_os("SKIT_EDIT_MARK").is_some() {
        let mut file = fs::OpenOptions::new().append(true).open(&target).expect("open target");
        file.write_all(b"# edited by probe\n").expect("edit target");
    }
}
"#,
    )
    .unwrap();
    let executable = root.join(editor_probe_name());
    let status = StdCommand::new("rustc")
        .arg(&source)
        .arg("-o")
        .arg(&executable)
        .status()
        .unwrap();
    assert!(status.success(), "failed to compile editor probe");
    executable
}

#[cfg(windows)]
fn editor_probe_name() -> &'static str {
    "editor-probe.exe"
}

#[cfg(not(windows))]
fn editor_probe_name() -> &'static str {
    "editor-probe"
}

fn tui_edit(
    data: &Path,
    state: &Path,
    config: &Path,
    editor: &Path,
    capture: &Path,
) -> (u32, String) {
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
    command.env("VISUAL", editor);
    command.env("EDITOR", editor);
    command.env("SKIT_EDIT_CAPTURE", capture);
    command.env("SKIT_EDIT_MARK", "1");
    let mut child = pair.slave.spawn_command(command).unwrap();
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().unwrap();
    let drain = thread::spawn(move || {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).unwrap();
        bytes
    });
    let mut writer = pair.master.take_writer().unwrap();

    // Crossterm asks for the cursor position during terminal setup. Answer it before sending user
    // input, then give each public interaction enough time to complete its synchronous host effect.
    thread::sleep(Duration::from_millis(60));
    writer.write_all(b"\x1b[1;1R").unwrap();
    writer.flush().unwrap();
    thread::sleep(Duration::from_millis(180));
    writer.write_all(b"e").unwrap();
    writer.flush().unwrap();
    thread::sleep(Duration::from_millis(300));
    writer.write_all(b"q").unwrap();
    writer.flush().unwrap();

    let status = child.wait().unwrap();
    drop(writer);
    let output = String::from_utf8_lossy(&drain.join().unwrap()).into_owned();
    (status.exit_code(), output)
}

#[test]
fn test_editable_source_copy_mode_points_at_the_stored_copy() {
    let data = TempDir::new().unwrap();
    let original = data.path().join("original.py");
    fs::write(&original, b"print('original')\n").unwrap();
    write_entry(
        data.path(),
        "a",
        "a",
        "python",
        "copy",
        &original,
        Some(("script.py", b"print(1)\n")),
    );

    let store = FileStore::new(data.path());
    let entry = store.resolve("a").unwrap();
    assert_eq!(
        store.payload_path(&entry).unwrap(),
        data.path().join("scripts/a/script.py")
    );
}

#[test]
fn test_editable_source_reference_mode_points_at_the_original() {
    let data = TempDir::new().unwrap();
    let original = data.path().join("orig.py");
    fs::write(&original, b"print(1)\n").unwrap();
    write_entry(
        data.path(),
        "r",
        "r",
        "python",
        "reference",
        &original,
        None,
    );

    let store = FileStore::new(data.path());
    let entry = store.resolve("r").unwrap();
    assert_eq!(store.payload_path(&entry).unwrap(), original);
}

#[test]
fn test_editable_source_command_entry_has_none() {
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    write_entry(
        data.path(),
        "c",
        "c",
        "command",
        "copy",
        Path::new(""),
        None,
    );

    let store = FileStore::new(data.path());
    let entry = store.resolve("c").unwrap();
    assert!(
        store.payload_path(&entry).is_err(),
        "a command template must not masquerade as an editable file"
    );

    let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
    command
        .env("SKIT_DATA_DIR", data.path())
        .env("SKIT_STATE_DIR", state.path())
        .env("SKIT_CONFIG_DIR", config.path())
        .env("SKIT_LANG", "en")
        .args(["edit", "c", "--no-input"])
        .assert()
        .code(2)
        .stderr(predicates::str::contains("does not have an editable source"));
}

#[test]
fn test_edit_opens_editor_and_reports() {
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    let tools = TempDir::new().unwrap();
    let original = data.path().join("original.py");
    fs::write(&original, b"print('original')\n").unwrap();
    write_entry(
        data.path(),
        "a",
        "a",
        "python",
        "copy",
        &original,
        Some(("script.py", b"print(1)\n")),
    );
    let editor = compile_editor(tools.path());
    let capture = tools.path().join("capture.txt");

    let (code, output) = tui_edit(
        data.path(),
        state.path(),
        config.path(),
        &editor,
        &capture,
    );
    assert_eq!(code, 0, "{output}");
    assert_eq!(
        fs::read_to_string(&capture).unwrap(),
        data.path()
            .join("scripts/a/script.py")
            .display()
            .to_string()
    );
    assert!(
        fs::read_to_string(data.path().join("scripts/a/script.py"))
            .unwrap()
            .contains("# edited by probe")
    );
    // Keep the Python-visible receipt, not the Rust implementation's current generic success copy.
    // If Rust prints only "Source saved", this is a deliberate parity finding.
    assert!(output.contains("Edited a."), "{output}");
}

#[test]
fn test_edit_command_entry_reports_no_source() {
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    let tools = TempDir::new().unwrap();
    write_entry(
        data.path(),
        "c",
        "c",
        "command",
        "copy",
        Path::new(""),
        None,
    );
    let editor = compile_editor(tools.path());
    let capture = tools.path().join("capture.txt");

    let (_code, output) = tui_edit(
        data.path(),
        state.path(),
        config.path(),
        &editor,
        &capture,
    );
    assert!(!capture.exists(), "the editor ran for a command entry");
    assert!(output.contains("no editable source"), "{output}");
}
