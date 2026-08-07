use std::sync::Mutex;

use skit_application::{
    Diagnostic, DiagnosticCode, EntryRepository, ExitClass, LibraryScan, LibraryService,
    RepositoryError,
};
use skit_domain::{Entry, EntryKind, EntryMeta, EntrySummary, Slug, StorageMode};

#[derive(Debug)]
struct FakeRepository {
    scan: LibraryScan,
    resolved: Entry,
    queries: Mutex<Vec<String>>,
}

impl EntryRepository for FakeRepository {
    fn scan(&self) -> Result<LibraryScan, RepositoryError> {
        Ok(self.scan.clone())
    }

    fn resolve(&self, query: &str) -> Result<Entry, RepositoryError> {
        self.queries.lock().unwrap().push(query.to_owned());
        Ok(self.resolved.clone())
    }
}

fn summary(slug: &str, name: &str) -> EntrySummary {
    EntrySummary {
        slug: Slug::parse(slug).unwrap(),
        name: name.to_owned(),
        kind: EntryKind::parse("command").unwrap(),
        mode: StorageMode::Copy,
        description: String::new(),
        target: None,
    }
}

fn entry() -> Entry {
    Entry {
        slug: Slug::parse("alpha").unwrap(),
        meta: EntryMeta::minimal("Alpha", EntryKind::parse("command").unwrap()),
    }
}

#[test]
fn list_is_deterministic_and_keeps_diagnostics() {
    let diagnostic = Diagnostic {
        code: DiagnosticCode::CorruptMetadata,
        slug: Some("broken".to_owned()),
        message: "bad TOML".to_owned(),
    };
    let repository = FakeRepository {
        scan: LibraryScan {
            entries: vec![summary("zulu", "zulu"), summary("alpha", "Alpha")],
            diagnostics: vec![diagnostic.clone()],
        },
        resolved: entry(),
        queries: Mutex::new(Vec::new()),
    };
    let service = LibraryService::new(repository);

    let scan = service.list().unwrap();

    assert_eq!(scan.entries[0].slug.as_str(), "alpha");
    assert_eq!(scan.entries[1].slug.as_str(), "zulu");
    assert_eq!(scan.diagnostics, vec![diagnostic]);
}

#[test]
fn show_delegates_the_exact_selector_to_the_repository() {
    let repository = FakeRepository {
        scan: LibraryScan::default(),
        resolved: entry(),
        queries: Mutex::new(Vec::new()),
    };
    let service = LibraryService::new(repository);

    let shown = service.show("Alpha").unwrap();

    assert_eq!(shown.slug.as_str(), "alpha");
    assert_eq!(service.repository().queries.lock().unwrap().as_slice(), ["Alpha"]);
}

#[test]
fn repository_failures_keep_the_cli_exit_contract() {
    assert_eq!(
        RepositoryError::NotFound {
            query: "missing".to_owned()
        }
        .exit_class(),
        ExitClass::NotFound
    );
    assert_eq!(
        RepositoryError::Ambiguous {
            query: "same".to_owned(),
            candidates: vec!["a".to_owned(), "b".to_owned()]
        }
        .exit_class(),
        ExitClass::Usage
    );
    assert_eq!(
        RepositoryError::Corrupt {
            slug: "bad".to_owned(),
            reason: "bad TOML".to_owned()
        }
        .exit_class(),
        ExitClass::Skit
    );
}
