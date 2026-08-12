//! Review-surface ports from Python `tests/test_add_feedback_contracts.py` at `main@206f9ef`.
//!
//! Python used `Prompt.ask`; Rust's equivalent interactive surface is the Add review form. These
//! tests require the same truthful Enter/automatic semantics to be visible on that real renderer,
//! rather than treating a generic "Python constraint" label as sufficient.

use std::path::PathBuf;

use ratatui_core::{backend::TestBackend, buffer::Buffer, terminal::Terminal};
use skit_application::SourcePermissions;
use skit_i18n::Locale;
use skit_tui::{AddScreenGeometry, AddScreenSession, render_add};
use skit_ui::{AddWorkflowState, KnownEntryKind, ReviewDefaults, ReviewState, SourceSnapshot};

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

fn rendered(review: ReviewState) -> String {
    let state = AddWorkflowState::from_review(review);
    let mut session = AddScreenSession::default();
    let mut terminal = Terminal::new(TestBackend::new(126, 36)).unwrap();
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

#[test]
fn test_python_ask_label_names_the_pin_and_enter_records_it() {
    let mut review = ReviewState::from_source(
        source(
            "pinned.py",
            b"#!/usr/bin/env python3.12\nimport requests\nprint(requests)\n",
        ),
        KnownEntryKind::Python,
        ReviewDefaults::default(),
    );
    assert_eq!(review.requires_python(), ">=3.12,<3.13");
    let text = rendered(review.clone());

    assert!(text.contains("Enter accepts the #! pin"), "{text}");
    assert!(!text.contains("leave empty"), "{text}");

    review.set_name("pin-kept");
    let entry = review.create_entry().unwrap();
    let stored = String::from_utf8(entry.payload.unwrap().bytes).unwrap();
    assert!(
        stored.contains("requires-python = \">=3.12,<3.13\""),
        "{stored}"
    );
}

#[test]
fn test_python_ask_dash_records_automatic_even_with_a_pin() {
    let mut review = ReviewState::from_source(
        source("pinned.py", b"#!/usr/bin/env python3.12\nprint(1)\n"),
        KnownEntryKind::Python,
        ReviewDefaults::default(),
    );
    assert_eq!(review.requires_python(), ">=3.12,<3.13");

    review.set_requires_python("-");
    review.set_name("automatic");

    assert_eq!(review.requires_python(), "");
    let entry = review.create_entry().unwrap();
    let stored = String::from_utf8(entry.payload.unwrap().bytes).unwrap();
    assert!(!stored.contains("requires-python"), "{stored}");
}

#[test]
fn test_python_ask_label_is_leave_empty_without_a_pin() {
    let mut review = ReviewState::from_source(
        source(
            "automatic.py",
            b"#!/usr/bin/env python3\nimport requests\nprint(requests)\n",
        ),
        KnownEntryKind::Python,
        ReviewDefaults::default(),
    );
    assert_eq!(review.requires_python(), "");
    let text = rendered(review.clone());

    assert!(text.contains("leave empty for automatic"), "{text}");
    assert!(!text.contains("Enter accepts the #! pin"), "{text}");

    review.set_requires_python("-");
    review.set_name("automatic-no-pin");
    let entry = review.create_entry().unwrap();
    let stored = String::from_utf8(entry.payload.unwrap().bytes).unwrap();
    assert!(!stored.contains("requires-python"), "{stored}");
}

#[test]
fn test_add_hints_suppresses_argv_when_a_framework_was_detected() {
    let review = ReviewState::from_source(
        source(
            "tool.sh",
            b"#!/usr/bin/env bash\nOPTS=\"n:v\"\nwhile getopts \"$OPTS\" o; do :; done\necho \"$@\"\n",
        ),
        KnownEntryKind::Shell,
        ReviewDefaults::default(),
    );
    assert!(review.onboarding().uses_argv);
    assert!(review.onboarding().uses_cli_framework());

    let text = rendered(review);

    assert!(text.contains("couldn't model them statically"), "{text}");
    assert!(
        !text.contains("This script reads command-line arguments;"),
        "generic argv hint duplicated the framework-specific notice: {text}"
    );
}

#[test]
fn test_add_hints_prints_argv_when_no_framework() {
    let review = ReviewState::from_source(
        source("tool.sh", b"#!/usr/bin/env bash\necho \"$@\"\n"),
        KnownEntryKind::Shell,
        ReviewDefaults::default(),
    );
    assert!(review.onboarding().uses_argv);
    assert!(!review.onboarding().uses_cli_framework());

    let text = rendered(review);

    assert!(text.contains("reads command-line arguments"), "{text}");
}
