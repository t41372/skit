//! Host-boundary ports from Python `tests/test_draft_and_reader_tui.py` at `main@206f9ef`.
//!
//! The draft-resume contract executes the typed Add commit against a real `FileStore` and consumes
//! the draft only after the successful repository result. Settings contracts create real shell
//! entries through the CLI and inspect the real Settings screen through a PTY.

use std::{
    fs,
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

use assert_cmd::Command;
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use skit_application::{
    EntryMutationRepository as _, EntryRepository as _, SourcePermissions,
};
use skit_store::FileStore;
use skit_ui::{
    AddAction, AddEffect, AddStage, AddWorkflowState, DraftSummary, KnownEntryKind, SourceSnapshot,
};
use tempfile::TempDir;

const DYN_SH: &str =
    "#!/usr/bin/env bash\nOUTDIR=/tmp\nOPTS=\"n:v\"\nwhile getopts \"$OPTS\" o; do :; done\necho $OUTDIR\n";
const MODELED_SH: &str =
    "#!/usr/bin/env bash\nCITY=Taipei\nwhile getopts 'n:v' o; do :; done\necho $CITY\n";

fn source_snapshot(path: &Path, bytes: &[u8], is_draft: bool) -> SourceSnapshot {
    SourceSnapshot {
        path: path.to_path_buf(),
        source_record: path.display().to_string(),
        bytes: bytes.to_vec(),
        permissions: SourcePermissions::default(),
        is_regular: true,
        is_directory: false,
        is_draft,
    }
}

#[test]
fn test_resume_bash_shebang_draft_lands_as_shell() {
    let data = TempDir::new().unwrap();
    let drafts = data.path().join("drafts");
    fs::create_dir_all(&drafts).unwrap();
    let draft = drafts.join("skit-new-ship.py");
    let bytes = b"#!/usr/bin/env bash\necho drafted\n";
    fs::write(&draft, bytes).unwrap();

    let mut workflow = AddWorkflowState::new(vec![DraftSummary {
        path: draft.clone(),
        modified: 1,
    }]);
    assert!(workflow.reduce(AddAction::SelectDraft(0)).is_empty());
    let effects = workflow.reduce(AddAction::Continue);
    let [AddEffect::InspectSource { request, path }] = effects.as_slice() else {
        panic!("resuming the highlighted draft must inspect that source first");
    };
    assert_eq!(path, &draft);
    let request = *request;
    assert!(
        workflow
            .reduce(AddAction::SourceInspected {
                request,
                result: Ok(source_snapshot(&draft, bytes, true)),
            })
            .is_empty()
    );
    assert_eq!(workflow.stage(), AddStage::Review);
    assert_eq!(workflow.review().unwrap().kind(), KnownEntryKind::Shell);
    assert!(workflow.review().unwrap().is_fresh());

    assert!(
        workflow
            .reduce(AddAction::SetReviewName("shipit".to_owned()))
            .is_empty()
    );
    let effects = workflow.reduce(AddAction::Save);
    let [AddEffect::Commit {
        request,
        entry,
        source,
    }] = effects.as_slice()
    else {
        panic!("accepting the TUI review must emit one atomic repository commit");
    };
    assert_eq!(entry.kind.as_str(), "shell");
    assert_eq!(entry.name, "shipit");
    assert!(source.as_ref().is_some_and(|snapshot| snapshot.is_draft));

    let store = FileStore::new(data.path());
    store.create((**entry).clone()).unwrap();
    assert_eq!(store.resolve("shipit").unwrap().meta.kind.as_str(), "shell");
    let request = *request;
    let followups = workflow.reduce(AddAction::CommitFinished {
        request,
        result: Ok("shipit".to_owned()),
    });
    let [AddEffect::ConsumeDraft(path)] = followups.as_slice() else {
        panic!("a successful copied draft commit must request draft consumption: {followups:?}");
    };
    assert_eq!(path, &draft);
    fs::remove_file(path).unwrap();
    assert!(!draft.exists(), "successful draft resume left the kept draft behind");
}

struct TuiSandbox {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    home: TempDir,
    empty_path: TempDir,
}

impl TuiSandbox {
    fn new() -> Self {
        let sandbox = Self {
            data: TempDir::new().unwrap(),
            state: TempDir::new().unwrap(),
            config: TempDir::new().unwrap(),
            home: TempDir::new().unwrap(),
            empty_path: TempDir::new().unwrap(),
        };
        fs::write(
            sandbox.config.path().join("config.toml"),
            "[mirror]\nenabled = false\n",
        )
        .unwrap();
        sandbox
    }

    fn command(&self) -> Command {
        let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
        command
            .env("SKIT_DATA_DIR", self.data.path())
            .env("SKIT_STATE_DIR", self.state.path())
            .env("SKIT_CONFIG_DIR", self.config.path())
            .env("SKIT_LANG", "en")
            .env("HOME", self.home.path())
            .env("USERPROFILE", self.home.path())
            .env("XDG_CONFIG_HOME", self.home.path().join("xdg-config"))
            .env("XDG_DATA_HOME", self.home.path().join("xdg-data"))
            .env("XDG_STATE_HOME", self.home.path().join("xdg-state"))
            .env("PATH", self.empty_path.path())
            .env("EDITOR", "__skit_test_no_editor__")
            .env("VISUAL", "__skit_test_no_editor__")
            .env_remove("FORCE_COLOR")
            .env_remove("NO_COLOR")
            .env_remove("CLICOLOR")
            .env_remove("CLICOLOR_FORCE")
            .current_dir(self.home.path());
        command
    }

    fn add_shell(&self, name: &str, body: &str) {
        let source = self.home.path().join(format!("{name}.sh"));
        fs::write(&source, body).unwrap();
        self.command()
            .arg("add")
            .arg(&source)
            .args(["--name", name, "--no-input"])
            .assert()
            .success();
    }

    fn run_settings(&self) -> (u32, String) {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 50,
                cols: 130,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let mut command = CommandBuilder::new(PathBuf::from(env!("CARGO_BIN_EXE_skit")));
        command.arg("tui");
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
        command.env("PATH", self.empty_path.path());
        command.env("EDITOR", "__skit_test_no_editor__");
        command.env("VISUAL", "__skit_test_no_editor__");
        let mut child = pair.slave.spawn_command(command).unwrap();
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader().unwrap();
        let drain = thread::spawn(move || {
            let mut bytes = Vec::new();
            reader.read_to_end(&mut bytes).unwrap();
            bytes
        });
        let mut writer = pair.master.take_writer().unwrap();
        thread::sleep(Duration::from_millis(60));
        writer.write_all(b"\x1b[1;1R").unwrap();
        writer.flush().unwrap();
        thread::sleep(Duration::from_millis(220));
        writer.write_all(b"p").unwrap();
        writer.flush().unwrap();
        thread::sleep(Duration::from_millis(420));
        writer.write_all(b"\x1b").unwrap();
        writer.flush().unwrap();
        thread::sleep(Duration::from_millis(120));
        writer.write_all(b"q").unwrap();
        writer.flush().unwrap();

        let status = child.wait().unwrap();
        drop(writer);
        let output = String::from_utf8_lossy(&drain.join().unwrap()).into_owned();
        (status.exit_code(), output)
    }
}

#[test]
fn test_settings_dynamic_optstring_offers_tick_checkboxes() {
    let sandbox = TuiSandbox::new();
    sandbox.add_shell("dyn", DYN_SH);
    let (code, output) = sandbox.run_settings();

    assert_eq!(code, 0, "{output}");
    assert!(output.contains("Entry settings"), "Settings did not open:\n{output}");
    assert!(
        output.contains("Detected but not yet managed"),
        "dynamic getopts did not offer unmanaged candidates:\n{output}"
    );
    assert!(output.contains("OUTDIR"), "the OUTDIR candidate is not visible:\n{output}");
    assert!(
        !output.contains("comes from its own command-line arguments"),
        "dynamic getopts was incorrectly treated as a modeled CLI form:\n{output}"
    );
}

#[test]
fn test_settings_modeled_getopts_hides_tick_checkboxes() {
    let sandbox = TuiSandbox::new();
    sandbox.add_shell("mod", MODELED_SH);
    let (code, output) = sandbox.run_settings();

    assert_eq!(code, 0, "{output}");
    assert!(output.contains("Entry settings"), "Settings did not open:\n{output}");
    assert!(
        output.contains("comes from its own command-line arguments"),
        "modeled getopts lost the leave-it-as-is notice:\n{output}"
    );
    assert!(
        !output.contains("Detected but not yet managed"),
        "modeled getopts still offered manage-a-constant checkboxes:\n{output}"
    );
}
