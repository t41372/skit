use std::{cell::Cell, collections::BTreeMap, path::PathBuf};

use skit_application::{
    EntryRepository, LibraryScan, RepositoryError,
    form_state::{FormStateRepository, LastRunState, PersistedFormState, StateWriteError},
    library_detail::{
        LibraryDetailRepository, LibraryEntrySnapshot, LibraryFormFacts, LibraryFormProjector,
        LibraryPromptRunner, LibraryRunAge, LibrarySurfaceService, LibraryTargetState,
    },
};
use skit_domain::{
    Entry, EntryKind, EntryMeta, EntrySettings, Slug,
    parameters::{ParamDecl, ParameterValue},
};
use time::OffsetDateTime;

#[derive(Debug)]
struct MemoryLibrary {
    entry: Entry,
    scans: Cell<usize>,
    detail_reads: Cell<usize>,
}

impl EntryRepository for MemoryLibrary {
    fn scan(&self) -> Result<LibraryScan, RepositoryError> {
        self.scans.set(self.scans.get() + 1);
        Ok(LibraryScan::default())
    }

    fn resolve(&self, _query: &str) -> Result<Entry, RepositoryError> {
        Ok(self.entry.clone())
    }
}

impl LibraryDetailRepository for MemoryLibrary {
    fn detail_snapshots(&self) -> Result<Vec<LibraryEntrySnapshot>, RepositoryError> {
        self.detail_reads.set(self.detail_reads.get() + 1);
        Ok(vec![LibraryEntrySnapshot {
            entry: self.entry.clone(),
            source: Some(b"source bytes\r\n".to_vec()),
            target: LibraryTargetState::Missing(PathBuf::from("/missing/demo.prompt")),
            original_source_exists: true,
        }])
    }
}

#[derive(Debug)]
struct MemoryState;

impl FormStateRepository for MemoryState {
    fn load(&self, _slug: &Slug) -> PersistedFormState {
        PersistedFormState {
            values: BTreeMap::from([
                ("name".to_owned(), "Ada".to_owned()),
                ("token".to_owned(), "must-not-surface".to_owned()),
            ]),
            presets: BTreeMap::from([("nightly".to_owned(), BTreeMap::new())]),
            last_run: LastRunState {
                at: Some("1970-01-01T00:00:00Z".to_owned()),
                exit: Some(7),
                values: None,
            },
            ..PersistedFormState::default()
        }
    }

    fn last_run(&self, _slug: &Slug) -> LastRunState {
        panic!("the detail projection needs the complete state snapshot")
    }

    fn update<T, F>(&self, _slug: &Slug, _update: F) -> Result<T, StateWriteError>
    where
        F: FnOnce(&mut PersistedFormState) -> T,
    {
        panic!("the Library read must not update state")
    }

    fn forget(&self, _slug: &Slug) -> Result<(), StateWriteError> {
        panic!("the Library read must not remove state")
    }
}

#[derive(Debug)]
struct FixedForm;

impl LibraryFormProjector for FixedForm {
    fn project(&self, _entry: &Entry, source: Option<&[u8]>) -> LibraryFormFacts {
        assert_eq!(source, Some(b"source bytes\r\n".as_slice()));
        let mut public = ParamDecl::new("name");
        public.default = Some(ParameterValue::String("World".to_owned()));
        let mut secret = ParamDecl::new("token");
        secret.secret = true;
        LibraryFormFacts {
            declarations: vec![public, secret],
            drifted: true,
        }
    }
}

fn prompt_entry() -> Entry {
    let mut meta = EntryMeta::minimal("Demo", EntryKind::parse("prompt").unwrap());
    meta.added_at = "1969-12-31T00:00:00Z".to_owned();
    meta.source = "/original/demo.prompt".to_owned();
    Entry {
        slug: Slug::parse("demo").unwrap(),
        meta,
    }
}

#[test]
fn the_application_service_builds_one_complete_surface_from_ports() {
    let repository = MemoryLibrary {
        entry: prompt_entry(),
        scans: Cell::new(0),
        detail_reads: Cell::new(0),
    };
    let state = MemoryState;
    let form = FixedForm;
    let effective_settings = |entry: &Entry, source: Option<&[u8]>| {
        assert_eq!(source, Some(b"source bytes\r\n".as_slice()));
        let mut settings = EntrySettings::from_meta(&entry.meta);
        settings.runner = "configured".to_owned();
        settings.dependencies = vec!["httpx".to_owned()];
        settings
    };
    let service = LibrarySurfaceService::new(&repository, &state, &form, effective_settings);

    let surface = service
        .load_at(&["configured".to_owned()], OffsetDateTime::UNIX_EPOCH)
        .unwrap();

    assert_eq!(repository.scans.get(), 1);
    assert_eq!(repository.detail_reads.get(), 1);
    let detail = &surface.details[&Slug::parse("demo").unwrap()];
    assert_eq!(detail.parameters[0].key, "name");
    assert_eq!(detail.parameters[0].value, "Ada");
    assert_eq!(detail.parameters[1].key, "token");
    assert_eq!(detail.parameters[1].value, "");
    assert!(detail.parameters[1].secret);
    assert_eq!(detail.presets, ["nightly"]);
    assert_eq!(detail.dependencies, ["httpx"]);
    assert_eq!(
        detail.prompt_runner,
        Some(LibraryPromptRunner::Configured("configured".to_owned()))
    );
    assert_eq!(
        detail.last_run.as_ref().unwrap().age,
        LibraryRunAge::JustNow
    );
    assert_eq!(detail.last_run.as_ref().unwrap().exit, Some(7));
    assert_eq!(
        detail.missing_target.as_deref(),
        Some("/missing/demo.prompt")
    );
    assert!(detail.drifted);
    assert!(detail.original_file_preserved);
}
