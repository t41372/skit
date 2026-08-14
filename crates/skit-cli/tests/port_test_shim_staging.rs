//! Exact staged-copy ports from Python v0.4 `tests/test_shim.py`.
//!
//! These cross the real CLI, FileStore, Python source injector, launch planner, and a real Python
//! child. The child reports the staged script path it actually received. Python v0.4 requires the
//! ordinary path to live in the OS temp directory, outside the persistent entry directory; the
//! entry directory is only a fallback when the OS temp directory is unavailable. A Rust mismatch
//! is intentionally red rather than normalized away.

use std::{fs, path::{Path, PathBuf}};

use assert_cmd::Command;
use skit_application::{CreateEntry, EntryMutationRepository as _, EntryPayload, EntryRepository as _, SourcePermissions, payload_stored_name};
use skit_domain::{EntryKind, EntrySettings, StorageMode};
use skit_language::{ParseOutcome, parse_document, write_managed_params};
use skit_runtime::{ProgramProbe as _, SystemProbe};
use skit_store::FileStore;
use tempfile::TempDir;

struct Sandbox {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    home: TempDir,
    python: PathBuf,
}

impl Sandbox {
    fn new() -> Self {
        let python = SystemProbe
            .find_program("python3")
            .or_else(|| SystemProbe.find_program("python"))
            .expect("Python v0.4 shim parity tests require a real Python interpreter");
        let sandbox = Self {
            data: TempDir::new().unwrap(),
            state: TempDir::new().unwrap(),
            config: TempDir::new().unwrap(),
            home: TempDir::new().unwrap(),
            python,
        };
        fs::write(sandbox.config.path().join("config.toml"), "[mirror]\nenabled = false\n").unwrap();
        fs::write(
            sandbox.home.path().join("run"),
            concat!(
                "from pathlib import Path\n",
                "import sys\n",
                "p = Path(sys.argv[-1]).resolve()\n",
                "print('STAGED_PATH=' + str(p))\n",
                "print('BODY_BEGIN')\n",
                "print(p.read_text(encoding='utf-8'), end='')\n",
                "print('BODY_END')\n",
            ),
        )
        .unwrap();
        sandbox
    }

    fn store(&self) -> FileStore { FileStore::new(self.data.path()) }

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
            .env_remove("FORCE_COLOR")
            .env_remove("NO_COLOR")
            .current_dir(self.home.path());
        command
    }

    fn create_entry(&self, name: &str) {
        let kind = EntryKind::parse("python").unwrap();
        let source = "CITY = 'Taipei'\nprint(CITY)\n";
        let ParseOutcome::Parsed(document) = parse_document("python", source) else {
            panic!("Python staging fixture must parse");
        };
        let declaration = document
            .analysis()
            .candidates
            .into_iter()
            .find(|candidate| candidate.declaration.name == "CITY")
            .expect("CITY must be a managed const")
            .declaration;
        let managed = write_managed_params("python", source, &[declaration]).unwrap();
        self.store()
            .create(CreateEntry {
                name: name.to_owned(),
                kind: kind.clone(),
                mode: StorageMode::Copy,
                source: format!("{name}.py"),
                workdir: "invoke".to_owned(),
                description: String::new(),
                payload: Some(EntryPayload {
                    bytes: managed.into_bytes(),
                    stored_name: Some(payload_stored_name(&kind, Path::new(&format!("{name}.py")))),
                    permissions: SourcePermissions::default(),
                }),
                settings: EntrySettings {
                    interpreter: self.python.display().to_string(),
                    ..EntrySettings::default()
                },
            })
            .unwrap();
    }

    fn run(&self, name: &str, configure_temp_failure: bool) -> std::process::Output {
        let mut command = self.command();
        if configure_temp_failure {
            let blocked = self.home.path().join("not-a-temp-directory");
            fs::write(&blocked, "file, not directory").unwrap();
            command
                .env("TMPDIR", &blocked)
                .env("TMP", &blocked)
                .env("TEMP", &blocked);
        }
        command
            .args(["run", name, "--set", "CITY=Kaohsiung", "--no-input"])
            .output()
            .unwrap()
    }

    fn entry_dir(&self, name: &str) -> PathBuf {
        let entry = self.store().resolve(name).unwrap();
        self.data.path().join("scripts").join(entry.slug.as_str())
    }
}

fn text(output: &std::process::Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr))
}

fn staged_path(output: &str) -> PathBuf {
    output
        .lines()
        .find_map(|line| line.strip_prefix("STAGED_PATH="))
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("real Python child did not report its staged source path:\n{output}"))
}

fn body(output: &str) -> &str {
    output
        .split_once("BODY_BEGIN\n")
        .and_then(|(_, rest)| rest.split_once("BODY_END"))
        .map(|(body, _)| body)
        .unwrap_or_else(|| panic!("real Python child did not report staged source contents:\n{output}"))
}

#[test]
fn test_write_injected_lands_outside_entry_dir() {
    let sandbox = Sandbox::new();
    sandbox.create_entry("outside");
    let entry_dir = sandbox.entry_dir("outside").canonicalize().unwrap();
    let output = sandbox.run("outside", false);
    let combined = text(&output);
    assert_eq!(output.status.code(), Some(0), "{combined}");

    let staged = staged_path(&combined);
    let parent = staged.parent().unwrap().canonicalize().unwrap();
    assert_ne!(parent, entry_dir, "injected plaintext source must not live in the persistent entry directory");
    assert!(
        staged.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.starts_with(".injected-") && name.ends_with(".py")),
        "unexpected injected temp filename: {}",
        staged.display()
    );
    assert!(body(&combined).contains("CITY = 'Kaohsiung'"), "{combined}");
    assert!(!staged.exists(), "staged plaintext copy survived the completed run: {}", staged.display());
}

#[test]
fn test_write_injected_falls_back_to_entry_dir_if_os_temp_unavailable() {
    let sandbox = Sandbox::new();
    sandbox.create_entry("fallback");
    let entry_dir = sandbox.entry_dir("fallback").canonicalize().unwrap();
    let output = sandbox.run("fallback", true);
    let combined = text(&output);
    assert_eq!(output.status.code(), Some(0), "{combined}");

    let staged = staged_path(&combined);
    let parent = staged.parent().unwrap().canonicalize().unwrap();
    assert_eq!(parent, entry_dir, "OS-temp failure must fall back to the entry directory");
    assert!(
        staged.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.starts_with(".injected-") && name.ends_with(".py")),
        "unexpected fallback injected filename: {}",
        staged.display()
    );
    assert!(body(&combined).contains("CITY = 'Kaohsiung'"), "{combined}");
    assert!(!staged.exists(), "fallback staged copy survived the completed run: {}", staged.display());
}
