//! Public PTY ports for the frozen Prompt-TUI settings-open and Library-edit contracts.
//!
//! These tests use the real `skit tui` binary, the real file store, and a real child editor.
//! A Rust editor probe changes the staged copy. The assertions then read the real store again.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::OnceLock,
};

use skit_application::{EntryMutationRepository as _, EntryRepository as _, SourcePermissions};
use skit_domain::{Entry, EntrySettings};
use skit_store::{FileConfigStore, FileStore};
use skit_ui::{KnownEntryKind, ReviewDefaults, ReviewState, SourceSnapshot};
use tempfile::TempDir;

#[path = "support/prompt_tui_pty.rs"]
mod prompt_tui_pty;
use prompt_tui_pty::TuiPty;

struct Sandbox {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    home: TempDir,
}

impl Sandbox {
    fn new() -> Self {
        Self {
            data: TempDir::new().unwrap(),
            state: TempDir::new().unwrap(),
            config: TempDir::new().unwrap(),
            home: TempDir::new().unwrap(),
        }
    }

    fn store(&self) -> FileStore {
        FileStore::new(self.data.path())
    }

    fn config(&self) -> FileConfigStore {
        FileConfigStore::new(self.config.path())
    }

    fn add(&self, name: &str, file_name: &str, body: &str, kind: KnownEntryKind) -> Entry {
        let path = self.home.path().join(file_name);
        fs::write(&path, body).unwrap();
        let review = ReviewState::from_source(
            SourceSnapshot {
                path: path.clone(),
                source_record: path.display().to_string(),
                bytes: body.as_bytes().to_vec(),
                permissions: SourcePermissions::default(),
                is_regular: true,
                is_directory: false,
                is_draft: false,
            },
            kind,
            ReviewDefaults {
                name: Some(name.to_owned()),
                ..ReviewDefaults::default()
            },
        );
        self.store().create(review.create_entry().unwrap()).unwrap()
    }

    fn prompt(&self, body: &str) -> Entry {
        self.add("greet", "greet.prompt.md", body, KnownEntryKind::Prompt)
    }

    fn python(&self, body: &str) -> Entry {
        self.add("job", "job.py", body, KnownEntryKind::Python)
    }

    fn params(&self, selector: &str) -> Vec<String> {
        let entry = self.store().resolve(selector).unwrap();
        EntrySettings::from_meta(&entry.meta).params
    }

    fn payload(&self, selector: &str) -> PathBuf {
        let store = self.store();
        let entry = store.resolve(selector).unwrap();
        store.payload_path(&entry).unwrap()
    }

    fn source_text(&self, selector: &str) -> String {
        fs::read_to_string(self.payload(selector)).unwrap()
    }

    fn editor_appends(&self, text: &str) {
        let append = self.home.path().join("editor-append.txt");
        fs::write(&append, text).unwrap();
        let command = format!(
            "{} {}",
            shlex_word(editor_probe()),
            shlex_word(&append)
        );
        self.config().set("editor", &command).unwrap();
    }

    fn tui(&self) -> TuiPty {
        TuiPty::spawn(
            self.data.path(),
            self.state.path(),
            self.config.path(),
            self.home.path(),
        )
    }
}

fn shlex_word(path: &Path) -> String {
    let text = path.to_string_lossy();
    format!("\"{}\"", text.replace('\\', "\\\\").replace('\"', "\\\""))
}

