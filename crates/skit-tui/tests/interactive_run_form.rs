use std::collections::BTreeMap;
use std::fs;

use ratatui_core::{
    backend::TestBackend, buffer::Buffer, layout::Rect, style::Color, terminal::Terminal,
};
use ratatui_crossterm::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use skit_application::tokens::TokenContext;
use skit_domain::parameters::{ParamDecl, ParameterType, ParameterValue};
use skit_i18n::Locale;
use skit_tui::{
    EventHandling, FilePickerEvent, FilePickerHit, FilePickerSession, HitTarget, TuiSession,
    ViewGeometry, render_file_picker, render_with_session,
};
use skit_ui::{
    Action, Effect, FormControl, FormField, FormPurpose, FormView, LibraryState, ModalState,
    PathOutputPolicy, PathPickerState, PathSelectionMode, PickerPurpose, RunFormContext,
    RunFormView, RunPathContext, Screen, UiCommand, UiKey,
};

const ACCENT: Color = Color::Rgb(0xD9, 0x77, 0x57);
const BOX_MAROON: Color = Color::Rgb(0x92, 0x35, 0x35);

fn key(code: KeyCode, modifiers: KeyModifiers) -> Event {
    Event::Key(KeyEvent::new(code, modifiers))
}

fn mouse(column: u16, row: u16) -> Event {
    mouse_with_kind(MouseEventKind::Down(MouseButton::Left), column, row)
}

