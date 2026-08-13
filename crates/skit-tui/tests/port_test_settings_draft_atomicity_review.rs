//! Public Add-review ports from Python `tests/test_settings_and_draft_review_atomicity.py` at
//! `main@206f9ef`. These tests use the real typed review constructor and Ratatui Add renderer; no
//! test-only fresh flag or dependency suggestion logic is reimplemented here.

use std::{fs, path::PathBuf};

use ratatui_core::{backend::TestBackend, buffer::Buffer, terminal::Terminal};
use skit_application::SourcePermissions;
use skit_domain::StorageMode;
use skit_i18n::Locale;
use skit_tui::{AddControlId, AddScreenGeometry, AddScreenSession, render_add};
use skit_ui::{
    AddWorkflowState, KnownEntryKind, ReviewDefaults, ReviewState, SourceSnapshot,
};
use tempfile::TempDir;

fn snapshot(path: impl Into<PathBuf>, bytes: &[u8], is_draft: bool) -> SourceSnapshot {
    let path = path.into();
    SourceSnapshot {
        source_record: path.display().to_string(),
        path,
        bytes: bytes.to_vec(),
        permissions: SourcePermissions::default(),
        is_regular: true,
        is_directory: false,
        is_draft,
    }
}

fn review(path: impl Into<PathBuf>, bytes: &[u8], is_draft: bool, kind: KnownEntryKind) -> ReviewState {
    ReviewState::from_source(snapshot(path, bytes, is_draft), kind, ReviewDefaults::default())
}

fn render(review: ReviewState) -> (String, AddScreenGeometry) {
    let workflow = AddWorkflowState::from_review(review);
    let mut session = AddScreenSession::default();
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    let mut geometry = AddScreenGeometry::default();
    terminal
        .draw(|frame| {
            geometry = render_add(frame, frame.area(), &workflow, &mut session, Locale::En);
        })
        .unwrap();
    (rendered(terminal.backend().buffer()), geometry)
}

fn rendered(buffer: &Buffer) -> String {
    (0..buffer.area.height)
        .map(|row| {
            (0..buffer.area.width)
                .map(|column| buffer[(column, row)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn has_hit(geometry: &AddScreenGeometry, target: &AddControlId) -> bool {
    geometry.hits.iter().any(|hit| &hit.target == target)
}

#[test]
fn test_add_panel_on_a_kept_draft_hides_storage_and_copies() {
    let mut review = review(
        "/tmp/skit-new-resume.py",
        b"print('resumed')\n",
        true,
        KnownEntryKind::Python,
    );
    assert!(review.is_fresh(), "draft identity did not derive fresh review behavior");
    review.set_name("resumed");
    let (text, geometry) = render(review.clone());
    assert!(
        !has_hit(&geometry, &AddControlId::Storage),
        "a kept draft exposed the copy/reference Storage control:\n{text}"
    );
    assert!(!text.contains("Storage mode"), "{text}");
    let entry = review.create_entry().unwrap();
    assert_eq!(entry.mode, StorageMode::Copy);
}

#[test]
fn test_prompt_panel_on_a_kept_draft_hides_storage_and_copies() {
    let mut review = review(
        "/tmp/skit-new-ask.prompt.md",
        b"Summarize {{text}}.\n",
        true,
        KnownEntryKind::Prompt,
    );
    assert!(review.is_fresh(), "prompt draft did not derive fresh review behavior");
    review.set_name("asker");
    let (text, geometry) = render(review.clone());
    assert!(
        !has_hit(&geometry, &AddControlId::Storage),
        "a kept prompt draft exposed the reference choice:\n{text}"
    );
    let entry = review.create_entry().unwrap();
    assert_eq!(entry.kind.as_str(), "prompt");
    assert_eq!(entry.mode, StorageMode::Copy);
}

#[test]
fn test_add_panel_on_a_nondraft_still_shows_storage() {
    let review = review(
        "/tmp/ondisk.py",
        b"print('ondisk')\n",
        false,
        KnownEntryKind::Python,
    );
    assert!(!review.is_fresh(), "ordinary source was misclassified as a kept draft");
    let (text, geometry) = render(review);
    assert!(
        has_hit(&geometry, &AddControlId::Storage),
        "ordinary source lost its copy/reference choice:\n{text}"
    );
    assert!(text.contains("Storage mode"), "{text}");
}

#[test]
fn test_add_panel_prefill_drops_a_pep508_illegal_import() {
    let review = review(
        "/tmp/mixed.py",
        "import café\nimport requests\nprint(café, requests)\n".as_bytes(),
        false,
        KnownEntryKind::Python,
    );
    let prefill = review.dependencies_text();
    assert!(!prefill.contains("café"), "illegal distribution name leaked into prefill: {prefill}");
    assert!(prefill.contains("requests"), "legal third-party import was lost: {prefill}");
}

#[test]
fn test_add_panel_prefill_drops_a_sibling_local_module() {
    let root = TempDir::new().unwrap();
    fs::write(root.path().join("helpers.py"), "def go():\n    return 1\n").unwrap();
    let source = root.path().join("uses_sib.py");
    let bytes = b"import helpers\nimport requests\nprint(helpers, requests)\n";
    fs::write(&source, bytes).unwrap();

    let review = review(&source, bytes, false, KnownEntryKind::Python);
    let prefill = review.dependencies_text();
    assert!(
        !prefill.contains("helpers"),
        "sibling local module was suggested as a package dependency: {prefill}"
    );
    assert!(prefill.contains("requests"), "legal third-party import was lost: {prefill}");
}
