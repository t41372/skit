//! The shared text editor on every surface, through the session.
//!
//! `crates/skit-cli/tests/tui_text_editing.rs` drives each key group in a real terminal for a
//! launch-menu box, the Add path box, the search box, an Entry settings box, and the multi-line
//! Description box. These cases give the same keys to the Rename dialog, the Preferences editor box,
//! the two boxes of the agent editor, the preset-name dialog, and the environment dialog through
//! the session, so a surface that routes a key elsewhere fails here.
//!
//! Every single-line box here also gets a drag, Ctrl+V with an empty clipboard, and a paste whose
//! first line is empty. The multi-line boxes of the launch menu and of a form get the clipboard
//! keys.

use std::collections::BTreeMap;

use ratatui_core::{backend::TestBackend, terminal::Terminal};
use ratatui_crossterm::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use skit_application::preferences::{
    AccentChoice, AfterRunChoice, InteractiveFormChoice, JavascriptChoice, MirrorConfiguration,
    PreferencesDraft, PreferencesSnapshot, ThemeChoice,
};
use skit_application::tokens::TokenContext;
use skit_domain::parameters::{ParamDecl, ParameterType, ParameterValue};
use skit_i18n::Locale;
use skit_tui::{
    AgentReviewSnapshotError, EventHandling, TuiSession, ViewGeometry, render_with_session,
};
use skit_ui::{
    Action, AddWorkflowState, FormField, FormPurpose, FormView, LibraryState, ModalState, NAME_KEY,
    PreferencesAction, PreferencesControlId, PreferencesView, RunFormContext, RunFormView,
    RunPathContext, RunnerEditorAction, Screen, SettingsInputs, SettingsView,
};

/// Draw one frame, and return its geometry and the cell of the hardware cursor.
fn draw(session: &mut TuiSession, state: &LibraryState) -> (ViewGeometry, (u16, u16)) {
    let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
    let mut geometry = ViewGeometry::default();
    terminal
        .draw(|frame| {
            geometry = render_with_session(frame, state, Locale::En, session);
        })
        .unwrap();
    let cursor = terminal.backend().cursor_position();
    (geometry, (cursor.x, cursor.y))
}

fn press(code: KeyCode, modifiers: KeyModifiers) -> Event {
    Event::Key(KeyEvent::new(code, modifiers))
}

fn pointer(kind: MouseEventKind, column: u16, row: u16) -> Event {
    Event::Mouse(MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    })
}

/// One text box on one surface, with the reader of its value.
struct Surface<F> {
    session: TuiSession,
    state: LibraryState,
    geometry: ViewGeometry,
    /// The cell of the hardware cursor in the last frame, which is the caret of the focused box.
    caret: (u16, u16),
    value: F,
    name: &'static str,
}

impl<F: Fn(&LibraryState) -> String> Surface<F> {
    fn new(state: LibraryState, value: F, name: &'static str) -> Self {
        let mut session = TuiSession::default();
        let (geometry, caret) = draw(&mut session, &state);
        Self {
            session,
            state,
            geometry,
            caret,
            value,
            name,
        }
    }

    /// Give one event to the session, apply its action, and draw the next frame.
    fn send(&mut self, event: Event) -> EventHandling {
        let handling = self
            .session
            .handle_event(event, &self.state, &self.geometry);
        if let EventHandling::Action(action) = &handling {
            self.state.update(action.clone());
        }
        (self.geometry, self.caret) = draw(&mut self.session, &self.state);
        handling
    }

    /// Give one event, then check the value of the box.
    fn step(&mut self, event: Event, expected: &str) -> EventHandling {
        let handling = self.send(event.clone());
        assert_eq!(
            (self.value)(&self.state),
            expected,
            "{}: {event:?} gave {handling:?}",
            self.name
        );
        handling
    }

    /// Type `text` one key at a time after the value that the box has.
    fn type_text(&mut self, text: &str) {
        let mut typed = (self.value)(&self.state);
        for character in text.chars() {
            typed.push(character);
            let _ = self.step(press(KeyCode::Char(character), KeyModifiers::NONE), &typed);
        }
    }

