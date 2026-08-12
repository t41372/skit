//! Ratatui ports from Python `tests/test_add_review_contracts.py` at `main@206f9ef`.
//!
//! Renderer contracts are tested through `render_add` and real hit geometry, not by inspecting
//! reducer fields alone. Commit contracts use the exact `ReviewState::create_entry` payload that the
//! host would persist. Red parity findings are left for the implementation agent.

use std::path::PathBuf;

use ratatui_core::{backend::TestBackend, buffer::Buffer, layout::Rect, terminal::Terminal};
use ratatui_crossterm::crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use skit_application::SourcePermissions;
use skit_domain::StorageMode;
use skit_i18n::Locale;
use skit_tui::{AddControlId, AddScreenGeometry, AddScreenSession, render_add};
use skit_ui::{
    AddAction, AddEffect, AddStage, AddWorkflowState, DraftSummary, KnownEntryKind, ReviewDefaults,
    ReviewLane, ReviewState, SourceSnapshot,
};

const DYN_SH: &[u8] = b"#!/usr/bin/env bash\nOUTDIR=/tmp\nOPTS=\"n:v\"\nwhile getopts \"$OPTS\" o; do :; done\necho $OUTDIR\n";
const MODELED_SH: &[u8] = b"#!/usr/bin/env bash\nCITY=Taipei\nwhile getopts 'n:v' o; do :; done\necho $CITY\n";
const CONST_PY: &[u8] = b"MESSAGE = 'Hello'\nTIMES = 3\nWIDTH = 40\nprint(MESSAGE)\n";

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

fn draft(path: &str, bytes: &[u8]) -> SourceSnapshot {
    let mut snapshot = source(path, bytes);
    snapshot.is_draft = true;
    snapshot
}

fn state(review: ReviewState) -> AddWorkflowState {
    AddWorkflowState::from_review(review)
}

fn draw(
    state: &AddWorkflowState,
    session: &mut AddScreenSession,
    width: u16,
    height: u16,
) -> (Terminal<TestBackend>, AddScreenGeometry) {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    let mut geometry = AddScreenGeometry::default();
    terminal
        .draw(|frame| {
            let area = frame.area();
            geometry = render_add(frame, area, state, session, Locale::En);
        })
        .unwrap();
    (terminal, geometry)
}

fn text_of(buffer: &Buffer) -> String {
    let area = buffer.area;
    let mut text = String::new();
    for row in area.y..area.y.saturating_add(area.height) {
        for column in area.x..area.x.saturating_add(area.width) {
            text.push_str(buffer[(column, row)].symbol());
        }
        text.push('\n');
    }
    text
}

fn rendered(state: &AddWorkflowState, width: u16, height: u16) -> (String, AddScreenGeometry) {
    let mut session = AddScreenSession::default();
    let (terminal, geometry) = draw(state, &mut session, width, height);
    (text_of(terminal.backend().buffer()), geometry)
}

fn resume(path: &str, bytes: &[u8]) -> AddWorkflowState {
    let mut workflow = AddWorkflowState::new(vec![DraftSummary {
        path: PathBuf::from(path),
        modified: 1,
    }]);
    let _ = workflow.reduce(AddAction::SelectDraft(0));
    let effects = workflow.reduce(AddAction::Continue);
    let [AddEffect::InspectSource { request, .. }] = effects.as_slice() else {
        panic!("resumed draft must ask the host to inspect its source");
    };
    let request = *request;
    let _ = workflow.reduce(AddAction::SourceInspected {
        request,
        result: Ok(draft(path, bytes)),
    });
    workflow
}

fn unknown_kind(path: &str, bytes: &[u8]) -> AddWorkflowState {
    let mut workflow = AddWorkflowState::new(Vec::new());
    let _ = workflow.reduce(AddAction::SetSourcePath(path.to_owned()));
    let effects = workflow.reduce(AddAction::Continue);
    let [AddEffect::InspectSource { request, .. }] = effects.as_slice() else {
        panic!("unknown source must be inspected");
    };
    let request = *request;
    let _ = workflow.reduce(AddAction::SourceInspected {
        request,
        result: Ok(source(path, bytes)),
    });
    workflow
}

fn key(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn hit<'a>(geometry: &'a AddScreenGeometry, target: &AddControlId) -> Option<&'a Rect> {
    geometry
        .hits
        .iter()
        .find(|region| &region.target == target)
        .map(|region| &region.area)
}

