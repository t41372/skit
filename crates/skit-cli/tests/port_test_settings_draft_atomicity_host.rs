//! Real host/TUI ports from Python `tests/test_settings_and_draft_review_atomicity.py` at
//! `main@206f9ef`. Settings edits travel through a real `skit tui` PTY into the real FileStore.
//! Navigation distances are derived from the typed Settings surface, never hard-coded.

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
use skit_domain::EntrySettings;
use skit_language::{UvMetadata, effective_uv_metadata_bytes};
use skit_store::FileStore;
use skit_ui::{
    AddAction, AddEffect, AddStage, AddWorkflowState, DEPENDENCIES_KEY, DependencyFlavor,
    DraftSummary, NAME_KEY, PYTHON_KEY, SettingsInputs, SettingsView, SourceSnapshot,
};
use tempfile::TempDir;

struct Sandbox {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    home: TempDir,
    empty_path: TempDir,
}

impl Sandbox {
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

    fn store(&self) -> FileStore {
        FileStore::new(self.data.path())
    }

    fn add_python(&self, name: &str) {
        let source = self.home.path().join(format!("{name}.py"));
        fs::write(&source, "print(1)\n").unwrap();
        self.command()
            .arg("add")
            .arg(&source)
            .args(["--name", name, "--no-input"])
            .assert()
            .success();
    }

    fn add_js(&self, name: &str) {
        let source = self.home.path().join(format!("{name}.js"));
        fs::write(&source, "console.log(1);\n").unwrap();
        self.command()
            .arg("add")
            .arg(&source)
            .args(["--name", name, "--no-input"])
            .assert()
            .success();
    }

    fn set_python_deps(&self, name: &str, dependencies: &[&str], python: &str) {
        let mut command = self.command();
        command.args(["deps", name]);
        for dependency in dependencies {
            command.args(["--dep", dependency]);
        }
        if !python.is_empty() {
            command.args(["--python", python]);
        }
        command.assert().success();
    }

    fn set_js_deps_without_install(&self, name: &str, dependencies: &[&str]) {
        let store = self.store();
        let entry = store.resolve(name).unwrap();
        let mut settings = EntrySettings::from_meta(&entry.meta);
        settings.dependencies = dependencies.iter().map(|value| (*value).to_owned()).collect();
        store
            .update_settings(&entry, &settings, &entry.meta.workdir)
            .unwrap();
    }

    fn add_taken_name(&self) {
        self.command()
            .args(["add", "--cmd", "echo ok", "--name", "taken", "--no-input"])
            .assert()
            .success();
    }

    fn python_uv_metadata(&self, name: &str) -> UvMetadata {
        let store = self.store();
        let entry = store.resolve(name).unwrap();
        let payload = store.payload_path(&entry).unwrap();
        let bytes = fs::read(payload).unwrap();
        let settings = EntrySettings::from_meta(&entry.meta);
        let stored = UvMetadata {
            dependencies: settings.dependencies,
            requires_python: settings.requires_python,
        };
        effective_uv_metadata_bytes(Some(&bytes), &stored)
    }

    fn js_dependencies(&self, name: &str) -> Vec<String> {
        let entry = self.store().resolve(name).unwrap();
        EntrySettings::from_meta(&entry.meta).dependencies
    }

    fn run_settings(&self, inputs: &[Vec<u8>]) -> String {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 50,
                cols: 140,
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
        thread::sleep(Duration::from_millis(320));
        for input in inputs {
            let _ = writer.write_all(input);
            let _ = writer.flush();
            thread::sleep(Duration::from_millis(180));
        }
        thread::sleep(Duration::from_millis(420));
        // Exit regardless of whether Save succeeded or correctly left a dirty Settings screen open.
        let _ = writer.write_all(b"\x03");
        let _ = writer.flush();
        thread::sleep(Duration::from_millis(100));
        let _ = writer.write_all(b"\x03");
        let _ = writer.flush();

        let _ = child.wait();
        drop(writer);
        String::from_utf8_lossy(&drain.join().unwrap()).into_owned()
    }
}

fn settings_model(kind: &str, name: &str, dependencies: &[&str], python: &str) -> SettingsView {
    SettingsView::from_inputs(&SettingsInputs {
        selector: name.to_owned(),
        kind: kind.to_owned(),
        name: name.to_owned(),
        source: format!("{name}.{kind}"),
        workdir: "invoke".to_owned(),
        supports_modes: true,
        has_original_file: true,
        has_stored_name: true,
        pinnable_interpreter: matches!(kind, "js" | "ts"),
        has_analyzer: true,
        dependency_flavor: Some(if kind == "python" {
            DependencyFlavor::Uv
        } else {
            DependencyFlavor::Npm
        }),
        effective_dependencies: dependencies.iter().map(|value| (*value).to_owned()).collect(),
        effective_requires_python: python.to_owned(),
        ..SettingsInputs::default()
    })
}

