//! Exact TUI-surface ports from Python `tests/test_draft_and_reader_tui.py` at `main@206f9ef`.
//!
//! Review contracts render through the real Ratatui Add screen. Keyboard contracts use the complete
//! `TuiSession` event router before dispatching the typed reducer action. Red behavior is a parity
//! finding; these tests do not adapt their expectations to the current Rust key handling.

use std::{fs, path::PathBuf};

use ratatui_core::{backend::TestBackend, buffer::Buffer, layout::Rect, terminal::Terminal};
use ratatui_crossterm::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use skit_application::SourcePermissions;
use skit_i18n::Locale;
use skit_tui::{
    AddControlId, AddScreenGeometry, AddScreenSession, AddTextField, EventHandling, TuiSession,
    ViewGeometry, render_add, render_with_session,
};
use skit_ui::{
    Action, AddAction, AddEffect, AddStage, AddWorkflowState, DraftSummary, Effect, KnownEntryKind,
    LibraryState, ReviewDefaults, ReviewState, Screen, SourceSnapshot,
};
use tempfile::TempDir;

const DYN_SH: &[u8] =
    b"#!/usr/bin/env bash\nOUTDIR=/tmp\nOPTS=\"n:v\"\nwhile getopts \"$OPTS\" o; do :; done\necho $OUTDIR\n";
const MODELED_SH: &[u8] =
    b"#!/usr/bin/env bash\nCITY=Taipei\nwhile getopts 'n:v' o; do :; done\necho $CITY\n";

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

fn review(path: &str, bytes: &[u8], kind: KnownEntryKind) -> ReviewState {
    ReviewState::from_source(snapshot(path, bytes, false), kind, ReviewDefaults::default())
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

fn render_review(workflow: &AddWorkflowState) -> (String, AddScreenGeometry) {
    let mut session = AddScreenSession::default();
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    let mut geometry = AddScreenGeometry::default();
    terminal
        .draw(|frame| {
            geometry = render_add(frame, frame.area(), workflow, &mut session, Locale::En);
        })
        .unwrap();
    (rendered(terminal.backend().buffer()), geometry)
}

fn has_add_hit(geometry: &AddScreenGeometry, target: &AddControlId) -> bool {
    geometry.hits.iter().any(|hit| &hit.target == target)
}

fn region_text(buffer: &Buffer, area: Rect) -> String {
    let mut output = String::new();
    for row in area.y..area.y.saturating_add(area.height).min(buffer.area.height) {
        for column in area.x..area.x.saturating_add(area.width).min(buffer.area.width) {
            output.push_str(buffer[(column, row)].symbol());
        }
    }
    output
}

fn position(buffer: &Buffer, needle: &str) -> (u16, u16) {
    for row in 0..buffer.area.height {
        let line = (0..buffer.area.width)
            .map(|column| buffer[(column, row)].symbol())
            .collect::<String>();
        if let Some(column) = line.find(needle) {
            return (u16::try_from(column).unwrap(), row);
        }
    }
    panic!("missing rendered text {needle:?}:\n{}", rendered(buffer));
}

fn key(code: KeyCode, modifiers: KeyModifiers) -> Event {
    Event::Key(KeyEvent::new(code, modifiers))
}

fn mouse(column: u16, row: u16) -> Event {
    Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column,
        row,
        modifiers: KeyModifiers::NONE,
    })
}

fn draw_full(
    session: &mut TuiSession,
    state: &LibraryState,
) -> (Terminal<TestBackend>, ViewGeometry) {
    let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
    let mut geometry = ViewGeometry::default();
    terminal
        .draw(|frame| {
            geometry = render_with_session(frame, state, Locale::En, session);
        })
        .unwrap();
    (terminal, geometry)
}

fn drive(
    session: &mut TuiSession,
    state: &mut LibraryState,
    geometry: &ViewGeometry,
    event: Event,
) -> (EventHandling, Effect) {
    let handling = session.handle_event(event, state, geometry);
    let effect = match &handling {
        EventHandling::Action(action) => state.update(action.clone()),
        EventHandling::Consumed | EventHandling::Ignored => Effect::None,
    };
    (handling, effect)
}

fn draft_state(paths: &[(PathBuf, u64)]) -> LibraryState {
    let drafts = paths
        .iter()
        .map(|(path, modified)| DraftSummary {
            path: path.clone(),
            modified: *modified,
        })
        .collect();
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Add(Box::new(AddWorkflowState::new(
        drafts,
    )))));
    state
}

