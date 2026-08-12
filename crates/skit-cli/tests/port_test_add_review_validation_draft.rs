//! Real host-side draft-consumption port for Python `test_fresh_draft_copy_flow_unlinks_the_file`.
//!
//! Rust's interactive CLI and TUI share the typed add-review/draft host pipeline. A real editor
//! probe captures the owned draft path, writes user content, and the real `skit add -e` transaction
//! must create a copy entry and physically unlink that captured draft after success.

use std::{
    fs,
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
    process::Command,
    thread,
};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use skit_application::EntryRepository as _;
use skit_domain::StorageMode;
use skit_store::FileStore;
use tempfile::TempDir;

struct Fixture {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    tools: TempDir,
    editor: PathBuf,
    capture: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let data = TempDir::new().unwrap();
        let state = TempDir::new().unwrap();
        let config = TempDir::new().unwrap();
        let tools = TempDir::new().unwrap();
        let capture = tools.path().join("draft.txt");
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
}

fn compile_editor(root: &Path) -> PathBuf {
    let source = root.join("review_draft_editor.rs");
    fs::write(
        &source,
        r#"
use std::{env, fs, path::PathBuf};
fn main() {
    let target = PathBuf::from(env::args_os().last().expect("draft path"));
    fs::write(
        env::var_os("SKIT_EDITOR_CAPTURE").expect("capture path"),
        target.to_string_lossy().as_bytes(),
    ).expect("capture draft");
    fs::write(&target, b"print('drafted')\n").expect("write authored source");
}
"#,
    )
    .unwrap();
    let executable = root.join(if cfg!(windows) {
        "review-draft-editor.exe"
    } else {
        "review-draft-editor"
    });
    let status = Command::new("rustc")
        .arg(source)
        .arg("-o")
        .arg(&executable)
        .status()
        .unwrap();
    assert!(status.success(), "failed to compile draft editor probe");
    executable
}

#[test]
fn test_fresh_draft_copy_flow_unlinks_the_file() {
    let fixture = Fixture::new();
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 30,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    let mut command = CommandBuilder::new(PathBuf::from(env!("CARGO_BIN_EXE_skit")));
    command.args(["add", "-e", "--name", "copied"]);
    command.env("SKIT_DATA_DIR", fixture.data.path());
    command.env("SKIT_STATE_DIR", fixture.state.path());
    command.env("SKIT_CONFIG_DIR", fixture.config.path());
    command.env("SKIT_LANG", "en");
    command.env("VISUAL", &fixture.editor);
    command.env("EDITOR", &fixture.editor);
    command.env("SKIT_EDITOR_CAPTURE", &fixture.capture);

    let mut child = pair.slave.spawn_command(command).unwrap();
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().unwrap();
    let drain = thread::spawn(move || {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).unwrap();
        bytes
    });
    let mut writer = pair.master.take_writer().unwrap();
    // Accept any default review choices the interactive CLI asks after the editor returns.
    writer.write_all(b"\n\n\n").unwrap();
    writer.flush().unwrap();
    let status = child.wait().unwrap();
    drop(writer);
    let output = String::from_utf8_lossy(&drain.join().unwrap()).into_owned();

    assert_eq!(status.exit_code(), 0, "{output}");
    let entry = fixture.store().resolve("copied").unwrap();
    assert_eq!(entry.meta.mode, StorageMode::Copy);
    let draft = PathBuf::from(fs::read_to_string(&fixture.capture).unwrap());
    assert_eq!(draft.parent(), Some(fixture.data.path().join("drafts").as_path()));
    assert!(
        !draft.exists(),
        "successful copy-mode authoring left the owned draft behind: {draft:?}"
    );
}
