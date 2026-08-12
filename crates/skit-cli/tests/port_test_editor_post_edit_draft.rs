//! Real-race port of Python `test_add_edit_python_post_edit_failure_keeps_the_draft` from
//! `tests/test_editor.py` at `main@206f9ef`.
//!
//! Python injects a `StoreError("disk full")` after the editor returns. Rust has no public fault
//! injection seam, so exercise a stronger real failure that matters to skit's multi-agent story:
//! after the user-authored draft is written, the editor child lets a second real `skit` process
//! create the requested name. The original add must lose the post-edit create race without deleting
//! the user's only authored copy or overwriting the winner.

use std::{
    fs,
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
    process::Command,
    thread,
};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use skit_application::EntryRepository as _;
use skit_domain::EntrySettings;
use skit_store::FileStore;
use tempfile::TempDir;

struct Fixture {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    tools: TempDir,
    capture: PathBuf,
    editor: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let data = TempDir::new().unwrap();
        let state = TempDir::new().unwrap();
        let config = TempDir::new().unwrap();
        let tools = TempDir::new().unwrap();
        let capture = tools.path().join("draft.txt");
        let editor = compile_racing_editor(tools.path());
        Self {
            data,
            state,
            config,
            tools,
            capture,
            editor,
        }
    }

    fn run(&self) -> (u32, String) {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 30,
                cols: 120,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let mut command = CommandBuilder::new(PathBuf::from(env!("CARGO_BIN_EXE_skit")));
        command.args(["add", "-e", "--name", "keptpy"]);
        command.env("SKIT_DATA_DIR", self.data.path());
        command.env("SKIT_STATE_DIR", self.state.path());
        command.env("SKIT_CONFIG_DIR", self.config.path());
        command.env("SKIT_LANG", "en");
        command.env("VISUAL", &self.editor);
        command.env("EDITOR", &self.editor);
        command.env("SKIT_EDITOR_CAPTURE", &self.capture);
        command.env("SKIT_BIN_FOR_EDITOR_RACE", env!("CARGO_BIN_EXE_skit"));
        let mut child = pair.slave.spawn_command(command).unwrap();
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader().unwrap();
        let drain = thread::spawn(move || {
            let mut bytes = Vec::new();
            reader.read_to_end(&mut bytes).unwrap();
            bytes
        });
        let mut writer = pair.master.take_writer().unwrap();
        // The editor path already has an explicit name; any later onboarding question accepts its
        // default. The post-edit conflict should normally abort before one is needed.
        let _ = writer.write_all(b"\n\n\n");
        let _ = writer.flush();
        let status = child.wait().unwrap();
        drop(writer);
        let output = String::from_utf8_lossy(&drain.join().unwrap()).into_owned();
        (status.exit_code(), output)
    }

    fn draft(&self) -> PathBuf {
        PathBuf::from(fs::read_to_string(&self.capture).unwrap())
    }
}

fn compile_racing_editor(root: &Path) -> PathBuf {
    let source = root.join("racing_editor.rs");
    fs::write(
        &source,
        r#"
use std::{env, fs, path::PathBuf, process::Command};
fn main() {
    let target = PathBuf::from(env::args_os().last().expect("draft path"));
    let capture = env::var_os("SKIT_EDITOR_CAPTURE").expect("capture path");
    fs::write(&target, b"import sys\nprint('drafted')\n").expect("write authored draft");
    fs::write(capture, target.to_string_lossy().as_bytes()).expect("capture draft path");

    // This second real skit process wins the name after the parent already passed preflight but
    // before the editor returns. It inherits the same isolated SKIT_* roots.
    let skit = env::var_os("SKIT_BIN_FOR_EDITOR_RACE").expect("skit binary");
    let status = Command::new(skit)
        .args(["add", "--cmd", "echo other-agent", "--name", "keptpy", "--no-input"])
        .status()
        .expect("launch competing skit");
    assert!(status.success(), "competing skit failed: {status:?}");
}
"#,
    )
    .unwrap();
    let executable = root.join(editor_executable_name());
    let status = Command::new("rustc")
        .arg(source)
        .arg("-o")
        .arg(&executable)
        .status()
        .unwrap();
    assert!(status.success(), "failed to compile racing editor probe");
    executable
}

#[cfg(windows)]
fn editor_executable_name() -> &'static str {
    "racing-editor.exe"
}

#[cfg(not(windows))]
fn editor_executable_name() -> &'static str {
    "racing-editor"
}

#[test]
fn test_add_edit_python_post_edit_failure_keeps_the_draft() {
    let fixture = Fixture::new();
    let (code, output) = fixture.run();

    assert_ne!(code, 0, "the losing post-edit create unexpectedly succeeded: {output}");
    assert!(
        output.contains("Your draft was kept at"),
        "post-edit failure did not tell the user where authored work survived: {output}"
    );

    let draft = fixture.draft();
    assert!(draft.exists(), "post-edit failure deleted the user's authored draft: {draft:?}");
    assert_eq!(
        fs::read_to_string(&draft).unwrap(),
        "import sys\nprint('drafted')\n",
        "the kept path is not the exact content the editor authored"
    );
    assert_eq!(
        draft.parent(),
        Some(fixture.data.path().join("drafts").as_path()),
        "kept authored work escaped the isolated skit drafts directory"
    );

    // The other agent's entry is authoritative and must survive untouched. The losing flow must not
    // partially replace it with the Python draft.
    let store = FileStore::new(fixture.data.path());
    let winner = store.resolve("keptpy").expect("the competing agent's entry disappeared");
    assert_eq!(winner.meta.kind.as_str(), "command");
    assert_eq!(EntrySettings::from_meta(&winner.meta).template, "echo other-agent");

    fs::remove_file(draft).unwrap();
}