    /// Check the two pastes that leave the box as it is.
    ///
    /// The session clipboard must be empty. Ctrl+V then has nothing to paste, and a paste whose
    /// first line is empty gives a single-line box nothing to keep. Neither reaches the screen as
    /// an action.
    fn pastes_that_change_nothing(&mut self) {
        let before = (self.value)(&self.state);
        let name = self.name;
        assert_eq!(
            self.step(press(KeyCode::Char('v'), KeyModifiers::CONTROL), &before),
            EventHandling::Consumed,
            "{name}: Ctrl+V with an empty clipboard did not stop in the box"
        );
        let pasted = self.step(Event::Paste("\nlater line".to_owned()), &before);
        assert!(
            !matches!(pasted, EventHandling::Action(_)),
            "{name}: a paste with an empty first line gave {pasted:?}"
        );
    }

    /// Select the first four characters of a new value with a drag, then cut them.
    ///
    /// The box must be empty. The release ends the drag and keeps the selection, so the cut
    /// removes exactly the dragged characters.
    fn drag_selects(&mut self) {
        self.type_text("drag me");
        let (caret, row) = self.caret;
        let start = caret - 7;
        for (kind, column) in [
            (MouseEventKind::Down(MouseButton::Left), start),
            (MouseEventKind::Drag(MouseButton::Left), start + 4),
            (MouseEventKind::Up(MouseButton::Left), start + 4),
        ] {
            let _ = self.step(pointer(kind, column, row), "drag me");
        }
        let _ = self.step(press(KeyCode::Char('x'), KeyModifiers::CONTROL), " me");
    }
}

/// Drive every key group on the focused text box of `state`, and read its value with `value`.
///
/// The box must start empty. Ways this can fail: a surface keeps a chord for itself; a pasted line
/// break submits or a pasted second line stays; a copy arms the quit; undo and redo do not reach
/// the box; a drag in the box does not select.
fn exercise(state: LibraryState, value: impl Fn(&LibraryState) -> String, name: &'static str) {
    let mut surface = Surface::new(state, value, name);
    surface.pastes_that_change_nothing();
    surface.type_text("one two");
    let _ = surface.step(press(KeyCode::Char('a'), KeyModifiers::CONTROL), "one two");
    let _ = surface.step(press(KeyCode::Char('X'), KeyModifiers::SHIFT), "Xone two");
    let _ = surface.step(press(KeyCode::Char('e'), KeyModifiers::CONTROL), "Xone two");
    let _ = surface.step(press(KeyCode::Char('w'), KeyModifiers::CONTROL), "Xone ");
    let pasted = surface.step(Event::Paste("a\nb".to_owned()), "Xone a");
    assert!(
        matches!(pasted, EventHandling::Action(ref action) if !matches!(action, Action::Submit)),
        "{name}: the paste gave {pasted:?}"
    );
    let _ = surface.step(press(KeyCode::Home, KeyModifiers::SHIFT), "Xone a");
    assert_eq!(
        surface.step(press(KeyCode::Char('c'), KeyModifiers::CONTROL), "Xone a"),
        EventHandling::Consumed,
        "{name}: Ctrl+C with a selection did not copy"
    );
    assert!(
        !matches!(
            surface.session.agent_review_snapshot(),
            Err(AgentReviewSnapshotError::ClockActive)
        ),
        "{name}: a copy armed the quit"
    );
    let _ = surface.step(press(KeyCode::End, KeyModifiers::NONE), "Xone a");
    let _ = surface.step(
        press(KeyCode::Char('v'), KeyModifiers::CONTROL),
        "Xone aXone a",
    );
    let _ = surface.step(press(KeyCode::Char('z'), KeyModifiers::CONTROL), "Xone a");
    let _ = surface.step(
        press(KeyCode::Char('y'), KeyModifiers::CONTROL),
        "Xone aXone a",
    );
    let _ = surface.step(press(KeyCode::Char('u'), KeyModifiers::CONTROL), "");
    surface.drag_selects();
}

/// Check the pointer and the pastes that change nothing on a box that the end-to-end cases drive
/// with keys.
///
/// The caret must be at the end of the value. Ctrl+U empties the box before the drag.
fn pointer_and_pastes(
    state: LibraryState,
    value: impl Fn(&LibraryState) -> String,
    name: &'static str,
) {
    let mut surface = Surface::new(state, value, name);
    surface.pastes_that_change_nothing();
    let _ = surface.step(press(KeyCode::Char('u'), KeyModifiers::CONTROL), "");
    surface.drag_selects();
}

