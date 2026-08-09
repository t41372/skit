use std::collections::BTreeMap;
use std::fs;

use ratatui_core::{backend::TestBackend, buffer::Buffer, style::Color, terminal::Terminal};
use ratatui_crossterm::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use skit_application::tokens::TokenContext;
use skit_domain::parameters::{ParamDecl, ParameterType, ParameterValue};
use skit_i18n::Locale;
use skit_tui::{EventHandling, HitTarget, TuiSession, ViewGeometry, render_with_session};
use skit_ui::{
    Action, Effect, FormControl, FormField, FormPurpose, FormView, LibraryState, ModalState,
    RunFormContext, RunFormView, RunPathContext, Screen, UiCommand,
};

const ACCENT: Color = Color::Rgb(0xD9, 0x77, 0x57);
const BOX_MAROON: Color = Color::Rgb(0x92, 0x35, 0x35);

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

fn form() -> RunFormView {
    let mut enabled = ParamDecl::new("enabled");
    enabled.parameter_type = ParameterType::Bool;
    enabled.default = Some(ParameterValue::Bool(false));
    enabled.prompt = "Enable upload?".to_owned();

    let mut format = ParamDecl::new("format");
    format.parameter_type = ParameterType::Choice;
    format.choices = vec!["json".to_owned(), "yaml".to_owned()];
    format.default = Some(ParameterValue::String("json".to_owned()));

    let name = ParamDecl::new("name");
    RunFormView::from_declarations(
        "demo",
        "Demo",
        &[name, enabled, format],
        &BTreeMap::from([("name".to_owned(), "Hllo".to_owned())]),
        &["claude".to_owned(), "codex".to_owned()],
        "claude",
        &BTreeMap::new(),
        "--verbose",
    )
}

fn text_run_form(value: &str) -> RunFormView {
    RunFormView::from_declarations(
        "unicode",
        "Unicode",
        &[ParamDecl::new("value")],
        &BTreeMap::from([("value".to_owned(), value.to_owned())]),
        &[],
        "",
        &BTreeMap::new(),
        "",
    )
}

fn buffer_text(buffer: &Buffer) -> String {
    buffer.content().iter().map(|cell| cell.symbol()).collect()
}

fn complete_surface_form() -> RunFormView {
    let mut count = ParamDecl::new("count");
    count.prompt = "Count".to_owned();
    count.parameter_type = ParameterType::Int;
    count.required = true;
    count.default = Some(ParameterValue::Integer(3));

    let mut path = ParamDecl::new("path");
    path.prompt = "Output".to_owned();
    path.parameter_type = ParameterType::Path;
    path.default = Some(ParameterValue::String("result.txt".to_owned()));
    path.multiple = true;

    let mut secret = ParamDecl::new("token");
    secret.prompt = "Token".to_owned();
    secret.secret = true;
    secret.env_source = "DEMO_TOKEN".to_owned();

    let mut form = RunFormView::from_declarations(
        "complete",
        "Complete",
        &[count, path, secret],
        &BTreeMap::from([
            ("count".to_owned(), "not-a-number".to_owned()),
            ("path".to_owned(), "{cwd}/*.rs".to_owned()),
        ]),
        &["claude".to_owned()],
        "claude",
        &BTreeMap::new(),
        "",
    )
    .with_context(RunFormContext {
        entry_kind: "python".to_owned(),
        path: Some(RunPathContext {
            workdir: "/child".to_owned(),
            invoke_cwd: "/invoke".to_owned(),
        }),
        tokens: TokenContext {
            cwd: "/invoke".to_owned(),
            home: Some("/home/demo".to_owned()),
            env: BTreeMap::new(),
            today: "2026-08-09".to_owned(),
            now: "10-11-12".to_owned(),
        },
    });
    form.drift_lines = vec!["The stored field moved in the source.".to_owned()];
    form.degraded_reason = Some("subparsers".to_owned());
    form
}

fn row_containing(buffer: &Buffer, needle: &str) -> u16 {
    (0..buffer.area.height)
        .find(|row| {
            (0..buffer.area.width)
                .map(|column| buffer[(column, *row)].symbol())
                .collect::<String>()
                .contains(needle)
        })
        .expect("expected rendered row")
}