fn select_draft_by_click(session: &mut TuiSession, state: &mut LibraryState, filename: &str) {
    let (terminal, geometry) = draw_full(session, state);
    let (x, y) = position(terminal.backend().buffer(), filename);
    let (handling, effect) = drive(session, state, &geometry, mouse(x, y));
    assert!(
        matches!(
            &handling,
            EventHandling::Action(Action::Add(AddAction::SelectDraft(_)))
        ),
        "clicking the draft did not select it: {handling:?}"
    );
    assert_eq!(effect, Effect::None);
    assert_eq!(
        state
            .add_workflow()
            .and_then(|workflow| workflow.source().selected_draft())
            .and_then(|draft| draft.path.file_name())
            .and_then(|name| name.to_str()),
        Some(filename)
    );
}

#[test]
fn test_review_versioned_shebang_shows_and_stores_pin() {
    let mut review = review(
        "v.py",
        b"#!/usr/bin/env python3.12\nprint('hi')\n",
        KnownEntryKind::Python,
    );
    assert_eq!(review.requires_python(), ">=3.12,<3.13");
    let workflow = AddWorkflowState::from_review(review.clone());
    let (text, geometry) = render_review(&workflow);
    assert!(text.contains(">=3.12,<3.13"), "derived pin is not visible:\n{text}");
    assert!(
        has_add_hit(
            &geometry,
            &AddControlId::Text(AddTextField::PythonConstraint)
        ),
        "derived pin is not in an editable Python-constraint control"
    );

    review.set_name("vpin");
    let entry = review.create_entry().unwrap();
    let stored = String::from_utf8(entry.payload.unwrap().bytes).unwrap();
    assert!(
        stored.contains("requires-python = \">=3.12,<3.13\""),
        "derived pin did not land in the copied PEP 723 block: {stored}"
    );
}

#[test]
fn test_review_pin_follows_a_shebang_edit_on_rescan() {
    let mut workflow = AddWorkflowState::from_review(review(
        "v.py",
        b"#!/usr/bin/env python3.12\nprint('hi')\n",
        KnownEntryKind::Python,
    ));
    assert_eq!(workflow.review().unwrap().requires_python(), ">=3.12,<3.13");

    let effects = workflow.reduce(AddAction::EditSource);
    let [AddEffect::EditSource { request, path }] = effects.as_slice() else {
        panic!("Edit source must emit one typed editor request");
    };
    assert_eq!(path, &PathBuf::from("v.py"));
    let request = *request;
    assert!(
        workflow
            .reduce(AddAction::SourceEdited {
                request,
                result: Ok(snapshot(
                    "v.py",
                    b"#!/usr/bin/env python3.11\nprint('hi')\n",
                    false,
                )),
            })
            .is_empty()
    );
    assert_eq!(workflow.review().unwrap().requires_python(), ">=3.11,<3.12");
    let (text, _) = render_review(&workflow);
    assert!(text.contains(">=3.11,<3.12"), "rescanned pin is not visible:\n{text}");
    assert!(!text.contains(">=3.12,<3.13"), "stale pin survived the rescan:\n{text}");
}

#[test]
fn test_review_explicit_python_is_not_overwritten_by_the_shebang() {
    let review = ReviewState::from_source(
        snapshot(
            "v.py",
            b"#!/usr/bin/env python3.12\nprint('hi')\n",
            false,
        ),
        KnownEntryKind::Python,
        ReviewDefaults {
            requires_python: Some(">=3.9".to_owned()),
            ..ReviewDefaults::default()
        },
    );
    assert_eq!(review.requires_python(), ">=3.9");
    let (text, _) = render_review(&AddWorkflowState::from_review(review));
    assert!(text.contains(">=3.9"), "explicit pin is not prefilled:\n{text}");
    assert!(!text.contains(">=3.12,<3.13"), "shebang overwrote the explicit pin:\n{text}");
}

#[test]
fn test_review_dynamic_optstring_keeps_ticks_and_space_chip() {
    let review = review("dyn.sh", DYN_SH, KnownEntryKind::Shell);
    assert_eq!(review.modeled_cli_field_count(), None);
    let (text, geometry) = render_review(&AddWorkflowState::from_review(review));
    assert!(text.contains("parses its own arguments"), "{text}");
    assert!(
        has_add_hit(&geometry, &AddControlId::Candidate("OUTDIR".to_owned())),
        "dynamic getopts lost the additive constant checkbox"
    );
    assert!(
        has_add_hit(&geometry, &AddControlId::ToggleFocused),
        "Space/Toggle is not a real clickable footer path"
    );
    assert!(text.contains("Space") && text.contains("Toggle"), "{text}");
}

#[test]
fn test_review_modeled_getopts_suppresses_ticks_and_space_chip() {
    let review = review("mod.sh", MODELED_SH, KnownEntryKind::Shell);
    assert_eq!(review.modeled_cli_field_count(), Some(2));
    let (text, geometry) = render_review(&AddWorkflowState::from_review(review));
    assert!(text.contains("skit read this script's own arguments"), "{text}");
    assert!(
        geometry
            .hits
            .iter()
            .all(|hit| !matches!(&hit.target, AddControlId::Candidate(_))),
        "modeled getopts still offers manage-a-constant checkboxes"
    );
    assert!(
        !has_add_hit(&geometry, &AddControlId::ToggleFocused),
        "modeled getopts advertises a dead Space/Toggle path"
    );
}