fn editor_probe() -> &'static Path {
    static EDITOR: OnceLock<PathBuf> = OnceLock::new();
    EDITOR
        .get_or_init(|| {
            let root = std::env::temp_dir().join(format!(
                "skit-prompt-tui-edit-probe-{}",
                std::process::id()
            ));
            fs::create_dir_all(&root).unwrap();
            let source = root.join("editor_probe.rs");
            fs::write(
                &source,
                r#"
use std::{
    env,
    fs::{self, OpenOptions},
    io::Write as _,
    path::PathBuf,
};

fn main() {
    let mut args = env::args_os().skip(1);
    let append = PathBuf::from(args.next().expect("append fixture"));
    let target = PathBuf::from(args.next().expect("editor target"));
    assert!(args.next().is_none(), "unexpected editor argument");
    let bytes = fs::read(append).expect("read append fixture");
    OpenOptions::new()
        .append(true)
        .open(target)
        .expect("open editor target")
        .write_all(&bytes)
        .expect("append editor content");
}
"#,
            )
            .unwrap();
            let executable = root.join(if cfg!(windows) {
                "editor-probe.exe"
            } else {
                "editor-probe"
            });
            let status = Command::new("rustc")
                .arg(&source)
                .arg("-o")
                .arg(&executable)
                .status()
                .expect("run rustc for editor probe");
            assert!(status.success(), "failed to compile editor probe");
            executable
        })
        .as_path()
}

fn open_library(tui: &mut TuiPty) -> usize {
    tui.wait_for("Library");
    tui.checkpoint()
}

fn begin_edit(tui: &mut TuiPty) -> usize {
    let checkpoint = open_library(tui);
    tui.send(b"e");
    checkpoint
}

fn expect_prompt_picker(tui: &mut TuiPty, checkpoint: usize, candidate: &str) -> String {
    let visible = tui.wait_for_any_after(checkpoint, &[candidate, "Source saved", "Edited greet."]);
    assert!(
        visible.contains(candidate),
        "editing a prompt with a new placeholder did not open the candidate picker: {visible}"
    );
    visible
}

#[test]
fn test_settings_surfaces_prompt_read_failure_from_open_race() {
    let sandbox = Sandbox::new();
    sandbox.add(
        "p",
        "race.prompt.md",
        "Do {{a}}\n",
        KnownEntryKind::Prompt,
    );
    let mut tui = sandbox.tui();
    let checkpoint = open_library(&mut tui);

    let payload = sandbox.payload("p");
    let moved = payload.with_extension("moved");
    fs::rename(&payload, &moved).unwrap();
    tui.send(b"p");

    let visible = tui.wait_for_any_after(
        checkpoint,
        &[
            "Entry settings",
            "No such file",
            "cannot find the file",
            "permission changed",
            "permission denied",
            "failed to read",
            "could not read",
            "read source",
        ],
    );
    assert!(
        !visible.contains("Entry settings"),
        "the settings-open race was silently converted into an empty settings screen: {visible}"
    );
    let lower = visible.to_lowercase();
    assert!(
        [
            "no such file",
            "cannot find",
            "permission changed",
            "permission denied",
            "failed",
            "could not",
            "read",
        ]
        .iter()
        .any(|needle| lower.contains(needle)),
        "the settings-open race did not surface a read failure: {visible}"
    );
}

#[test]
fn test_library_edit_prompt_offers_picker_and_manages_the_selection() {
    let sandbox = Sandbox::new();
    sandbox.prompt("Say hello.\n");
    sandbox.editor_appends("\n{{username}}\n");
    let mut tui = sandbox.tui();
    let checkpoint = begin_edit(&mut tui);
    let picker = expect_prompt_picker(&mut tui, checkpoint, "username");
    assert!(
        picker.contains("Choose variables"),
        "the new placeholder appeared without the candidate-picker contract: {picker}"
    );

    let done = tui.checkpoint();
    tui.send(&[0x13]);
    let visible = tui.wait_for_after(done, "Now managed: username");
    assert!(visible.contains("Now managed: username"));
    assert_eq!(sandbox.params("greet"), vec!["username".to_owned()]);
    assert!(sandbox.source_text("greet").contains("{{username}}"));
}