#[test]
fn mature_input_edits_unicode_at_the_cursor_and_renders_focus_style() {
    let mut state = state_with_form(form());
    state.update(Action::FocusField(1));
    let mut session = TuiSession::default();
    let (_, geometry) = draw(&mut session, &state, 100, 28);

    assert_eq!(
        drive(
            &mut session,
            &mut state,
            &geometry,
            key(KeyCode::Home, KeyModifiers::NONE),
        ),
        EventHandling::Consumed
    );
    drive(
        &mut session,
        &mut state,
        &geometry,
        key(KeyCode::Right, KeyModifiers::NONE),
    );
    drive(
        &mut session,
        &mut state,
        &geometry,
        key(KeyCode::Char('e'), KeyModifiers::NONE),
    );
    drive(
        &mut session,
        &mut state,
        &geometry,
        key(KeyCode::Char('界'), KeyModifiers::NONE),
    );

    assert_eq!(
        state.run_form().unwrap().fields()[1].control.value(),
        "He界llo"
    );
    let (terminal, geometry) = draw(&mut session, &state, 100, 28);
    let input = geometry
        .hits
        .iter()
        .find(|hit| hit.action == HitTarget::FocusField(1))
        .unwrap();
    let cursor = terminal.backend().cursor_position();
    assert!(
        input.rect.contains(cursor),
        "the mature input must expose its real terminal cursor"
    );
    let cells = terminal.backend().buffer().content();
    assert!(
        cells
            .iter()
            .any(|cell| cell.symbol() == "│" && cell.fg == ACCENT),
        "the focused mature input must show its accent border"
    );
    assert!(
        cells
            .iter()
            .any(|cell| cell.symbol() == "╭" && cell.fg == BOX_MAROON),
        "the launch form must keep the main rounded maroon panel"
    );
}

#[test]
fn run_surface_renders_main_banners_field_affordances_feedback_and_errors() {
    let mut state = state_with_form(complete_surface_form());
    state.update(Action::SetRunGlobCount {
        field: 2,
        value: "{cwd}/*.rs".to_owned(),
        count: 2,
    });
    assert_eq!(state.update(Action::Submit), Effect::None);
    let mut session = TuiSession::default();
    let (terminal, _) = draw(&mut session, &state, 120, 44);
    let rendered = buffer_text(terminal.backend().buffer());

    for expected in [
        "The stored field moved in the source.",
        "This script has subcommands skit can't model",
        "none yet — fill the form and press Ctrl+S to save one",
        "Count",
        "required",
        "whole number",
        "Count needs a whole number — you typed 'not-a-number'.",
        "browse",
        "▾ insert",
        "↺ default",
        "→ /invoke/*.rs",
        "✓ matches 2 file(s)",
        "never saved to disk",
        "Ctrl+N New agent…",
    ] {
        assert!(
            rendered.contains(expected),
            "missing run-form copy: {expected}"
        );
    }
    for icon in ["📁", "🔒"] {
        assert!(
            terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .any(|cell| cell.symbol() == icon),
            "missing run-form icon: {icon}"
        );
    }
}

#[test]
fn every_visible_run_field_affordance_has_a_typed_mouse_action() {
    let state = state_with_form(complete_surface_form());
    let mut session = TuiSession::default();
    let (_, geometry) = draw(&mut session, &state, 120, 44);

    for (command, expected) in [
        (UiCommand::BrowsePath, Action::OpenRunFilePicker(2)),
        (UiCommand::InsertValue, Action::OpenRunTokenMenuFor(2)),
        (UiCommand::ResetDefault, Action::ResetRunField(2)),
    ] {
        let area = geometry
            .hits
            .iter()
            .find_map(|hit| {
                (hit.action == HitTarget::RunFieldCommand { field: 2, command }).then_some(hit.rect)
            })
            .expect("the visible field chip must expose its typed click region");
        assert_eq!(
            session.handle_event(mouse(area.x, area.y), &state, &geometry),
            EventHandling::Action(expected)
        );
    }

    let area = geometry
        .hits
        .iter()
        .find_map(|hit| {
            (hit.action == HitTarget::Command(UiCommand::NewRunner)).then_some(hit.rect)
        })
        .expect("the runner picker must show its New agent mouse door");
    assert_eq!(
        session.handle_event(mouse(area.x, area.y), &state, &geometry),
        EventHandling::Action(Action::OpenRunRunnerEditor)
    );
}

