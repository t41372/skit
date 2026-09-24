//! Chips and buttons show where the pointer rests and which target a press holds.
//!
//! Ways these tests can fail: a move onto a chip asks for no frame, so the look never shows; a
//! move inside one chip asks for a frame each time; the hover look stays after the pointer
//! leaves; a press shows nothing until the release; a release elsewhere leaves the pressed look;
//! one surface (a dialog, Preferences, Health, the runner editor) has no look at all.

use std::{collections::BTreeMap, path::PathBuf};

use ratatui_core::{backend::TestBackend, buffer::Buffer, style::Modifier, terminal::Terminal};
use ratatui_crossterm::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use skit_application::{
    AgentScope, AgentTarget, LibraryScan,
    preferences::{
        AccentChoice, AfterRunChoice, InteractiveFormChoice, JavascriptChoice, MirrorConfiguration,
        PreferencesDraft, PreferencesSnapshot, ThemeChoice,
    },
    tokens::TokenContext,
};
use skit_domain::{
    EntryKind, EntrySummary, Slug, StorageMode,
    parameters::{ParamDecl, ParameterType},
};
use skit_i18n::Locale;
use skit_tui::{EventHandling, TuiSession, ViewGeometry, render_with_session};
use skit_ui::{
    Action, AddWorkflowState, HealthIssue, HealthIssueKind, HealthSnapshot, HealthView,
    LibraryState, MirrorHealth, PreferencesAction, PreferencesControlId, PreferencesView,
    RunFormContext, RunFormView, RunPathContext, Screen, SettingsInputs, SettingsView, UvHealth,
};
use tempfile::TempDir;

fn library() -> LibraryState {
    LibraryState::from_scan(LibraryScan {
        entries: vec![EntrySummary {
            slug: Slug::parse("alpha").unwrap(),
            name: "Alpha".to_owned(),
            kind: EntryKind::parse("command").unwrap(),
            mode: StorageMode::Copy,
            description: String::new(),
            target: None,
        }],
        diagnostics: Vec::new(),
    })
}

fn preferences() -> PreferencesView {
    PreferencesView::new(PreferencesDraft::from_snapshot(PreferencesSnapshot {
        language: String::new(),
        available_languages: vec!["en".to_owned(), "zh-CN".to_owned(), "zh-TW".to_owned()],
        effective_language: "en".to_owned(),
        editor: String::new(),
        editor_fallback: Some("vim".to_owned()),
        form: InteractiveFormChoice::Tui,
        after_run: AfterRunChoice::Exit,
        theme: ThemeChoice::Terminal,
        accent: AccentChoice::Auto,
        javascript: JavascriptChoice::Automatic,
        bash_path: None,
        runners: Vec::new(),
        mirror: MirrorConfiguration::default(),
    }))
}

fn health() -> HealthView {
    HealthView::new(HealthSnapshot {
        uv: UvHealth::NotRequired,
        entry_count: 1,
        issues: vec![HealthIssue {
            slug: "alpha".to_owned(),
            name: "Alpha".to_owned(),
            kind: HealthIssueKind::MissingTarget,
        }],
        invalid_runner_rows: Vec::new(),
        mirror: MirrorHealth::Off,
        library_path: "/data/scripts".to_owned(),
        library_size: "1 KiB".to_owned(),
        diagnostics: Vec::new(),
    })
}

fn draw(session: &mut TuiSession, state: &LibraryState) -> (Buffer, ViewGeometry) {
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    let mut geometry = ViewGeometry::default();
    terminal
        .draw(|frame| {
            geometry = render_with_session(frame, state, Locale::En, session);
        })
        .unwrap();
    (terminal.backend().buffer().clone(), geometry)
}

/// The column and row of the first cell of `needle`, searched from the bottom row up.
fn position(buffer: &Buffer, needle: &str) -> (u16, u16) {
    for row in (0..buffer.area.height).rev() {
        let cells = (0..buffer.area.width)
            .map(|column| buffer[(column, row)].symbol())
            .collect::<Vec<_>>();
        for column in 0..cells.len() {
            if cells[column..].concat().starts_with(needle) {
                return (u16::try_from(column).unwrap(), row);
            }
        }
    }
    panic!("missing {needle:?}");
}

