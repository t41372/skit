//! Unknown-selector `skit edit` ports from Python `tests/test_editor.py` at `main@206f9ef`.
//!
//! The PTY cases make the confirmation prompt real. The editor probe is also real, so declining or
//! running non-interactively can prove that no editor process was launched and no entry appeared.

use std::{
    fs,
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
    process::Command,
    thread,
};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use skit_application::EntryRepository as _;
use skit_store::FileStore;
use tempfile::TempDir;

struct UnknownFixture {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    tools: TempDir,
    editor: PathBuf,
    capture: PathBuf,
}

impl UnknownFixture {
    fn new() -> Self {
        let data = TempDir::new().unwrap();
        let state = TempDir::new().unwrap();
        let config = TempDir::new().unwrap();
        let tools = TempDir::new().unwrap();
        let capture = tools.path().join("editor-target.txt");
        let editor = compile_editor(tools.path());
        Self {
            data,
            state,
            config,
            tools,
            editor,
            capture,
        }
    }

    fn store(&self) -> FileStore {
        FileStore::new(self.data.path())
    }

    fn run_pty(&self, selector: &str, input: &[u8]) -> (u32, String) {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 30,
                cols: 120,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let mut command = CommandBuilder::new(PathBuf::from(env!("CARGO_BIN_EXE_skit")));
        command.args(["edit", selector]);
        command.env("SKIT_DATA_DIR", self.data.path());
        command.env("SKIT_STATE_DIR", self.state.path());
        command.env("SKIT_CONFIG_DIR", self.config.path());
        command.env("SKIT_LANG", "en");
        command.env("VISUAL", &self.editor);
        command.env("EDITOR", &self.editor);
        command.env("SKIT_EDITOR_CAPTURE", &self.capture);
        command.env("SKIT_EDITOR_CONTENT", "import requests\nprint('hi')\n");

        let mut child = pair.slave.spawn_command(command).unwrap();
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().unwrap();
        let drain = thread::spawn(move || {
            let mut bytes = Vec::new();
            reader.read_to_end(&mut bytes).unwrap();
            bytes
        });
        let mut writer = pair.master.take_writer().unwrap();
        writer.write_all(input).unwrap();
        writer.flush().unwrap();
        let status = child.wait().unwrap();
        drop(writer);
        let output = String::from_utf8_lossy(&drain.join().unwrap()).into_owned();
        (status.exit_code(), output)
    }

    fn run_noninteractive(&self, selector: &str) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_skit"))
            .env("SKIT_DATA_DIR", self.data.path())
            .env("SKIT_STATE_DIR", self.state.path())
            .env("SKIT_CONFIG_DIR", self.config.path())
            .env("SKIT_LANG", "en")
            .env("VISUAL", &self.editor)
            .env("EDITOR", &self.editor)
            .env("SKIT_EDITOR_CAPTURE", &self.capture)
            .env("SKIT_EDITOR_CONTENT", "import requests\nprint('hi')\n")
            .args(["edit", selector])
            .output()
            .unwrap()
    }
}

fn compile_editor(root: &Path) -> PathBuf {
    let source = root.join("unknown_editor.rs");
    fs::write(
        &source,
        r#"
use std::{env, fs, path::PathBuf};
fn main() {
    let target = PathBuf::from(env::args_os().last().expect("editor target"));
    fs::write(env::var_os("SKIT_EDITOR_CAPTURE").expect("capture"), target.to_string_lossy().as_bytes())
        .expect("capture target");
    fs::write(&target, env::var("SKIT_EDITOR_CONTENT").expect("content").as_bytes())
        .expect("write edited source");
}
"#,
    )
    .unwrap();
    let output = root.join(editor_name());
    let status = Command::new("rustc")
        .arg(source)
        .arg("-o")
        .arg(&output)
        .status()
        .unwrap();
    assert!(status.success());
    output
}

#[cfg(windows)]
fn editor_name() -> &'static str {
    "unknown-editor.exe"
}

#[cfg(not(windows))]
fn editor_name() -> &'static str {
    "unknown-editor"
}

fn combined(output: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn test_edit_unknown_confirmed_creates() {
    let fixture = UnknownFixture::new();

    let (code, output) = fixture.run_pty("newscript", b"y\n\n\n\n");
    assert_eq!(code, 0, "{output}");
    let entry = fixture.store().resolve("newscript").unwrap();
    assert_eq!(entry.meta.kind.as_str(), "python");
    let stored = fixture.store().payload_path(&entry).unwrap();
    assert!(fs::read_to_string(stored).unwrap().contains("requests"));
    assert!(fixture.capture.exists(), "confirmed creation never launched the editor");
}

#[test]
fn test_edit_unknown_declined_creates_nothing() {
    let fixture = UnknownFixture::new();

    let (code, output) = fixture.run_pty("nope", b"n\n");
    assert_eq!(code, 0, "{output}");
    assert!(fixture.store().resolve("nope").is_err());
    assert!(!fixture.capture.exists(), "declining creation still launched the editor");
}

#[test]
fn test_edit_unknown_non_interactive_errors() {
    let fixture = UnknownFixture::new();

    let output = fixture.run_noninteractive("ghost");
    assert_eq!(output.status.code(), Some(1), "{}", combined(&output));
    assert!(fixture.store().resolve("ghost").is_err());
    assert!(!fixture.capture.exists(), "non-interactive unknown edit launched the editor");
}
