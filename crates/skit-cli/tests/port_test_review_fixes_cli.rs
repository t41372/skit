//! Public-process/store-boundary ports from Python `tests/test_review_fixes.py` at `main@206f9ef`.
//!
//! The staged-source test uses a real child probe so filename, permissions, and injected bytes are
//! observed while the temporary file is alive. Dependency tests read the authoritative store after
//! the real CLI transaction and keep the user's reference source byte-exact.

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use skit_application::{CreateEntry, EntryMutationRepository as _, EntryRepository as _, LibraryService};
use skit_domain::{EntryKind, EntrySettings, StorageMode};
use skit_store::FileStore;
use tempfile::TempDir;

struct Fixture {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    home: TempDir,
    tools: TempDir,
}

impl Fixture {
    fn new() -> Self {
        Self {
            data: TempDir::new().unwrap(),
            state: TempDir::new().unwrap(),
            config: TempDir::new().unwrap(),
            home: TempDir::new().unwrap(),
            tools: TempDir::new().unwrap(),
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_skit"));
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

    fn store(&self) -> FileStore {
        FileStore::new(self.data.path())
    }

    fn ok(&self, args: &[&str]) -> Output {
        let output = self.command().args(args).output().unwrap();
        assert_success(&output, args);
        output
    }
}

fn assert_success(output: &Output, args: &[&str]) {
    assert!(
        output.status.success(),
        "args={args:?}\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

#[test]
fn test_update_dependencies_reference_mode() {
    let fixture = Fixture::new();
    let source = fixture.home.path().join("tool.py");
    let original = b"print('hi')\n";
    fs::write(&source, original).unwrap();
    fixture.ok(&[
        "add",
        source.to_str().unwrap(),
        "--name",
        "refdeps",
        "--ref",
        "--no-input",
    ]);

    fixture.ok(&["deps", "refdeps", "--dep", "httpx"]);

    assert_eq!(fs::read(&source).unwrap(), original, "reference dependency edit touched the original");
    let output = fixture.ok(&["deps", "refdeps", "--json"]);
    let document: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(document["dependencies"], serde_json::json!(["httpx"]));
}

#[test]
fn test_update_dependencies_exe_entry() {
    let fixture = Fixture::new();
    let executable = fixture.home.path().join(if cfg!(windows) { "tool.exe" } else { "tool" });
    fs::write(&executable, b"opaque executable fixture").unwrap();
    let store = fixture.store();
    let created = store
        .create(CreateEntry {
            name: "execdeps".to_owned(),
            kind: EntryKind::parse("exe").unwrap(),
            mode: StorageMode::Reference,
            source: executable.display().to_string(),
            workdir: "invoke".to_owned(),
            description: String::new(),
            payload: None,
            settings: EntrySettings::default(),
        })
        .unwrap();
    assert_eq!(created.meta.kind.as_str(), "exe");

    fixture.ok(&["deps", "execdeps", "--dep", "libssl"]);

    let output = fixture.ok(&["deps", "execdeps", "--json"]);
    let document: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(document["dependencies"], serde_json::json!(["libssl"]));
}

fn compile_stage_probe(root: &Path) -> PathBuf {
    let source = root.join("stage_probe.rs");
    fs::write(
        &source,
        r#"
use std::{env, fs, fs::OpenOptions, io::Write as _, path::PathBuf};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;

fn main() {
    let args = env::args_os().skip(1).map(PathBuf::from).collect::<Vec<_>>();
    let strings = args.iter().map(|arg| arg.to_string_lossy().into_owned()).collect::<Vec<_>>();
    let script = strings.windows(2).find_map(|pair| (pair[0] == "--script").then(|| PathBuf::from(&pair[1]))).expect("--script path");
    let metadata = fs::metadata(&script).expect("staged source metadata");
    #[cfg(unix)]
    let mode = metadata.permissions().mode() & 0o777;
    #[cfg(not(unix))]
    let mode = 0_u32;
    let content = fs::read_to_string(&script).expect("staged source text").replace('\n', "\\n");
    let name = script.file_name().unwrap().to_string_lossy();
    let capture = env::var_os("SKIT_STAGE_CAPTURE").expect("capture path");
    let mut file = OpenOptions::new().create(true).append(true).open(capture).unwrap();
    writeln!(file, "{}\t{:o}\t{}", name, mode, content).unwrap();
}
"#,
    )
    .unwrap();
    let output = root.join(if cfg!(windows) { "stage-probe.exe" } else { "stage-probe" });
    let status = Command::new("rustc")
        .arg(source)
        .arg("-o")
        .arg(&output)
        .status()
        .unwrap();
    assert!(status.success(), "failed to compile staged-source child probe");
    output
}

#[test]
fn test_write_injected_unique_and_private() {
    let fixture = Fixture::new();
    let source = fixture.home.path().join("managed.py");
    fs::write(&source, b"VALUE = 1\nprint(VALUE)\n").unwrap();
    fixture.ok(&[
        "add",
        source.to_str().unwrap(),
        "--name",
        "managed",
        "--no-input",
    ]);
    fixture.ok(&["params", "managed", "--manage", "VALUE"]);

    let probe = compile_stage_probe(fixture.tools.path());
    let capture = fixture.tools.path().join("staged.txt");
    let service = LibraryService::new(fixture.store());
    let entry = service.show("managed").unwrap();
    let mut settings = EntrySettings::from_meta(&entry.meta);
    settings.interpreter = probe.display().to_string();
    service
        .update_settings(&entry, &settings, &entry.meta.workdir)
        .unwrap();

    for value in ["2", "3"] {
        let mut command = fixture.command();
        let output = command
            .env("SKIT_STAGE_CAPTURE", &capture)
            .args(["run", "managed", "--set", &format!("VALUE={value}"), "--no-input"])
            .output()
            .unwrap();
        assert_success(&output, &["run", "managed"]);
    }

    let rows = fs::read_to_string(&capture)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    assert_eq!(rows.len(), 2, "child did not observe both staged launches: {rows:?}");
    let parsed = rows
        .iter()
        .map(|row| row.splitn(3, '\t').collect::<Vec<_>>())
        .collect::<Vec<_>>();
    assert!(parsed.iter().all(|row| row.len() == 3), "malformed child capture: {rows:?}");
    let names = parsed.iter().map(|row| row[0]).collect::<Vec<_>>();
    assert_ne!(names[0], names[1], "two launches reused one injected filename");
    for name in &names {
        assert!(name.starts_with(".injected-"), "staged source name lost Python contract: {name}");
        assert!(name.ends_with(".py"), "staged Python source lost .py suffix: {name}");
    }
    #[cfg(unix)]
    for row in &parsed {
        assert_eq!(row[1], "600", "staged source was not private: {row:?}");
    }
    assert!(parsed[0][2].contains("VALUE = 2"), "first injection did not reach staged bytes: {:?}", parsed[0]);
    assert!(parsed[1][2].contains("VALUE = 3"), "second injection did not reach staged bytes: {:?}", parsed[1]);
    assert!(parsed.iter().all(|row| row[2].contains("print(VALUE)")), "unrelated source was lost: {parsed:?}");
}