#[test]
fn checkbox_radio_and_picker_have_keyboard_and_mouse_paths() {
    let mut state = state_with_form(form());
    let mut session = TuiSession::default();
    let (terminal, geometry) = draw(&mut session, &state, 100, 28);

    let checkbox = geometry
        .hits
        .iter()
        .find(|hit| hit.action == HitTarget::FocusField(2))
        .unwrap();
    drive(
        &mut session,
        &mut state,
        &geometry,
        mouse(checkbox.rect.x, checkbox.rect.y),
    );
    assert!(matches!(
        state.run_form().unwrap().fields()[2].control,
        FormControl::Checkbox { checked: true }
    ));
    assert_eq!(state.focused_form_field(), Some(2));

    state.update(Action::FocusField(3));
    let (_, geometry) = draw(&mut session, &state, 100, 28);
    drive(
        &mut session,
        &mut state,
        &geometry,
        key(KeyCode::Right, KeyModifiers::NONE),
    );
    assert_eq!(
        state.run_form().unwrap().fields()[3].control.value(),
        "yaml"
    );

    state.update(Action::FocusField(0));
    let (_, geometry) = draw(&mut session, &state, 100, 28);
    assert_eq!(
        drive(
            &mut session,
            &mut state,
            &geometry,
            key(KeyCode::Enter, KeyModifiers::NONE),
        ),
        EventHandling::Consumed
    );
    drive(
        &mut session,
        &mut state,
        &geometry,
        key(KeyCode::Down, KeyModifiers::NONE),
    );
    drive(
        &mut session,
        &mut state,
        &geometry,
        key(KeyCode::Enter, KeyModifiers::NONE),
    );
    assert_eq!(
        state.run_form().unwrap().fields()[0].control.value(),
        "codex"
    );

    assert!(buffer_text(terminal.backend().buffer()).contains("Enable upload?"));
}

#[test]
fn focus_auto_scrolls_a_long_form_and_wheel_scroll_uses_the_shared_viewport() {
    let declarations = (0..16)
        .map(|index| ParamDecl::new(format!("field-{index}")))
        .collect::<Vec<_>>();
    let form = RunFormView::from_declarations(
        "long",
        "Long form",
        &declarations,
        &BTreeMap::new(),
        &[],
        "",
        &BTreeMap::new(),
        "",
    );
    let mut state = state_with_form(form);
    state.update(Action::FocusField(15));
    let mut session = TuiSession::default();
    let (terminal, geometry) = draw(&mut session, &state, 70, 18);

    let text = buffer_text(terminal.backend().buffer());
    assert!(
        text.contains("field-15"),
        "focused controls must auto-scroll into view"
    );
    assert!(geometry.first_visible > 0);

    let handling = session.handle_event(
        Event::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: geometry.rows.x,
            row: geometry.rows.y,
            modifiers: KeyModifiers::NONE,
        }),
        &state,
        &geometry,
    );
    assert_eq!(handling, EventHandling::Consumed);
    let (_, after) = draw(&mut session, &state, 70, 18);
    assert!(after.first_visible < geometry.first_visible);
}

#[test]
fn single_line_input_moves_and_deletes_complete_graphemes() {
    let mut state = state_with_form(text_run_form("e\u{301}👨‍👩‍👧‍👦x"));
    let mut session = TuiSession::default();
    let (_, geometry) = draw(&mut session, &state, 80, 18);

    drive(
        &mut session,
        &mut state,
        &geometry,
        key(KeyCode::Home, KeyModifiers::NONE),
    );
    drive(
        &mut session,
        &mut state,
        &geometry,
        key(KeyCode::Right, KeyModifiers::NONE),
    );
    drive(
        &mut session,
        &mut state,
        &geometry,
        key(KeyCode::Delete, KeyModifiers::NONE),
    );

    assert_eq!(
        state.run_form().unwrap().fields()[0].control.value(),
        "e\u{301}x",
        "one Delete must remove the complete ZWJ family after one grapheme move"
    );
}

