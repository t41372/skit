use ratatui_core::{backend::TestBackend, buffer::Buffer, terminal::Terminal};
use ratatui_crossterm::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use skit_domain::parameters::{ParamDecl, ParameterDelivery};
use skit_i18n::Locale;
use skit_tui::{EventHandling, TuiSession, ViewGeometry, render_with_session};
use skit_ui::{
    Action, Effect, FieldValue, FormPurpose, LibraryState, MANAGE_KEY, ModalState, Screen,
    SettingsAction, SettingsInputs, SettingsView, TypedValue,
};

fn managed(name: &str) -> ParamDecl {
    let mut declaration = ParamDecl::new(name);
    declaration.delivery = ParameterDelivery::Placeholder;
    declaration
}

fn settings(count: usize) -> SettingsView {
    SettingsView::from_inputs(&SettingsInputs {
        selector: "p".to_owned(),
        kind: "prompt".to_owned(),
        name: "p".to_owned(),
        source: "/work/p.prompt.md".to_owned(),
        workdir: "invoke".to_owned(),
        supports_modes: true,
        has_original_file: true,
        has_stored_name: true,
        declared_schema: true,
        interpolate: true,
        managed: vec![managed("a")],
        candidates: (0..count).map(|index| format!("u{index}")).collect(),
        configured_runners: vec!["claude".to_owned()],
        ..SettingsInputs::default()
    })
}

fn state(count: usize) -> LibraryState {
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Settings(Box::new(settings(count)))));
    state
}

fn render(
    session: &mut TuiSession,
    state: &LibraryState,
    width: u16,
    height: u16,
) -> (Terminal<TestBackend>, ViewGeometry) {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
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

fn mouse(x: u16, y: u16) -> Event {
    Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: x,
        row: y,
        modifiers: KeyModifiers::NONE,
    })
}

fn drive(
    session: &mut TuiSession,
    state: &mut LibraryState,
    geometry: &ViewGeometry,
    event: Event,
) -> (EventHandling, Effect) {
    let handling = session.handle_event(event, state, geometry);
    let effect = if let EventHandling::Action(action) = &handling {
        state.update(action.clone())
    } else {
        Effect::None
    };
    (handling, effect)
}

fn locate(buffer: &Buffer, needle: &str) -> (u16, u16) {
    for y in 0..buffer.area.height {
        let mut row = String::new();
        for x in 0..buffer.area.width {
            row.push_str(buffer[(x, y)].symbol());
        }
        if let Some(offset) = row.find(needle) {
            return (u16::try_from(offset).unwrap_or(0).saturating_add(1), y);
        }
    }
    panic!("missing rendered text {needle:?}");
}

fn expected_hidden() -> FieldValue {
    FieldValue::Explicit(TypedValue::Choices(vec!["u28".to_owned()]))
}

#[test]
fn test_settings_candidate_picker_reaches_a_hidden_name_and_waits_for_outer_save() {
    let mut state = state(29);
    let mut session = TuiSession::default();
    let (_, geometry) = render(&mut session, &state, 110, 40);
    let (opened, effect) = drive(
        &mut session,
        &mut state,
        &geometry,
        key(KeyCode::Char('o'), KeyModifiers::CONTROL),
    );
    assert_eq!(opened, EventHandling::Consumed, "Ctrl+O did not open the settings full candidate picker");
    assert_eq!(effect, Effect::None, "opening the picker performed host work");

    let (_, mut geometry) = render(&mut session, &state, 110, 40);
    for character in "u28".chars() {
        let (handling, effect) = drive(
            &mut session,
            &mut state,
            &geometry,
            key(KeyCode::Char(character), KeyModifiers::NONE),
        );
        assert_eq!(handling, EventHandling::Consumed);
        assert_eq!(effect, Effect::None);
        geometry = render(&mut session, &state, 110, 40).1;
    }
    for code in [KeyCode::Enter, KeyCode::Char(' ')] {
        let (handling, effect) = drive(
            &mut session,
            &mut state,
            &geometry,
            key(code, KeyModifiers::NONE),
        );
        assert_eq!(handling, EventHandling::Consumed);
        assert_eq!(effect, Effect::None);
        geometry = render(&mut session, &state, 110, 40).1;
    }
    let (_, effect) = drive(
        &mut session,
        &mut state,
        &geometry,
        key(KeyCode::Char('s'), KeyModifiers::CONTROL),
    );
    assert_eq!(effect, Effect::None, "picker Done wrote settings before the outer Save");
    assert_eq!(state.settings_view().unwrap().submitted_values().get(MANAGE_KEY), Some(&expected_hidden()));

    let effect = state.update(Action::Settings(SettingsAction::Save));
    let Effect::Submit { purpose: FormPurpose::Settings, selector, values } = effect else {
        panic!("outer Save did not produce the settings host write: {effect:?}")
    };
    assert_eq!(selector.as_deref(), Some("p"));
    assert_eq!(values.get(MANAGE_KEY), Some(&expected_hidden()));
}