/// Select a typed line in a multi-line box, then copy, cut, and paste it back.
///
/// Ways this can fail: the session does not find the multi-line box, so Ctrl+C arms the quit and
/// Ctrl+X and Ctrl+V do nothing.
fn multi_line_clipboard(
    state: LibraryState,
    value: impl Fn(&LibraryState) -> String,
    name: &'static str,
) {
    let mut surface = Surface::new(state, value, name);
    surface.type_text("drag me");
    let _ = surface.step(press(KeyCode::Home, KeyModifiers::SHIFT), "drag me");
    assert_eq!(
        surface.step(press(KeyCode::Char('c'), KeyModifiers::CONTROL), "drag me"),
        EventHandling::Consumed,
        "{name}: Ctrl+C with a selection did not copy"
    );
    let _ = surface.step(press(KeyCode::Char('x'), KeyModifiers::CONTROL), "");
    let _ = surface.step(press(KeyCode::Char('v'), KeyModifiers::CONTROL), "drag me");
}

/// A launch menu with an agent picker before its one parameter, so it can open the agent editor.
fn run_form() -> RunFormView {
    RunFormView::from_declarations(
        "demo",
        "Demo",
        &[ParamDecl::new("name")],
        &BTreeMap::new(),
        &["codex".to_owned()],
        "codex",
        &BTreeMap::new(),
        "",
    )
}

fn run_state() -> LibraryState {
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Run(Box::new(run_form()))));
    state
}

/// A launch menu whose first field is its one parameter, with the focus on it.
fn parameter_form() -> RunFormView {
    RunFormView::from_declarations(
        "demo",
        "Demo",
        &[ParamDecl::new("name")],
        &BTreeMap::new(),
        &[],
        "",
        &BTreeMap::new(),
        "",
    )
}

fn run_value(state: &LibraryState) -> String {
    state.run_form().unwrap().fields()[0]
        .control
        .value()
        .to_owned()
}

fn agent_editor_state() -> LibraryState {
    let mut state = run_state();
    state.update(Action::OpenRunRunnerEditor);
    state
}

#[test]
fn the_rename_dialog_uses_every_editing_key_group() {
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Form(FormView {
        purpose: FormPurpose::Rename,
        title: "Rename {}".to_owned(),
        title_arguments: vec!["Demo".to_owned()],
        translate_title: false,
        selector: Some("demo".to_owned()),
        fields: vec![FormField::text("name", "Name", "")],
        focused: 0,
        submit_label: "Rename".to_owned(),
    })));
    exercise(
        state,
        |state| state.form().unwrap().fields[0].value.clone(),
        "Rename",
    );
}

#[test]
fn the_preferences_editor_box_uses_every_editing_key_group() {
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Preferences(Box::new(
        PreferencesView::new(PreferencesDraft::from_snapshot(PreferencesSnapshot {
            language: String::new(),
            available_languages: vec!["en".to_owned()],
            effective_language: "en".to_owned(),
            editor: String::new(),
            editor_fallback: None,
            form: InteractiveFormChoice::Tui,
            after_run: AfterRunChoice::Exit,
            theme: ThemeChoice::Terminal,
            accent: AccentChoice::Auto,
            javascript: JavascriptChoice::Automatic,
            bash_path: None,
            runners: Vec::new(),
            mirror: MirrorConfiguration::default(),
        })),
    ))));
    state.update(Action::Preferences(PreferencesAction::Focus(
        PreferencesControlId::Editor,
    )));
    exercise(
        state,
        |state| match state.screen() {
            Screen::Preferences(view) => view.draft().editor.clone(),
            _ => panic!("Preferences closed"),
        },
        "Preferences",
    );
}

#[test]
fn the_agent_editor_uses_every_editing_key_group() {
    exercise(
        agent_editor_state(),
        |state| match state.modal() {
            Some(ModalState::RunnerEditor { view, .. }) => view.name().to_owned(),
            _ => panic!("the agent editor closed"),
        },
        "agent editor",
    );
}

#[test]
fn the_agent_editor_command_box_uses_every_editing_key_group() {
    let mut state = agent_editor_state();
    state.update(Action::RunnerEditor(RunnerEditorAction::FocusNext));
    exercise(
        state,
        |state| match state.modal() {
            Some(ModalState::RunnerEditor { view, .. }) => view.command().to_owned(),
            _ => panic!("the agent editor closed"),
        },
        "agent command",
    );
}

#[test]
fn the_preset_name_dialog_uses_every_editing_key_group() {
    let mut state = run_state();
    state.update(Action::OpenRunPresetSave);
    exercise(
        state,
        |state| match state.modal() {
            Some(ModalState::RunPresetName { value, .. }) => value.clone(),
            _ => panic!("the preset dialog closed"),
        },
        "preset name",
    );
}