#[test]
fn run_shortcuts_keep_submit_and_preset_save_distinct() {
    let state = state_with_form(text_run_form("value"));
    let mut session = TuiSession::default();
    let (_, geometry) = draw(&mut session, &state, 80, 18);

    assert_eq!(
        session.handle_event(
            key(KeyCode::Char('r'), KeyModifiers::CONTROL),
            &state,
            &geometry,
        ),
        EventHandling::Action(Action::Submit)
    );
    assert_eq!(
        session.handle_event(
            key(KeyCode::Char('s'), KeyModifiers::CONTROL),
            &state,
            &geometry,
        ),
        EventHandling::Action(Action::OpenRunPresetSave)
    );
    assert_eq!(
        session.handle_event(
            key(KeyCode::Char('t'), KeyModifiers::CONTROL),
            &state,
            &geometry,
        ),
        EventHandling::Action(Action::OpenRunTokenMenu)
    );
}

#[test]
fn preset_name_modal_is_unicode_editable_and_submits_the_exact_snapshot() {
    let form = RunFormView::from_declarations(
        "demo",
        "Demo",
        &[ParamDecl::new("name")],
        &BTreeMap::from([("name".to_owned(), "Ada".to_owned())]),
        &[],
        "",
        &BTreeMap::from([("夜間".to_owned(), BTreeMap::new())]),
        "",
    );
    let mut state = state_with_form(form);
    let mut session = TuiSession::default();
    let (_, geometry) = draw(&mut session, &state, 80, 20);
    drive(
        &mut session,
        &mut state,
        &geometry,
        key(KeyCode::Char('s'), KeyModifiers::CONTROL),
    );
    assert!(matches!(
        state.modal(),
        Some(ModalState::RunPresetName { .. })
    ));

    let (_, geometry) = draw(&mut session, &state, 80, 20);
    for character in "夜間".chars() {
        drive(
            &mut session,
            &mut state,
            &geometry,
            key(KeyCode::Char(character), KeyModifiers::NONE),
        );
    }
    assert!(matches!(
        state.modal(),
        Some(ModalState::RunPresetName { value, .. }) if value == "夜間"
    ));
    let (terminal, geometry) = draw(&mut session, &state, 80, 20);
    let rendered = buffer_text(terminal.backend().buffer());
    assert!(rendered.contains("Save as preset"));
    assert!(
        rendered.contains("This overwrites the existing preset")
            && rendered.contains('夜')
            && rendered.contains('間'),
        "{rendered}"
    );

    let EventHandling::Action(action) =
        session.handle_event(key(KeyCode::Enter, KeyModifiers::NONE), &state, &geometry)
    else {
        panic!("Enter did not submit the preset name");
    };
    assert_eq!(
        state.update(action),
        Effect::SaveRunPreset {
            selector: "demo".to_owned(),
            name: "夜間".to_owned(),
            values: BTreeMap::from([("name".to_owned(), "Ada".to_owned())]),
            secret_names: Default::default(),
        }
    );
}

#[test]
fn token_list_picker_has_keyboard_and_mouse_insertion_paths() {
    let form = text_run_form("prefix ").with_context(RunFormContext {
        entry_kind: "python".to_owned(),
        path: Some(RunPathContext {
            workdir: "/work".to_owned(),
            invoke_cwd: "/invoke".to_owned(),
        }),
        tokens: TokenContext {
            cwd: "/invoke".to_owned(),
            home: Some("/home/alice".to_owned()),
            env: BTreeMap::new(),
            today: "2026-08-08".to_owned(),
            now: "12-00-00".to_owned(),
        },
    });
    let mut state = state_with_form(form);
    let mut session = TuiSession::default();
    let (_, geometry) = draw(&mut session, &state, 80, 24);
    drive(
        &mut session,
        &mut state,
        &geometry,
        key(KeyCode::Char('t'), KeyModifiers::CONTROL),
    );

    let (terminal, geometry) = draw(&mut session, &state, 80, 24);
    let row = row_containing(
        terminal.backend().buffer(),
        "Directory at run time (changes with where you run)",
    );
    assert_eq!(
        drive(&mut session, &mut state, &geometry, mouse(10, row)),
        EventHandling::Action(Action::SetRunFieldValueAndCloseModal {
            field: 0,
            value: "prefix {cwd}".to_owned(),
        })
    );
    assert!(state.modal().is_none());
    assert_eq!(
        state.run_form().unwrap().fields()[0].control.value(),
        "prefix {cwd}"
    );

    let (_, geometry) = draw(&mut session, &state, 80, 24);
    drive(
        &mut session,
        &mut state,
        &geometry,
        key(KeyCode::Char('t'), KeyModifiers::CONTROL),
    );
    let (_, geometry) = draw(&mut session, &state, 80, 24);
    assert_eq!(
        session.handle_event(key(KeyCode::Enter, KeyModifiers::NONE), &state, &geometry),
        EventHandling::Action(Action::SetRunFieldValueAndCloseModal {
            field: 0,
            value: "prefix {cwd}{cwd}".to_owned(),
        })
    );
}

