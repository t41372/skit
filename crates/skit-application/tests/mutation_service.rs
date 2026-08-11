use std::sync::Mutex;

use skit_application::{
    CreateEntry, EntryMutationRepository, EntryPayload, EntryRepository, LibraryScan,
    LibraryService, RepositoryError, SourcePermissions, UpdateEntry,
};
use skit_domain::{
    Entry, EntryKind, EntryMeta, EntrySettings, Slug, StorageMode,
    parameters::{ParamDecl, ParameterBinding, ParameterDelivery, ParameterType},
};

#[derive(Debug)]
struct RecordingRepository {
    entry: Entry,
    calls: Mutex<Vec<String>>,
}

impl EntryRepository for RecordingRepository {
    fn scan(&self) -> Result<LibraryScan, RepositoryError> {
        Ok(LibraryScan::default())
    }

    fn resolve(&self, _query: &str) -> Result<Entry, RepositoryError> {
        Ok(self.entry.clone())
    }
}

impl EntryMutationRepository for RecordingRepository {
    fn create(&self, request: CreateEntry) -> Result<Entry, RepositoryError> {
        self.calls.lock().unwrap().push(format!(
            "create:{}:{}:{}",
            request.name, request.settings.template, request.settings.interpolate
        ));
        Ok(self.entry.clone())
    }

    fn claim_identity(&self, entry: &Entry) -> Result<Entry, RepositoryError> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("claim:{}", entry.slug));
        Ok(self.entry.clone())
    }

    fn describe(&self, entry: &Entry, description: &str) -> Result<Entry, RepositoryError> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("describe:{}:{description}", entry.slug));
        Ok(self.entry.clone())
    }

    fn update_settings(
        &self,
        entry: &Entry,
        _settings: &EntrySettings,
        workdir: &str,
    ) -> Result<Entry, RepositoryError> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("settings:{}:{workdir}", entry.slug));
        Ok(self.entry.clone())
    }

    fn update_entry(&self, entry: &Entry, update: UpdateEntry) -> Result<Entry, RepositoryError> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("update:{}:{}", entry.slug, update.name));
        Ok(self.entry.clone())
    }

    fn rename(&self, entry: &Entry, name: &str) -> Result<Entry, RepositoryError> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("rename:{}:{name}", entry.slug));
        Ok(self.entry.clone())
    }

    fn remove(&self, entry: &Entry) -> Result<String, RepositoryError> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("remove:{}", entry.slug));
        Ok(entry.meta.name.clone())
    }

    fn commit_copy_edit(
        &self,
        entry: &Entry,
        bytes: &[u8],
        expected_source_hash: &str,
    ) -> Result<Entry, RepositoryError> {
        self.calls.lock().unwrap().push(format!(
            "edit:{}:{}:{expected_source_hash}",
            entry.slug,
            String::from_utf8_lossy(bytes)
        ));
        Ok(self.entry.clone())
    }
}

fn entry() -> Entry {
    Entry {
        slug: Slug::parse("alpha").unwrap(),
        meta: EntryMeta::minimal("Alpha", EntryKind::parse("command").unwrap()),
    }
}

#[test]
fn mutation_use_cases_delegate_every_value_to_the_port() {
    let expected = entry();
    let service = LibraryService::new(RecordingRepository {
        entry: expected.clone(),
        calls: Mutex::new(Vec::new()),
    });
    let request = CreateEntry {
        name: "Created".to_owned(),
        kind: EntryKind::parse("python").unwrap(),
        mode: StorageMode::Copy,
        source: "/tmp/created.py".to_owned(),
        workdir: "invoke".to_owned(),
        description: "new".to_owned(),
        payload: Some(EntryPayload {
            bytes: b"print('created')\n".to_vec(),
            stored_name: Some("script.py".to_owned()),
            permissions: SourcePermissions::default(),
        }),
        settings: EntrySettings {
            template: "run {value}".to_owned(),
            interpolate: false,
            ..EntrySettings::default()
        },
    };

    assert_eq!(service.add(request).unwrap(), expected);
    assert_eq!(service.claim_identity(&expected).unwrap(), expected);
    assert_eq!(service.describe(&expected, "described").unwrap(), expected);
    let command_settings = EntrySettings {
        template: "true".to_owned(),
        ..EntrySettings::default()
    };
    assert_eq!(
        service
            .update_settings(&expected, &command_settings, "store")
            .unwrap(),
        expected
    );
    assert_eq!(
        service
            .update_entry(
                &expected,
                UpdateEntry {
                    name: "Updated".to_owned(),
                    description: "complete".to_owned(),
                    settings: command_settings,
                    workdir: "invoke".to_owned(),
                    source: None,
                    expected_source_hash: String::new(),
                },
            )
            .unwrap(),
        expected
    );
    assert_eq!(service.rename(&expected, "Renamed").unwrap(), expected);
    assert_eq!(service.remove(&expected).unwrap(), "Alpha");
    assert_eq!(
        service
            .commit_copy_edit(&expected, b"edited", "sha256:base")
            .unwrap(),
        expected
    );
    assert_eq!(
        service.repository().calls.lock().unwrap().as_slice(),
        [
            "create:Created:run {value}:false",
            "claim:alpha",
            "describe:alpha:described",
            "settings:alpha:store",
            "update:alpha:Updated",
            "rename:alpha:Renamed",
            "remove:alpha",
            "edit:alpha:edited:sha256:base",
        ]
    );
}

#[test]
fn settings_policy_refuses_invalid_workdirs_and_parameter_invariants_before_the_port() {
    let expected = entry();
    let service = LibraryService::new(RecordingRepository {
        entry: expected.clone(),
        calls: Mutex::new(Vec::new()),
    });
    let base = EntrySettings {
        template: "true".to_owned(),
        ..EntrySettings::default()
    };

    for workdir in ["relative/path", ""] {
        assert!(matches!(
            service.update_settings(&expected, &base, workdir),
            Err(RepositoryError::InvalidMutation { .. })
        ));
    }

    let mut mismatch = ParamDecl::new("name");
    mismatch.binding = ParameterBinding::Const;
    mismatch.delivery = ParameterDelivery::Flag;
    let mut settings = base.clone();
    settings.parameters = vec![mismatch];
    assert!(matches!(
        service.update_settings(&expected, &settings, "invoke"),
        Err(RepositoryError::InvalidMutation { .. })
    ));

    let mut empty_choice = ParamDecl::new("mode");
    empty_choice.parameter_type = ParameterType::Choice;
    settings.parameters = vec![empty_choice];
    assert!(matches!(
        service.update_settings(&expected, &settings, "invoke"),
        Err(RepositoryError::InvalidMutation { .. })
    ));
    assert!(service.repository().calls.lock().unwrap().is_empty());
}
