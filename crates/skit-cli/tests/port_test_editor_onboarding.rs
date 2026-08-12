//! Editor-draft onboarding ports from Python `tests/test_editor.py` at `main@206f9ef`.
//!
//! These tests keep the review interactive through a real PTY. The editor writes real source, the
//! default-selected onboarding candidates are accepted with Enter, and assertions read the stored
//! managed declarations back from the rewritten source. Output-only checks are not enough here.

use std::{
    collections::BTreeSet,
    fs,
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
    process::Command,
    thread,
};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use skit_application::EntryRepository as _;
use skit_language::managed_params;
use skit_store::FileStore;
use tempfile::TempDir;

struct OnboardingFixture {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    _tools: TempDir,
    editor: PathBuf,
}

impl OnboardingFixture {
    fn new() -> Self {
        let data = TempDir::new().unwrap();
        let state = TempDir::new().unwrap();
        let config = TempDir::new().unwrap();
        let tools = TempDir::new().unwrap();
        let editor = compile_editor(tools.path());
        Self {
            data,
            state,
            config,
            _tools: tools,
            editor,
        }
    }

    fn store(&self) -> FileStore {
        FileStore::new(self.data.path())
    }

    fn run(&self, name: &str, content: &str) -> (u32, String) {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 40,
                cols: 140,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let mut command = CommandBuilder::new(PathBuf::from(env!("CARGO_BIN_EXE_skit")));
        command.args(["add", "-e", "--name", name]);
        command.env("SKIT_DATA_DIR", self.data.path());
        command.env("SKIT_STATE_DIR", self.state.path());
        command.env("SKIT_CONFIG_DIR", self.config.path());
        command.env("SKIT_LANG", "en");
        command.env("VISUAL", &self.editor);
        command.env("EDITOR", &self.editor);
        command.env("SKIT_EDITOR_CONTENT", content);

        let mut child = pair.slave.spawn_command(command).unwrap();
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().unwrap();
        let drain = thread::spawn(move || {
            let mut bytes = Vec::new();
            reader.read_to_end(&mut bytes).unwrap();
            bytes
        });
        let mut writer = pair.master.take_writer().unwrap();
        // All offered stable candidates begin selected. The first Enter therefore means the same as
        // Python's patched "all" answer. Extra Enters accept any later optional review prompts.
        writer.write_all(b"\r\r\r\r").unwrap();
        writer.flush().unwrap();
        let status = child.wait().unwrap();
        drop(writer);
        let output = String::from_utf8_lossy(&drain.join().unwrap()).into_owned();
        (status.exit_code(), output)
    }
}

fn compile_editor(root: &Path) -> PathBuf {
    let source = root.join("onboarding_editor.rs");
    fs::write(
        &source,
        r#"
use std::{env, fs, path::PathBuf};
fn main() {
    let target = PathBuf::from(env::args_os().last().expect("editor target"));
    fs::write(
        target,
        env::var("SKIT_EDITOR_CONTENT").expect("editor content").as_bytes(),
    )
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
    assert!(status.success(), "failed to compile onboarding editor probe");
    output
}

#[cfg(windows)]
fn editor_name() -> &'static str {
    "onboarding-editor.exe"
}

#[cfg(not(windows))]
fn editor_name() -> &'static str {
    "onboarding-editor"
}

#[test]
fn test_add_edit_shell_draft_onboards_picked_constants() {
    let fixture = OnboardingFixture::new();
    let source = "#!/usr/bin/env bash\nCITY=Taipei\nAPI_KEY=secret\necho $CITY\n";

    let (code, output) = fixture.run("deploy", source);
    assert_eq!(code, 0, "{output}");
    let entry = fixture.store().resolve("deploy").unwrap();
    assert_eq!(entry.meta.kind.as_str(), "shell");
    let stored_path = fixture.store().payload_path(&entry).unwrap();
    let stored = fs::read_to_string(stored_path).unwrap();
    let managed = managed_params("shell", &stored);
    assert_eq!(
        managed
            .iter()
            .map(|declaration| declaration.name.as_str())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from(["API_KEY", "CITY"]),
        "the interactive editor lane did not persist every picked shell constant: {stored}"
    );
}

#[test]
fn test_add_edit_writes_and_reports_managed_and_secret() {
    let fixture = OnboardingFixture::new();
    let source = "API_KEY = 'x'\nprint(API_KEY)\n";

    let (code, output) = fixture.run("fresh", source);
    assert_eq!(code, 0, "{output}");
    let entry = fixture.store().resolve("fresh").unwrap();
    assert_eq!(entry.meta.kind.as_str(), "python");
    let stored = fs::read_to_string(fixture.store().payload_path(&entry).unwrap()).unwrap();
    let managed = managed_params("python", &stored);
    let [declaration] = managed.as_slice() else {
        panic!("expected exactly one managed API_KEY declaration: {managed:?}\n{stored}");
    };
    assert_eq!(declaration.name, "API_KEY");
    assert!(declaration.secret, "API_KEY lost the secret classification");
    assert!(output.contains("Managed parameters: API_KEY"), "{output}");
    assert!(
        output.contains("Secret parameter values are never saved by skit: API_KEY"),
        "{output}"
    );
}
