//! Public-process ports from Python `tests/test_edit.py` at `main@206f9ef`.
//!
//! These tests use authoritative stored source as the oracle. Source-management edits must rewrite
//! the real copy, read-only views must leave its bytes unchanged, reference edits must not touch the
//! original, and `skit edit` on a command must refuse before a real editor probe can run.

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command as ProcessCommand, Output},
};

use assert_cmd::Command;
use skit_application::{
    CreateEntry, EntryMutationRepository, EntryPayload, EntryRepository, SourcePermissions,
};
use skit_domain::{
    EntryKind, EntrySettings, StorageMode,
    parameters::{ParamDecl, ParameterBinding, ParameterDelivery, ParameterType},
};
use skit_language::{managed_params, write_managed_params_bytes};
use skit_store::FileStore;
use tempfile::TempDir;

const SCRIPT: &[u8] = concat!(
    "CITY = \"Taipei\"\n",
    "RETRIES = 3\n",
    "who = input(\"Name: \")\n",
    "print(CITY, RETRIES, who)\n",
)
.as_bytes();

struct Sandbox {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    home: TempDir,
    tools: TempDir,
}

impl Sandbox {
    fn new() -> Self {
        Self {
            data: TempDir::new().unwrap(),
            state: TempDir::new().unwrap(),
            config: TempDir::new().unwrap(),
            home: TempDir::new().unwrap(),
            tools: TempDir::new().unwrap(),
        }
    }

    fn store(&self) -> FileStore {
        FileStore::new(self.data.path())
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
            .current_dir(self.home.path());
        command
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command().args(args).output().unwrap()
    }

    fn create_python_copy(&self, name: &str, bytes: &[u8]) -> skit_domain::Entry {
        self.store()
            .create(CreateEntry {
                name: name.to_owned(),
                kind: EntryKind::parse("python").unwrap(),
                mode: StorageMode::Copy,
                source: self
                    .home
                    .path()
                    .join(format!("{name}.py"))
                    .display()
                    .to_string(),
                workdir: "invoke".to_owned(),
                description: String::new(),
                payload: Some(EntryPayload {
                    bytes: bytes.to_vec(),
                    stored_name: Some("script.py".to_owned()),
                    permissions: SourcePermissions::default(),
                }),
                settings: EntrySettings::default(),
            })
            .unwrap()
    }

    fn create_managed_fixture(&self, name: &str) -> skit_domain::Entry {
        let declarations = vec![
            source_decl("CITY", ParameterType::Str),
            source_decl("RETRIES", ParameterType::Int),
            source_decl("GONE", ParameterType::Str),
        ];
        let bytes = write_managed_params_bytes("python", SCRIPT, &declarations).unwrap();
        self.create_python_copy(name, &bytes)
    }

    fn payload(&self, selector: &str) -> PathBuf {
        let store = self.store();
        let entry = store.resolve(selector).unwrap();
        store.payload_path(&entry).unwrap()
    }

    fn managed(&self, selector: &str) -> Vec<ParamDecl> {
        let text = fs::read_to_string(self.payload(selector)).unwrap();
        managed_params("python", &text)
    }
}

fn source_decl(name: &str, parameter_type: ParameterType) -> ParamDecl {
    let mut declaration = ParamDecl::new(name);
    declaration.binding = ParameterBinding::Const;
    declaration.delivery = ParameterDelivery::Inject;
    declaration.parameter_type = parameter_type;
    declaration
}

