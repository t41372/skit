//! TUI/review ports from Python `tests/test_prompt_utf8.py` at `main@206f9ef`.

use std::path::PathBuf;

use ratatui_core::{backend::TestBackend, buffer::Buffer, terminal::Terminal};
use skit_application::SourcePermissions;
use skit_i18n::{Locale, Localize as _};
use skit_tui::{AddScreenGeometry, AddScreenSession, render_add};
use skit_ui::{
    AddProblem, AddWorkflowState, KnownEntryKind, ReviewDefaults, ReviewState, SourceSnapshot,
};

fn source(path: &str, bytes: &[u8]) -> SourceSnapshot {
    SourceSnapshot {
        path: PathBuf::from(path),
        source_record: path.to_owned(),
        bytes: bytes.to_vec(),
        permissions: SourcePermissions::default(),
        is_regular: true,
        is_directory: false,
        is_draft: false,
    }
}

fn rendered(review: ReviewState) -> String {
    let state = AddWorkflowState::from_review(review);
    let mut session = AddScreenSession::default();
    let mut terminal = Terminal::new(TestBackend::new(118, 34)).unwrap();
    let mut geometry = AddScreenGeometry::default();
    terminal
        .draw(|frame| {
            let area = frame.area();
            geometry = render_add(frame, area, &state, &mut session, Locale::En);
        })
        .unwrap();
    assert!(geometry.body.height > 0);
    text_of(terminal.backend().buffer())
}

fn text_of(buffer: &Buffer) -> String {
    let mut text = String::new();
    for row in buffer.area.y..buffer.area.y.saturating_add(buffer.area.height) {
        for column in buffer.area.x..buffer.area.x.saturating_add(buffer.area.width) {
            text.push_str(buffer[(column, row)].symbol());
        }
        text.push('\n');
    }
    text
}

#[test]
fn test_stdin_prompt_inprocess_rejects_invalid_utf8_with_real_byte_offset() {
    let mut review = ReviewState::from_source(
        source("stdin", b"bad \xff prompt\n"),
        KnownEntryKind::Prompt,
        ReviewDefaults::default(),
    );
    review.set_name("stdin-bad");

    let error = review.create_entry().unwrap_err();

    assert!(matches!(error, AddProblem::InvalidPromptEncoding { .. }));
    let message = error.message().localize(Locale::En);
    assert!(message.contains("UTF-8"), "{message}");
    assert!(
        message.contains("offset 4") || message.contains("byte 4"),
        "in-process prompt error lost the real byte offset: {message}"
    );
    assert!(!message.contains('\u{fffd}'), "{message}");
}

#[test]
fn test_tui_initial_add_review_rejects_invalid_prompt_without_replacement_character() {
    let mut review = ReviewState::from_source(
        source("bad.prompt.md", b"hello \xff world\n"),
        KnownEntryKind::Prompt,
        ReviewDefaults::default(),
    );
    review.set_name("bad");

    let text = rendered(review);

    assert!(text.contains("isn't valid UTF-8"), "{text}");
    assert!(!text.contains('\u{fffd}'), "TUI replacement-decoded the prompt: {text}");
}

#[test]
fn test_tui_review_rescan_and_settings_reject_invalid_prompt_without_replacement_character() {
    let mut review = ReviewState::from_source(
        source("edit.prompt.md", b"hello {{name}}\n"),
        KnownEntryKind::Prompt,
        ReviewDefaults::default(),
    );
    review.set_name("edit");

    review.rescan(b"hello \xff changed\n".to_vec());
    let text = rendered(review);

    assert!(text.contains("isn't valid UTF-8"), "{text}");
    assert!(!text.contains('\u{fffd}'), "review rescan replacement-decoded the prompt: {text}");
}
