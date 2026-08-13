//! CLI/store required-command ports from Python `tests/test_interpreters.py` at `main@206f9ef`.

use std::{fs, path::{Path, PathBuf}};

use assert_cmd::Command;
use skit_application::{
    CreateEntry, EntryMutationRepository as _, EntryPayload, EntryRepository as _, SourcePermissions,
};
use skit_domain::{EntryKind, EntrySettings, StorageMode};
use skit_store::FileStore;
use tempfile::TempDir;

const MISSING: &str = "__skit_test_missing_ffmpeg_7c391__";

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

    fn needs(&self, name: &str) -> Vec<String> {
        let entry = self.store().resolve(name).unwrap();
        EntrySettings::from_meta(&entry.meta).needs
    }

    fn create_exe_with_need(&self, name: &str, need: &str) {
        let source = PathBuf::from(env!("CARGO_BIN_EXE_skit"));
        let bytes = fs::read(&source).unwrap();
        let mut settings = EntrySettings::default();
        settings.needs = vec![need.to_owned()];
        let mut create = CreateEntry {
            name: name.to_owned(),
            kind: EntryKind::parse("exe").unwrap(),
            mode: StorageMode::Reference,
            source: source.display().to_string(),
            workdir: "origin".to_owned(),
            description: String::new(),
            payload: Some(EntryPayload {
                bytes,
                stored_name: None,
                permissions: permissions(&source),
            }),
            settings: EntrySettings::default(),
        };
        create.settings = settings;
        self.store().create(create).unwrap();
    }
}

fn permissions(path: &Path) -> SourcePermissions {
    let metadata = fs::metadata(path).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        SourcePermissions {
            readonly: metadata.permissions().readonly(),
            unix_mode: Some(metadata.permissions().mode() & 0o7777),
        }
    }
    #[cfg(not(unix))]
    {
        SourcePermissions {
            readonly: metadata.permissions().readonly(),
            unix_mode: None,
        }
    }
}

fn combined(output: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn test_deps_need_sets_the_list() {
    let sandbox = Sandbox::new();
    sandbox.add_python("p");
    sandbox
        .command()
        .args(["deps", "p", "--need", "ffmpeg", "--need", "jq"])
        .assert()
        .success();
    assert_eq!(sandbox.needs("p"), vec!["ffmpeg".to_owned(), "jq".to_owned()]);
}

#[test]
fn test_deps_clear_needs() {
    let sandbox = Sandbox::new();
    sandbox.add_python("p");
    sandbox
        .command()
        .args(["deps", "p", "--need", "ffmpeg"])
        .assert()
        .success();
    assert_eq!(sandbox.needs("p"), vec!["ffmpeg".to_owned()]);
    sandbox
        .command()
        .args(["deps", "p", "--clear-needs"])
        .assert()
        .success();
    assert!(sandbox.needs("p").is_empty());
}

#[test]
fn test_deps_need_rejects_blank() {
    let sandbox = Sandbox::new();
    sandbox.add_python("p");
    let output = sandbox
        .command()
        .args(["deps", "p", "--need", "   "])
        .output()
        .unwrap();
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(2), "{text}");
    assert!(sandbox.needs("p").is_empty(), "blank --need mutated metadata");
}

#[test]
fn test_run_preflight_missing_needs_does_not_spawn() {
    let sandbox = Sandbox::new();
    sandbox.create_exe_with_need("prog", MISSING);
    let output = sandbox
        .command()
        .args(["run", "prog", "--no-input", "--", "--version"])
        .output()
        .unwrap();
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(126), "{text}");
    assert!(text.contains(MISSING), "missing command was not named: {text}");
    assert!(
        !String::from_utf8_lossy(&output.stdout)
            .contains(&format!("skit {}", env!("CARGO_PKG_VERSION"))),
        "the executable child ran despite the missing required command: {text}"
    );
}

#[test]
fn test_doctor_flags_missing_needs() {
    let sandbox = Sandbox::new();
    sandbox.create_exe_with_need("needful", MISSING);
    let output = sandbox.command().arg("doctor").output().unwrap();
    let text = combined(&output);
    assert!(text.contains("needful"), "doctor omitted the affected entry: {text}");
    assert!(text.contains(MISSING), "doctor omitted the missing required command: {text}");
}

#[test]
fn test_cli_add_need_option_records_needs() {
    let sandbox = Sandbox::new();
    let source = sandbox.home.path().join("needful.sh");
    fs::write(&source, "#!/bin/sh\necho hi\n").unwrap();
    sandbox
        .command()
        .arg("add")
        .arg(&source)
        .args(["--name", "needful", "--need", "jq", "--no-input"])
        .assert()
        .success();
    assert_eq!(sandbox.needs("needful"), vec!["jq".to_owned()]);
}
