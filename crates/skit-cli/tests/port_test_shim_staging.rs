//! Exact staged-copy ports from Python v0.4 `tests/test_shim.py`.
//!
//! The stored Python source reports the path and bytes of the file that Python actually executes.
//! This keeps the owner on the real CLI launch seam and does not depend on an unused inspector.

use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use assert_cmd::Command;
use skit_application::{
    CreateEntry, EntryMutationRepository as _, EntryPayload, EntryRepository as _,
    SourcePermissions, payload_stored_name,
};
use skit_domain::{EntryKind, EntrySettings, StorageMode};
use skit_language::{ParseOutcome, parse_document, write_managed_params};
use skit_runtime::{ProgramProbe as _, SystemProbe};
use skit_store::FileStore;
use tempfile::TempDir;

const SOURCE: &str = concat!(
    "from pathlib import Path\n",
    "_skit_path = Path(__file__).resolve()\n",
    "print('STAGED_PATH=' + str(_skit_path))\n",
    "print('STAGED_MODE=' + oct(_skit_path.stat().st_mode & 0o777))\n",
    "print('BODY_HEX=' + _skit_path.read_bytes().hex())\n",
    "CITY = 'Taipei'\n",
    "print('CITY=' + CITY)\n",
);

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
            .expect("the frozen Shim staging contracts require Python on this platform");
        let sandbox = Self {
            data: TempDir::new().unwrap(),
            state: TempDir::new().unwrap(),
            config: TempDir::new().unwrap(),
            home: TempDir::new().unwrap(),
            python,
        };
        fs::write(
            sandbox.config.path().join("config.toml"),
            "[mirror]\nenabled = false\n",
        )
        .unwrap();
        // A pinned Python program occupies the v0.4 uv-compatible interpreter slot, so the launch
        // is `python run --no-project --script PATH`. This adapter only dispatches that protocol;
        // the stored source itself, not this file, reports and reads the staged source.
        fs::write(
            sandbox.home.path().join("run"),
            concat!(
                "import runpy\n",
                "import sys\n",
                "i = sys.argv.index('--script')\n",
                "script = sys.argv[i + 1]\n",
                "sys.argv = [script, *sys.argv[i + 2:]]\n",
                "runpy.run_path(script, run_name='__main__')\n",
            ),
        )
        .unwrap();
        sandbox
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
            .env_remove("FORCE_COLOR")
            .env_remove("NO_COLOR")
            .current_dir(self.home.path());
        command
    }

    fn create_entry(&self, name: &str) {
        let kind = EntryKind::parse("python").unwrap();
        let ParseOutcome::Parsed(document) = parse_document("python", SOURCE) else {
            panic!("Python staging fixture must parse");
        };
        let declaration = document
            .analysis()
            .candidates
            .into_iter()
            .find(|candidate| candidate.declaration.name == "CITY")
            .expect("CITY must be a managed constant")
            .declaration;
        let managed = write_managed_params("python", SOURCE, &[declaration]).unwrap();
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

    fn run(&self, name: &str, break_os_temp: bool) -> std::process::Output {
        let mut command = self.command();
        if break_os_temp {
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

    fn authoritative_bytes(&self, name: &str) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
        let entry = self.store().resolve(name).unwrap();
        let entry_dir = self.entry_dir(name);
        let payload = self.store().payload_path(&entry).unwrap();
        (
            fs::read(payload).unwrap(),
            fs::read(entry_dir.join("meta.toml")).unwrap(),
            fs::read(self.config.path().join("config.toml")).unwrap(),
        )
    }
}

fn output_text(output: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn reported_path(output: &str) -> PathBuf {
    output
        .lines()
        .find_map(|line| line.strip_prefix("STAGED_PATH="))
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("real Python child did not report its source path:\n{output}"))
}

fn reported_body(output: &str) -> Vec<u8> {
    let encoded = output
        .lines()
        .find_map(|line| line.strip_prefix("BODY_HEX="))
        .unwrap_or_else(|| panic!("real Python child did not report its source bytes:\n{output}"));
    assert_eq!(encoded.len() % 2, 0, "Python emitted malformed hex");
    encoded
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let text = std::str::from_utf8(pair).unwrap();
            u8::from_str_radix(text, 16).unwrap()
        })
        .collect()
}

fn assert_private_mode(output: &str) {
    #[cfg(unix)]
    assert!(output.lines().any(|line| line == "STAGED_MODE=0o600"));
}

fn assert_authoritative_bytes_unchanged(
    sandbox: &Sandbox,
    name: &str,
    before: &(Vec<u8>, Vec<u8>, Vec<u8>),
) {
    assert_eq!(&sandbox.authoritative_bytes(name), before);
}

