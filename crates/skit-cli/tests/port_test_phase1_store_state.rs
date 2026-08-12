//! Store/state ports from Python `tests/test_phase1.py` at `main@206f9ef`.
//! Python's `store.add_python` convenience seam is represented by Rust's public add-review use case
//! followed by `LibraryService<FileStore>::add`, so metadata ownership is tested without injecting
//! unrelated CLI-parser policy.

use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    process::{Command, Output},
};

use skit_application::{
    CreateEntry, EntryMutationRepository as _, EntryRepository as _, LibraryService,
    SourcePermissions,
    form_state::FormStateService,
};
use skit_domain::{
    EntryKind, EntrySettings, Slug, StorageMode,
    parameters::ParamDecl,
};
use skit_language::read_uv_metadata;
use skit_store::{FileFormStateStore, FileStore};
use skit_ui::{KnownEntryKind, ReviewDefaults, ReviewState, SourceSnapshot};
use tempfile::TempDir;

struct Fixture {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    home: TempDir,
}

impl Fixture {
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

    fn meta_path(&self, slug: &Slug) -> PathBuf {
        self.data
            .path()
            .join("scripts")
            .join(slug.as_str())
            .join("meta.toml")
    }
}

fn snapshot(path: PathBuf, bytes: &[u8]) -> SourceSnapshot {
    SourceSnapshot {
        source_record: path.display().to_string(),
        path,
        bytes: bytes.to_vec(),
        permissions: SourcePermissions::default(),
        is_regular: true,
        is_directory: false,
        is_draft: false,
    }
}

