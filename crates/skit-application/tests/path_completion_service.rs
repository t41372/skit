use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use skit_application::{
    path_completion::{
        DirectoryEntry, DirectoryReadError, DirectoryReadFilter, DirectoryReader,
        PathCompletionContext, PathCompletionKind, PathCompletionProvider, PathCompletionRequest,
        PathCompletionService, PathInputDialect, looks_pathy,
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

#[test]
fn directory_filter_checks_prefix_and_hidden_names_independently() {
    let visible = DirectoryReadFilter::new("no", false);
    assert!(visible.accepts("notes.md"));
    assert!(!visible.accepts("other.md"));
    assert!(!visible.accepts(".notes.md"));

    let hidden = DirectoryReadFilter::new(".no", true);
    assert!(hidden.accepts(".notes.md"));
    assert!(!hidden.accepts("notes.md"));
}

#[test]
fn every_path_marker_activates_only_its_own_dialect() {
    for piece in ["~notes", "{cwd}notes", "dir/notes"] {
        assert!(looks_pathy(piece, PathInputDialect::Posix), "{piece}");
    }
    assert!(looks_pathy(r"dir\notes", PathInputDialect::Windows));
    assert!(!looks_pathy(r"dir\notes", PathInputDialect::Posix));
    assert!(!looks_pathy("notes", PathInputDialect::Posix));
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
fn object_safe_completion_delegates_to_the_reader() {
    let service = PathCompletionService::new(RecordingReader::default());
    let provider: &dyn PathCompletionProvider = &service;
    assert_eq!(
        provider.complete(&shlexy_request("run no")),
        Some("run notes.md".to_owned())
    );
}

#[test]
fn empty_paths_and_ordinary_text_do_not_read_a_directory() {
    for (value, kind) in [
        ("", PathCompletionKind::Path),
        ("notes", PathCompletionKind::Text),
    ] {
        let reader = RecordingReader::default();
        let calls = Arc::clone(&reader.calls);
        let service = PathCompletionService::new(reader);
        let mut request = shlexy_request(value);
        request.kind = kind;
        assert_eq!(service.complete(&request), None, "{value}");
        assert!(calls.lock().unwrap().is_empty(), "{value}");
    }
}

fn shlexy_request(value: &str) -> PathCompletionRequest {
    PathCompletionRequest {
        value: value.to_owned(),
        kind: PathCompletionKind::Path,
        shlexy: true,
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
    }
}

#[test]
fn a_quote_anywhere_in_the_last_word_stops_completion() {
    // In an argument line skit completes only the last word, and only while that word carries no
    // quote of either kind. One quote is enough to stop it: skit cannot tell where the word ends,
    // so it must not read a directory or offer anything.
    for value in ["run 'note", "run \"note", "run 'note\""] {
        let reader = RecordingReader::default();
        let calls = Arc::clone(&reader.calls);
        let service = PathCompletionService::new(reader);

        assert_eq!(service.complete(&shlexy_request(value)), None, "{value}");
        assert!(calls.lock().unwrap().is_empty(), "{value}");
    }

    // Without a quote the last word is the one completed, not the whole line.
    let reader = RecordingReader::default();
    let calls = Arc::clone(&reader.calls);
    let service = PathCompletionService::new(reader);
    let _ = service.complete(&shlexy_request("run no"));
    assert_eq!(
        *calls.lock().unwrap(),
        [(
            PathBuf::from("/work"),
            2_000,
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