#[test]
fn test_library_edit_prompt_picker_cancel_leaves_it_literal() {
    let sandbox = Sandbox::new();
    sandbox.prompt("Say hello.\n");
    sandbox.editor_appends("\n{{username}}\n");
    let mut tui = sandbox.tui();
    let checkpoint = begin_edit(&mut tui);
    expect_prompt_picker(&mut tui, checkpoint, "username");

    let cancel = tui.checkpoint();
    tui.send(&[0x1b]);
    let visible = tui.wait_for_after(cancel, "Edited greet.");
    assert!(visible.contains("Edited greet."));
    assert!(sandbox.params("greet").is_empty());
    assert!(
        sandbox.source_text("greet").contains("{{username}}"),
        "cancelling management also discarded the editor's source change"
    );
}

#[test]
fn test_library_edit_prompt_picker_done_with_no_ticks_manages_nothing() {
    let sandbox = Sandbox::new();
    sandbox.prompt("Say hello.\n");
    sandbox.editor_appends("\n{{username}}\n");
    let mut tui = sandbox.tui();
    let checkpoint = begin_edit(&mut tui);
    expect_prompt_picker(&mut tui, checkpoint, "username");

    tui.send(b" ");
    let done = tui.checkpoint();
    tui.send(&[0x13]);
    let visible = tui.wait_for_after(done, "Edited greet.");
    assert!(visible.contains("Edited greet."));
    assert!(sandbox.params("greet").is_empty());
    assert!(sandbox.source_text("greet").contains("{{username}}"));
}

#[test]
fn test_library_edit_prompt_preserves_existing_managed() {
    let sandbox = Sandbox::new();
    sandbox.prompt("{{kept}}\n");
    assert_eq!(
        sandbox.params("greet"),
        vec!["kept".to_owned()],
        "fixture must start with the existing placeholder managed"
    );
    sandbox.editor_appends("\n{{added}}\n");
    let mut tui = sandbox.tui();
    let checkpoint = begin_edit(&mut tui);
    expect_prompt_picker(&mut tui, checkpoint, "added");

    tui.send(&[0x13]);
    assert_eq!(
        sandbox.params("greet"),
        vec!["kept".to_owned(), "added".to_owned()]
    );
    let source = sandbox.source_text("greet");
    assert!(source.contains("{{kept}}") && source.contains("{{added}}"));
}

#[test]
fn test_library_edit_prompt_no_new_placeholder_shows_no_picker() {
    let sandbox = Sandbox::new();
    sandbox.prompt("{{a}}\n");
    assert_eq!(sandbox.params("greet"), vec!["a".to_owned()]);
    sandbox.editor_appends("\nmore prose\n");
    let mut tui = sandbox.tui();
    let checkpoint = begin_edit(&mut tui);
    let visible = tui.wait_for_any_after(
        checkpoint,
        &["Edited greet.", "Choose variables", "Source saved"],
    );
    assert!(
        visible.contains("Edited greet."),
        "prompt edit without a new placeholder did not complete with the frozen status: {visible}"
    );
    assert!(
        !visible.contains("Choose variables"),
        "prompt edit without a new placeholder opened a picker: {visible}"
    );
    assert_eq!(sandbox.params("greet"), vec!["a".to_owned()]);
    assert!(sandbox.source_text("greet").contains("more prose"));
}

#[test]
fn test_library_edit_non_prompt_never_offers_the_picker() {
    let sandbox = Sandbox::new();
    sandbox.python("print(1)\n");
    sandbox.editor_appends("");
    let mut tui = sandbox.tui();
    let checkpoint = begin_edit(&mut tui);
    let visible = tui.wait_for_any_after(
        checkpoint,
        &["Edited job.", "Choose variables", "Source saved"],
    );
    assert!(
        visible.contains("Edited job."),
        "non-prompt edit did not complete with the frozen status: {visible}"
    );
    assert!(
        !visible.contains("Choose variables"),
        "non-prompt edit opened a prompt candidate picker: {visible}"
    );
    assert_eq!(sandbox.source_text("job"), "print(1)\n");
}
