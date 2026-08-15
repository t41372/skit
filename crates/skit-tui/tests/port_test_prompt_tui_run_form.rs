use std::collections::BTreeMap;

use ratatui_core::{backend::TestBackend, buffer::Buffer, terminal::Terminal};
use ratatui_crossterm::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use skit_domain::parameters::ParamDecl;
use skit_i18n::Locale;
use skit_tui::{EventHandling, TuiSession, ViewGeometry, render_with_session};
use skit_ui::{
    Action, Effect, FieldValue, FormControl, FormPurpose, LibraryState, RunFormView, Screen,
    TypedValue,
};

fn form(declarations: &[ParamDecl], runners: &[&str], default: &str) -> RunFormView {
    RunFormView::from_declarations(
        "p",
        "Prompt",
        declarations,
        &BTreeMap::new(),
        &runners
            .iter()
            .map(|value| (*value).to_owned())
            .collect::<Vec<_>>(),
        default,
        &BTreeMap::new(),
        "",
    )
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
        .draw(|frame| geometry = render_with_session(frame, state, Locale::En, session))
        .unwrap();
    (terminal, geometry)
}

fn key(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
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
) -> EventHandling {
    let handling = session.handle_event(event, state, geometry);
    if let EventHandling::Action(action) = &handling {
        let _ = state.update(action.clone());
    }
    handling
}

fn submitted_runner(effect: Effect) -> String {
    let Effect::Submit {
        purpose: FormPurpose::Run,
        values,
        ..
    } = effect
    else {
        panic!("run form did not submit: {effect:?}");
    };
    match values.get("_skit_runner").expect("runner field") {
        FieldValue::Explicit(TypedValue::Choice(value)) => value.clone(),
        other => panic!("runner lost typed choice semantics: {other:?}"),
    }
}

fn buffer_text(buffer: &Buffer) -> String {
    let mut output = String::new();
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            output.push_str(buffer[(x, y)].symbol());
        }
        output.push('\n');
    }
    output
}

fn row_containing(buffer: &Buffer, needle: &str) -> u16 {
    (0..buffer.area.height)
        .find(|y| {
            let mut row = String::new();
            for x in 0..buffer.area.width {
                row.push_str(buffer[(x, *y)].symbol());
            }
            row.contains(needle)
        })
        .unwrap_or_else(|| panic!("missing rendered row {needle:?}"))
}

#[test]
fn test_form_picker_defaults_to_the_pin_and_submits_it() {
    let mut state = state_with_form(form(
        &[ParamDecl::new("a")],
        &["claude", "codex"],
        "codex",
    ));
    assert_eq!(state.run_form().unwrap().fields()[0].control.value(), "codex");
    state.update(Action::SetFieldValue {
        field: 1,
        value: "1".to_owned(),
    });
    assert_eq!(submitted_runner(state.update(Action::Submit)), "codex");
}

#[test]
fn test_form_picker_keyboard_pick_runs_and_remembers() {
    let mut state = state_with_form(form(
        &[ParamDecl::new("a")],
        &["claude", "codex"],
        "claude",
    ));
    state.update(Action::SetFieldValue {
        field: 1,
        value: "1".to_owned(),
    });
    state.update(Action::FocusField(0));
    let mut session = TuiSession::default();
    let (_, geometry) = draw(&mut session, &state, 90, 24);
    assert_eq!(
        drive(&mut session, &mut state, &geometry, key(KeyCode::Enter)),
        EventHandling::Consumed
    );
    let (_, geometry) = draw(&mut session, &state, 90, 24);
    drive(&mut session, &mut state, &geometry, key(KeyCode::Down));
    drive(&mut session, &mut state, &geometry, key(KeyCode::Enter));
    assert_eq!(state.run_form().unwrap().fields()[0].control.value(), "codex");
    assert_eq!(submitted_runner(state.update(Action::Submit)), "codex");
}

#[test]
fn test_form_picker_mouse_click_picks_a_runner() {
    let mut state = state_with_form(form(
        &[ParamDecl::new("a")],
        &["claude", "codex"],
        "claude",
    ));
    state.update(Action::FocusField(0));
    let mut session = TuiSession::default();
    let (_, geometry) = draw(&mut session, &state, 90, 24);
    drive(&mut session, &mut state, &geometry, key(KeyCode::Enter));

    let (terminal, geometry) = draw(&mut session, &state, 90, 24);
    let rendered = buffer_text(terminal.backend().buffer());
    assert!(rendered.contains("codex"), "runner option not rendered: {rendered}");
    let row = row_containing(terminal.backend().buffer(), "codex");
    drive(&mut session, &mut state, &geometry, mouse(8, row));
    assert_eq!(state.run_form().unwrap().fields()[0].control.value(), "codex");
}

#[test]
fn test_prompt_with_no_placeholders_still_shows_the_form_for_the_picker() {
    let state = state_with_form(form(&[], &["claude", "codex"], "claude"));
    let run = state.run_form().unwrap();
    assert!(!run.has_parameters());
    assert!(run.has_runner_picker());
    assert!(
        run.fields()
            .iter()
            .any(|field| matches!(field.control, FormControl::Choice(_)) && field.key == "_skit_runner")
    );
    let mut session = TuiSession::default();
    let (terminal, _) = draw(&mut session, &state, 90, 20);
    let rendered = buffer_text(terminal.backend().buffer());
    assert!(
        rendered.contains("Runner") && rendered.contains("claude"),
        "promptless prompt lost runner form: {rendered}"
    );
}

#[test]
fn test_unicode_placeholder_is_a_working_tui_field() {
    let declaration = ParamDecl::new("目标");
    let mut state = state_with_form(form(&[declaration], &["claude"], "claude"));
    let index = state
        .run_form()
        .unwrap()
        .fields()
        .iter()
        .position(|field| field.key == "value:目标")
        .expect("unicode field");
    state.update(Action::SetFieldValue {
        field: index,
        value: "src/主程式.py".to_owned(),
    });
    let Effect::Submit { values, .. } = state.update(Action::Submit) else {
        panic!("unicode prompt form did not submit");
    };
    assert_eq!(values.get("value:目标").unwrap().as_text(), "src/主程式.py");
}