#[test]
fn test_review_one_field_getopts_says_singular() {
    let review = review(
        "one.sh",
        b"#!/usr/bin/env bash\nwhile getopts \"n:\" o; do :; done\n",
        KnownEntryKind::Shell,
    );
    assert_eq!(review.modeled_cli_field_count(), Some(1));
    let (text, _) = render_review(&AddWorkflowState::from_review(review));
    assert!(text.contains("(1 field)"), "{text}");
    assert!(!text.contains("(1 fields)"), "{text}");
}

#[test]
fn test_review_multi_field_getopts_says_plural() {
    let review = review(
        "many.sh",
        b"#!/usr/bin/env bash\nwhile getopts \"n:v\" o; do :; done\n",
        KnownEntryKind::Shell,
    );
    assert_eq!(review.modeled_cli_field_count(), Some(2));
    let (text, _) = render_review(&AddWorkflowState::from_review(review));
    assert!(text.contains("(2 fields)"), "{text}");
}

#[test]
fn test_ctrl_d_deletes_the_highlighted_draft_after_confirm() {
    let root = TempDir::new().unwrap();
    let keep = root.path().join("skit-new-keep.py");
    let doomed = root.path().join("skit-new-doomed.py");
    fs::write(&keep, "print('keep')\n").unwrap();
    fs::write(&doomed, "print('doomed')\n").unwrap();
    let mut state = draft_state(&[(keep.clone(), 1), (doomed.clone(), 2)]);
    let mut session = TuiSession::default();
    select_draft_by_click(&mut session, &mut state, "skit-new-doomed.py");

    let (_, geometry) = draw_full(&mut session, &state);
    let (handling, effect) = drive(
        &mut session,
        &mut state,
        &geometry,
        key(KeyCode::Char('d'), KeyModifiers::CONTROL),
    );
    assert!(
        matches!(
            &handling,
            EventHandling::Action(Action::Add(AddAction::DeleteSelectedDraft))
        ),
        "Ctrl+D did not request draft confirmation: {handling:?}"
    );
    assert_eq!(effect, Effect::None);
    assert_eq!(state.add_workflow().unwrap().stage(), AddStage::ConfirmDraftDelete);

    let (_, geometry) = draw_full(&mut session, &state);
    let (handling, effect) = drive(
        &mut session,
        &mut state,
        &geometry,
        key(KeyCode::Char('y'), KeyModifiers::NONE),
    );
    assert!(
        matches!(&handling, EventHandling::Action(_)),
        "the Python v0.4 confirmation key `y` was not accepted: {handling:?}"
    );
    let Effect::Add(effects) = effect else {
        panic!("confirming with y did not reach the typed draft-delete host boundary");
    };
    let [AddEffect::DeleteDraft { request, path }] = effects.as_slice() else {
        panic!("confirming draft deletion must emit exactly one DeleteDraft effect: {effects:?}");
    };
    assert_eq!(path, &doomed);
    fs::remove_file(path).unwrap();
    let request = *request;
    assert_eq!(
        state.update(Action::Add(AddAction::DraftDeleted {
            request,
            result: Ok(()),
        })),
        Effect::None
    );

    assert!(!doomed.exists());
    assert!(keep.exists());
    let workflow = state.add_workflow().unwrap();
    assert_eq!(workflow.stage(), AddStage::Source);
    assert_eq!(workflow.source().listed_drafts().len(), 1);
    assert_eq!(workflow.source().listed_drafts()[0].path, keep);
    let (terminal, _) = draw_full(&mut session, &state);
    assert!(
        rendered(terminal.backend().buffer()).contains("Deleted the draft"),
        "successful deletion was not surfaced to the user"
    );
}

#[test]
fn test_ctrl_d_confirm_esc_keeps_the_draft() {
    let root = TempDir::new().unwrap();
    let draft = root.path().join("skit-new-safe.py");
    fs::write(&draft, "print('safe')\n").unwrap();
    let mut state = draft_state(&[(draft.clone(), 1)]);
    let mut session = TuiSession::default();
    select_draft_by_click(&mut session, &mut state, "skit-new-safe.py");

    let (_, geometry) = draw_full(&mut session, &state);
    let (_, effect) = drive(
        &mut session,
        &mut state,
        &geometry,
        key(KeyCode::Char('d'), KeyModifiers::CONTROL),
    );
    assert_eq!(effect, Effect::None);
    assert_eq!(state.add_workflow().unwrap().stage(), AddStage::ConfirmDraftDelete);

    let (_, geometry) = draw_full(&mut session, &state);
    let (handling, effect) = drive(
        &mut session,
        &mut state,
        &geometry,
        key(KeyCode::Esc, KeyModifiers::NONE),
    );
    assert!(matches!(&handling, EventHandling::Action(_)), "{handling:?}");
    assert_eq!(effect, Effect::None);
    assert_eq!(state.add_workflow().unwrap().stage(), AddStage::Source);
    assert!(draft.exists());
    assert_eq!(state.add_workflow().unwrap().source().listed_drafts().len(), 1);
}

