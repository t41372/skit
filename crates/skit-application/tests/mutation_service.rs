use std::sync::Mutex;

use skit_application::{
    CreateEntry, EntryMutationRepository, EntryPayload, EntryRepository, ExternalCopyEdit,
    FinalizeExternalCopyEditError, FinalizedExternalCopyEdit, LibraryScan, LibraryService,
    PreparedEntryUpdateError, RepositoryError, SourcePermissions, UpdateEntry,
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

#[derive(Debug)]
struct RecordedExternalEdit {
    entry: Entry,
    path: std::path::PathBuf,
}

impl ExternalCopyEdit for RecordedExternalEdit {
    fn entry(&self) -> &Entry {
        &self.entry
    }

    fn path(&self) -> &std::path::Path {
        &self.path
    }
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
    type ExternalEdit = RecordedExternalEdit;

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

    fn preflight_update_entry(&self, entry: &Entry, name: &str) -> Result<Entry, RepositoryError> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("preflight:{}:{name}", entry.slug));
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

    fn prepare_external_copy_edit(
        &self,
        entry: &Entry,
    ) -> Result<Self::ExternalEdit, RepositoryError> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("prepare-external-edit:{}", entry.slug));
        Ok(RecordedExternalEdit {
            entry: self.entry.clone(),
            path: "/tmp/script.py".into(),
        })
    }

    fn finalize_external_copy_edit(
        &self,
        edit: &Self::ExternalEdit,
    ) -> Result<FinalizedExternalCopyEdit, FinalizeExternalCopyEditError> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("finalize-external-edit:{}", edit.entry().slug));
        Ok(FinalizedExternalCopyEdit::new(
            self.entry.clone(),
            b"edited externally".to_vec(),
        ))
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
    let external = service.prepare_external_copy_edit(&expected).unwrap();
    assert_eq!(external.entry(), &expected);
    assert_eq!(external.path(), std::path::Path::new("/tmp/script.py"));
    let finalized = service.finalize_external_copy_edit(&external).unwrap();
    assert_eq!(finalized.entry(), &expected);
    assert_eq!(finalized.bytes(), b"edited externally");
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
            "prepare-external-edit:alpha",
            "finalize-external-edit:alpha",
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

#[test]
fn prepared_updates_claim_before_preparation_and_never_write_after_a_preparation_failure() {
    let expected = entry();
    let service = LibraryService::new(RecordingRepository {
        entry: expected.clone(),
        calls: Mutex::new(Vec::new()),
    });
    let update = UpdateEntry {
        name: expected.meta.name.clone(),
        description: "changed".to_owned(),
        settings: EntrySettings::default(),
        workdir: "invoke".to_owned(),
        source: None,
        expected_source_hash: String::new(),
    };

    let error = service
        .update_entry_after_preparation(&expected, update.clone(), |_| Err("cleanup refused"))
        .unwrap_err();

    assert_eq!(
        error,
        PreparedEntryUpdateError::Preparation("cleanup refused")
    );
    assert_eq!(
        service.repository().calls.lock().unwrap().as_slice(),
        ["preflight:alpha:Alpha"]
    );

    service.repository().calls.lock().unwrap().clear();
    let mut prepared_slug = None;
    assert_eq!(
        service
            .update_entry_after_preparation(&expected, update, |claimed| {
                prepared_slug = Some(claimed.slug.clone());
                Ok::<_, &str>(())
            })
            .unwrap(),
        expected
    );
    assert_eq!(prepared_slug.as_ref(), Some(&expected.slug));
    assert_eq!(
        service.repository().calls.lock().unwrap().as_slice(),
        ["preflight:alpha:Alpha", "update:alpha:Alpha"]
    );
}