fn assert_expected_run_state(sandbox: &Sandbox, slug: &str) {
    let state_file = sandbox
        .state
        .path()
        .join("values")
        .join(format!("{slug}.toml"));
    let document = fs::read_to_string(&state_file)
        .unwrap()
        .parse::<toml::Table>()
        .unwrap();
    assert_eq!(
        document.keys().map(String::as_str).collect::<BTreeSet<_>>(),
        BTreeSet::from(["last_run", "values"])
    );
    let values = document["values"].as_table().unwrap();
    assert_eq!(values.len(), 1);
    assert_eq!(values["CITY"].as_str(), Some("Kaohsiung"));
    let last_run = document["last_run"].as_table().unwrap();
    assert_eq!(
        last_run.keys().map(String::as_str).collect::<BTreeSet<_>>(),
        BTreeSet::from(["at", "exit", "values"])
    );
    assert_eq!(last_run["exit"].as_integer(), Some(0));
    time::OffsetDateTime::parse(
        last_run["at"].as_str().unwrap(),
        &time::format_description::well_known::Rfc3339,
    )
    .expect("last-run timestamp must use RFC 3339");
    let snapshot = last_run["values"].as_table().unwrap();
    assert_eq!(snapshot.len(), 1);
    assert_eq!(snapshot["CITY"].as_str(), Some("Kaohsiung"));

    let root_entries = fs::read_dir(sandbox.state.path())
        .unwrap()
        .filter_map(Result::ok)
        .map(|item| item.file_name().to_string_lossy().into_owned())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        root_entries,
        BTreeSet::from([".locks".to_owned(), "values".to_owned()])
    );
    let value_entries = fs::read_dir(sandbox.state.path().join("values"))
        .unwrap()
        .filter_map(Result::ok)
        .map(|item| item.file_name().to_string_lossy().into_owned())
        .collect::<BTreeSet<_>>();
    assert_eq!(value_entries, BTreeSet::from([format!("{slug}.toml")]));
    let lock_entries = fs::read_dir(sandbox.state.path().join(".locks"))
        .unwrap()
        .filter_map(Result::ok)
        .map(|item| item.file_name().to_string_lossy().into_owned())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        lock_entries,
        BTreeSet::from([format!("{slug}.values.lock")])
    );
}

#[test]
fn test_write_injected_lands_outside_entry_dir() {
    let sandbox = Sandbox::new();
    sandbox.create_entry("outside");
    let before = sandbox.authoritative_bytes("outside");
    let entry_dir = sandbox.entry_dir("outside").canonicalize().unwrap();
    let output = sandbox.run("outside", false);
    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(0), "{text}");

    let staged = reported_path(&text);
    assert_ne!(staged.parent().unwrap().canonicalize().unwrap(), entry_dir);
    assert!(
        staged
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(".injected-") && name.ends_with(".py"))
    );
    assert_private_mode(&text);
    let body = String::from_utf8(reported_body(&text)).unwrap();
    assert!(body.contains("CITY = 'Kaohsiung'"), "{body}");
    assert!(text.contains("CITY=Kaohsiung"), "{text}");
    assert!(!staged.exists(), "staged source survived the completed run");
    assert_authoritative_bytes_unchanged(&sandbox, "outside", &before);
    assert_expected_run_state(&sandbox, "outside");
}

#[test]
fn test_write_injected_falls_back_to_entry_dir_if_os_temp_unavailable() {
    let sandbox = Sandbox::new();
    sandbox.create_entry("fallback");
    let before = sandbox.authoritative_bytes("fallback");
    let entry_dir = sandbox.entry_dir("fallback").canonicalize().unwrap();
    let output = sandbox.run("fallback", true);
    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(0), "{text}");

    let staged = reported_path(&text);
    assert_eq!(staged.parent().unwrap().canonicalize().unwrap(), entry_dir);
    assert!(
        staged
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(".injected-") && name.ends_with(".py"))
    );
    assert_private_mode(&text);
    let body = String::from_utf8(reported_body(&text)).unwrap();
    assert!(body.contains("CITY = 'Kaohsiung'"), "{body}");
    assert!(text.contains("CITY=Kaohsiung"), "{text}");
    assert!(!staged.exists(), "fallback staged source survived the run");
    assert!(
        fs::read_dir(&entry_dir)
            .unwrap()
            .filter_map(Result::ok)
            .all(|item| !item.file_name().to_string_lossy().starts_with(".injected-"))
    );
    assert_authoritative_bytes_unchanged(&sandbox, "fallback", &before);
    assert_expected_run_state(&sandbox, "fallback");
}