#[test]
fn the_environment_dialog_uses_every_editing_key_group() {
    let form = RunFormView::from_declarations(
        "demo",
        "Demo",
        &[ParamDecl::new("name")],
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
            env: BTreeMap::from([("HOME".to_owned(), "/home/demo".to_owned())]),
            today: "2026-08-08".to_owned(),
            now: "12-00-00".to_owned(),
        },
    });
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Run(Box::new(form))));
    let field = state.run_form().unwrap().focused();
    state.update(Action::OpenRunTokenMenu);
    state.update(Action::OpenRunEnvironmentPicker(field));
    exercise(
        state,
        |state| match state.modal() {
            Some(ModalState::RunEnvironmentPicker { query, .. }) => query.clone(),
            _ => panic!("the environment dialog closed"),
        },
        "environment",
    );
}

#[test]
fn the_search_box_selects_by_drag_and_keeps_an_empty_paste_out() {
    let mut state = LibraryState::default();
    state.update(Action::BeginSearch);
    state.update(Action::SetSearchQuery("old".to_owned()));
    pointer_and_pastes(state, |state| state.query().to_owned(), "search");
}

#[test]
fn a_launch_menu_box_selects_by_drag_and_keeps_an_empty_paste_out() {
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Run(Box::new(parameter_form()))));
    pointer_and_pastes(state, run_value, "launch menu");
}

#[test]
fn the_add_path_box_selects_by_drag_and_keeps_an_empty_paste_out() {
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Add(Box::new(
        AddWorkflowState::new(Vec::new()),
    ))));
    pointer_and_pastes(
        state,
        |state| state.add_workflow().unwrap().source().path.clone(),
        "Add path",
    );
}

#[test]
fn an_entry_settings_box_selects_by_drag_and_keeps_an_empty_paste_out() {
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Settings(Box::new(
        SettingsView::from_inputs(&SettingsInputs {
            selector: "demo".to_owned(),
            kind: "python".to_owned(),
            name: "Demo".to_owned(),
            source: "/demo/demo.py".to_owned(),
            workdir: "invoke".to_owned(),
            ..SettingsInputs::default()
        }),
    ))));
    pointer_and_pastes(
        state,
        |state| match state.screen() {
            Screen::Settings(view) => view.field(NAME_KEY).unwrap().value().as_text(),
            _ => panic!("Entry settings closed"),
        },
        "Entry settings",
    );
}

#[test]
fn a_multi_line_launch_menu_box_copies_cuts_and_pastes_its_selection() {
    let mut value = serde_json::to_value(parameter_form()).unwrap();
    value["fields"][0]["control"]["text"]["multiline"] = true.into();
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Run(Box::new(
        serde_json::from_value(value).unwrap(),
    ))));
    multi_line_clipboard(state, run_value, "multi-line launch menu");
}

#[test]
fn a_multi_line_form_box_copies_cuts_and_pastes_its_selection() {
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Form(FormView {
        purpose: FormPurpose::Settings,
        title: "Notes".to_owned(),
        title_arguments: Vec::new(),
        translate_title: false,
        selector: None,
        fields: vec![FormField::multiline("body", "Body", "")],
        focused: 0,
        submit_label: "Save".to_owned(),
    })));
    multi_line_clipboard(
        state,
        |state| state.form().unwrap().fields[0].value.clone(),
        "multi-line form",
    );
}

/// Ctrl+C on a launch-menu control that has no text keeps the two-press quit.
///
/// Ways this can fail: the session takes Ctrl+C on a checkbox as a copy, so the first press of the
/// quit does nothing.
#[test]
fn ctrl_c_on_a_checkbox_keeps_the_two_press_quit() {
    let mut enabled = ParamDecl::new("enabled");
    enabled.parameter_type = ParameterType::Bool;
    enabled.default = Some(ParameterValue::Bool(false));
    let form = RunFormView::from_declarations(
        "demo",
        "Demo",
        &[enabled],
        &BTreeMap::new(),
        &[],
        "",
        &BTreeMap::new(),
        "",
    );
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Run(Box::new(form))));
    let mut session = TuiSession::default();
    let (geometry, _) = draw(&mut session, &state);
    assert_eq!(
        session.handle_event(
            press(KeyCode::Char('c'), KeyModifiers::CONTROL),
            &state,
            &geometry,
        ),
        EventHandling::Consumed
    );
    assert!(matches!(
        session.agent_review_snapshot(),
        Err(AgentReviewSnapshotError::ClockActive)
    ));
}
