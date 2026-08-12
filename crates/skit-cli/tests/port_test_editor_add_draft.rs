//! Public-process ports of the `skit add --edit` draft slice in Python `tests/test_editor.py` at
//! `main@206f9ef`.
//!
//! The Python tests patch the editor and interactivity gates. This port instead gives the real CLI a
//! PTY and a real child editor probe. That preserves the user-facing contract while making the
//! safety properties stronger: we can prove when the editor did or did not launch, which draft path
//! it received, whether user-authored drafts survived refusals, and what was actually persisted.

#![cfg(unix)]

use std::{
    fs,
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
    process::{Command, Output},
    thread,
};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use skit_application::EntryRepository as _;
use skit_domain::EntrySettings;
use skit_store::FileStore;
use tempfile::TempDir;

struct DraftFixture {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    tools: TempDir,
    capture: PathBuf,
    editor: PathBuf,
}

impl DraftFixture {
    fn new() -> Self {
        let data = TempDir::new().unwrap();
        let state = TempDir::new().unwrap();
        let config = TempDir::new().unwrap();
        let tools = TempDir::new().unwrap();
        let capture = tools.path().join("draft-path.txt");
        let editor = compile_editor(tools.path());
        Self {
            data,
            state,
            config,
            tools,
            capture,
            editor,
        }
    }

    fn store(&self) -> FileStore {
        FileStore::new(self.data.path())
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_skit"));
        command
            .env("SKIT_DATA_DIR", self.data.path())
            .env("SKIT_STATE_DIR", self.state.path())
            .env("SKIT_CONFIG_DIR", self.config.path())
            .env("SKIT_LANG", "en")
            .env_remove("VISUAL")
            .env("EDITOR", &self.editor)
            .env("SKIT_EDITOR_CAPTURE", &self.capture);
        command
    }

    fn run_noninteractive(&self, args: &[&str], content: Option<&str>) -> Output {
        let mut command = self.command();
        command.args(args);
        match content {
            Some(value) => {
                command.env("SKIT_EDITOR_CONTENT", value);
            }
            None => {
                command.env_remove("SKIT_EDITOR_CONTENT");
            }
        }
        command.output().unwrap()
    }

    fn run_interactive(&self, args: &[&str], content: Option<&str>, input: &[u8]) -> (u32, String) {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 30,
                cols: 120,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let mut command = CommandBuilder::new(PathBuf::from(env!("CARGO_BIN_EXE_skit")));
        for arg in args {
            command.arg(arg);
        }
        command.env("SKIT_DATA_DIR", self.data.path());
        command.env("SKIT_STATE_DIR", self.state.path());
        command.env("SKIT_CONFIG_DIR", self.config.path());
        command.env("SKIT_LANG", "en");
        command.env("EDITOR", &self.editor);
        command.env("SKIT_EDITOR_CAPTURE", &self.capture);
        match content {
            Some(value) => command.env("SKIT_EDITOR_CONTENT", value),
            None => command.env_remove("SKIT_EDITOR_CONTENT"),
        };

        let mut child = pair.slave.spawn_command(command).unwrap();
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().unwrap();
        let drain = thread::spawn(move || {
            let mut bytes = Vec::new();
            reader.read_to_end(&mut bytes).unwrap();
            bytes
        });
        let mut writer = pair.master.take_writer().unwrap();
        if !input.is_empty() {
            writer.write_all(input).unwrap();
            writer.flush().unwrap();
        }
        let status = child.wait().unwrap();
        drop(writer);
        let output = String::from_utf8_lossy(&drain.join().unwrap()).into_owned();
        (status.exit_code(), output)
    }

    fn captured_draft(&self) -> PathBuf {
        PathBuf::from(fs::read_to_string(&self.capture).unwrap())
    }