#[test]
fn environment_picker_fuzzy_filters_and_accepts_keyboard_or_mouse_values() {
    let form = text_run_form("").with_context(RunFormContext {
        entry_kind: "python".to_owned(),
        path: Some(RunPathContext {
            workdir: "/work".to_owned(),
            invoke_cwd: "/invoke".to_owned(),
        }),
        tokens: TokenContext {
            cwd: "/invoke".to_owned(),
            home: Some("/home/alice".to_owned()),
            env: BTreeMap::from([
                ("HOME".to_owned(), "/home/alice".to_owned()),
                ("HTTP_PROXY".to_owned(), "http://proxy".to_owned()),
                ("SKIT_PROFILE".to_owned(), "test".to_owned()),
            ]),
            today: "2026-08-08".to_owned(),
            now: "12-00-00".to_owned(),
        },
    });
    let mut state = state_with_form(form);
    let mut session = TuiSession::default();
    let (_, geometry) = draw(&mut session, &state, 80, 24);
    drive(
        &mut session,
        &mut state,
        &geometry,
        key(KeyCode::Char('t'), KeyModifiers::CONTROL),
    );
    let (_, geometry) = draw(&mut session, &state, 80, 24);
    for _ in 0..5 {
        drive(
            &mut session,
            &mut state,
            &geometry,
            key(KeyCode::Down, KeyModifiers::NONE),
        );
    }
    drive(
        &mut session,
        &mut state,
        &geometry,
        key(KeyCode::Enter, KeyModifiers::NONE),
    );
    assert!(matches!(
        state.modal(),
        Some(ModalState::RunEnvironmentPicker { .. })
    ));

    let (_, geometry) = draw(&mut session, &state, 80, 24);
    for character in "hme".chars() {
        drive(
            &mut session,
            &mut state,
            &geometry,
            key(KeyCode::Char(character), KeyModifiers::NONE),
        );
    }
    assert!(matches!(
        state.modal(),
        Some(ModalState::RunEnvironmentPicker { visible, .. }) if visible == &["HOME".to_owned()]
    ));
    let (terminal, geometry) = draw(&mut session, &state, 80, 24);
    let home_row = row_containing(terminal.backend().buffer(), "HOME");
    drive(&mut session, &mut state, &geometry, mouse(12, home_row));
    assert_eq!(
        state.run_form().unwrap().fields()[0].control.value(),
        "{env:HOME}"
    );

    let (_, geometry) = draw(&mut session, &state, 80, 24);
    drive(
        &mut session,
        &mut state,
        &geometry,
        key(KeyCode::Char('t'), KeyModifiers::CONTROL),
    );
    let (_, geometry) = draw(&mut session, &state, 80, 24);
    for _ in 0..5 {
        drive(
            &mut session,
            &mut state,
            &geometry,
            key(KeyCode::Down, KeyModifiers::NONE),
        );
    }
    drive(
        &mut session,
        &mut state,
        &geometry,
        key(KeyCode::Enter, KeyModifiers::NONE),
    );
    let (_, geometry) = draw(&mut session, &state, 80, 24);
    for character in "UNSET_NAME".chars() {
        drive(
            &mut session,
            &mut state,
            &geometry,
            key(KeyCode::Char(character), KeyModifiers::NONE),
        );
    }
    assert!(matches!(
        state.modal(),
        Some(ModalState::RunEnvironmentPicker { visible, .. }) if visible.is_empty()
    ));
    drive(
        &mut session,
        &mut state,
        &geometry,
        key(KeyCode::Enter, KeyModifiers::NONE),
    );
    assert_eq!(
        state.run_form().unwrap().fields()[0].control.value(),
        "{env:HOME}{env:UNSET_NAME}"
    );
}