#[test]
fn test_high_unmodeled_self_parser_writes_ticked_candidate() {
    let mut review = ReviewState::from_source(
        source("dyn.sh", DYN_SH),
        KnownEntryKind::Shell,
        ReviewDefaults::default(),
    );
    review.set_name("dynh");
    assert_eq!(review.modeled_cli_field_count(), None);
    review.set_candidate_selected("OUTDIR", true);
    let workflow = state(review.clone());
    let (_, geometry) = rendered(&workflow, 120, 38);
    assert!(
        hit(&geometry, &AddControlId::Candidate("OUTDIR".to_owned())).is_some(),
        "unmodeled self-parser did not render the candidate checkbox"
    );

    let entry = review.create_entry().unwrap();
    let stored = String::from_utf8(entry.payload.unwrap().bytes).unwrap();
    assert!(stored.contains("[tool.skit]"));
    assert!(stored.contains("name = \"OUTDIR\""));
}

#[test]
fn test_high_modeled_form_collects_nothing_without_crashing() {
    let mut review = ReviewState::from_source(
        source("mod.sh", MODELED_SH),
        KnownEntryKind::Shell,
        ReviewDefaults::default(),
    );
    review.set_name("modh");
    assert!(review.modeled_cli_field_count().is_some_and(|count| count > 0));
    assert!(review.candidates().is_empty());
    let workflow = state(review.clone());
    let (_, geometry) = rendered(&workflow, 120, 38);
    assert!(
        geometry
            .hits
            .iter()
            .all(|region| !matches!(region.target, AddControlId::Candidate(_)))
    );

    let entry = review.create_entry().unwrap();
    assert_eq!(entry.kind.as_str(), "shell");
}

#[test]
fn test_prompt_draft_with_shebang_body_resumes_into_prompt_review() {
    let workflow = resume(
        "skit-new-p.prompt.md",
        b"#!/usr/bin/env bash\nSummarize {{text}}.\n",
    );

    assert_eq!(workflow.stage(), AddStage::Review);
    let review = workflow.review().expect("draft opened a review");
    assert_eq!(review.lane(), ReviewLane::Prompt);
    let (_, geometry) = rendered(&workflow, 120, 38);
    assert!(hit(&geometry, &AddControlId::Interpolate).is_some());
    assert!(hit(&geometry, &AddControlId::Runner).is_some());
}

#[test]
fn test_reference_note_modeled_keeps_wrap_and_short_line() {
    let mut review = ReviewState::from_source(
        source("mod.sh", MODELED_SH),
        KnownEntryKind::Shell,
        ReviewDefaults::default(),
    );
    review.set_name("modref");
    review.set_storage(StorageMode::Reference);
    let workflow = state(review.clone());
    let (text, _) = rendered(&workflow, 120, 32);

    assert!(text.contains("skit read this script's own arguments"), "{text}");
    assert!(
        text.contains("Link the original: skit never writes to the file."),
        "{text}"
    );
    assert!(!text.contains("parameter setup is skipped"), "{text}");
    assert_eq!(review.create_entry().unwrap().mode, StorageMode::Reference);
}

#[test]
fn test_reference_note_unmodeled_folds_and_keeps_old_line() {
    let mut review = ReviewState::from_source(
        source("dyn.sh", DYN_SH),
        KnownEntryKind::Shell,
        ReviewDefaults::default(),
    );
    review.set_storage(StorageMode::Reference);
    let workflow = state(review);
    let (text, geometry) = rendered(&workflow, 120, 32);

    assert!(text.contains("parameter setup is skipped"), "{text}");
    assert!(
        geometry
            .hits
            .iter()
            .all(|region| !matches!(region.target, AddControlId::Candidate(_))),
        "reference-mode unmodeled candidates remained interactive"
    );
    assert!(!text.contains("OUTDIR"), "folded candidate leaked into the rendered panel: {text}");
}

#[test]
fn test_kind_pick_modal_label_switches_on_shebang() {
    let with_shebang = unknown_kind("foo.xyz", b"#!/usr/bin/env mystery\necho hi\n");
    assert_eq!(with_shebang.stage(), AddStage::Kind);
    let (text, _) = rendered(&with_shebang, 110, 30);
    assert!(
        text.contains("The #! in foo.xyz names no interpreter skit knows. What is it?"),
        "{text}"
    );

    let without_shebang = unknown_kind("foo.xyz", b"opaque bytes\n");
    assert_eq!(without_shebang.stage(), AddStage::Kind);
    let (text, _) = rendered(&without_shebang, 110, 30);
    assert!(
        text.contains("What is foo.xyz? skit can't tell from the name."),
        "{text}"
    );
}

