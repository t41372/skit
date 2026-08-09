use std::sync::Mutex;

use skit_application::{
    Diagnostic, DiagnosticCode, EntryRepository, ExitClass, LibraryScan, LibraryService,
    RepositoryError, RepositoryOperation,
};
use skit_domain::{Entry, EntryKind, EntryMeta, EntrySummary, Slug, StorageMode};
use skit_i18n::{Locale, Message};

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

#[derive(Debug)]
struct FailingRepository {
    error: RepositoryError,
}

impl EntryRepository for FailingRepository {
    fn scan(&self) -> Result<LibraryScan, RepositoryError> {
        Err(self.error.clone())
    }

    fn resolve(&self, _query: &str) -> Result<Entry, RepositoryError> {
        Err(self.error.clone())
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
fn diagnostic_keeps_stable_english_json_and_localizes_human_output() {
    let diagnostic = Diagnostic::from_message(
        DiagnosticCode::InvalidSlug,
        Some("坏-slug".to_owned()),
        Message::new("invalid entry slug: {}").with("坏-slug"),
    );

    assert_eq!(
        serde_json::to_value(&diagnostic).unwrap(),
        serde_json::json!({
            "code": "invalid_slug",
            "slug": "坏-slug",
            "message": "invalid entry slug: 坏-slug",
        })
    );
    assert_eq!(diagnostic.localize(Locale::ZhCn), "无效的条目短名：坏-slug");
    assert_eq!(diagnostic.localize(Locale::ZhTw), "無效的項目短名：坏-slug");
}

#[test]
fn list_is_deterministic_and_keeps_diagnostics() {
    let repository = FakeRepository {
        scan: LibraryScan {
            entries: vec![
                summary("zulu", "zulu"),
                summary("beta", "Alpha"),
                summary("alpha", "Alpha"),
                summary("gamma", "Aardvark"),
            ],
            diagnostics: vec![
                Diagnostic::plain(
                    DiagnosticCode::CorruptMetadata,
                    Some("zulu".to_owned()),
                    "later".to_owned(),
                ),
                Diagnostic::plain(DiagnosticCode::Io, None, "global".to_owned()),
                Diagnostic::plain(
                    DiagnosticCode::InvalidSlug,
                    Some("alpha".to_owned()),
                    "first".to_owned(),
                ),
            ],
        },
        resolved: entry(),
        queries: Mutex::new(Vec::new()),
    };
    let service = LibraryService::new(repository);

    let scan = service.list().unwrap();

    assert_eq!(
        scan.entries
            .iter()
            .map(|entry| entry.slug.as_str())
            .collect::<Vec<_>>(),
        ["alpha", "beta", "gamma", "zulu"]
    );
    assert_eq!(
        scan.diagnostics
            .iter()
            .map(|diagnostic| diagnostic.slug.as_deref())
            .collect::<Vec<_>>(),
        [None, Some("alpha"), Some("zulu")]
    );
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
    assert_eq!(
        service.repository().queries.lock().unwrap().as_slice(),
        ["Alpha"]
    );
}

#[test]
fn repository_failures_keep_the_cli_exit_and_display_contracts() {
    let errors = [
        (
            RepositoryError::NotFound {
                query: "missing".to_owned(),
            },
            ExitClass::NotFound,
            "entry not found: missing",
        ),
        (
            RepositoryError::Ambiguous {
                query: "same".to_owned(),
                candidates: vec!["a".to_owned(), "b".to_owned()],
            },
            ExitClass::Usage,
            "entry name \"same\" is ambiguous",
        ),
        (
            RepositoryError::Conflict {
                name: "Taken".to_owned(),
                slug: "taken".to_owned(),
            },
            ExitClass::Usage,
            "entry \"Taken\" already exists at slug \"taken\"",
        ),
        (
            RepositoryError::InvalidMutation {
                reason: Message::new("reference entries cannot be edited as copies"),
            },
            ExitClass::Usage,
            "invalid entry mutation: reference entries cannot be edited as copies",
        ),
        (
            RepositoryError::StaleEntry {
                slug: "reused".to_owned(),
            },
            ExitClass::Skit,
            "entry \"reused\" changed while this operation was underway",
        ),
        (
            RepositoryError::SourceChanged {
                slug: "edited".to_owned(),
                expected: "sha256:old".to_owned(),
                actual: "sha256:new".to_owned(),
            },
            ExitClass::Skit,
            "entry \"edited\" source changed while this edit was underway (expected sha256:old, found sha256:new)",
        ),
        (
            RepositoryError::Corrupt {
                slug: "bad".to_owned(),
                reason: "bad TOML".to_owned(),
            },
            ExitClass::Skit,
            "entry \"bad\" has corrupt metadata",
        ),
        (
            RepositoryError::Io {
                operation: "read",
                path: "/tmp/meta.toml".to_owned(),
                reason: "denied".to_owned(),
            },
            ExitClass::Skit,
            "could not read /tmp/meta.toml: denied",
        ),
    ];

    for (error, class, message) in errors {
        assert_eq!(error.exit_class(RepositoryOperation::Launch), class);
        assert_eq!(
            error.exit_class(RepositoryOperation::Manage),
            if matches!(error, RepositoryError::InvalidMutation { .. }) {
                ExitClass::Usage
            } else {
                ExitClass::Failure
            }
        );
        assert!(error.to_string().contains(message));
    }
}

#[test]
fn all_non_child_exit_codes_are_stable() {
    assert_eq!(ExitClass::Failure.code(), 1);
    assert_eq!(ExitClass::Usage.code(), 2);
    assert_eq!(ExitClass::Skit.code(), 125);
    assert_eq!(ExitClass::NotExecutable.code(), 126);
    assert_eq!(ExitClass::NotFound.code(), 127);
    assert_eq!(ExitClass::Aborted.code(), 130);
}

#[test]
fn repository_errors_are_propagated_without_frontend_guessing() {
    let error = RepositoryError::Io {
        operation: "scan",
        path: "/library".to_owned(),
        reason: "offline".to_owned(),
    };
    let service = LibraryService::new(FailingRepository {
        error: error.clone(),
    });

    assert_eq!(service.list().unwrap_err(), error);
    assert_eq!(service.show("anything").unwrap_err(), error);
}

#[test]
fn diagnostic_codes_keep_stable_machine_spelling() {
    assert_eq!(
        serde_json::to_string(&DiagnosticCode::InvalidSlug).unwrap(),
        "\"invalid_slug\""
    );
    assert_eq!(
        serde_json::from_str::<DiagnosticCode>("\"corrupt_metadata\"").unwrap(),
        DiagnosticCode::CorruptMetadata
    );
}