#[test]
fn test_ctrl_d_while_editing_a_field_is_the_inputs_delete_right() {
    let root = TempDir::new().unwrap();
    let draft = root.path().join("skit-new-edit.py");
    fs::write(&draft, "print('edit')\n").unwrap();
    let mut state = draft_state(&[(draft.clone(), 1)]);
    let mut session = TuiSession::default();

    for character in ['a', 'b', 'c'] {
        let (_, geometry) = draw_full(&mut session, &state);
        let (handling, _) = drive(
            &mut session,
            &mut state,
            &geometry,
            key(KeyCode::Char(character), KeyModifiers::NONE),
        );
        assert!(matches!(&handling, EventHandling::Action(_)), "{handling:?}");
    }
    assert_eq!(state.add_workflow().unwrap().source().path, "abc");

    let (_, geometry) = draw_full(&mut session, &state);
    let _ = drive(
        &mut session,
        &mut state,
        &geometry,
        key(KeyCode::Home, KeyModifiers::NONE),
    );
    let (_, geometry) = draw_full(&mut session, &state);
    let _ = drive(
        &mut session,
        &mut state,
        &geometry,
        key(KeyCode::Right, KeyModifiers::NONE),
    );
    let (_, geometry) = draw_full(&mut session, &state);
    let _ = drive(
        &mut session,
        &mut state,
        &geometry,
        key(KeyCode::Char('d'), KeyModifiers::CONTROL),
    );

    assert_eq!(
        state.add_workflow().unwrap().stage(),
        AddStage::Source,
        "Ctrl+D inside Source path opened draft confirmation"
    );
    assert!(draft.exists(), "editing chord touched the kept draft");
    assert_eq!(
        state.add_workflow().unwrap().source().path,
        "ac",
        "Ctrl+D did not perform the input widget's delete-right operation"
    );
}

#[test]
fn test_delete_draft_action_is_a_noop_when_no_drafts() {
    let mut workflow = AddWorkflowState::new(Vec::new());
    assert!(workflow.reduce(AddAction::DeleteSelectedDraft).is_empty());
    assert_eq!(workflow.stage(), AddStage::Source);
    assert!(workflow.source().listed_drafts().is_empty());
}

#[test]
fn test_delete_draft_action_is_a_noop_when_nothing_highlighted() {
    let mut workflow = AddWorkflowState::new(vec![DraftSummary {
        path: PathBuf::from("skit-new-none.py"),
        modified: 1,
    }]);
    assert!(workflow.source().selected_draft().is_none());
    assert!(workflow.reduce(AddAction::DeleteSelectedDraft).is_empty());
    assert_eq!(workflow.stage(), AddStage::Source);
    assert_eq!(workflow.source().listed_drafts().len(), 1);
}

#[test]
fn test_delete_draft_chip_only_renders_when_drafts_exist() {
    let empty = AddWorkflowState::new(Vec::new());
    let mut session = AddScreenSession::default();
    let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
    let mut empty_geometry = AddScreenGeometry::default();
    terminal
        .draw(|frame| {
            empty_geometry = render_add(frame, frame.area(), &empty, &mut session, Locale::En);
        })
        .unwrap();
    assert!(
        empty_geometry
            .hits
            .iter()
            .all(|hit| hit.target != AddControlId::DeleteDraft)
    );
    assert!(!rendered(terminal.backend().buffer()).contains("Ctrl+D"));

    let present = AddWorkflowState::new(vec![DraftSummary {
        path: PathBuf::from("skit-new-present.py"),
        modified: 1,
    }]);
    let mut session = AddScreenSession::default();
    let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
    let mut present_geometry = AddScreenGeometry::default();
    terminal
        .draw(|frame| {
            present_geometry = render_add(frame, frame.area(), &present, &mut session, Locale::En);
        })
        .unwrap();
    assert!(
        present_geometry
            .hits
            .iter()
            .filter(|hit| hit.target == AddControlId::DeleteDraft)
            .any(|hit| region_text(terminal.backend().buffer(), hit.area).contains("Ctrl+D")),
        "drafts exist but no clickable Ctrl+D chip was rendered:\n{}",
        rendered(terminal.backend().buffer())
    );
}