#[test]
fn run_file_picker_uses_the_shared_explorer_for_keyboard_mouse_and_missing_roots() {
    let temp = tempfile::tempdir().unwrap();
    let workdir = temp.path().join("work");
    fs::create_dir(&workdir).unwrap();
    fs::write(workdir.join("alpha.txt"), "alpha").unwrap();
    fs::write(workdir.join("beta file*.txt"), "beta").unwrap();
    let mut path = ParamDecl::new("path");
    path.parameter_type = ParameterType::Path;
    let form = RunFormView::from_declarations(
        "files",
        "Files",
        &[path],
        &BTreeMap::from([("path".to_owned(), "old.txt".to_owned())]),
        &[],
        "",
        &BTreeMap::new(),
        "",
    )
    .with_context(RunFormContext {
        entry_kind: "python".to_owned(),
        path: Some(RunPathContext {
            workdir: workdir.display().to_string(),
            invoke_cwd: temp.path().display().to_string(),
        }),
        tokens: TokenContext {
            cwd: temp.path().display().to_string(),
            home: None,
            env: BTreeMap::new(),
            today: "2026-08-08".to_owned(),
            now: "12-00-00".to_owned(),
        },
    });
    let mut state = state_with_form(form);
    let mut session = TuiSession::default();
    let (_, geometry) = draw(&mut session, &state, 84, 26);
    drive(
        &mut session,
        &mut state,
        &geometry,
        key(KeyCode::Char('t'), KeyModifiers::CONTROL),
    );
    let (_, geometry) = draw(&mut session, &state, 84, 26);
    drive(
        &mut session,
        &mut state,
        &geometry,
        key(KeyCode::Enter, KeyModifiers::NONE),
    );
    assert!(matches!(
        state.modal(),
        Some(ModalState::RunFilePicker { .. })
    ));

    let (terminal, geometry) = draw(&mut session, &state, 84, 26);
    let rendered = buffer_text(terminal.backend().buffer());
    assert!(rendered.contains("alpha.txt"), "{rendered}");
    for character in "alpha".chars() {
        drive(
            &mut session,
            &mut state,
            &geometry,
            key(KeyCode::Char(character), KeyModifiers::NONE),
        );
    }
    let (_, geometry) = draw(&mut session, &state, 84, 26);
    drive(
        &mut session,
        &mut state,
        &geometry,
        key(KeyCode::Enter, KeyModifiers::NONE),
    );
    assert_eq!(
        state.run_form().unwrap().fields()[0].control.value(),
        "alpha.txt"
    );

    let (_, geometry) = draw(&mut session, &state, 84, 26);
    drive(
        &mut session,
        &mut state,
        &geometry,
        key(KeyCode::Char('t'), KeyModifiers::CONTROL),
    );
    let (_, geometry) = draw(&mut session, &state, 84, 26);
    drive(
        &mut session,
        &mut state,
        &geometry,
        key(KeyCode::Enter, KeyModifiers::NONE),
    );
    let (terminal, geometry) = draw(&mut session, &state, 84, 26);
    let beta_row = row_containing(terminal.backend().buffer(), "beta file*.txt");
    drive(&mut session, &mut state, &geometry, mouse(12, beta_row));
    assert_eq!(
        state.run_form().unwrap().fields()[0].control.value(),
        "beta file*.txt"
    );

    let missing = temp.path().join("gone").join("child");
    let mut path = ParamDecl::new("path");
    path.parameter_type = ParameterType::Path;
    let missing_form = RunFormView::from_declarations(
        "missing",
        "Missing",
        &[path],
        &BTreeMap::new(),
        &[],
        "",
        &BTreeMap::new(),
        "",
    )
    .with_context(RunFormContext {
        entry_kind: "python".to_owned(),
        path: Some(RunPathContext {
            workdir: missing.display().to_string(),
            invoke_cwd: temp.path().display().to_string(),
        }),
        tokens: TokenContext {
            cwd: temp.path().display().to_string(),
            home: None,
            env: BTreeMap::new(),
            today: "2026-08-08".to_owned(),
            now: "12-00-00".to_owned(),
        },
    });
    let mut state = state_with_form(missing_form);
    let (_, geometry) = draw(&mut session, &state, 84, 26);
    drive(
        &mut session,
        &mut state,
        &geometry,
        key(KeyCode::Char('t'), KeyModifiers::CONTROL),
    );
    let (_, geometry) = draw(&mut session, &state, 84, 26);
    drive(
        &mut session,
        &mut state,
        &geometry,
        key(KeyCode::Enter, KeyModifiers::NONE),
    );
    let (terminal, _) = draw(&mut session, &state, 84, 26);
    assert!(
        buffer_text(terminal.backend().buffer())
            .contains("The entry's working directory is missing — starting here instead.")
    );
}