fn tabs_between(view: &SettingsView, from: &str, to: &str) -> usize {
    let keys = view.focusable_keys();
    let from = keys
        .iter()
        .position(|key| *key == from)
        .unwrap_or_else(|| panic!("Settings fixture has no {from:?}: {keys:?}"));
    let to = keys
        .iter()
        .position(|key| *key == to)
        .unwrap_or_else(|| panic!("Settings fixture has no {to:?}: {keys:?}"));
    to.checked_sub(from)
        .unwrap_or_else(|| panic!("target {to} precedes source {from} in {keys:?}"))
}

fn push_tabs(inputs: &mut Vec<Vec<u8>>, count: usize) {
    inputs.extend((0..count).map(|_| b"\t".to_vec()));
}

fn replace_current(inputs: &mut Vec<Vec<u8>>, old_value: &str, new_value: &str) {
    inputs.push(b"\x1b[F".to_vec()); // End
    inputs.extend((0..old_value.chars().count()).map(|_| vec![0x7f]));
    if !new_value.is_empty() {
        inputs.push(new_value.as_bytes().to_vec());
    }
}

fn save(inputs: &mut Vec<Vec<u8>>) {
    inputs.push(b"\x13".to_vec()); // Ctrl+S
}

fn source_snapshot(path: &Path, bytes: &[u8]) -> SourceSnapshot {
    SourceSnapshot {
        path: path.to_path_buf(),
        source_record: path.display().to_string(),
        bytes: bytes.to_vec(),
        permissions: SourcePermissions::default(),
        is_regular: true,
        is_directory: false,
        is_draft: true,
    }
}

#[test]
fn test_settings_bad_dep_refuses_the_whole_save_including_the_rename() {
    let sandbox = Sandbox::new();
    sandbox.add_python("orig");
    let model = settings_model("python", "orig", &[], "");
    assert_eq!(model.focused(), NAME_KEY);
    let mut inputs = Vec::new();
    replace_current(&mut inputs, "orig", "renamed");
    push_tabs(&mut inputs, tabs_between(&model, NAME_KEY, DEPENDENCIES_KEY));
    replace_current(&mut inputs, "", "@@@");
    save(&mut inputs);

    let output = sandbox.run_settings(&inputs);
    assert!(
        output.to_lowercase().contains("package requirement"),
        "invalid package requirement did not surface as an error: {output}"
    );
    assert!(sandbox.store().resolve("renamed").is_err(), "invalid deps still committed rename");
    assert_eq!(sandbox.store().resolve("orig").unwrap().meta.name, "orig");
}

#[test]
fn test_settings_bad_python_refuses_the_whole_save_including_the_rename() {
    let sandbox = Sandbox::new();
    sandbox.add_python("orig2");
    let model = settings_model("python", "orig2", &[], "");
    let mut inputs = Vec::new();
    replace_current(&mut inputs, "orig2", "renamed2");
    push_tabs(&mut inputs, tabs_between(&model, NAME_KEY, PYTHON_KEY));
    replace_current(&mut inputs, "", "not-a-version");
    save(&mut inputs);

    let output = sandbox.run_settings(&inputs);
    assert!(
        output.to_lowercase().contains("version constraint"),
        "invalid Python constraint did not surface as an error: {output}"
    );
    assert!(sandbox.store().resolve("renamed2").is_err(), "invalid pin still committed rename");
    assert_eq!(sandbox.store().resolve("orig2").unwrap().meta.name, "orig2");
}

#[test]
fn test_settings_dash_python_saves_as_automatic() {
    let sandbox = Sandbox::new();
    sandbox.add_python("autoset");
    sandbox.set_python_deps("autoset", &["requests"], ">=3.11");
    let model = settings_model("python", "autoset", &["requests"], ">=3.11");
    let mut inputs = Vec::new();
    push_tabs(&mut inputs, tabs_between(&model, NAME_KEY, PYTHON_KEY));
    replace_current(&mut inputs, ">=3.11", "-");
    save(&mut inputs);
    let _ = sandbox.run_settings(&inputs);

    let metadata = sandbox.python_uv_metadata("autoset");
    assert_eq!(metadata.requires_python, "");
    assert_eq!(metadata.dependencies, vec!["requests".to_owned()]);
}

#[test]
fn test_settings_valid_deps_and_python_save_normally() {
    let sandbox = Sandbox::new();
    sandbox.add_python("okset");
    let model = settings_model("python", "okset", &[], "");
    let mut inputs = Vec::new();
    push_tabs(&mut inputs, tabs_between(&model, NAME_KEY, DEPENDENCIES_KEY));
    replace_current(&mut inputs, "", "requests>=2,<3");
    push_tabs(
        &mut inputs,
        tabs_between(&model, DEPENDENCIES_KEY, PYTHON_KEY),
    );
    replace_current(&mut inputs, "", "~=3.12");
    save(&mut inputs);
    let _ = sandbox.run_settings(&inputs);

    let metadata = sandbox.python_uv_metadata("okset");
    assert_eq!(metadata.dependencies, vec!["requests>=2,<3".to_owned()]);
    assert_eq!(metadata.requires_python, "~=3.12");
}

