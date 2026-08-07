use std::sync::Mutex;

use skit_application::{
    CreateEntry, EntryMutationRepository, EntryPayload, EntryRepository, LibraryScan,
    LibraryService, RepositoryError, SourcePermissions,
};
use skit_domain::{Entry, EntryKind, EntryMeta, Slug, StorageMode};

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
        self.calls
            .lock()
            .unwrap()
            .push(format!("create:{}", request.name));
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
    };

    assert_eq!(service.add(request).unwrap(), expected);
    assert_eq!(service.claim_identity(&expected).unwrap(), expected);
    assert_eq!(service.describe(&expected, "described").unwrap(), expected);
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
            "create:Created",
            "claim:alpha",
            "describe:alpha:described",
            "rename:alpha:Renamed",
            "remove:alpha",
            "edit:alpha:edited:sha256:base",
        ]
    );
}