fn mouse(kind: MouseEventKind, (column, row): (u16, u16)) -> Event {
    Event::Mouse(MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    })
}

fn hovered(buffer: &Buffer, (column, row): (u16, u16)) -> bool {
    buffer[(column, row)]
        .modifier
        .contains(Modifier::BOLD | Modifier::UNDERLINED)
}

/// Move the pointer onto the first cell of `needle`, and return the frame drawn after it.
fn hover(session: &mut TuiSession, state: &LibraryState, needle: &str) -> (Buffer, (u16, u16)) {
    let (buffer, _) = draw(session, state);
    let at = position(&buffer, needle);
    assert!(
        !hovered(&buffer, at),
        "{needle} looked hovered before the move"
    );
    assert!(
        session.track_pointer(&mouse(MouseEventKind::Moved, at), state),
        "a move onto {needle} asked for no frame"
    );
    let (buffer, _) = draw(session, state);
    assert!(hovered(&buffer, at), "{needle} has no hover look");
    (buffer, at)
}

#[test]
fn a_footer_chip_shows_the_hover_and_holds_the_pressed_look_until_the_release() {
    let state = library();
    let mut session = TuiSession::default();
    let (before, _) = draw(&mut session, &state);
    let (_, at) = hover(&mut session, &state, "? Help");
    let label = (at.0 + 2, at.1);

    // A move inside the same chip, and a key, ask for no new frame.
    assert!(!session.track_pointer(&mouse(MouseEventKind::Moved, label), &state));
    assert!(!session.track_pointer(&Event::Paste("x".to_owned()), &state));

    let (_, geometry) = draw(&mut session, &state);
    assert_eq!(
        session.handle_event(
            mouse(MouseEventKind::Down(MouseButton::Left), label),
            &state,
            &geometry
        ),
        EventHandling::Consumed
    );
    let (pressed, _) = draw(&mut session, &state);
    assert_ne!(
        pressed[label].modifier.contains(Modifier::REVERSED),
        before[label].modifier.contains(Modifier::REVERSED),
        "a press shows no pressed look"
    );

    // A release on another cell activates nothing and ends the pressed look.
    let elsewhere = (0, 0);
    let _ = session.track_pointer(&mouse(MouseEventKind::Moved, elsewhere), &state);
    let (_, geometry) = draw(&mut session, &state);
    let _ = session.handle_event(
        mouse(MouseEventKind::Up(MouseButton::Left), elsewhere),
        &state,
        &geometry,
    );
    let (released, _) = draw(&mut session, &state);
    assert_eq!(
        released[label].modifier.contains(Modifier::REVERSED),
        before[label].modifier.contains(Modifier::REVERSED)
    );
    assert!(!hovered(&released, label), "the hover look stayed behind");
}

#[test]
fn a_lost_focus_drops_the_hover_look() {
    let state = library();
    let mut session = TuiSession::default();
    let (_, at) = hover(&mut session, &state, "? Help");
    assert!(session.track_pointer(&Event::FocusLost, &state));
    let (buffer, _) = draw(&mut session, &state);
    assert!(!hovered(&buffer, at));
    assert!(!session.track_pointer(&Event::FocusLost, &state));
}

#[test]
fn dialog_chips_show_the_hover_and_the_press() {
    let mut state = library();
    state.update(Action::AskRemove);
    let mut session = TuiSession::default();
    let (_, at) = hover(&mut session, &state, " y Remove ");
    let (_, geometry) = draw(&mut session, &state);
    assert_eq!(
        session.handle_event(
            mouse(MouseEventKind::Down(MouseButton::Left), at),
            &state,
            &geometry
        ),
        EventHandling::Consumed
    );
    let (pressed, _) = draw(&mut session, &state);
    assert!(pressed[at].modifier.contains(Modifier::REVERSED));

    let mut guard = LibraryState::default();
    guard.update(Action::Present(Screen::Preferences(
        Box::new(preferences()),
    )));
    guard.update(Action::Preferences(PreferencesAction::SetEditor(
        "micro".to_owned(),
    )));
    guard.update(Action::Preferences(PreferencesAction::Close));
    let mut session = TuiSession::default();
    let _ = hover(&mut session, &guard, " y Discard ");
}