#[test]
fn test_settings_npm_deps_are_not_pep508_validated() {
    let sandbox = Sandbox::new();
    sandbox.add_js("jsset");
    let model = settings_model("js", "jsset", &[], "");
    assert!(model.field(PYTHON_KEY).is_none(), "npm Settings unexpectedly has a Python field");
    let mut inputs = Vec::new();
    push_tabs(&mut inputs, tabs_between(&model, NAME_KEY, DEPENDENCIES_KEY));
    replace_current(&mut inputs, "", "@scope/thing");
    save(&mut inputs);
    let _ = sandbox.run_settings(&inputs);

    assert_eq!(
        sandbox.js_dependencies("jsset"),
        vec!["@scope/thing".to_owned()]
    );
}

#[test]
fn test_settings_name_conflict_is_refused_before_npm_clear() {
    let sandbox = Sandbox::new();
    // The Library is activity-ordered. Create the conflict first so `js-original`, the entry this
    // test edits, is the newest selected row when the real TUI opens.
    sandbox.add_taken_name();
    sandbox.add_js("js-original");
    sandbox.set_js_deps_without_install("js-original", &["chalk"]);
    let entry = sandbox.store().resolve("js-original").unwrap();
    let entry_dir = sandbox.data.path().join("scripts").join(entry.slug.as_str());
    fs::write(entry_dir.join(".skit-deps"), "owned stamp").unwrap();
    fs::write(
        entry_dir.join("package.json"),
        "{\"name\":\"skit-private-entry\",\"private\":true,\"skit\":{\"generated\":true}}\n",
    )
    .unwrap();
    fs::create_dir_all(entry_dir.join("node_modules/sentinel")).unwrap();

    let model = settings_model("js", "js-original", &["chalk"], "");
    let mut inputs = Vec::new();
    replace_current(&mut inputs, "js-original", "taken");
    push_tabs(&mut inputs, tabs_between(&model, NAME_KEY, DEPENDENCIES_KEY));
    replace_current(&mut inputs, "chalk", "");
    save(&mut inputs);
    let output = sandbox.run_settings(&inputs);

    assert!(output.contains("already taken"), "name conflict was not surfaced: {output}");
    let after = sandbox.store().resolve("js-original").unwrap();
    assert_eq!(after.meta.name, "js-original");
    assert_eq!(sandbox.js_dependencies("js-original"), vec!["chalk".to_owned()]);
    assert!(entry_dir.join(".skit-deps").is_file(), "name precheck already cleared stamp");
    assert!(entry_dir.join("package.json").is_file(), "name precheck already cleared manifest");
    assert!(
        entry_dir.join("node_modules/sentinel").is_dir(),
        "name precheck already cleared node_modules"
    );
}

#[test]
fn test_resumed_draft_through_the_tui_add_lane_is_consumed() {
    let sandbox = Sandbox::new();
    let drafts = sandbox.data.path().join("drafts");
    fs::create_dir_all(&drafts).unwrap();
    let draft = drafts.join("skit-new-consumeme.py");
    let bytes = b"print('bye')\n";
    fs::write(&draft, bytes).unwrap();

    let mut workflow = AddWorkflowState::new(vec![DraftSummary {
        path: draft.clone(),
        modified: 1,
    }]);
    assert!(workflow.reduce(AddAction::SelectDraft(0)).is_empty());
    let effects = workflow.reduce(AddAction::Continue);
    let [AddEffect::InspectSource { request, path }] = effects.as_slice() else {
        panic!("TUI add lane did not inspect the resumed draft");
    };
    assert_eq!(path, &draft);
    let request = *request;
    assert!(
        workflow
            .reduce(AddAction::SourceInspected {
                request,
                result: Ok(source_snapshot(&draft, bytes)),
            })
            .is_empty()
    );
    assert_eq!(workflow.stage(), AddStage::Review);
    assert!(
        workflow
            .reduce(AddAction::SetReviewName("consumed".to_owned()))
            .is_empty()
    );
    let effects = workflow.reduce(AddAction::Save);
    let [AddEffect::Commit {
        request,
        entry,
        source: _,
    }] = effects.as_slice()
    else {
        panic!("accepted TUI review did not emit one commit");
    };
    sandbox.store().create((**entry).clone()).unwrap();
    assert_eq!(
        sandbox.store().resolve("consumed").unwrap().meta.mode,
        skit_domain::StorageMode::Copy
    );
    let request = *request;
    let followups = workflow.reduce(AddAction::CommitFinished {
        request,
        result: Ok("consumed".to_owned()),
    });
    let [AddEffect::ConsumeDraft(path)] = followups.as_slice() else {
        panic!("successful copied draft commit did not request consumption: {followups:?}");
    };
    assert_eq!(path, &draft);
    fs::remove_file(path).unwrap();
    assert!(!draft.exists());
}