#[test]
fn multiline_input_supports_selection_replacement_and_undo() {
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Form(FormView {
        purpose: FormPurpose::Settings,
        title: "Edit description".to_owned(),
        title_arguments: Vec::new(),
        translate_title: false,
        selector: None,
        fields: vec![FormField::multiline("description", "Description", "ab")],
        focused: 0,
        submit_label: "Save".to_owned(),
    })));
    let mut session = TuiSession::default();
    let (_, geometry) = draw(&mut session, &state, 80, 18);

    drive(
        &mut session,
        &mut state,
        &geometry,
        key(KeyCode::End, KeyModifiers::NONE),
    );
    drive(
        &mut session,
        &mut state,
        &geometry,
        key(KeyCode::Left, KeyModifiers::SHIFT),
    );
    drive(
        &mut session,
        &mut state,
        &geometry,
        key(KeyCode::Char('界'), KeyModifiers::NONE),
    );
    assert_eq!(state.form().unwrap().fields[0].value, "a界");

    drive(
        &mut session,
        &mut state,
        &geometry,
        key(KeyCode::Char('z'), KeyModifiers::CONTROL),
    );
    assert_eq!(state.form().unwrap().fields[0].value, "ab");
}

/// The run form the demo tape records: the `greet` argparse surface.
///
/// `docs/assets/demo/scripts/en/greet.py` declares name, count, shout and names, and
/// `docs/assets/demo/shots.tape` records it at 1280x780 with JetBrains Mono 20 and padding 20.
/// Measuring the shipped frames gives a 12.19 px column and a 26.33 px row, so the terminal is
/// 101x28 cells.
fn demo_greet_form() -> RunFormView {
    let mut name = ParamDecl::new("name");
    name.help = "Who to greet".to_owned();
    name.default = Some(ParameterValue::String("World".to_owned()));

    let mut count = ParamDecl::new("count");
    count.parameter_type = ParameterType::Int;
    count.help = "How many times to greet".to_owned();
    count.default = Some(ParameterValue::Integer(1));

    let mut shout = ParamDecl::new("shout");
    shout.parameter_type = ParameterType::Bool;
    shout.help = "Greet in UPPERCASE".to_owned();
    shout.default = Some(ParameterValue::Bool(false));

    let mut names = ParamDecl::new("names");
    names.parameter_type = ParameterType::Path;
    names.help = "Also greet everyone in this file, one per line".to_owned();

    RunFormView::from_declarations(
        "greet",
        "greet",
        &[name, count, shout, names],
        &BTreeMap::from([
            ("name".to_owned(), "Ada".to_owned()),
            ("count".to_owned(), "3".to_owned()),
            ("shout".to_owned(), "true".to_owned()),
            ("names".to_owned(), "names.txt".to_owned()),
        ]),
        &[],
        "",
        &BTreeMap::new(),
        "",
    )
    .with_context(RunFormContext {
        entry_kind: "python".to_owned(),
        path: Some(RunPathContext {
            workdir: "/demo".to_owned(),
            invoke_cwd: "/demo".to_owned(),
        }),
        tokens: TokenContext {
            cwd: "/demo".to_owned(),
            home: Some("/root".to_owned()),
            env: BTreeMap::new(),
            today: "2026-08-09".to_owned(),
            now: "12-00-00".to_owned(),
        },
    })
}