fn mouse_with_kind(kind: MouseEventKind, column: u16, row: u16) -> Event {
    Event::Mouse(MouseEvent {
        kind,
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
    draw_in_locale(session, state, width, height, Locale::En)
}

fn draw_in_locale(
    session: &mut TuiSession,
    state: &LibraryState,
    width: u16,
    height: u16,
    locale: Locale,
) -> (Terminal<TestBackend>, ViewGeometry) {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    let mut geometry = ViewGeometry::default();
    terminal
        .draw(|frame| {
            geometry = render_with_session(frame, state, locale, session);
        })
        .unwrap();
    (terminal, geometry)
}

fn assert_run_widget_owned_footer(session: &TuiSession, state: &LibraryState) {
    let keys = |command| {
        session
            .advertised_command_bindings(state, command)
            .into_iter()
            .map(|binding| binding.key)
            .collect::<Vec<_>>()
    };
    assert_eq!(keys(UiCommand::Submit), [UiKey::Character('r')]);
    assert_eq!(keys(UiCommand::FocusNext), [UiKey::Tab]);
    assert_eq!(keys(UiCommand::FocusPrevious), [UiKey::BackTab]);
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

fn with_multiline_run_field(form: RunFormView, index: usize) -> RunFormView {
    let mut value = serde_json::to_value(form).unwrap();
    value["fields"][index]["control"]["text"]["multiline"] = true.into();
    serde_json::from_value(value).unwrap()
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

fn buffer_position(buffer: &Buffer, needle: &str) -> (u16, u16) {
    for row in 0..buffer.area.height {
        let line = (0..buffer.area.width)
            .map(|column| buffer[(column, row)].symbol())
            .collect::<String>();
        if let Some(column) = line.find(needle) {
            return (u16::try_from(column).unwrap(), row);
        }
    }
    panic!("expected rendered text: {needle}");
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
            EventHandling::Action(expected.clone())
        );
        assert_eq!(
            skit_tui::map_event(mouse(area.x, area.y), &state, &geometry),
            Some(expected)
        );
        for kind in [MouseEventKind::Moved, MouseEventKind::Up(MouseButton::Left)] {
            assert_eq!(
                skit_tui::map_event(
                    Event::Mouse(MouseEvent {
                        kind,
                        column: area.x,
                        row: area.y,
                        modifiers: KeyModifiers::NONE,
                    }),
                    &state,
                    &geometry,
                ),
                None
            );
        }
    }

    let mut stale = geometry.clone();
    stale.hits.push(skit_tui::HitRegion {
        rect: Rect::new(0, 0, 1, 1),
        action: HitTarget::RunFieldCommand {
            field: 7,
            command: UiCommand::Back,
        },
    });
    assert_eq!(
        skit_tui::map_event(mouse(0, 0), &state, &stale),
        Some(Action::Back)
    );

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
        .find(|hit| hit.action == HitTarget::ToggleField(2))
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
    let yaml = geometry
        .hits
        .iter()
        .find(|hit| {
            hit.action
                == HitTarget::SelectFieldOption {
                    field: 3,
                    option: 1,
                }
        })
        .expect("the radio option must expose its exact typed mouse endpoint");
    drive(
        &mut session,
        &mut state,
        &geometry,
        mouse(yaml.rect.x, yaml.rect.y),
    );
    assert_eq!(
        state.run_form().unwrap().fields()[3].control.value(),
        "yaml"
    );
    drive(
        &mut session,
        &mut state,
        &geometry,
        key(KeyCode::Left, KeyModifiers::NONE),
    );
    assert_eq!(
        state.run_form().unwrap().fields()[3].control.value(),
        "json"
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
fn run_and_generic_form_hits_require_a_mouse_button_press() {
    let run = state_with_form(form());
    let mut session = TuiSession::default();
    let (_, geometry) = draw(&mut session, &run, 100, 28);
    let run_hit = geometry
        .hits
        .iter()
        .find(|hit| hit.action == HitTarget::ToggleField(2))
        .expect("the checkbox must expose its mouse hit area");

    for kind in [
        MouseEventKind::Moved,
        MouseEventKind::Up(MouseButton::Left),
        MouseEventKind::Drag(MouseButton::Left),
    ] {
        assert_eq!(
            session.handle_event(
                mouse_with_kind(kind, run_hit.rect.x, run_hit.rect.y),
                &run,
                &geometry,
            ),
            EventHandling::Ignored,
            "a run-form hit must ignore {kind:?}"
        );
    }
    assert_eq!(
        session.handle_event(mouse(run_hit.rect.x, run_hit.rect.y), &run, &geometry),
        EventHandling::Action(Action::ToggleField(2))
    );

    let mut form = LibraryState::default();
    form.update(Action::Present(Screen::Form(FormView {
        purpose: FormPurpose::Settings,
        title: "Edit entry".to_owned(),
        title_arguments: Vec::new(),
        translate_title: false,
        selector: None,
        fields: vec![
            FormField::text("name", "Name", "demo"),
            FormField::text("description", "Description", ""),
        ],
        focused: 0,
        submit_label: "Save".to_owned(),
    })));
    let (_, geometry) = draw(&mut session, &form, 80, 18);
    let form_hit = geometry
        .hits
        .iter()
        .find(|hit| hit.action == HitTarget::FocusField(1))
        .expect("the second form field must expose its mouse hit area");

    for kind in [
        MouseEventKind::Moved,
        MouseEventKind::Up(MouseButton::Left),
        MouseEventKind::Drag(MouseButton::Left),
    ] {
        assert_eq!(
            session.handle_event(
                mouse_with_kind(kind, form_hit.rect.x, form_hit.rect.y),
                &form,
                &geometry,
            ),
            EventHandling::Ignored,
            "a generic-form hit must ignore {kind:?}"
        );
    }
    assert_eq!(
        session.handle_event(mouse(form_hit.rect.x, form_hit.rect.y), &form, &geometry),
        EventHandling::Action(Action::FocusField(1))
    );
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
fn ctrl_t_token_menu_localizes_every_row_in_simplified_chinese() {
    let mut path = ParamDecl::new("path");
    path.parameter_type = ParameterType::Path;
    let form = RunFormView::from_declarations(
        "paths",
        "Paths",
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
            workdir: "/work".to_owned(),
            invoke_cwd: "/invoke".to_owned(),
        }),
        tokens: TokenContext {
            cwd: "/invoke".to_owned(),
            home: Some("/home/demo".to_owned()),
            env: BTreeMap::new(),
            today: "2026-08-24".to_owned(),
            now: "12-00-00".to_owned(),
        },
    });
    let mut state = state_with_form(form);
    let mut session = TuiSession::default();
    let (_, geometry) = draw(&mut session, &state, 100, 24);
    assert_eq!(
        drive(
            &mut session,
            &mut state,
            &geometry,
            key(KeyCode::Char('t'), KeyModifiers::CONTROL),
        ),
        EventHandling::Action(Action::OpenRunTokenMenu)
    );

    let (terminal, _) = draw_in_locale(&mut session, &state, 100, 24, Locale::ZhCn);
    let rendered = buffer_text(terminal.backend().buffer());
    // Ratatui's TestBackend exposes the continuation cell of each wide glyph as a space.
    let compact = rendered.replace(' ', "");
    for translated in [
        "文件或文件夹…",
        "运行时所在目录（跟着你在哪运行而变）",
        "此刻目录（固定路径）",
        "今天日期",
        "当前时间",
        "主目录",
        "环境变量…",
    ] {
        assert!(
            compact.contains(translated),
            "missing {translated}: {rendered}"
        );
    }
    for source in [
        "File or folder…",
        "Directory at run time (changes with where you run)",
        "This directory, as a fixed path",
        "Today's date",
        "Current time",
        "Home directory",
        "Environment variable…",
    ] {
        assert!(
            !rendered.contains(source),
            "untranslated {source}: {rendered}"
        );
    }
}

#[test]
fn an_active_modal_owns_its_footer_after_an_underlying_picker_was_open() {
    let mut state = state_with_form(complete_surface_form());
    let runner = state
        .run_form()
        .unwrap()
        .fields()
        .iter()
        .position(|field| matches!(field.role, skit_ui::RunFieldRole::Runner))
        .unwrap();
    let path = state
        .run_form()
        .unwrap()
        .fields()
        .iter()
        .position(|field| {
            matches!(
                &field.role,
                skit_ui::RunFieldRole::Parameter { name } if name == "path"
            )
        })
        .unwrap();
    state.update(Action::FocusField(runner));
    let mut session = TuiSession::default();
    let (_, geometry) = draw(&mut session, &state, 100, 28);
    assert_eq!(
        session.handle_event(key(KeyCode::Enter, KeyModifiers::NONE), &state, &geometry,),
        EventHandling::Consumed
    );
    assert!(
        session
            .advertised_command_bindings(&state, UiCommand::Back)
            .is_empty(),
        "the open picker owns Escape before a modal opens"
    );

    state.update(Action::OpenRunTokenMenuFor(path));
    assert!(matches!(
        state.modal(),
        Some(ModalState::RunTokenMenu { .. })
    ));
    let (_, geometry) = draw(&mut session, &state, 100, 28);
    assert_eq!(
        session
            .advertised_command_bindings(&state, UiCommand::CloseModal)
            .iter()
            .map(|binding| binding.key)
            .collect::<Vec<_>>(),
        [UiKey::Escape]
    );
    let close = geometry
        .hits
        .iter()
        .find(|hit| hit.action == HitTarget::Command(UiCommand::CloseModal))
        .expect("the active modal Close chip must stay clickable")
        .rect;
    let mut mouse_session = session.try_fork().unwrap();
    assert_eq!(
        mouse_session.handle_event(mouse(close.x, close.y), &state, &geometry),
        EventHandling::Action(Action::Back)
    );
    assert_eq!(
        session.handle_event(key(KeyCode::Esc, KeyModifiers::NONE), &state, &geometry,),
        EventHandling::Action(Action::Back)
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
    // The picked name carries a space and glob metacharacters to prove literal placement. The
    // brackets are the metacharacters Python's glob.escape centers on, and Windows permits them
    // where it forbids a star, so one spelling serves every host.
    fs::write(workdir.join("beta file[1].txt"), "beta").unwrap();
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
    let beta_row = row_containing(terminal.backend().buffer(), "beta file[1].txt");
    drive(&mut session, &mut state, &geometry, mouse(12, beta_row));
    assert_eq!(
        state.run_form().unwrap().fields()[0].control.value(),
        "beta file[1].txt"
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
fn every_visible_file_picker_footer_action_has_a_key_and_mouse_twin_at_every_size_tier() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("alpha.txt"), "alpha").unwrap();
    let contract = PathPickerState::new(
        PickerPurpose::Argument,
        temp.path().to_path_buf(),
        PathSelectionMode::FileOrDirectory,
        PathOutputPolicy::RelativeTo(temp.path().to_path_buf()),
        false,
    );
    let is_footer = |target: &FilePickerHit| {
        matches!(
            target,
            FilePickerHit::Accept
                | FilePickerHit::Cancel
                | FilePickerHit::Up
                | FilePickerHit::Hidden
        )
    };
    let key_for = |target: &FilePickerHit| match target {
        FilePickerHit::Accept => key(KeyCode::Enter, KeyModifiers::NONE),
        FilePickerHit::Cancel => key(KeyCode::Esc, KeyModifiers::NONE),
        FilePickerHit::Up => key(KeyCode::Backspace, KeyModifiers::NONE),
        FilePickerHit::Hidden => key(KeyCode::Char('h'), KeyModifiers::CONTROL),
        _ => panic!("non-footer file-picker target: {target:?}"),
    };

    let mut inventory_session = FilePickerSession::new(contract.clone());
    let mut inventory_terminal = Terminal::new(TestBackend::new(200, 30)).unwrap();
    let mut inventory_geometry = Default::default();
    inventory_terminal
        .draw(|frame| {
            inventory_geometry =
                render_file_picker(frame, frame.area(), &mut inventory_session, Locale::En);
        })
        .unwrap();
    let expected = inventory_geometry
        .hits
        .iter()
        .filter(|hit| is_footer(&hit.target))
        .map(|hit| hit.target.clone())
        .collect::<Vec<_>>();
    assert_eq!(expected.len(), 4, "the production footer inventory changed");

    // These are the exact modal-body areas produced by 120x30, 46x12, and 24x6 terminals.
    for (width, height) in [(120, 24), (46, 6), (24, 5)] {
        let mut seen = Vec::new();
        for page in 0..16 {
            let mut page_session = FilePickerSession::new(contract.clone());
            let mut page_terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            let mut page_geometry = Default::default();
            for step in 0..=page {
                page_terminal
                    .draw(|frame| {
                        page_geometry =
                            render_file_picker(frame, frame.area(), &mut page_session, Locale::En);
                    })
                    .unwrap();
                if step < page {
                    let footer = page_geometry
                        .hits
                        .iter()
                        .find(|hit| is_footer(&hit.target))
                        .expect("each footer page has a visible mouse target");
                    assert_eq!(
                        page_session.handle_event(
                            mouse_with_kind(
                                MouseEventKind::ScrollDown,
                                footer.area.x,
                                footer.area.y,
                            ),
                            &page_geometry,
                        ),
                        Some(FilePickerEvent::Changed)
                    );
                }
            }

            for hit in page_geometry
                .hits
                .iter()
                .filter(|hit| is_footer(&hit.target))
            {
                if seen.contains(&hit.target) {
                    continue;
                }
                seen.push(hit.target.clone());

                let mut key_session = FilePickerSession::new(contract.clone());
                let mut key_terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                let mut key_geometry = Default::default();
                key_terminal
                    .draw(|frame| {
                        key_geometry =
                            render_file_picker(frame, frame.area(), &mut key_session, Locale::En);
                    })
                    .unwrap();
                let key_result = key_session.handle_event(key_for(&hit.target), &key_geometry);

                let mut mouse_session = FilePickerSession::new(contract.clone());
                let mut mouse_terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                let mut mouse_geometry = Default::default();
                for step in 0..=page {
                    mouse_terminal
                        .draw(|frame| {
                            mouse_geometry = render_file_picker(
                                frame,
                                frame.area(),
                                &mut mouse_session,
                                Locale::En,
                            );
                        })
                        .unwrap();
                    if step < page {
                        let footer = mouse_geometry
                            .hits
                            .iter()
                            .find(|candidate| is_footer(&candidate.target))
                            .unwrap();
                        let _ = mouse_session.handle_event(
                            mouse_with_kind(
                                MouseEventKind::ScrollDown,
                                footer.area.x,
                                footer.area.y,
                            ),
                            &mouse_geometry,
                        );
                    }
                }
                let mouse_hit = mouse_geometry
                    .hits
                    .iter()
                    .find(|candidate| candidate.target == hit.target)
                    .expect("the same typed target is visible on the same footer page");
                let mouse_result = mouse_session
                    .handle_event(mouse(mouse_hit.area.x, mouse_hit.area.y), &mouse_geometry);
                assert_eq!(
                    mouse_result, key_result,
                    "file-picker {:?} key and mouse diverged at {width}x{height}",
                    hit.target
                );
            }
            if seen.len() == expected.len() {
                break;
            }
        }
        assert!(
            seen.len() == expected.len() && expected.iter().all(|item| seen.contains(item)),
            "file-picker footer dropped actions at {width}x{height}: expected={expected:?} seen={seen:?}"
        );
    }
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

#[test]
fn central_session_run_control_event_matrix_keeps_widget_priority() {
    let mut state = state_with_form(form());
    let mut session = TuiSession::default();
    let (_, geometry) = draw(&mut session, &state, 72, 20);

    for (code, modifiers, expected) in [
        (KeyCode::Char('r'), KeyModifiers::CONTROL, Action::Submit),
        (
            KeyCode::Char('s'),
            KeyModifiers::CONTROL,
            Action::OpenRunPresetSave,
        ),
        (
            KeyCode::Char('t'),
            KeyModifiers::CONTROL,
            Action::OpenRunTokenMenu,
        ),
        (
            KeyCode::Char('o'),
            KeyModifiers::CONTROL,
            Action::ResetFocusedRunField,
        ),
        (
            KeyCode::Char('n'),
            KeyModifiers::CONTROL,
            Action::OpenRunRunnerEditor,
        ),
        (KeyCode::Esc, KeyModifiers::NONE, Action::Back),
    ] {
        assert_eq!(
            session.handle_event(key(code, modifiers), &state, &geometry),
            EventHandling::Action(expected)
        );
    }
    let ctrl_c = key(KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert_eq!(
        session.handle_event(ctrl_c.clone(), &state, &geometry),
        EventHandling::Consumed
    );
    assert_eq!(
        session.handle_event(ctrl_c, &state, &geometry),
        EventHandling::Action(Action::Quit)
    );
    for code in [KeyCode::PageUp, KeyCode::PageDown] {
        assert_eq!(
            session.handle_event(key(code, KeyModifiers::NONE), &state, &geometry),
            EventHandling::Consumed
        );
    }

    let field_index = |key: &str| {
        state
            .run_form()
            .unwrap()
            .fields()
            .iter()
            .position(|field| field.key == key)
            .unwrap()
    };
    let name = field_index("value:name");
    let enabled = field_index("value:enabled");
    let format = field_index("value:format");
    state.update(Action::FocusField(name));
    for code in [KeyCode::Enter, KeyCode::Down, KeyCode::Up, KeyCode::Null] {
        let _ = session.handle_event(key(code, KeyModifiers::NONE), &state, &geometry);
    }
    let typed = session.handle_event(
        key(KeyCode::Char('x'), KeyModifiers::NONE),
        &state,
        &geometry,
    );
    assert!(matches!(typed, EventHandling::Action(_)), "{typed:?}");
    assert!(matches!(
        session.handle_event(Event::Paste("paste".to_owned()), &state, &geometry),
        EventHandling::Action(Action::SetFieldValue { field, .. }) if field == name
    ));

    state.update(Action::FocusField(enabled));
    for code in [
        KeyCode::Char(' '),
        KeyCode::Enter,
        KeyCode::Down,
        KeyCode::Up,
        KeyCode::Null,
    ] {
        let _ = session.handle_event(key(code, KeyModifiers::NONE), &state, &geometry);
    }
    assert_eq!(
        session.handle_event(Event::Paste("ignored".to_owned()), &state, &geometry),
        EventHandling::Ignored
    );

    state.update(Action::FocusField(format));
    for code in [
        KeyCode::Left,
        KeyCode::Right,
        KeyCode::Char(' '),
        KeyCode::Enter,
        KeyCode::Down,
        KeyCode::Up,
        KeyCode::Null,
    ] {
        let _ = session.handle_event(key(code, KeyModifiers::NONE), &state, &geometry);
    }
    for event in [
        mouse_with_kind(MouseEventKind::Moved, 0, 0),
        mouse_with_kind(MouseEventKind::Up(MouseButton::Left), 0, 0),
        Event::FocusGained,
        Event::FocusLost,
        Event::Resize(2, 2),
        Event::Key(KeyEvent::new_with_kind(
            KeyCode::Char('x'),
            KeyModifiers::NONE,
            ratatui_crossterm::crossterm::event::KeyEventKind::Release,
        )),
    ] {
        assert_eq!(
            session.handle_event(event, &state, &geometry),
            EventHandling::Ignored
        );
    }
    let _ = session.handle_event(
        mouse_with_kind(MouseEventKind::ScrollDown, geometry.rows.x, geometry.rows.y),
        &state,
        &geometry,
    );
    for hit in &geometry.hits {
        let _ = session.handle_event(mouse(hit.rect.x, hit.rect.y), &state, &geometry);
    }
}

#[test]
fn central_session_picker_textarea_and_generic_form_matrix_uses_public_screens() {
    let multiline = ParamDecl::new("lines");
    let form = with_multiline_run_field(
        RunFormView::from_declarations(
            "matrix",
            "Matrix",
            &[multiline],
            &BTreeMap::new(),
            &["a", "b", "c", "d", "e"].map(str::to_owned),
            "a",
            &BTreeMap::new(),
            "",
        ),
        1,
    );
    let mut state = state_with_form(form);
    state.update(Action::FocusField(0));
    let mut session = TuiSession::default();
    let (_, geometry) = draw(&mut session, &state, 54, 16);
    assert_eq!(
        session.handle_event(key(KeyCode::Left, KeyModifiers::NONE), &state, &geometry),
        EventHandling::Ignored
    );
    assert_eq!(
        session.handle_event(key(KeyCode::Enter, KeyModifiers::NONE), &state, &geometry),
        EventHandling::Consumed
    );
    for code in [
        KeyCode::Home,
        KeyCode::End,
        KeyCode::PageUp,
        KeyCode::PageDown,
        KeyCode::Up,
        KeyCode::Down,
        KeyCode::Char(' '),
    ] {
        let _ = session.handle_event(key(code, KeyModifiers::NONE), &state, &geometry);
    }
    let (_, open) = draw(&mut session, &state, 54, 16);
    for hit in &open.hits {
        let _ = session.handle_event(mouse(hit.rect.x, hit.rect.y), &state, &open);
    }
    state.update(Action::FocusField(0));
    let (_, picker_geometry) = draw(&mut session, &state, 54, 16);
    assert_run_widget_owned_footer(&session, &state);
    let _ = session.handle_event(
        key(KeyCode::Enter, KeyModifiers::NONE),
        &state,
        &picker_geometry,
    );
    assert_eq!(
        session.handle_event(
            key(KeyCode::Left, KeyModifiers::NONE),
            &state,
            &picker_geometry,
        ),
        EventHandling::Ignored
    );
    let (terminal, picker_geometry) = draw(&mut session, &state, 54, 16);
    let picker_hit = picker_geometry
        .hits
        .iter()
        .find(|hit| hit.action == HitTarget::FocusField(0))
        .unwrap();
    assert_eq!(
        session.handle_event(
            mouse(picker_hit.rect.x, picker_hit.rect.y),
            &state,
            &picker_geometry,
        ),
        EventHandling::Consumed
    );
    assert!(buffer_text(terminal.backend().buffer()).contains('a'));
    state.update(Action::FocusField(1));
    let (_, textarea_geometry) = draw(&mut session, &state, 54, 16);
    assert_run_widget_owned_footer(&session, &state);
    for code in [
        KeyCode::Char('x'),
        KeyCode::Enter,
        KeyCode::Tab,
        KeyCode::Up,
        KeyCode::Down,
        KeyCode::Left,
        KeyCode::Right,
        KeyCode::Home,
        KeyCode::End,
        KeyCode::PageUp,
        KeyCode::PageDown,
        KeyCode::Backspace,
        KeyCode::Delete,
        KeyCode::Null,
    ] {
        let _ = session.handle_event(key(code, KeyModifiers::NONE), &state, &textarea_geometry);
    }
    for (code, modifiers) in [
        (KeyCode::Char('z'), KeyModifiers::CONTROL),
        (
            KeyCode::Char('z'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ),
        (KeyCode::Char('y'), KeyModifiers::CONTROL),
    ] {
        let _ = session.handle_event(key(code, modifiers), &state, &textarea_geometry);
    }
    let pasted = session.handle_event(
        Event::Paste("one\ntwo".to_owned()),
        &state,
        &textarea_geometry,
    );
    assert!(
        matches!(
            pasted,
            EventHandling::Action(Action::SetFieldValue { field: 1, .. })
        ),
        "{pasted:?}"
    );

    let mut generic = LibraryState::default();
    generic.update(Action::Present(Screen::Form(FormView {
        purpose: FormPurpose::Settings,
        title: "Generic".to_owned(),
        title_arguments: Vec::new(),
        translate_title: false,
        selector: Some("demo".to_owned()),
        fields: vec![
            FormField::text("name", "Name", "value"),
            FormField::multiline("body", "Body", "line"),
        ],
        focused: 0,
        submit_label: "Save".to_owned(),
    })));
    let (_, generic_geometry) = draw(&mut session, &generic, 50, 14);
    for event in [
        key(KeyCode::Char('s'), KeyModifiers::CONTROL),
        key(KeyCode::Esc, KeyModifiers::NONE),
        key(KeyCode::Tab, KeyModifiers::NONE),
        key(KeyCode::BackTab, KeyModifiers::SHIFT),
        key(KeyCode::Enter, KeyModifiers::NONE),
        key(KeyCode::Down, KeyModifiers::NONE),
        key(KeyCode::Up, KeyModifiers::NONE),
        key(KeyCode::PageDown, KeyModifiers::NONE),
        key(KeyCode::Null, KeyModifiers::NONE),
        Event::Paste("typed".to_owned()),
        mouse_with_kind(MouseEventKind::Moved, 0, 0),
        mouse_with_kind(MouseEventKind::Up(MouseButton::Left), 0, 0),
        Event::FocusGained,
        Event::Resize(1, 1),
    ] {
        let _ = session.handle_event(event, &generic, &generic_geometry);
    }
    for hit in &generic_geometry.hits {
        let _ = session.handle_event(mouse(hit.rect.x, hit.rect.y), &generic, &generic_geometry);
    }
    generic.update(Action::FocusField(1));
    let (_, generic_textarea_geometry) = draw(&mut session, &generic, 24, 8);
    for (code, modifiers) in [
        (KeyCode::Char('x'), KeyModifiers::NONE),
        (KeyCode::Enter, KeyModifiers::NONE),
        (KeyCode::Up, KeyModifiers::NONE),
        (KeyCode::Down, KeyModifiers::NONE),
        (KeyCode::Null, KeyModifiers::NONE),
        (KeyCode::Char('z'), KeyModifiers::CONTROL),
        (
            KeyCode::Char('z'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ),
    ] {
        let _ = session.handle_event(key(code, modifiers), &generic, &generic_textarea_geometry);
    }
    assert!(matches!(
        session.handle_event(
            Event::Paste("multi\nline".to_owned()),
            &generic,
            &generic_textarea_geometry,
        ),
        EventHandling::Action(Action::SetFieldValue { field: 1, .. })
    ));
    generic.update(Action::FocusField(0));
    let (_, input_geometry) = draw(&mut session, &generic, 50, 14);
    let input = session.handle_event(
        key(KeyCode::Char('x'), KeyModifiers::NONE),
        &generic,
        &input_geometry,
    );
    if let EventHandling::Action(action) = input {
        generic.update(action);
    }
    assert_eq!(
        session.handle_event(
            key(KeyCode::Left, KeyModifiers::NONE),
            &generic,
            &input_geometry,
        ),
        EventHandling::Consumed
    );

    let mut scrolling = LibraryState::default();
    scrolling.update(Action::Present(Screen::Form(FormView {
        purpose: FormPurpose::Settings,
        title: "Scrolling".to_owned(),
        title_arguments: Vec::new(),
        translate_title: false,
        selector: None,
        fields: (0..8)
            .map(|index| FormField::multiline(format!("f{index}"), "Body", "line"))
            .collect(),
        focused: 0,
        submit_label: "Save".to_owned(),
    })));
    let (_, scrolling_geometry) = draw(&mut session, &scrolling, 30, 8);
    assert_eq!(
        session.handle_event(
            mouse_with_kind(
                MouseEventKind::ScrollDown,
                scrolling_geometry.rows.x,
                scrolling_geometry.rows.y,
            ),
            &scrolling,
            &scrolling_geometry,
        ),
        EventHandling::Consumed
    );
    scrolling.update(Action::FocusField(7));
    let (_, _) = draw(&mut session, &scrolling, 30, 8);
    scrolling.update(Action::FocusField(0));
    let (_, _) = draw(&mut session, &scrolling, 30, 8);
}

#[test]
fn central_session_serialized_run_contract_covers_notes_validation_and_tiny_layouts() {
    let mut ratio = ParamDecl::new("ratio");
    ratio.parameter_type = ParameterType::Float;
    ratio.required = true;
    ratio.degraded = true;
    ratio.help = "A visible ratio help row".to_owned();
    ratio.env_source = "RATIO".to_owned();

    let mut choice = ParamDecl::new("choice");
    choice.parameter_type = ParameterType::Choice;
    choice.choices = ["first-long-label", "second-long-label", "third-long-label"]
        .map(str::to_owned)
        .to_vec();

    let mut path = ParamDecl::new("path");
    path.parameter_type = ParameterType::Path;
    path.multiple = true;

    let mut enabled = ParamDecl::new("enabled");
    enabled.parameter_type = ParameterType::Bool;

    let form = RunFormView::from_declarations(
        "coverage",
        "Coverage",
        &[ratio, choice, path, enabled],
        &BTreeMap::from([
            ("ratio".to_owned(), "{env:MISSING}".to_owned()),
            ("path".to_owned(), "*.missing".to_owned()),
        ]),
        &["runner".to_owned()],
        "runner",
        &BTreeMap::new(),
        "",
    )
    .with_context(RunFormContext {
        entry_kind: "python".to_owned(),
        path: Some(RunPathContext {
            workdir: "/work".to_owned(),
            invoke_cwd: "/invoke".to_owned(),
        }),
        tokens: TokenContext {
            cwd: "/invoke".to_owned(),
            home: None,
            env: BTreeMap::new(),
            today: "2026-08-20".to_owned(),
            now: "12-00-00".to_owned(),
        },
    });
    let mut value = serde_json::to_value(form).unwrap();
    value["degraded_reason"] = "dynamic".into();
    value["drift_lines"] = serde_json::json!(["stored source drift"]);
    value["fields"][1]["validation_error"] = "invalid_type".into();
    value["fields"][2]["validation_error"] = "invalid_choice".into();
    value["fields"][3]["validation_error"] = "invalid_choice".into();
    value["fields"][4]["validation_error"] = "invalid_type".into();
    value["fields"][5]["validation_error"] = "required".into();
    value["fields"][3]["feedback"] = serde_json::json!({
        "expanded": "/work/example",
        "token_error": {
            "missing_environment": {
                "name": "MISSING",
                "token": "{env:MISSING}"
            }
        },
        "glob_count": 0
    });
    let form: RunFormView = serde_json::from_value(value).unwrap();
    let mut state = state_with_form(form);
    let mut session = TuiSession::default();

    let (terminal, geometry) = draw(&mut session, &state, 34, 80);
    let rendered = buffer_text(terminal.backend().buffer());
    for expected in [
        "stored source drift",
        "couldn't read",
        "required",
        "ratio help",
        "matches no files",
    ] {
        assert!(
            rendered.contains(expected),
            "missing {expected}: {rendered}"
        );
    }
    let (radio_x, radio_y) = buffer_position(terminal.backend().buffer(), "first-long-label");
    assert!(matches!(
        session.handle_event(mouse(radio_x, radio_y), &state, &geometry),
        EventHandling::Action(Action::SelectFieldOption { .. })
    ));
    assert_eq!(
        session.handle_event(mouse(0, 0), &state, &geometry),
        EventHandling::Ignored
    );

    let mut invalid_text = serde_json::to_value(state.run_form().unwrap()).unwrap();
    invalid_text["fields"][3]["validation_error"] = "invalid_type".into();
    let invalid_text: RunFormView = serde_json::from_value(invalid_text).unwrap();
    let invalid_text = state_with_form(invalid_text);
    let mut invalid_text_session = TuiSession::default();
    let (terminal, _) = draw(&mut invalid_text_session, &invalid_text, 72, 80);
    assert!(buffer_text(terminal.backend().buffer()).contains("needs text"));

    let (_, _) = draw(&mut session, &state, 1, 5);
    state.update(Action::FocusField(5));
    let (_, _) = draw(&mut session, &state, 20, 5);
    state.update(Action::FocusField(0));
    let (_, _) = draw(&mut session, &state, 20, 5);
    for _ in 0..4 {
        let _ = session.handle_event(
            key(KeyCode::PageDown, KeyModifiers::NONE),
            &state,
            &geometry,
        );
    }
    let (_, _) = draw(&mut session, &state, 72, 100);

    let mut subcommands = state.run_form().unwrap().clone();
    subcommands.degraded_reason = Some("subcommands".to_owned());
    let subcommands = state_with_form(subcommands);
    let mut session = TuiSession::default();
    let (terminal, _) = draw(&mut session, &subcommands, 72, 18);
    assert!(buffer_text(terminal.backend().buffer()).contains("subcommands"));

    let mut empty_choice = serde_json::to_value(state.run_form().unwrap()).unwrap();
    empty_choice["fields"][2]["control"] = serde_json::json!({
        "choice": {
            "options": [],
            "selected": "",
            "presentation": "radio"
        }
    });
    empty_choice["focused"] = 2.into();
    let empty_choice: RunFormView = serde_json::from_value(empty_choice).unwrap();
    let empty_choice = state_with_form(empty_choice);
    let mut session = TuiSession::default();
    let (_, geometry) = draw(&mut session, &empty_choice, 72, 18);
    assert_eq!(
        session.handle_event(
            key(KeyCode::Left, KeyModifiers::NONE),
            &empty_choice,
            &geometry,
        ),
        EventHandling::Consumed
    );
}

#[test]
fn central_session_modal_insertion_preserves_serialized_control_boundaries() {
    let text = ParamDecl::new("text");
    let mut toggle = ParamDecl::new("toggle");
    toggle.parameter_type = ParameterType::Bool;
    let form = with_multiline_run_field(
        RunFormView::from_declarations(
            "modal",
            "Modal",
            &[text, toggle],
            &BTreeMap::new(),
            &[],
            "",
            &BTreeMap::new(),
            "",
        )
        .with_context(RunFormContext {
            entry_kind: "python".to_owned(),
            path: None,
            tokens: TokenContext {
                cwd: "/invoke".to_owned(),
                home: None,
                env: BTreeMap::new(),
                today: "2026-08-20".to_owned(),
                now: "12-00-00".to_owned(),
            },
        }),
        0,
    );
    let mut state = state_with_form(form);
    state.update(Action::OpenRunTokenMenuFor(0));
    let mut session = TuiSession::default();
    let (_, geometry) = draw(&mut session, &state, 72, 20);
    assert!(matches!(
        session.handle_event(key(KeyCode::Enter, KeyModifiers::NONE), &state, &geometry),
        EventHandling::Action(Action::SetRunFieldValueAndCloseModal { field: 0, .. })
    ));

    for field in [1_usize, 999] {
        let mut value = serde_json::to_value(&state).unwrap();
        value["modal"] = serde_json::json!({
            "run_token_menu": {
                "field": field,
                "options": ["today"]
            }
        });
        let malformed: LibraryState = serde_json::from_value(value).unwrap();
        let (_, geometry) = draw(&mut session, &malformed, 72, 20);
        assert_eq!(
            session.handle_event(
                key(KeyCode::Enter, KeyModifiers::NONE),
                &malformed,
                &geometry,
            ),
            EventHandling::Ignored
        );
    }
}
