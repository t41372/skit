//! Positive navigation pilots from Python `tests/test_tui_nav.py` at `main@206f9ef`.
//!
//! Do not replace these with command-registry inspection. The Python contract explicitly requires an
//! advertised key to have a positive pilot test on each surface. This file starts with Run Form and
//! exercises the actual mature-widget session, reducer focus, and clickable footer chips.

use std::collections::BTreeMap;

use ratatui_core::{backend::TestBackend, buffer::Buffer, terminal::Terminal};
use ratatui_crossterm::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use skit_domain::parameters::ParamDecl;
use skit_i18n::Locale;
use skit_tui::{EventHandling, HitTarget, TuiSession, ViewGeometry, render_with_session};
use skit_ui::{Action, LibraryState, RunFormView, Screen, UiCommand};

fn two_field_state() -> LibraryState {
    let form = RunFormView::from_declarations(
        "two",
        "Two",
        &[ParamDecl::new("CITY"), ParamDecl::new("NAME")],
        &BTreeMap::from([
            ("CITY".to_owned(), "x".to_owned()),
            ("NAME".to_owned(), "y".to_owned()),
        ]),
        &[],
        "",
        &BTreeMap::new(),
        "",
    );
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Run(Box::new(form))));
    state
}

fn draw(
    session: &mut TuiSession,
    state: &LibraryState,
) -> (Terminal<TestBackend>, ViewGeometry) {
    let mut terminal = Terminal::new(TestBackend::new(130, 40)).unwrap();
    let mut geometry = ViewGeometry::default();
    terminal
        .draw(|frame| {
            geometry = render_with_session(frame, state, Locale::En, session);
        })
        .unwrap();
    (terminal, geometry)
}

fn key(code: KeyCode, modifiers: KeyModifiers) -> Event {
    Event::Key(KeyEvent::new(code, modifiers))
}

fn click(target: &skit_tui::HitRegion) -> Event {
    Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: target.rect.x,
        row: target.rect.y,
        modifiers: KeyModifiers::NONE,
    })
}

fn apply(
    session: &mut TuiSession,
    state: &mut LibraryState,
    geometry: &ViewGeometry,
    event: Event,
) -> EventHandling {
    let handling = session.handle_event(event, state, geometry);
    if let EventHandling::Action(action) = &handling {
        state.update(action.clone());
    }
    handling
}

fn text(buffer: &Buffer) -> String {
    buffer.content().iter().map(|cell| cell.symbol()).collect()
}

#[test]
fn test_run_form_boots_typeable_and_arrows_walk_the_fields() {
    let mut state = two_field_state();
    let mut session = TuiSession::default();
    let (terminal, geometry) = draw(&mut session, &state);

    assert_eq!(
        state.focused_form_field(),
        Some(0),
        "Run Form must boot on the first typeable field"
    );
    let rendered = text(terminal.backend().buffer());
    assert!(rendered.contains("Tab"), "forward field navigation is not advertised: {rendered}");
    assert!(rendered.contains('↓'), "Down-arrow field navigation is not advertised: {rendered}");
    assert!(rendered.contains("Shift+Tab"), "backward field navigation is not advertised: {rendered}");
    assert!(rendered.contains('↑'), "Up-arrow field navigation is not advertised: {rendered}");

    assert_eq!(
        apply(
            &mut session,
            &mut state,
            &geometry,
            key(KeyCode::Down, KeyModifiers::NONE),
        ),
        EventHandling::Action(Action::FocusNext)
    );
    assert_eq!(state.focused_form_field(), Some(1));

    let (_, geometry) = draw(&mut session, &state);
    assert_eq!(
        apply(
            &mut session,
            &mut state,
            &geometry,
            key(KeyCode::Up, KeyModifiers::NONE),
        ),
        EventHandling::Action(Action::FocusPrevious)
    );
    assert_eq!(state.focused_form_field(), Some(0));

    let (_, geometry) = draw(&mut session, &state);
    let forward = geometry
        .hits
        .iter()
        .find(|hit| hit.action == HitTarget::Command(UiCommand::FocusNext))
        .unwrap_or_else(|| panic!("Run Form footer did not expose a clickable forward-field chip"));
    assert_eq!(
        apply(&mut session, &mut state, &geometry, click(forward)),
        EventHandling::Action(Action::FocusNext)
    );
    assert_eq!(state.focused_form_field(), Some(1));

    let (_, geometry) = draw(&mut session, &state);
    let backward = geometry
        .hits
        .iter()
        .find(|hit| hit.action == HitTarget::Command(UiCommand::FocusPrevious))
        .unwrap_or_else(|| panic!("Run Form footer did not expose a clickable backward-field chip"));
    assert_eq!(
        apply(&mut session, &mut state, &geometry, click(backward)),
        EventHandling::Action(Action::FocusPrevious)
    );
    assert_eq!(state.focused_form_field(), Some(0));

    let (_, geometry) = draw(&mut session, &state);
    assert_eq!(
        apply(
            &mut session,
            &mut state,
            &geometry,
            key(KeyCode::Tab, KeyModifiers::NONE),
        ),
        EventHandling::Action(Action::FocusNext)
    );
    assert_eq!(state.focused_form_field(), Some(1));

    let (_, geometry) = draw(&mut session, &state);
    assert_eq!(
        apply(
            &mut session,
            &mut state,
            &geometry,
            key(KeyCode::BackTab, KeyModifiers::SHIFT),
        ),
        EventHandling::Action(Action::FocusPrevious)
    );
    assert_eq!(state.focused_form_field(), Some(0));
}