#[test]
fn test_settings_candidate_picker_selection_is_discardable() {
    let mut state = state(22);
    let mut session = TuiSession::default();
    let (terminal, geometry) = render(&mut session, &state, 110, 40);
    let choose = locate(terminal.backend().buffer(), "Choose variables");
    let (opened, effect) = drive(&mut session, &mut state, &geometry, mouse(choose.0, choose.1));
    assert_eq!(opened, EventHandling::Consumed, "mouse Choose variables did not open the settings picker");
    assert_eq!(effect, Effect::None);

    let (terminal, geometry) = render(&mut session, &state, 110, 40);
    let all = locate(terminal.backend().buffer(), "Select all");
    let (_, effect) = drive(&mut session, &mut state, &geometry, mouse(all.0, all.1));
    assert_eq!(effect, Effect::None);
    let (terminal, geometry) = render(&mut session, &state, 110, 40);
    let done = locate(terminal.backend().buffer(), "Done");
    let (_, effect) = drive(&mut session, &mut state, &geometry, mouse(done.0, done.1));
    assert_eq!(effect, Effect::None, "picker Done performed a host save");
    assert!(state.settings_view().unwrap().is_dirty(), "picker selection did not arm the settings dirty guard");

    assert_eq!(state.update(Action::Settings(SettingsAction::Close)), Effect::None);
    assert!(matches!(state.modal(), Some(ModalState::ConfirmDiscardChanges)));
    assert_eq!(state.update(Action::DiscardChanges), Effect::None);
    assert!(matches!(state.screen(), Screen::Library));
}

#[test]
fn test_settings_candidate_picker_cancel_and_unchanged_done_are_noops() {
    let mut state = state(22);
    let mut session = TuiSession::default();
    let (_, geometry) = render(&mut session, &state, 110, 40);
    assert_eq!(
        drive(&mut session, &mut state, &geometry, key(KeyCode::Char('o'), KeyModifiers::CONTROL)).0,
        EventHandling::Consumed
    );
    let (_, geometry) = render(&mut session, &state, 110, 40);
    assert_eq!(drive(&mut session, &mut state, &geometry, key(KeyCode::Esc, KeyModifiers::NONE)).1, Effect::None);
    assert!(!state.settings_view().unwrap().is_dirty(), "cancelled picker dirtied settings");
    assert!(state.settings_view().unwrap().submitted_values().is_empty());

    let (_, geometry) = render(&mut session, &state, 110, 40);
    assert_eq!(
        drive(&mut session, &mut state, &geometry, key(KeyCode::Char('o'), KeyModifiers::CONTROL)).0,
        EventHandling::Consumed
    );
    let (_, geometry) = render(&mut session, &state, 110, 40);
    assert_eq!(
        drive(&mut session, &mut state, &geometry, key(KeyCode::Char('s'), KeyModifiers::CONTROL)).1,
        Effect::None
    );
    assert!(!state.settings_view().unwrap().is_dirty(), "unchanged picker Done dirtied settings");
    assert!(state.settings_view().unwrap().submitted_values().is_empty());
}

#[test]
fn test_settings_candidate_picker_tolerates_preview_recompose() {
    let mut state = state(22);
    let mut session = TuiSession::default();
    let (_, geometry) = render(&mut session, &state, 110, 40);
    assert_eq!(
        drive(&mut session, &mut state, &geometry, key(KeyCode::Char('o'), KeyModifiers::CONTROL)).0,
        EventHandling::Consumed
    );

    // Recompose/resize the settings preview behind the overlay before changing the full selection.
    // The overlay must own name-keyed state, not references to the old inline preview cells.
    let (terminal, geometry) = render(&mut session, &state, 42, 10);
    let all = locate(terminal.backend().buffer(), "Select all");
    assert_eq!(drive(&mut session, &mut state, &geometry, mouse(all.0, all.1)).1, Effect::None);
    let (terminal, geometry) = render(&mut session, &state, 42, 10);
    let done = locate(terminal.backend().buffer(), "Done");
    assert_eq!(drive(&mut session, &mut state, &geometry, mouse(done.0, done.1)).1, Effect::None);

    let expected = (0..22).map(|index| format!("u{index}")).collect::<Vec<_>>();
    assert_eq!(
        state.settings_view().unwrap().submitted_values().get(MANAGE_KEY),
        Some(&FieldValue::Explicit(TypedValue::Choices(expected)))
    );
}