fn combined(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn assert_success(output: &Output) {
    assert_eq!(output.status.code(), Some(0), "{}", combined(output));
}

#[test]
fn test_add_brings_candidate_under_management() {
    let sandbox = Sandbox::new();
    sandbox.create_python_copy("managed-add", SCRIPT);
    assert_success(&sandbox.run(&["params", "managed-add", "--manage", "CITY"]));

    let output = sandbox.run(&["params", "managed-add", "--manage", "RETRIES"]);
    assert_success(&output);

    let declarations = sandbox.managed("managed-add");
    assert_eq!(
        declarations
            .iter()
            .map(|item| item.name.as_str())
            .collect::<Vec<_>>(),
        ["CITY", "RETRIES"]
    );
    assert_eq!(declarations[1].parameter_type, ParameterType::Int);
}

#[test]
fn test_add_input_candidate_by_display_name() {
    let sandbox = Sandbox::new();
    sandbox.create_python_copy("managed-input", SCRIPT);

    let output = sandbox.run(&["params", "managed-input", "--manage", "input-1"]);
    assert_success(&output);

    let declarations = sandbox.managed("managed-input");
    assert_eq!(declarations.len(), 1);
    assert_eq!(declarations[0].binding, ParameterBinding::Input);
    assert_eq!(declarations[0].order, 0);
}

#[test]
fn test_remove_and_secret_toggles() {
    let sandbox = Sandbox::new();
    sandbox.create_python_copy("managed-remove", SCRIPT);
    assert_success(&sandbox.run(&["params", "managed-remove", "--manage", "CITY"]));
    assert_success(&sandbox.run(&["params", "managed-remove", "--manage", "RETRIES"]));

    let output = sandbox.run(&[
        "params",
        "managed-remove",
        "--unmanage",
        "CITY",
        "--secret",
        "RETRIES",
        "--prompt",
        "RETRIES=N: ",
    ]);
    assert_success(&output);

    let declarations = sandbox.managed("managed-remove");
    assert_eq!(declarations.len(), 1);
    assert_eq!(declarations[0].name, "RETRIES");
    assert!(declarations[0].secret);
    assert_eq!(declarations[0].prompt, "N: ");
}

#[test]
fn test_cli_resync_prunes_and_persists() {
    let sandbox = Sandbox::new();
    sandbox.create_managed_fixture("edit-resync");

    let output = sandbox.run(&["params", "edit-resync", "--resync"]);
    assert_success(&output);

    let names = sandbox
        .managed("edit-resync")
        .into_iter()
        .map(|item| item.name)
        .collect::<Vec<_>>();
    assert_eq!(names, ["CITY", "RETRIES"]);
}

#[test]
fn test_cli_secret_and_prompt_persist() {
    let sandbox = Sandbox::new();
    sandbox.create_managed_fixture("edit-secret");

    let output = sandbox.run(&[
        "params",
        "edit-secret",
        "--secret",
        "CITY",
        "--prompt",
        "CITY=Where? ",
    ]);
    assert_success(&output);

    let declarations = sandbox.managed("edit-secret");
    let city = declarations
        .iter()
        .find(|item| item.name == "CITY")
        .expect("CITY remains managed");
    assert!(city.secret);
    assert_eq!(city.prompt, "Where? ");
}

#[test]
fn test_cli_params_view_no_ops() {
    let sandbox = Sandbox::new();
    sandbox.create_managed_fixture("edit-view");
    let path = sandbox.payload("edit-view");
    let before = fs::read(&path).unwrap();

    let output = sandbox.run(&["params", "edit-view"]);
    assert_success(&output);

    assert!(combined(&output).contains("CITY"), "{}", combined(&output));
    assert_eq!(
        fs::read(&path).unwrap(),
        before,
        "read-only params view rewrote the source"
    );
    assert_eq!(sandbox.managed("edit-view").len(), 3);
}

#[test]
fn test_cli_bad_prompt_is_warned_not_fatal() {
    let sandbox = Sandbox::new();
    sandbox.create_managed_fixture("edit-warning");
    let path = sandbox.payload("edit-warning");
    let before = fs::read(&path).unwrap();

    let output = sandbox.run(&["params", "edit-warning", "--prompt", "no-equals-sign"]);

    assert_eq!(
        output.status.code(),
        Some(0),
        "Python treats a malformed prompt tweak as a warning, not a fatal edit:\n{}",
        combined(&output)
    );
    assert_eq!(fs::read(path).unwrap(), before);
}

#[test]
fn test_cli_params_edit_reference_refused() {
    let sandbox = Sandbox::new();
    let source = sandbox.home.path().join("ref.py");
    fs::write(&source, SCRIPT).unwrap();
    sandbox
        .store()
        .create(CreateEntry {
            name: "ref".to_owned(),
            kind: EntryKind::parse("python").unwrap(),
            mode: StorageMode::Reference,
            source: source.display().to_string(),
            workdir: "origin".to_owned(),
            description: String::new(),
            payload: None,
            settings: EntrySettings::default(),
        })
        .unwrap();
    let before = fs::read(&source).unwrap();

    let output = sandbox.run(&["params", "ref", "--resync"]);

    assert_eq!(output.status.code(), Some(1), "{}", combined(&output));
    assert_eq!(
        fs::read(source).unwrap(),
        before,
        "reference source was modified by params --resync"
    );
}

#[test]
fn test_cli_edit_command_entry_has_no_source() {
    let sandbox = Sandbox::new();
    sandbox
        .store()
        .create(CreateEntry {
            name: "ec".to_owned(),
            kind: EntryKind::parse("command").unwrap(),
            mode: StorageMode::Reference,
            source: String::new(),
            workdir: "invoke".to_owned(),
            description: String::new(),
            payload: None,
            settings: EntrySettings {
                template: "echo {x}".to_owned(),
                ..EntrySettings::default()
            },
        })
        .unwrap();
    let capture = sandbox.tools.path().join("editor-ran");
    let editor = compile_editor_probe(sandbox.tools.path(), &capture);

    let output = sandbox
        .command()
        .env("VISUAL", &editor)
        .env("EDITOR", &editor)
        .env("SKIT_EDIT_CAPTURE", &capture)
        .args(["edit", "ec"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1), "{}", combined(&output));
    assert!(
        !capture.exists(),
        "command-entry edit launched the editor before refusing"
    );
}

fn compile_editor_probe(root: &Path, capture: &Path) -> PathBuf {
    let source = root.join("must_not_edit.rs");
    fs::write(
        &source,
        r#"
use std::{env, fs};
fn main() {
    fs::write(env::var_os("SKIT_EDIT_CAPTURE").expect("capture"), b"launched").unwrap();
}
"#,
    )
    .unwrap();
    let executable = root.join(if cfg!(windows) {
        "must-not-edit.exe"
    } else {
        "must-not-edit"
    });
    let status = ProcessCommand::new("rustc")
        .arg(source)
        .arg("-o")
        .arg(&executable)
        .status()
        .unwrap();
    assert!(status.success(), "failed to compile editor launch probe");
    assert!(!capture.exists());
    executable
}
