//! Real-PTY editor-lane ports from Python `tests/test_add_lane_contracts.py` at `main@206f9ef`.
//!
//! A compiled editor probe makes launch ordering observable. Tests that require refusal before the
//! editor set `SKIT_EDITOR_FAIL_IF_CALLED`; an incorrect implementation then records the launch and
//! exits the probe immediately instead of hanging in a later interactive step.

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

struct Fixture {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    home: TempDir,
    _tools: TempDir,
    editor: PathBuf,
    capture: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let data = TempDir::new().unwrap();
        let state = TempDir::new().unwrap();
        let config = TempDir::new().unwrap();
        let home = TempDir::new().unwrap();
        let tools = TempDir::new().unwrap();
        let capture = tools.path().join("editor-capture.txt");
        let editor = compile_editor(tools.path());
        Self {
            data,
            state,
            config,
            home,
            _tools: tools,
            editor,
            capture,
        }
    }

    fn store(&self) -> FileStore {
        FileStore::new(self.data.path())
    }

    fn run(
        &self,
        args: &[&str],
        editor_content: Option<&str>,
        fail_if_called: bool,
        input: &[u8],
    ) -> (u32, String) {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 32,
                cols: 132,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let mut command = CommandBuilder::new(PathBuf::from(env!("CARGO_BIN_EXE_skit")));
        command.args(args);
        command.cwd(self.home.path());
        command.env("TERM", "xterm-256color");
        command.env("SKIT_DATA_DIR", self.data.path());
        command.env("SKIT_STATE_DIR", self.state.path());
        command.env("SKIT_CONFIG_DIR", self.config.path());
        command.env("SKIT_LANG", "en");
        command.env("HOME", self.home.path());
        command.env("USERPROFILE", self.home.path());
        command.env("XDG_CONFIG_HOME", self.home.path().join("xdg-config"));
        command.env("XDG_DATA_HOME", self.home.path().join("xdg-data"));
        command.env("XDG_STATE_HOME", self.home.path().join("xdg-state"));
        command.env("VISUAL", &self.editor);
        command.env("EDITOR", &self.editor);
        command.env("SKIT_EDITOR_CAPTURE", &self.capture);
        if let Some(content) = editor_content {
            command.env("SKIT_EDITOR_CONTENT", content);
        } else {
            command.env_remove("SKIT_EDITOR_CONTENT");
        }
        if fail_if_called {
            command.env("SKIT_EDITOR_FAIL_IF_CALLED", "1");
        } else {
            command.env_remove("SKIT_EDITOR_FAIL_IF_CALLED");
        }

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
        let output = String::from_utf8_lossy(&drain.join().unwrap())
            .replace("\r\n", "\n")
            .replace('\r', "");
        (status.exit_code(), output)
    }

    fn assert_editor_not_called(&self) {
        assert!(
            !self.capture.exists(),
            "editor was launched before the lane-level refusal: {}",
            fs::read_to_string(&self.capture).unwrap_or_default()
        );
    }

    fn captured_draft(&self) -> PathBuf {
        PathBuf::from(fs::read_to_string(&self.capture).unwrap())
    }
}

fn compile_editor(root: &Path) -> PathBuf {
    let source = root.join("lane_editor.rs");
    fs::write(
        &source,
        r#"
use std::{env, fs, path::PathBuf};
fn main() {
    let target = PathBuf::from(env::args_os().last().expect("draft target"));
    fs::write(
        env::var_os("SKIT_EDITOR_CAPTURE").expect("capture path"),
        target.to_string_lossy().as_bytes(),
    ).expect("capture editor target");
    if env::var_os("SKIT_EDITOR_FAIL_IF_CALLED").is_some() {
        std::process::exit(73);
    }
    if let Some(content) = env::var_os("SKIT_EDITOR_CONTENT") {
        fs::write(&target, content.to_string_lossy().as_bytes()).expect("write editor content");
    }
}
"#,
    )
    .unwrap();
    let executable = root.join(if cfg!(windows) {
        "lane-editor.exe"
    } else {
        "lane-editor"
    });
    let status = Command::new("rustc")
        .arg(source)
        .arg("-o")
        .arg(&executable)
        .status()
        .unwrap();
    assert!(status.success(), "failed to compile editor probe");
    executable
}