fn combined(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn test_add_python_copy_injects_pep723() {
    let fixture = Fixture::new();
    let original = b"import requests\nprint('ok')\n";
    let source = fixture.home.path().join("s.py");
    fs::write(&source, original).unwrap();
    let review = ReviewState::from_source(
        snapshot(source.clone(), original),
        KnownEntryKind::Python,
        ReviewDefaults {
            name: Some("copy-pep".to_owned()),
            dependencies: vec!["requests".to_owned()],
            requires_python: Some(">=3.11".to_owned()),
            ..ReviewDefaults::default()
        },
    );
    let request = review.create_entry().unwrap();
    let service = LibraryService::new(fixture.store());
    let entry = service.add(request).unwrap();
    let store = service.repository();
    let stored = fs::read_to_string(store.payload_path(&entry).unwrap()).unwrap();
    let metadata = read_uv_metadata(&stored).unwrap();

    assert_eq!(metadata.dependencies, ["requests"]);
    assert_eq!(metadata.requires_python, ">=3.11");
    assert_eq!(
        fs::read(&source).unwrap(),
        original,
        "copy add touched the user's original"
    );
    let stored_settings = EntrySettings::from_meta(&entry.meta);
    assert!(
        stored_settings.dependencies.is_empty(),
        "copy entry duplicated PEP 723 deps in meta"
    );
    assert!(
        stored_settings.requires_python.is_empty(),
        "copy entry duplicated requires-python in meta"
    );
    let meta_text = fs::read_to_string(fixture.meta_path(&entry.slug)).unwrap();
    assert!(
        !meta_text.contains("requests"),
        "copy meta duplicated package dependency: {meta_text}"
    );
    assert!(
        !meta_text.contains(">=3.11"),
        "copy meta duplicated Python constraint: {meta_text}"
    );
}

#[test]
fn test_add_python_reference_records_in_meta() {
    let fixture = Fixture::new();
    let original = b"print('hi')\n";
    let source = fixture.home.path().join("ref.py");
    fs::write(&source, original).unwrap();
    let review = ReviewState::from_source(
        snapshot(source.clone(), original),
        KnownEntryKind::Python,
        ReviewDefaults {
            name: Some("reference-pep".to_owned()),
            reference: true,
            dependencies: vec!["requests".to_owned()],
            requires_python: Some(">=3.11".to_owned()),
            ..ReviewDefaults::default()
        },
    );
    let request = review.create_entry().unwrap();
    let service = LibraryService::new(fixture.store());
    let entry = service.add(request).unwrap();
    let settings = EntrySettings::from_meta(&entry.meta);

    assert_eq!(entry.meta.mode, StorageMode::Reference);
    assert_eq!(settings.dependencies, ["requests"]);
    assert_eq!(settings.requires_python, ">=3.11");
    assert_eq!(
        fs::read(&source).unwrap(),
        original,
        "reference add touched the original"
    );
    let meta_text = fs::read_to_string(fixture.meta_path(&entry.slug)).unwrap();
    assert!(
        meta_text.contains("requests"),
        "reference meta lost dependency: {meta_text}"
    );
    assert!(
        meta_text.contains(">=3.11"),
        "reference meta lost Python constraint: {meta_text}"
    );
}

#[test]
fn test_add_python_existing_block_not_touched() {
    let fixture = Fixture::new();
    let existing = concat!(
        "# /// script\n",
        "# dependencies = [\"requests\"]\n",
        "# requires-python = \">=3.11\"\n",
        "# ///\n",
        "print('x')\n",
    );
    let source = fixture.home.path().join("existing.py");
    fs::write(&source, existing).unwrap();
    let review = ReviewState::from_source(
        snapshot(source, existing.as_bytes()),
        KnownEntryKind::Python,
        ReviewDefaults {
            name: Some("existing-pep".to_owned()),
            dependencies: vec!["other".to_owned()],
            requires_python: Some(">=3.12".to_owned()),
            ..ReviewDefaults::default()
        },
    );
    let service = LibraryService::new(fixture.store());
    let entry = service.add(review.create_entry().unwrap()).unwrap();
    let store = service.repository();

    assert_eq!(
        fs::read(store.payload_path(&entry).unwrap()).unwrap(),
        existing.as_bytes()
    );
    let settings = EntrySettings::from_meta(&entry.meta);
    assert!(settings.dependencies.is_empty());
    assert!(settings.requires_python.is_empty());
    let effective = read_uv_metadata(existing).unwrap();
    assert_eq!(effective.dependencies, ["requests"]);
    assert_eq!(effective.requires_python, ">=3.11");
}

#[test]
fn test_argstate_roundtrip_and_forget() {
    let state_root = TempDir::new().unwrap();
    let service = FormStateService::new(FileFormStateStore::new(state_root.path()));
    let slug = Slug::parse("demo").unwrap();
    let declaration = ParamDecl::new("x");
    let values = BTreeMap::from([("x".to_owned(), "1".to_owned())]);

    service
        .save_last(
            &slug,
            &[declaration],
            Some(&values),
            Some(vec!["--foo".to_owned()]),
            false,
        )
        .unwrap();
    let loaded = service.load(&slug);
    assert_eq!(loaded.values, values);
    assert_eq!(loaded.extra_args, vec!["--foo".to_owned()]);

    service.forget(&slug).unwrap();
    let empty = service.load(&slug);
    assert!(empty.values.is_empty());
    assert!(empty.extra_args.is_empty());
    assert!(empty.presets.is_empty());
}

#[test]
fn test_remove_clears_argstate() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let entry = store
        .create(CreateEntry {
            name: "bye".to_owned(),
            kind: EntryKind::parse("command").unwrap(),
            mode: StorageMode::Reference,
            source: String::new(),
            workdir: "invoke".to_owned(),
            description: String::new(),
            payload: None,
            settings: EntrySettings {
                template: "echo ok".to_owned(),
                ..EntrySettings::default()
            },
        })
        .unwrap();
    let state = FormStateService::new(FileFormStateStore::new(fixture.state.path()));
    let declaration = ParamDecl::new("x");
    let values = BTreeMap::from([("x".to_owned(), "1".to_owned())]);
    state
        .save_last(
            &entry.slug,
            &[declaration],
            Some(&values),
            Some(vec!["--foo".to_owned()]),
            false,
        )
        .unwrap();
    assert!(!state.load(&entry.slug).values.is_empty());

    let output = fixture
        .command()
        .args(["remove", entry.slug.as_str(), "-y", "--no-input"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", combined(&output));

    let cleared = state.load(&entry.slug);
    assert!(cleared.values.is_empty());
    assert!(cleared.extra_args.is_empty());
    assert!(fixture.store().resolve(entry.slug.as_str()).is_err());
}
