use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use skit_application::{
    path_completion::{
        DirectoryEntry, DirectoryReadError, DirectoryReadFilter, DirectoryReader,
        PathCompletionContext, PathCompletionKind, PathCompletionRequest, PathCompletionService,
        PathInputDialect, looks_pathy,
    },
    tokens::TokenContext,
};

#[derive(Clone, Debug, Default)]
struct RecordingReader {
    calls: Arc<Mutex<Vec<(PathBuf, usize, DirectoryReadFilter)>>>,
}

#[test]
fn windows_drive_roots_use_the_same_separator_activation_as_other_paths() {
    for piece in [r"C:\\", "D:/", r"z:\\work", "Q:/work"] {
        assert!(looks_pathy(piece, PathInputDialect::Windows), "{piece}");
    }
    assert!(!looks_pathy("C:relative", PathInputDialect::Windows));
    assert!(!looks_pathy("C:relative", PathInputDialect::Posix));
}

impl DirectoryReader for RecordingReader {
    fn read_directory(
        &self,
        path: &Path,
        scan_cap: usize,
        filter: &DirectoryReadFilter,
    ) -> Result<Vec<DirectoryEntry>, DirectoryReadError> {
        self.calls
            .lock()
            .unwrap()
            .push((path.to_path_buf(), scan_cap, filter.clone()));
        Ok(vec![DirectoryEntry::file("notes.md")])
    }
}

#[test]
fn cwd_expansion_and_directory_filter_have_one_application_owned_contract() {
    let reader = RecordingReader::default();
    let calls = Arc::clone(&reader.calls);
    let service = PathCompletionService::with_scan_cap(reader, 17);
    let request = PathCompletionRequest {
        value: "{cwd}/no".to_owned(),
        kind: PathCompletionKind::Path,
        shlexy: false,
        placeholder_braces: false,
        dialect: PathInputDialect::Posix,
        context: PathCompletionContext {
            workdir: PathBuf::from("/child-workdir"),
            tokens: TokenContext {
                cwd: "/invoke-authority".to_owned(),
                home: None,
                env: BTreeMap::new(),
                today: "2026-08-21".to_owned(),
                now: "12-00-00".to_owned(),
            },
        },
    };

    assert_eq!(
        service.complete(&request),
        Some("{cwd}/notes.md".to_owned())
    );
    assert_eq!(
        *calls.lock().unwrap(),
        [(
            PathBuf::from("/invoke-authority"),
            17,
            DirectoryReadFilter::new("no", false),
        )]
    );
}

#[test]
fn a_dot_prefix_explicitly_enables_hidden_directory_names() {
    let reader = RecordingReader::default();
    let calls = Arc::clone(&reader.calls);
    let service = PathCompletionService::new(reader);
    let request = PathCompletionRequest {
        value: ".h".to_owned(),
        kind: PathCompletionKind::Path,
        shlexy: false,
        placeholder_braces: false,
        dialect: PathInputDialect::Posix,
        context: PathCompletionContext {
            workdir: PathBuf::from("/work"),
            tokens: TokenContext {
                cwd: "/invoke".to_owned(),
                home: None,
                env: BTreeMap::new(),
                today: "2026-08-21".to_owned(),
                now: "12-00-00".to_owned(),
            },
        },
    };

    let _ = service.complete(&request);

    assert_eq!(
        calls.lock().unwrap()[0].2,
        DirectoryReadFilter::new(".h", true)
    );
}