fn flat(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn test_editor_lane_versioned_python_shebang_onboards_as_python() {
    let fixture = Fixture::new();
    let (code, output) = fixture.run(
        &["add", "-e", "-n", "vpy"],
        Some("#!/usr/bin/env python3.12\nprint('hi')\n"),
        false,
        b"\n\n\n",
    );

    assert_eq!(code, 0, "{output}");
    assert_eq!(
        fixture.store().resolve("vpy").unwrap().meta.kind.as_str(),
        "python"
    );
}

#[test]
fn test_prompt_editor_bogus_runner_refused_before_the_editor() {
    let fixture = Fixture::new();
    let (code, output) = fixture.run(
        &["add", "--prompt", "--runner", "bogus", "-n", "p"],
        None,
        true,
        b"",
    );

    assert_eq!(code, 2, "{output}");
    assert!(flat(&output).contains("Unknown runner"), "{output}");
    fixture.assert_editor_not_called();
    let drafts = fixture.data.path().join("drafts");
    assert!(!drafts.exists() || fs::read_dir(drafts).unwrap().next().is_none());
}

#[test]
fn test_edit_no_input_is_refused_with_the_pipe_spelling() {
    let fixture = Fixture::new();
    let (code, output) = fixture.run(&["add", "-e", "-n", "x", "--no-input"], None, true, b"");

    assert_eq!(code, 2, "{output}");
    assert!(output.contains("skit add - -n NAME"), "{output}");
    fixture.assert_editor_not_called();
}

#[test]
fn test_prompt_editor_no_input_in_a_terminal_is_refused() {
    let fixture = Fixture::new();
    let (code, output) = fixture.run(
        &["add", "--prompt", "-n", "p", "--no-input"],
        None,
        true,
        b"",
    );

    assert_eq!(code, 2, "{output}");
    assert!(output.contains("skit add - --prompt -n NAME"), "{output}");
    fixture.assert_editor_not_called();
}

#[test]
fn test_edit_description_flag_wins_over_python_docstring() {
    let fixture = Fixture::new();
    let (code, output) = fixture.run(
        &["add", "-e", "-n", "dpy", "--description", "flag wins"],
        Some("\"\"\"Docstring one\"\"\"\nprint(1)\n"),
        false,
        b"\n\n\n",
    );

    assert_eq!(code, 0, "{output}");
    assert_eq!(
        fixture.store().resolve("dpy").unwrap().meta.description,
        "flag wins"
    );
}

#[test]
fn test_edit_description_flag_on_non_python_draft_is_stored() {
    let fixture = Fixture::new();
    let (code, output) = fixture.run(
        &["add", "-e", "-n", "dsh", "--description", "shell note"],
        Some("#!/usr/bin/env bash\necho hi\n"),
        false,
        b"",
    );

    assert_eq!(code, 0, "{output}");
    let entry = fixture.store().resolve("dsh").unwrap();
    assert_eq!(entry.meta.kind.as_str(), "shell");
    assert_eq!(entry.meta.description, "shell note");
}

#[test]
fn test_edit_post_editor_refusal_keeps_draft_and_announces_short() {
    let fixture = Fixture::new();
    let (code, output) = fixture.run(
        &["add", "-e", "-n", "d", "--dep", "foo"],
        Some("#!/usr/bin/env bash\necho drafted\n"),
        false,
        b"",
    );
    let shown = flat(&output);

    assert_eq!(code, 2, "{shown}");
    assert!(shown.contains("python flags"), "{shown}");
    assert!(shown.contains("Your draft was kept at"), "{shown}");
    assert!(
        !shown.contains("fix the problem and add it with"),
        "{shown}"
    );
    let draft = fixture.captured_draft();
    assert!(
        draft.exists(),
        "post-editor refusal destroyed the user's only draft"
    );
    assert!(fixture.store().resolve("d").is_err());
}
