//! Missing-target / marker ports from Python `tests/test_launcher.py` at `main@206f9ef`.
//!
//! Python exposes `target_missing()` plus `missing_marker()`. Rust projects the same fact into the
//! typed Library detail and the Ratatui pane. Each test checks both: the typed path must be exact,
//! and the user-visible marker must be `⚠ missing: PATH`. This is stronger than a source-string or
//! boolean-only substitute.

use std::{
    fs,
    path::{Path, PathBuf},
};

use ratatui_core::{backend::TestBackend, terminal::Terminal};
use skit_i18n::Locale;
use skit_store::{FileStore, library_surface};
use skit_tui::{TuiSession, render_with_session};
use skit_ui::LibraryState;
use tempfile::TempDir;

struct Fixture {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    origin: TempDir,
}

impl Fixture {
    fn new() -> Self {
        Self {
            data: TempDir::new().unwrap(),
            state: TempDir::new().unwrap(),
            config: TempDir::new().unwrap(),
            origin: TempDir::new().unwrap(),
        }
    }

    fn entry_dir(&self, slug: &str) -> PathBuf {
        self.data.path().join("scripts").join(slug)
    }

    fn write_entry(
        &self,
        slug: &str,
        name: &str,
        kind: &str,
        mode: &str,
        source: &Path,
        payload: Option<(&str, &[u8])>,
    ) {
        let directory = self.entry_dir(slug);
        fs::create_dir_all(&directory).unwrap();
        if let Some((filename, bytes)) = payload {
            fs::write(directory.join(filename), bytes).unwrap();
        }
        let template = (kind == "command")
            .then_some("template = \"echo hi\"\n")
            .unwrap_or("");
        fs::write(
            directory.join("meta.toml"),
            format!(
                concat!(
                    "schema = 1\n",
                    "name = {name:?}\n",
                    "kind = {kind:?}\n",
                    "mode = {mode:?}\n",
                    "source = {source:?}\n",
                    "source_hash = \"\"\n",
                    "added_at = \"2026-08-12T00:00:00Z\"\n",
                    "id = \"0123456789abcdef0123456789abcdef\"\n",
                    "workdir = \"invoke\"\n",
                    "description = \"\"\n",
                    "{template}",
                ),
                name = name,
                kind = kind,
                mode = mode,
                source = source.display().to_string(),
                template = template,
            ),
        )
        .unwrap();
        FileStore::new(self.data.path()).rebuild_registry().unwrap();
    }

    fn projection(&self, slug: &str) -> (Option<String>, String) {
        let store = FileStore::new(self.data.path());
        let surface = library_surface(&store, self.state.path(), self.config.path()).unwrap();
        let entry = surface
            .scan
            .entries
            .iter()
            .find(|entry| entry.slug.as_str() == slug)
            .unwrap_or_else(|| panic!("fixture has no {slug} entry"));
        let missing = surface
            .details
            .get(&entry.slug)
            .and_then(|detail| detail.missing_target.clone());

        let mut state = LibraryState::from_library_surface(surface);
        while state
            .selected()
            .is_some_and(|selected| selected.slug.as_str() != slug)
        {
            let before = state.selected().map(|selected| selected.slug.clone());
            state.update(skit_ui::Action::Next);
            assert_ne!(
                state.selected().map(|selected| selected.slug.clone()),
                before,
                "could not select fixture entry {slug}"
            );
        }
        let mut session = TuiSession::default();
        let mut terminal = Terminal::new(TestBackend::new(140, 40)).unwrap();
        terminal
            .draw(|frame| {
                let _ = render_with_session(frame, &state, Locale::En, &mut session);
            })
            .unwrap();
        let width = usize::from(terminal.backend().buffer().area.width);
        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .chunks(width)
            .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n");
        (missing, rendered)
    }
}

fn assert_exact_missing(fixture: &Fixture, slug: &str, expected: &Path) {
    let (missing, rendered) = fixture.projection(slug);
    assert_eq!(
        missing.as_deref(),
        Some(expected.display().to_string().as_str())
    );
    let marker = format!("⚠ missing: {}", expected.display());
    assert!(
        rendered.contains(&marker),
        "typed projection knew the missing target but the Library marker drifted:\n{rendered}"
    );
}

#[test]
fn test_target_missing_false_for_healthy_python_entry() {
    let fixture = Fixture::new();
    let original = fixture.origin.path().join("healthy.py");
    fs::write(&original, "print(1)\n").unwrap();
    fixture.write_entry(
        "healthy",
        "Healthy",
        "python",
        "copy",
        &original,
        Some(("script.py", b"print(1)\n")),
    );

    let (missing, rendered) = fixture.projection("healthy");
    assert_eq!(missing, None);
    assert!(!rendered.contains("⚠ missing:"), "{rendered}");
}

#[test]
fn test_target_missing_true_when_python_copy_deleted() {
    let fixture = Fixture::new();
    let original = fixture.origin.path().join("copy.py");
    fs::write(&original, "print(1)\n").unwrap();
    fixture.write_entry(
        "copy",
        "Copy",
        "python",
        "copy",
        &original,
        Some(("script.py", b"print(1)\n")),
    );
    let expected = fixture.entry_dir("copy").join("script.py");
    fs::remove_file(&expected).unwrap();

    assert_exact_missing(&fixture, "copy", &expected);
}

#[test]
fn test_target_missing_true_when_python_reference_source_deleted() {
    let fixture = Fixture::new();
    let source = fixture.origin.path().join("ref.py");
    fs::write(&source, "print(1)\n").unwrap();
    fixture.write_entry(
        "reference",
        "Reference",
        "python",
        "reference",
        &source,
        None,
    );
    fs::remove_file(&source).unwrap();

    assert_exact_missing(&fixture, "reference", &source);
}

#[test]
fn test_target_missing_true_when_exe_deleted() {
    let fixture = Fixture::new();
    let executable = fixture
        .origin
        .path()
        .join(if cfg!(windows) { "tool.exe" } else { "tool" });
    fs::write(&executable, b"placeholder").unwrap();
    fixture.write_entry(
        "executable",
        "Executable",
        "exe",
        "reference",
        &executable,
        None,
    );
    fs::remove_file(&executable).unwrap();

    assert_exact_missing(&fixture, "executable", &executable);
}

#[test]
fn test_target_missing_never_true_for_command_entries() {
    let fixture = Fixture::new();
    fixture.write_entry(
        "command",
        "Command",
        "command",
        "copy",
        Path::new("/this/path/intentionally/does/not/exist"),
        None,
    );

    let (missing, rendered) = fixture.projection("command");
    assert_eq!(missing, None);
    assert!(!rendered.contains("⚠ missing:"), "{rendered}");
}