    fn seed_taken_name(&self, name: &str) {
        let output = self
            .command()
            .args(["add", "--cmd", "echo old", "--name", name, "--no-input"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "failed to seed {name}: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!self.capture.exists(), "seeding a command unexpectedly launched the editor");
    }
}

fn compile_editor(root: &Path) -> PathBuf {
    let source = root.join("draft_editor.rs");
    fs::write(
        &source,
        r#"
use std::{env, fs, path::PathBuf};
fn main() {
    let target = PathBuf::from(env::args_os().last().expect("draft path"));
    let capture = env::var_os("SKIT_EDITOR_CAPTURE").expect("capture path");
    fs::write(capture, target.to_string_lossy().as_bytes()).expect("capture draft path");
    if let Ok(content) = env::var("SKIT_EDITOR_CONTENT") {
        fs::write(&target, content.as_bytes()).expect("write draft");
    }
}
"#,
    )
    .unwrap();
    let executable = root.join("draft-editor");
    let status = Command::new("rustc")
        .arg(source)
        .arg("-o")
        .arg(&executable)
        .status()
        .unwrap();
    assert!(status.success(), "failed to compile draft editor probe");
    executable
}

fn assert_no_entry(fixture: &DraftFixture, name: &str) {
    assert!(
        fixture.store().resolve(name).is_err(),
        "an entry named {name:?} was fabricated after a refused/cancelled draft"
    );
}

fn combined(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn test_add_edit_creates_in_editor() {
    let fixture = DraftFixture::new();
    let (code, output) = fixture.run_interactive(
        &["add", "-e", "--name", "fresh"],
        Some("import rich\nprint('x')\n"),
        b"\n\n\n",
    );
    assert_eq!(code, 0, "{output}");
    let entry = fixture.store().resolve("fresh").unwrap();
    assert_eq!(entry.meta.kind.as_str(), "python");
    let stored = fixture.store().payload_path(&entry).unwrap();
    assert!(fs::read_to_string(stored).unwrap().contains("rich"));
}

#[test]
fn test_add_edit_bash_shebang_draft_becomes_a_shell_entry() {
    let fixture = DraftFixture::new();
    let (code, output) = fixture.run_interactive(
        &["add", "-e", "--name", "deploy"],
        Some("#!/usr/bin/env bash\n# Ship it\necho drafted\n"),
        b"",
    );
    assert_eq!(code, 0, "{output}");
    let entry = fixture.store().resolve("deploy").unwrap();
    assert_eq!(entry.meta.kind.as_str(), "shell");
    assert!(
        fs::read_to_string(fixture.store().payload_path(&entry).unwrap())
            .unwrap()
            .contains("echo drafted")
    );
}

#[test]
fn test_add_edit_js_shebang_draft_scans_npm_deps() {
    let fixture = DraftFixture::new();
    let (code, output) = fixture.run_interactive(
        &["add", "-e", "--name", "colorized"],
        Some("#!/usr/bin/env node\nimport chalk from 'chalk'\nconsole.log(chalk)\n"),
        b"",
    );
    assert_eq!(code, 0, "{output}");
    let entry = fixture.store().resolve("colorized").unwrap();
    assert_eq!(entry.meta.kind.as_str(), "js");
    assert_eq!(EntrySettings::from_meta(&entry.meta).dependencies, vec!["chalk"]);
}

#[test]
fn test_add_edit_zsh_draft_records_interpreter_and_dry_run_names_zsh() {
    let fixture = DraftFixture::new();
    let (code, output) = fixture.run_interactive(
        &["add", "-e", "--name", "zjob"],
        Some("#!/usr/bin/env zsh\necho hi\n"),
        b"",
    );
    assert_eq!(code, 0, "{output}");
    let entry = fixture.store().resolve("zjob").unwrap();
    assert_eq!(entry.meta.kind.as_str(), "shell");
    assert_eq!(EntrySettings::from_meta(&entry.meta).interpreter, "zsh");

    let dry = fixture
        .command()
        .args(["run", "zjob", "--dry-run", "--no-input"])
        .output()
        .unwrap();
    assert!(dry.status.success(), "{}", combined(&dry));
    assert!(combined(&dry).contains("zsh"), "{}", combined(&dry));
}

#[test]
fn test_add_edit_dep_flag_on_non_python_draft_is_refused() {
    let fixture = DraftFixture::new();
    let (code, output) = fixture.run_interactive(
        &["add", "-e", "--name", "d", "--dep", "rich"],
        Some("#!/usr/bin/env bash\necho drafted\n"),
        b"",
    );
    assert_eq!(code, 2, "{output}");
    assert!(output.contains("python flags"), "{output}");
    assert!(output.contains("shell"), "{output}");
    assert_no_entry(&fixture, "d");
}

#[test]
fn test_add_edit_python_name_taken_refuses_before_the_editor() {
    let fixture = DraftFixture::new();
    fixture.seed_taken_name("taken");

    let (code, output) = fixture.run_interactive(
        &["add", "-e", "--name", "taken"],
        Some("print('must never be written')\n"),
        b"",
    );
    assert_eq!(code, 1, "{output}");
    assert!(output.contains("already taken"), "{output}");
    assert!(!fixture.capture.exists(), "name conflict was discovered only after launching editor");
}

#[test]
fn test_add_edit_rejects_path() {
    let fixture = DraftFixture::new();
    let source = fixture.data.path().join("input.py");
    fs::write(&source, b"print(1)\n").unwrap();

    let output = fixture.run_noninteractive(
        &["add", "-e", source.to_str().unwrap()],
        Some("print('must never be written')\n"),
    );
    assert_eq!(output.status.code(), Some(2), "{}", combined(&output));
    assert!(!fixture.capture.exists(), "editor launched even though --edit had a source path");
}

#[test]
fn test_add_edit_non_interactive_errors() {
    let fixture = DraftFixture::new();

    let output = fixture.run_noninteractive(
        &["add", "-e", "--name", "x"],
        Some("print('must never be written')\n"),
    );
    assert_eq!(output.status.code(), Some(2), "{}", combined(&output));
    assert!(!fixture.capture.exists(), "non-interactive --edit launched an editor");
    assert_no_entry(&fixture, "x");
}

#[test]
fn test_add_edit_empty_content_adds_nothing() {
    let fixture = DraftFixture::new();

    let (code, output) = fixture.run_interactive(&["add", "-e", "--name", "ghost"], None, b"");
    assert_eq!(code, 0, "{output}");
    assert!(output.contains("Nothing was written"), "{output}");
    assert_no_entry(&fixture, "ghost");
}

#[test]
fn test_add_edit_unregistered_shebang_refused_keeps_draft() {
    let fixture = DraftFixture::new();
    let (code, output) = fixture.run_interactive(
        &["add", "-e", "--name", "aw"],
        Some("#!/usr/bin/awk -f\nBEGIN { print 1 }\n"),
        b"",
    );
    assert_eq!(code, 2, "{output}");
    assert!(output.contains("names no interpreter skit knows"), "{output}");
    assert!(output.contains("--kind"), "{output}");
    let draft = fixture.captured_draft();
    assert!(draft.exists(), "user-authored refused draft was deleted: {draft:?}");
    assert_eq!(draft.parent().unwrap(), fixture.data.path().join("drafts"));
    assert_no_entry(&fixture, "aw");
    fs::remove_file(draft).unwrap();
}

#[test]
fn test_add_edit_untouched_starter_unlinks_the_draft() {
    let fixture = DraftFixture::new();

    let (code, output) = fixture.run_interactive(&["add", "-e", "--name", "ghost"], None, b"");
    assert_eq!(code, 0, "{output}");
    assert!(output.contains("Nothing was written"), "{output}");
    let draft = fixture.captured_draft();
    assert!(!draft.exists(), "untouched starter litter survived: {draft:?}");
}

#[test]
fn test_add_prompt_editor_untouched_starter_unlinks_the_draft() {
    let fixture = DraftFixture::new();

    let (code, output) =
        fixture.run_interactive(&["add", "--prompt", "--name", "ghostp"], None, b"");
    assert_eq!(code, 0, "{output}");
    assert!(output.contains("Nothing was written"), "{output}");
    let draft = fixture.captured_draft();
    assert!(!draft.exists(), "untouched prompt starter litter survived: {draft:?}");
}

#[test]
fn test_add_edit_prompts_for_name_when_omitted() {
    let fixture = DraftFixture::new();
    let (code, output) = fixture.run_interactive(
        &["add", "-e"],
        Some("print('x')\n"),
        b"prompted\n",
    );
    assert_eq!(code, 0, "{output}");
    assert_eq!(fixture.store().resolve("prompted").unwrap().meta.kind.as_str(), "python");
}

#[test]
fn test_add_edit_blank_name_errors() {
    let fixture = DraftFixture::new();
    let (code, output) = fixture.run_interactive(
        &["add", "-e"],
        Some("print('must never be written')\n"),
        b"   \n",
    );
    assert_eq!(code, 2, "{output}");
    assert!(!fixture.capture.exists(), "blank name was rejected only after launching editor");
}

#[test]
fn test_add_edit_editor_error_exits_one() {
    let fixture = DraftFixture::new();
    let missing = fixture.tools.path().join("cannot-launch-editor");
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 20,
            cols: 100,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    let mut command = CommandBuilder::new(PathBuf::from(env!("CARGO_BIN_EXE_skit")));
    command.args(["add", "-e", "--name", "x"]);
    command.env("SKIT_DATA_DIR", fixture.data.path());
    command.env("SKIT_STATE_DIR", fixture.state.path());
    command.env("SKIT_CONFIG_DIR", fixture.config.path());
    command.env("SKIT_LANG", "en");
    command.env("EDITOR", &missing);
    let mut child = pair.slave.spawn_command(command).unwrap();
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().unwrap();
    let drain = thread::spawn(move || {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).unwrap();
        bytes
    });
    let status = child.wait().unwrap();
    let output = String::from_utf8_lossy(&drain.join().unwrap()).into_owned();

    assert_eq!(status.exit_code(), 1, "{output}");
    assert!(output.contains("cannot launch"), "{output}");
    assert_no_entry(&fixture, "x");
}

#[test]
fn test_add_edit_name_conflict_exits_one() {
    let fixture = DraftFixture::new();
    fixture.seed_taken_name("dup");

    let (code, output) = fixture.run_interactive(
        &["add", "-e", "--name", "dup"],
        Some("print('x')\n"),
        b"",
    );
    assert_eq!(code, 1, "{output}");
    assert!(output.contains("dup"), "{output}");
    assert!(output.contains("taken"), "{output}");
}
