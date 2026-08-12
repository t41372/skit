//! Add-surface positive navigation pilots from Python `tests/test_tui_nav.py` at `main@206f9ef`.
//!
//! The Python contract is deliberately stricter than "Tab works": both arrow twins must move focus,
//! both direction chips must be visibly advertised and clickable, and Source/Review must boot on the
//! first text control rather than their scroll container. Current Rust behavior may fail these tests;
//! that is a parity finding, not a reason to weaken them.

use ratatui_core::{backend::TestBackend, buffer::Buffer, layout::Rect, terminal::Terminal};
use ratatui_crossterm::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use skit_application::SourcePermissions;
use skit_i18n::Locale;
use skit_tui::{
    AddControlId, AddScreenEvent, AddScreenGeometry, AddScreenSession, AddTextField, render_add,
};
use skit_ui::{AddWorkflowState, KnownEntryKind, ReviewDefaults, ReviewState, SourceSnapshot};

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

fn draw(
    session: &mut AddScreenSession,
    state: &AddWorkflowState,
) -> (Terminal<TestBackend>, AddScreenGeometry) {
    let mut terminal = Terminal::new(TestBackend::new(130, 40)).unwrap();
    let mut geometry = AddScreenGeometry::default();
    terminal
        .draw(|frame| {
            let area = frame.area();
            geometry = render_add(frame, area, state, session, Locale::En);
        })
        .unwrap();
    (terminal, geometry)
}

fn region_text(buffer: &Buffer, area: Rect) -> String {
    let mut out = String::new();
    for row in area.y..area.y.saturating_add(area.height).min(buffer.area.height) {
        for column in area.x..area.x.saturating_add(area.width).min(buffer.area.width) {
            out.push_str(buffer[(column, row)].symbol());
        }
    }
    out
}

fn click_advertised_chip(
    session: &mut AddScreenSession,
    state: &AddWorkflowState,
    terminal: &Terminal<TestBackend>,
    geometry: &AddScreenGeometry,
    label: &str,
) -> Option<AddScreenEvent> {
    let hit = geometry
        .hits
        .iter()
        .find(|hit| region_text(terminal.backend().buffer(), hit.area).contains(label))
        .unwrap_or_else(|| panic!("missing clickable advertised navigation chip {label:?}"));
    session.handle_event(mouse(hit.area.x, hit.area.y), state, geometry)
}

fn assert_source_focus(session: &AddScreenSession, field: AddTextField) {
    assert_eq!(session.focused(), Some(&AddControlId::Text(field)));
}

#[test]
fn test_add_source_arrows_walk_path_template_name() {
    let state = AddWorkflowState::new(Vec::new());
    let mut session = AddScreenSession::default();
    let (_, geometry) = draw(&mut session, &state);
    assert_source_focus(&session, AddTextField::SourcePath);

    assert_eq!(
        session.handle_event(key(KeyCode::Down, KeyModifiers::NONE), &state, &geometry),
        Some(AddScreenEvent::Changed)
    );
    assert_source_focus(&session, AddTextField::CommandTemplate);

    let (_, geometry) = draw(&mut session, &state);
    assert_eq!(
        session.handle_event(key(KeyCode::Up, KeyModifiers::NONE), &state, &geometry),
        Some(AddScreenEvent::Changed)
    );
    assert_source_focus(&session, AddTextField::SourcePath);

    let (terminal, geometry) = draw(&mut session, &state);
    assert_eq!(
        click_advertised_chip(&mut session, &state, &terminal, &geometry, "Tab/↓"),
        Some(AddScreenEvent::Changed)
    );
    assert_source_focus(&session, AddTextField::CommandTemplate);

    let (terminal, geometry) = draw(&mut session, &state);
    assert_eq!(
        click_advertised_chip(&mut session, &state, &terminal, &geometry, "Shift+Tab/↑",),
        Some(AddScreenEvent::Changed)
    );
    assert_source_focus(&session, AddTextField::SourcePath);

    let (_, geometry) = draw(&mut session, &state);
    assert_eq!(
        session.handle_event(key(KeyCode::Tab, KeyModifiers::NONE), &state, &geometry),
        Some(AddScreenEvent::Changed)
    );
    assert_source_focus(&session, AddTextField::CommandTemplate);

    let (_, geometry) = draw(&mut session, &state);
    assert_eq!(
        session.handle_event(
            key(KeyCode::BackTab, KeyModifiers::SHIFT),
            &state,
            &geometry
        ),
        Some(AddScreenEvent::Changed)
    );
    assert_source_focus(&session, AddTextField::SourcePath);
}

fn review_state() -> AddWorkflowState {
    let review = ReviewState::from_source(
        SourceSnapshot {
            path: "job.py".into(),
            source_record: "job.py".to_owned(),
            bytes: b"CITY = \"x\"\nprint(CITY)\n".to_vec(),
            permissions: SourcePermissions::default(),
            is_regular: true,
            is_directory: false,
            is_draft: false,
        },
        KnownEntryKind::Python,
        ReviewDefaults::default(),
    );
    AddWorkflowState::from_review(review)
}

#[test]
fn test_add_review_boots_on_name_and_arrows_move() {
    let state = review_state();
    let mut session = AddScreenSession::default();
    let (_, geometry) = draw(&mut session, &state);
    assert_source_focus(&session, AddTextField::ReviewName);

    assert_eq!(
        session.handle_event(key(KeyCode::Down, KeyModifiers::NONE), &state, &geometry),
        Some(AddScreenEvent::Changed)
    );
    assert_source_focus(&session, AddTextField::ReviewDescription);

    let (_, geometry) = draw(&mut session, &state);
    assert_eq!(
        session.handle_event(key(KeyCode::Up, KeyModifiers::NONE), &state, &geometry),
        Some(AddScreenEvent::Changed)
    );
    assert_source_focus(&session, AddTextField::ReviewName);

    let (terminal, geometry) = draw(&mut session, &state);
    assert_eq!(
        click_advertised_chip(&mut session, &state, &terminal, &geometry, "Tab/↓"),
        Some(AddScreenEvent::Changed)
    );
    assert_source_focus(&session, AddTextField::ReviewDescription);

    let (terminal, geometry) = draw(&mut session, &state);
    assert_eq!(
        click_advertised_chip(&mut session, &state, &terminal, &geometry, "Shift+Tab/↑",),
        Some(AddScreenEvent::Changed)
    );
    assert_source_focus(&session, AddTextField::ReviewName);

    let (_, geometry) = draw(&mut session, &state);
    assert_eq!(
        session.handle_event(key(KeyCode::Tab, KeyModifiers::NONE), &state, &geometry),
        Some(AddScreenEvent::Changed)
    );
    assert_source_focus(&session, AddTextField::ReviewDescription);

    let (_, geometry) = draw(&mut session, &state);
    assert_eq!(
        session.handle_event(
            key(KeyCode::BackTab, KeyModifiers::SHIFT),
            &state,
            &geometry
        ),
        Some(AddScreenEvent::Changed)
    );
    assert_source_focus(&session, AddTextField::ReviewName);
}