#[test]
fn test_review_names_extra_arguments_field_once() {
    let bytes = b"#!/usr/bin/env bash\nOPTS=\"n:v\"\nwhile getopts \"$OPTS\" o; do :; done\necho \"$@\"\n";
    let review = ReviewState::from_source(
        source("dynargv.sh", bytes),
        KnownEntryKind::Shell,
        ReviewDefaults::default(),
    );
    assert!(review.onboarding().uses_argv);
    assert!(review.onboarding().uses_cli_framework());
    let workflow = state(review);
    let (text, _) = rendered(&workflow, 120, 38);

    assert_eq!(text.matches("extra-arguments field").count(), 1, "{text}");
}

#[test]
fn test_rv_python_typed_constraint_lands_in_stored_copy() {
    let mut review = ReviewState::from_source(
        source("plain.py", b"print(1)\n"),
        KnownEntryKind::Python,
        ReviewDefaults::default(),
    );
    assert_eq!(review.requires_python(), "");
    review.set_requires_python(">=3.10");
    review.set_name("pytyped");

    let entry = review.create_entry().unwrap();
    let stored = String::from_utf8(entry.payload.unwrap().bytes).unwrap();
    assert!(stored.contains("requires-python = \">=3.10\""), "{stored}");
}

#[test]
fn test_rv_python_empty_means_automatic() {
    let mut review = ReviewState::from_source(
        source("plain.py", b"print(1)\n"),
        KnownEntryKind::Python,
        ReviewDefaults::default(),
    );
    review.set_requires_python("");
    review.set_name("pyauto");

    let entry = review.create_entry().unwrap();
    let stored = String::from_utf8(entry.payload.unwrap().bytes).unwrap();
    assert!(!stored.contains("requires-python"), "{stored}");
}

#[test]
fn test_rv_python_typed_value_survives_an_edit_rescan() {
    let mut review = ReviewState::from_source(
        source("v.py", b"#!/usr/bin/env python3.12\nprint(1)\n"),
        KnownEntryKind::Python,
        ReviewDefaults::default(),
    );
    assert_eq!(review.requires_python(), ">=3.12,<3.13");
    review.set_requires_python(">=3.9");

    review.rescan(b"#!/usr/bin/env python3.11\nprint(1)\n".to_vec());

    assert_eq!(review.requires_python(), ">=3.9");
    review.set_name("pypin");
    let entry = review.create_entry().unwrap();
    let stored = String::from_utf8(entry.payload.unwrap().bytes).unwrap();
    assert!(stored.contains("requires-python = \">=3.9\""), "{stored}");
    assert!(!stored.contains(">=3.11,<3.12"), "auto pin overwrote typed value: {stored}");
}

#[test]
fn test_resumed_draft_has_no_storage_section() {
    let workflow = resume("skit-new-fresh.py", b"print('fresh')\n");
    assert_eq!(workflow.stage(), AddStage::Review);
    assert!(workflow.review().unwrap().is_fresh());
    let (text, geometry) = rendered(&workflow, 120, 34);

    assert!(hit(&geometry, &AddControlId::Storage).is_none());
    assert!(
        !text.contains("Keep a copy —") && !text.contains("Link the original —"),
        "fresh draft rendered the Storage selector: {text}"
    );
}

#[test]
fn test_short_terminal_scrolls_focused_candidate_into_view() {
    let review = ReviewState::from_source(
        source("banner.py", CONST_PY),
        KnownEntryKind::Python,
        ReviewDefaults::default(),
    );
    assert!(review.candidates().len() >= 3);
    let workflow = state(review);
    let mut session = AddScreenSession::default();
    let (_, mut geometry) = draw(&workflow, &mut session, 106, 30);
    let candidates = workflow
        .review()
        .unwrap()
        .candidates()
        .iter()
        .map(|candidate| AddControlId::Candidate(candidate.declaration.name.clone()))
        .collect::<Vec<_>>();
    let last = candidates.last().unwrap().clone();

    for _ in 0..32 {
        if session.focused() == Some(&last) {
            break;
        }
        let event = session.handle_event(key(KeyCode::Tab), &workflow, &geometry);
        assert!(event.is_some(), "Tab was ignored before reaching the last candidate");
        let (_, next) = draw(&workflow, &mut session, 106, 30);
        geometry = next;
        if let Some(focused) = session.focused()
            && let Some(area) = hit(&geometry, focused)
        {
            assert!(geometry.body.contains((area.x, area.y).into()));
            let bottom = area.y.saturating_add(area.height.saturating_sub(1));
            assert!(geometry.body.contains((area.x, bottom).into()));
        }
    }

    assert_eq!(session.focused(), Some(&last));
    assert!(hit(&geometry, &last).is_some(), "focused last candidate is outside rendered hits");
    assert!(geometry.first_visible > 0, "focus reached the last candidate without scrolling");
}