#[test]
fn preferences_buttons_health_chips_and_runner_editor_chips_show_the_hover() {
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Preferences(
        Box::new(preferences()),
    )));
    // The button sits below the fold; the focus brings it into view.
    state.update(Action::Preferences(PreferencesAction::Focus(
        PreferencesControlId::NewRunner,
    )));
    let mut session = TuiSession::default();
    let _ = draw(&mut session, &state);
    let _ = hover(&mut session, &state, " New agent… ");

    state.update(Action::Preferences(
        PreferencesAction::PresentAgentSkillTargets(vec![AgentTarget {
            name: "codex".to_owned(),
            scope: AgentScope::User,
            base: PathBuf::from("/home/demo/.codex"),
        }]),
    ));
    let mut session = TuiSession::default();
    let _ = hover(&mut session, &state, " Cancel ");

    let mut runner = LibraryState::default();
    runner.update(Action::Present(Screen::Preferences(
        Box::new(preferences()),
    )));
    runner.update(Action::Preferences(PreferencesAction::NewRunner));
    let mut session = TuiSession::default();
    let _ = hover(&mut session, &runner, "Esc Cancel");

    let mut checkup = LibraryState::default();
    checkup.update(Action::Present(Screen::Health(Box::new(health()))));
    let mut session = TuiSession::default();
    let _ = hover(&mut session, &checkup, "Esc Back");
}

/// Press the primary button on `at`, and return the frame drawn after it.
fn press(session: &mut TuiSession, state: &LibraryState, at: (u16, u16)) -> Buffer {
    let (_, geometry) = draw(session, state);
    assert_eq!(
        session.handle_event(
            mouse(MouseEventKind::Down(MouseButton::Left), at),
            state,
            &geometry
        ),
        EventHandling::Consumed,
        "a press on {at:?} armed nothing"
    );
    draw(session, state).0
}

/// The column and row of the first cell of `needle`, searched from the top row down.
fn topmost_position(buffer: &Buffer, needle: &str) -> (u16, u16) {
    for row in 0..buffer.area.height {
        let cells = (0..buffer.area.width)
            .map(|column| buffer[(column, row)].symbol())
            .collect::<Vec<_>>();
        for column in 0..cells.len() {
            if cells[column..].concat().starts_with(needle) {
                return (u16::try_from(column).unwrap(), row);
            }
        }
    }
    panic!("missing {needle:?}");
}

fn reversed(buffer: &Buffer, at: (u16, u16)) -> bool {
    buffer[at].modifier.contains(Modifier::REVERSED)
}

/// A launch menu with one path parameter whose working directory is `workdir`.
fn launch_menu(workdir: &std::path::Path) -> LibraryState {
    let mut path = ParamDecl::new("path");
    path.parameter_type = ParameterType::Path;
    let form = RunFormView::from_declarations(
        "demo",
        "Demo",
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
            workdir: workdir.display().to_string(),
            invoke_cwd: workdir.display().to_string(),
        }),
        tokens: TokenContext {
            cwd: workdir.display().to_string(),
            home: None,
            env: BTreeMap::new(),
            today: "2026-11-08".to_owned(),
            now: "10-11-12".to_owned(),
        },
    });
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Run(Box::new(form))));
    state
}

// Ways this can fail: a button in the body of the Add screen, or a chip in its footer, shows no
// look under the pointer; a press shows no pressed look; or a press on a button also marks the
// footer chip that names the same action, so two targets look held.
#[test]
fn add_buttons_and_footer_chips_show_the_hover_and_the_press() {
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Add(Box::new(
        AddWorkflowState::new(Vec::new()),
    ))));

    let mut session = TuiSession::default();
    let (before, button) = hover(&mut session, &state, "Browse…");
    let pressed = press(&mut session, &state, button);
    assert_ne!(
        reversed(&pressed, button),
        reversed(&before, button),
        "a press on an Add button shows no pressed look"
    );

    let mut session = TuiSession::default();
    let (before, chip) = hover(&mut session, &state, "[Esc] Cancel");
    let pressed = press(&mut session, &state, chip);
    assert_ne!(
        reversed(&pressed, chip),
        reversed(&before, chip),
        "a press on an Add footer chip shows no pressed look"
    );
    // The Cancel button in the body names the same action, but the press holds only the chip.
    let body_cancel = topmost_position(&before, "Cancel");
    assert!(body_cancel.1 < chip.1, "the body has no Cancel button");
    assert_eq!(
        reversed(&pressed, body_cancel),
        reversed(&before, body_cancel),
        "a press on the footer chip also marked the body button"
    );
}

