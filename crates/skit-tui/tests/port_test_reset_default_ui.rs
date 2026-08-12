//! Ratatui ports of the run-form/reset and input-binding UI contracts from Python
//! `tests/test_reset_default_ui.py` at `main@206f9ef`.
//!
//! These use the mature `TuiSession + LibraryState` reducer loop and real render hit regions. The
//! reset chip is clicked through its geometry; Ctrl+O is dispatched through the same event path the
//! terminal uses. No test calls a recreated reset helper.

use std::collections::BTreeMap;

use ratatui_core::{backend::TestBackend, buffer::Buffer, terminal::Terminal};
use ratatui_crossterm::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use skit_domain::parameters::{ParamDecl, ParameterBinding, ParameterType, ParameterValue};
use skit_i18n::Locale;
use skit_tui::{EventHandling, HitTarget, TuiSession, ViewGeometry, render_with_session};
use skit_ui::{
    Action, FormControl, LibraryState, RunFormView, Screen, UiCommand,
};

const INPUT_BINDING_HINT: &str = "Leave empty and the script will ask you in the terminal.";

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

fn state_with_form(form: RunFormView) -> LibraryState {
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Run(Box::new(form))));
    state
}

fn draw(
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

fn drive(
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

fn rendered(buffer: &Buffer) -> String {
    buffer.content().iter().map(|cell| cell.symbol()).collect()
}

fn text_default(name: &str, default: Option<&str>, secret: bool) -> ParamDecl {
    let mut declaration = ParamDecl::new(name);
    declaration.secret = secret;
    declaration.default = default.map(|value| ParameterValue::String(value.to_owned()));
    declaration
}

fn form(
    declarations: &[ParamDecl],
    saved: &[(&str, &str)],
    runners: &[String],
) -> RunFormView {
    RunFormView::from_declarations(
        "demo",
        "Demo",
        declarations,
        &saved
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect(),
        runners,
        runners.first().map_or("", String::as_str),
        &BTreeMap::new(),
        "",
    )
}

fn parameter_value(state: &LibraryState, name: &str) -> String {
    state
        .run_form()
        .unwrap()
        .fields()
        .iter()
        .find(|field| field.key == format!("value:{name}"))
        .unwrap_or_else(|| panic!("missing parameter field {name}"))
        .control
        .value()
}

fn parameter_index(state: &LibraryState, name: &str) -> usize {
    state
        .run_form()
        .unwrap()
        .fields()
        .iter()
        .position(|field| field.key == format!("value:{name}"))
        .unwrap_or_else(|| panic!("missing parameter field {name}"))
}

fn reset_hit(geometry: &ViewGeometry, field: usize) -> Option<ratatui_core::layout::Rect> {
    geometry.hits.iter().find_map(|hit| {
        (hit.action
            == HitTarget::RunFieldCommand {
                field,
                command: UiCommand::ResetDefault,
            })
        .then_some(hit.rect)
    })
}

#[test]
fn test_ctrl_o_from_focused_field_restores_default_over_remembered_value() {
    let declaration = text_default("greeting", Some("hello"), false);
    let mut state = state_with_form(form(&[declaration], &[("greeting", "world")], &[]));
    let field = parameter_index(&state, "greeting");
    state.update(Action::FocusField(field));
    assert_eq!(parameter_value(&state, "greeting"), "world");
    let mut session = TuiSession::default();
    let (_, geometry) = draw(&mut session, &state, 100, 24);

    assert_eq!(
        drive(
            &mut session,
            &mut state,
            &geometry,
            key(KeyCode::Char('o'), KeyModifiers::CONTROL),
        ),
        EventHandling::Action(Action::ResetFocusedRunField)
    );
    assert_eq!(parameter_value(&state, "greeting"), "hello");
}

#[test]
fn test_reset_field_by_key_restores_text_bool_and_choice_defaults() {
    let greeting = text_default("greeting", Some("hello"), false);
    let mut flag = ParamDecl::new("flag");
    flag.parameter_type = ParameterType::Bool;
    flag.default = Some(ParameterValue::Bool(false));
    let mut mode = ParamDecl::new("mode");
    mode.parameter_type = ParameterType::Choice;
    mode.choices = vec!["a".to_owned(), "b".to_owned()];
    mode.default = Some(ParameterValue::String("a".to_owned()));
    let mut state = state_with_form(form(
        &[greeting, flag, mode],
        &[("greeting", "world"), ("flag", "true"), ("mode", "b")],
        &[],
    ));

    assert_eq!(parameter_value(&state, "greeting"), "world");
    assert_eq!(parameter_value(&state, "flag"), "true");
    assert_eq!(parameter_value(&state, "mode"), "b");
    for name in ["greeting", "flag", "mode"] {
        let index = parameter_index(&state, name);
        state.update(Action::ResetRunField(index));
    }

    assert_eq!(parameter_value(&state, "greeting"), "hello");
    assert_eq!(parameter_value(&state, "flag"), "false");
    assert_eq!(parameter_value(&state, "mode"), "a");
}

#[test]
fn test_reset_chip_mouse_click_restores_the_default() {
    let declaration = text_default("greeting", Some("hello"), false);
    let mut state = state_with_form(form(&[declaration], &[("greeting", "world")], &[]));
    let field = parameter_index(&state, "greeting");
    let mut session = TuiSession::default();
    let (_, geometry) = draw(&mut session, &state, 120, 30);
    let area = reset_hit(&geometry, field).expect("defaulted field must render a reset chip");

    assert_eq!(
        drive(
            &mut session,
            &mut state,
            &geometry,
            mouse(area.x, area.y),
        ),
        EventHandling::Action(Action::ResetRunField(field))
    );
    assert_eq!(parameter_value(&state, "greeting"), "hello");
}

#[test]
fn test_reset_chip_present_for_default_absent_for_secret_and_no_default() {
    let with_default = text_default("withdef", Some("hi"), false);
    let secret = text_default("sekret", Some("s"), true);
    let no_default = text_default("nodef", None, false);
    let state = state_with_form(form(&[with_default, secret, no_default], &[], &[]));
    let mut session = TuiSession::default();
    let (terminal, geometry) = draw(&mut session, &state, 120, 32);

    let with_index = parameter_index(&state, "withdef");
    let secret_index = parameter_index(&state, "sekret");
    let none_index = parameter_index(&state, "nodef");
    assert!(reset_hit(&geometry, with_index).is_some());
    assert!(reset_hit(&geometry, secret_index).is_none());
    assert!(reset_hit(&geometry, none_index).is_none());
    let text = rendered(terminal.backend().buffer());
    assert_eq!(text.matches("↺ default").count(), 1, "{text}");
}

#[test]
fn test_choice_default_outside_its_choices_gets_no_chip_and_no_ctrl_o() {
    let mut off = ParamDecl::new("env");
    off.parameter_type = ParameterType::Choice;
    off.choices = vec!["dev".to_owned(), "prod".to_owned()];
    off.default = Some(ParameterValue::String("staging".to_owned()));
    let off_state = state_with_form(form(&[off], &[], &[]));
    let off_index = parameter_index(&off_state, "env");
    let mut off_session = TuiSession::default();
    let (off_terminal, off_geometry) = draw(&mut off_session, &off_state, 100, 24);
    assert!(reset_hit(&off_geometry, off_index).is_none());
    let off_text = rendered(off_terminal.backend().buffer());
    assert!(!off_text.contains("↺ default"), "{off_text}");
    assert!(!off_text.contains("Ctrl+O"), "{off_text}");

    let mut on = ParamDecl::new("env");
    on.parameter_type = ParameterType::Choice;
    on.choices = vec!["dev".to_owned(), "prod".to_owned()];
    on.default = Some(ParameterValue::String("dev".to_owned()));
    let mut on_state = state_with_form(form(&[on], &[("env", "prod")], &[]));
    let on_index = parameter_index(&on_state, "env");
    let mut on_session = TuiSession::default();
    let (on_terminal, on_geometry) = draw(&mut on_session, &on_state, 100, 24);
    assert!(reset_hit(&on_geometry, on_index).is_some());
    let on_text = rendered(on_terminal.backend().buffer());
    assert!(on_text.contains("↺ default"), "{on_text}");
    assert!(on_text.contains("Ctrl+O"), "{on_text}");
    on_state.update(Action::ResetRunField(on_index));
    assert_eq!(parameter_value(&on_state, "env"), "dev");
}

#[test]
fn test_footer_advertises_ctrl_o_only_when_some_field_is_resettable() {
    let resettable = state_with_form(form(
        &[text_default("g", Some("h"), false)],
        &[],
        &[],
    ));
    let none = state_with_form(form(
        &[
            text_default("s", Some("x"), true),
            text_default("p", None, false),
        ],
        &[],
        &[],
    ));

    for (state, expected) in [(&resettable, true), (&none, false)] {
        let mut session = TuiSession::default();
        let (terminal, _) = draw(&mut session, state, 110, 28);
        let text = rendered(terminal.backend().buffer());
        assert_eq!(text.contains("Ctrl+O"), expected, "{text}");
        assert_eq!(text.contains("Reset to default"), expected, "{text}");
    }
}

#[test]
fn test_ctrl_o_on_field_without_default_leaves_value_unchanged() {
    let declaration = text_default("plain", None, false);
    let mut state = state_with_form(form(&[declaration], &[("plain", "typed")], &[]));
    let field = parameter_index(&state, "plain");
    state.update(Action::FocusField(field));
    let mut session = TuiSession::default();
    let (_, geometry) = draw(&mut session, &state, 100, 24);

    assert_eq!(
        drive(
            &mut session,
            &mut state,
            &geometry,
            key(KeyCode::Char('o'), KeyModifiers::CONTROL),
        ),
        EventHandling::Action(Action::ResetFocusedRunField)
    );
    state.update(Action::ResetRunField(usize::MAX));
    assert_eq!(parameter_value(&state, "plain"), "typed");
}

#[test]
fn test_ctrl_o_with_focus_outside_any_field_row_is_a_no_op() {
    let greeting = text_default("greeting", Some("hello"), false);
    let mut state = state_with_form(form(
        &[greeting],
        &[("greeting", "world")],
        &["claude".to_owned()],
    ));
    // The runner is the first run-form control but has no definition default.
    state.update(Action::FocusField(0));
    let mut session = TuiSession::default();
    let (_, geometry) = draw(&mut session, &state, 100, 26);

    assert_eq!(
        drive(
            &mut session,
            &mut state,
            &geometry,
            key(KeyCode::Char('o'), KeyModifiers::CONTROL),
        ),
        EventHandling::Action(Action::ResetFocusedRunField)
    );
    assert_eq!(parameter_value(&state, "greeting"), "world");
}

#[test]
fn test_input_binding_field_renders_the_ask_in_terminal_hint() {
    let mut question = ParamDecl::new("q");
    question.binding = ParameterBinding::Input;
    let state = state_with_form(form(&[question], &[], &[]));
    let mut session = TuiSession::default();
    let (terminal, _) = draw(&mut session, &state, 100, 24);
    let text = rendered(terminal.backend().buffer());

    assert!(text.contains(INPUT_BINDING_HINT), "{text}");
    assert_eq!(text.matches("ask you in the terminal").count(), 1, "{text}");
}

#[test]
fn test_plain_const_field_renders_no_input_binding_hint() {
    let state = state_with_form(form(&[ParamDecl::new("c")], &[], &[]));
    let mut session = TuiSession::default();
    let (terminal, _) = draw(&mut session, &state, 100, 24);
    let text = rendered(terminal.backend().buffer());

    assert!(!text.contains(INPUT_BINDING_HINT), "{text}");
    assert!(!text.contains("ask you in the terminal"), "{text}");
}