/// The demo terminal must show every field the demo is about, and say so when it cannot.
///
/// Version 0.4 fits this exact form in this exact window: the label carries its own chips
/// (`src/skit/tui_form.py:190-215` builds one `Static` from label plus browse/insert/default), the
/// preset row is one line reading `Preset: …` (`:741-757`), and the body is a `VerticalScroll`
/// whose scrollbar tells the user the content continues (`:380-384`). The Rust form spent an extra
/// row per field on a second chip line plus three rows on a duplicated title, so the path field —
/// the scene the README builds around — was off screen with nothing to say so.
#[test]
fn the_demo_run_form_fits_its_recorded_terminal_and_shows_a_scroll_affordance() {
    let state = state_with_form(demo_greet_form());
    let mut session = TuiSession::default();
    let (terminal, _) = draw(&mut session, &state, 101, 28);
    let rendered = buffer_text(terminal.backend().buffer());

    // The preset row keeps its label, exactly as version 0.4 writes it.
    assert!(
        rendered.contains("Preset: none yet"),
        "no labelled preset row:\n{rendered}"
    );
    // Every field the demo tape shows must be on screen.
    for fact in [
        "name",
        "Who to greet",
        "count",
        "How many times to greet",
        "shout",
        "Greet in UPPERCASE",
        "names",
        "names.txt",
        "Also greet everyone in this file",
    ] {
        assert!(rendered.contains(fact), "missing {fact}:\n{rendered}");
    }
    // The title belongs to the form panel alone, never twice.
    assert_eq!(
        rendered.matches("Run greet").count(),
        1,
        "the title is duplicated:\n{rendered}"
    );
    // Content still taller than the viewport must advertise itself.
    assert!(
        rendered.contains('█') || rendered.contains('▐') || rendered.contains('║'),
        "no scroll affordance:\n{rendered}"
    );

    // The tail of the form is reachable: focus moves into it and the viewport follows.
    let mut state = state;
    let (_, geometry) = draw(&mut session, &state, 101, 28);
    for _ in 1..form_field_count(&state) {
        drive(
            &mut session,
            &mut state,
            &geometry,
            key(KeyCode::Tab, KeyModifiers::NONE),
        );
    }
    let (terminal, _) = draw(&mut session, &state, 101, 28);
    let rendered = buffer_text(terminal.backend().buffer());
    assert!(
        rendered.contains("Extra arguments"),
        "the argument tail never comes into view:\n{rendered}"
    );
}

fn form_field_count(state: &LibraryState) -> usize {
    state.run_form().map_or(0, |form| form.fields().len())
}

/// A window too short for the form must still say the content continues.
#[test]
fn a_short_run_form_window_renders_a_scroll_affordance() {
    let state = state_with_form(demo_greet_form());
    let mut session = TuiSession::default();
    let (terminal, _) = draw(&mut session, &state, 101, 14);
    let rendered = buffer_text(terminal.backend().buffer());
    assert!(
        rendered.contains('█') || rendered.contains('▐') || rendered.contains('║'),
        "no scroll affordance in a short window:\n{rendered}"
    );
}

/// The footer must advertise the arrow keys, which already move between fields.
///
/// Version 0.4's shared nav chip is two key-only pills, `Tab/↓` and `Shift+Tab/↑`
/// (`src/skit/tui_footer.py:82-94`), bound to the same actions Tab and Shift+Tab fire
/// (`:76-79`). Advertising only Tab strands anyone who tabs one field too far.
#[test]
fn the_run_footer_advertises_both_navigation_directions() {
    let state = state_with_form(demo_greet_form());
    let mut session = TuiSession::default();
    let (terminal, geometry) = draw(&mut session, &state, 101, 28);
    let rendered = buffer_text(terminal.backend().buffer());
    assert!(rendered.contains("Tab/↓"), "{rendered}");
    assert!(rendered.contains("Shift+Tab/↑"), "{rendered}");

    // The advertised keys really move focus, forward and back.
    let mut state = state;
    let focused = |state: &LibraryState| state.run_form().map(RunFormView::focused);
    assert_eq!(focused(&state), Some(0));
    drive(
        &mut session,
        &mut state,
        &geometry,
        key(KeyCode::Down, KeyModifiers::NONE),
    );
    assert_eq!(focused(&state), Some(1));
    drive(
        &mut session,
        &mut state,
        &geometry,
        key(KeyCode::Up, KeyModifiers::NONE),
    );
    assert_eq!(focused(&state), Some(0));
    // Tab and Shift+Tab are the same movement the chip pairs the arrows with.
    drive(
        &mut session,
        &mut state,
        &geometry,
        key(KeyCode::Tab, KeyModifiers::NONE),
    );
    assert_eq!(focused(&state), Some(1));
    drive(
        &mut session,
        &mut state,
        &geometry,
        key(KeyCode::BackTab, KeyModifiers::SHIFT),
    );
    assert_eq!(focused(&state), Some(0));
}