// Ways this can fail: the chips of the file picker footer show no look under the pointer or no
// pressed look, on the Add screen or in the launch menu.
#[test]
fn file_picker_footer_chips_show_the_hover_and_the_press() {
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Add(Box::new(
        AddWorkflowState::new(Vec::new()),
    ))));
    let mut session = TuiSession::default();
    let (_, geometry) = draw(&mut session, &state);
    assert_eq!(
        session.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL)),
            &state,
            &geometry
        ),
        EventHandling::Consumed
    );
    let (before, chip) = hover(&mut session, &state, "[Esc] Cancel");
    let pressed = press(&mut session, &state, chip);
    assert_ne!(reversed(&pressed, chip), reversed(&before, chip));

    let root = TempDir::new().unwrap();
    let mut state = launch_menu(root.path());
    state.update(Action::OpenRunFilePicker(0));
    let mut session = TuiSession::default();
    let (before, chip) = hover(&mut session, &state, "[Backspace] Back");
    let pressed = press(&mut session, &state, chip);
    assert_ne!(reversed(&pressed, chip), reversed(&before, chip));
}

// Ways this can fail: the chips of the preset-name dialog show no look under the pointer or no
// pressed look.
#[test]
fn preset_name_dialog_chips_show_the_hover_and_the_press() {
    let root = TempDir::new().unwrap();
    let mut state = launch_menu(root.path());
    state.update(Action::OpenRunPresetSave);
    let mut session = TuiSession::default();
    let (before, chip) = hover(&mut session, &state, "Esc Cancel");
    let pressed = press(&mut session, &state, chip);
    assert_ne!(reversed(&pressed, chip), reversed(&before, chip));
}

// Ways this can fail: the footer chips of Entry settings show no look under the pointer or no
// pressed look.
#[test]
fn entry_settings_footer_chips_show_the_hover_and_the_press() {
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Settings(Box::new(
        SettingsView::from_inputs(&SettingsInputs {
            selector: "alpha".to_owned(),
            kind: "command".to_owned(),
            name: "Alpha".to_owned(),
            ..SettingsInputs::default()
        }),
    ))));
    let mut session = TuiSession::default();
    let (before, chip) = hover(&mut session, &state, "Esc Back");
    let pressed = press(&mut session, &state, chip);
    assert_ne!(reversed(&pressed, chip), reversed(&before, chip));
}

// Ways this can fail: the chips of the prompt variable picker, which takes the place of the Entry
// settings footer while it is open, show no look under the pointer or no pressed look.
#[test]
fn prompt_variable_picker_footer_chips_show_the_hover_and_the_press() {
    let names = (0..25).map(|index| format!("v{index}")).collect::<Vec<_>>();
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Settings(Box::new(
        SettingsView::from_inputs(&SettingsInputs {
            selector: "flood".to_owned(),
            kind: "prompt".to_owned(),
            name: "Flood".to_owned(),
            declared_schema: true,
            interpolate: true,
            candidates: names,
            ..SettingsInputs::default()
        }),
    ))));
    let mut session = TuiSession::default();
    let (_, geometry) = draw(&mut session, &state);
    assert_eq!(
        session.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL)),
            &state,
            &geometry
        ),
        EventHandling::Consumed
    );
    let (opened, _) = draw(&mut session, &state);
    let _ = topmost_position(&opened, "Choose prompt variables");
    let (before, chip) = hover(&mut session, &state, "[Esc] Cancel");
    let pressed = press(&mut session, &state, chip);
    assert_ne!(reversed(&pressed, chip), reversed(&before, chip));
}
