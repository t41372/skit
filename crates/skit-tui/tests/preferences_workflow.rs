use ratatui_core::{backend::TestBackend, buffer::Buffer, terminal::Terminal};
use ratatui_crossterm::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use skit_application::preferences::{
    AfterRunChoice, InteractiveFormChoice, JavascriptChoice, MirrorConfiguration, PreferencesDraft,
    PreferencesSnapshot,
};
use skit_i18n::Locale;
use skit_tui::{EventHandling, HitTarget, TuiSession, render_with_session};
use skit_ui::{
    Action, LibraryState, ModalState, PreferencesAction, PreferencesControlId, PreferencesView,
    Screen, UiCommand,
};

fn draw(
    session: &mut TuiSession,
    state: &LibraryState,
    width: u16,
    height: u16,
) -> Terminal<TestBackend> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| {
            let _ = render_with_session(frame, state, Locale::En, session);
        })
        .unwrap();
    terminal
}

fn rendered_text(buffer: &Buffer) -> String {
    buffer.content().iter().map(|cell| cell.symbol()).collect()
}

fn buffer_position(buffer: &Buffer, needle: &str) -> (u16, u16) {
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

fn preferences() -> PreferencesView {
    PreferencesView::new(PreferencesDraft::from_snapshot(PreferencesSnapshot {
        language: String::new(),
        available_languages: vec!["en".to_owned(), "zh-CN".to_owned(), "zh-TW".to_owned()],
        effective_language: "en".to_owned(),
        editor: String::new(),
        editor_fallback: Some("vim".to_owned()),
        form: InteractiveFormChoice::Tui,
        after_run: AfterRunChoice::Exit,
        javascript: JavascriptChoice::Automatic,
        bash_path: None,
        runners: Vec::new(),
        mirror: MirrorConfiguration::default(),
    }))
}

#[test]
fn dirty_preferences_discard_guard_has_exact_keys_and_clickable_actions() {
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Preferences(
        Box::new(preferences()),
    )));
    state.update(Action::Preferences(PreferencesAction::SetEditor(
        "micro".to_owned(),
    )));
    state.update(Action::Preferences(PreferencesAction::Close));
    assert_eq!(state.modal(), Some(&ModalState::ConfirmDiscardChanges));

    let mut terminal = Terminal::new(TestBackend::new(72, 20)).unwrap();
    let mut session = TuiSession::default();
    let mut geometry = Default::default();
    terminal
        .draw(|frame| {
            geometry = render_with_session(frame, &state, Locale::En, &mut session);
        })
        .unwrap();
    let rendered = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    // Version 0.4 shows the question once inside the confirmation panel.
    assert_eq!(rendered.matches("Discard unsaved changes?").count(), 1);
    assert!(rendered.contains("Discard"));
    assert!(rendered.contains("Keep editing"));

    assert_eq!(
        session.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            &state,
            &geometry,
        ),
        EventHandling::Action(Action::KeepEditing)
    );
    assert_eq!(
        session.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE)),
            &state,
            &geometry,
        ),
        EventHandling::Action(Action::DiscardChanges)
    );
    assert_eq!(
        session.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE)),
            &state,
            &geometry,
        ),
        EventHandling::Action(Action::KeepEditing)
    );
    // Version 0.4 binds `y` and `escape,n` and nothing else
    // (`src/skit/tui_settings.py:43-46`). Enter must not throw the user's work away: the answer
    // reached by reflex has to be the safe one.
    assert_ne!(
        session.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            &state,
            &geometry,
        ),
        EventHandling::Action(Action::DiscardChanges),
        "Enter discarded unsaved work"
    );

    for (command, expected) in [
        (UiCommand::DiscardChanges, Action::DiscardChanges),
        (UiCommand::KeepEditing, Action::KeepEditing),
    ] {
        let area = geometry
            .hits
            .iter()
            .find_map(|hit| (hit.action == HitTarget::Command(command)).then_some(hit.rect))
            .expect("the visible discard action must be clickable");
        assert_eq!(
            session.handle_event(
                Event::Mouse(MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    column: area.x,
                    row: area.y,
                    modifiers: KeyModifiers::NONE,
                }),
                &state,
                &geometry,
            ),
            EventHandling::Consumed,
            "primary Down must arm {command:?} without activating it"
        );
        assert_eq!(
            session.handle_event(
                Event::Mouse(MouseEvent {
                    kind: MouseEventKind::Up(MouseButton::Left),
                    column: area.x,
                    row: area.y,
                    modifiers: KeyModifiers::NONE,
                }),
                &state,
                &geometry,
            ),
            EventHandling::Action(expected)
        );
    }
}

#[test]
fn resize_only_keeps_the_focused_preferences_control_visible() {
    let mut view = preferences();
    let _ = view.update(PreferencesAction::Focus(PreferencesControlId::NpmChoice));
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Preferences(Box::new(view))));
    let mut session = TuiSession::default();

    let large = draw(&mut session, &state, 70, 100);
    assert!(rendered_text(large.backend().buffer()).contains("npm registry"));
    let small = draw(&mut session, &state, 70, 18);
    let rendered = rendered_text(small.backend().buffer());
    assert!(
        rendered.contains("npm registry"),
        "resize-only reflow hid the focused Preferences control: {rendered}"
    );
}

#[test]
fn preferences_arm_cannot_survive_a_release_owned_by_the_global_footer() {
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Preferences(
        Box::new(preferences()),
    )));
    state.update(Action::Preferences(PreferencesAction::SetEditor(
        "editor-probe".to_owned(),
    )));
    let mut session = TuiSession::default();
    let mut terminal = Terminal::new(TestBackend::new(100, 28)).unwrap();
    let mut geometry = Default::default();
    terminal
        .draw(|frame| {
            geometry = render_with_session(frame, &state, Locale::En, &mut session);
        })
        .unwrap();
    let editor = buffer_position(terminal.backend().buffer(), "editor-probe");
    let footer = geometry
        .hits
        .iter()
        .find(|hit| matches!(hit.action, HitTarget::Command(_)))
        .expect("the global footer must have a hit")
        .rect;
    let mouse = |kind, column, row| {
        Event::Mouse(MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        })
    };

    assert_eq!(
        session.handle_event(
            mouse(MouseEventKind::Down(MouseButton::Left), editor.0, editor.1),
            &state,
            &geometry,
        ),
        EventHandling::Consumed
    );
    assert_eq!(
        session.handle_event(
            mouse(MouseEventKind::Up(MouseButton::Left), footer.x, footer.y),
            &state,
            &geometry,
        ),
        EventHandling::Ignored
    );
    assert_eq!(
        session.handle_event(
            mouse(MouseEventKind::Up(MouseButton::Left), editor.0, editor.1),
            &state,
            &geometry,
        ),
        EventHandling::Ignored,
        "a release in another owner must cancel the Preferences arm"
    );
}

/// The Preferences agent editor is a modal above Preferences: it renders there and stages there.
#[test]
fn the_preferences_agent_editor_opens_renders_and_stages_without_a_host_write() {
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Preferences(
        Box::new(preferences()),
    )));
    state.update(Action::Preferences(PreferencesAction::Focus(
        PreferencesControlId::NewRunner,
    )));
    let mut session = TuiSession::default();
    let mut geometry = Default::default();
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal
        .draw(|frame| {
            geometry = render_with_session(frame, &state, Locale::En, &mut session);
        })
        .unwrap();
    assert!(rendered_text(terminal.backend().buffer()).contains("New agent…"));

    let door = buffer_position(terminal.backend().buffer(), "New agent…");
    let mouse = |kind, column, row| {
        Event::Mouse(MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        })
    };
    for kind in [
        MouseEventKind::Down(MouseButton::Left),
        MouseEventKind::Up(MouseButton::Left),
    ] {
        if let EventHandling::Action(action) =
            session.handle_event(mouse(kind, door.0, door.1), &state, &geometry)
        {
            assert_eq!(
                state.update(action),
                skit_ui::Effect::None,
                "the door opens a modal the reducer owns"
            );
        }
    }
    assert!(matches!(
        state.modal(),
        Some(ModalState::RunnerEditor {
            owner: skit_ui::RunnerEditorOwner::Preferences,
            ..
        })
    ));

    let terminal = draw(&mut session, &state, 100, 30);
    assert!(rendered_text(terminal.backend().buffer()).contains("New agent"));

    // A pointer press inside the modal cancels the Preferences arm below it, never the modal.
    assert_eq!(
        session.handle_event(
            mouse(MouseEventKind::Down(MouseButton::Left), 0, 0),
            &state,
            &geometry,
        ),
        EventHandling::Consumed
    );

    for action in [
        skit_ui::RunnerEditorAction::SetName("local".to_owned()),
        skit_ui::RunnerEditorAction::SetCommand("local {{prompt}}".to_owned()),
        skit_ui::RunnerEditorAction::Submit,
    ] {
        assert_eq!(
            state.update(Action::RunnerEditor(action)),
            skit_ui::Effect::None,
            "the Preferences editor never asks the host to write"
        );
    }
    assert_eq!(state.modal(), None);
    assert_eq!(
        state
            .preferences()
            .unwrap()
            .draft()
            .runner_rows()
            .iter()
            .filter_map(|row| row.name().map(str::to_owned))
            .collect::<Vec<_>>(),
        ["local"]
    );

    let terminal = draw(&mut session, &state, 100, 30);
    let rendered = rendered_text(terminal.backend().buffer());
    assert!(rendered.contains("local"), "{rendered}");
    assert!(rendered.contains("Added"), "{rendered}");
}

/// The focused agent list owns Up, Down, Enter, Delete and Backspace before any global command.
/// The walker's smallest profile still paints the list, its chips, and nothing outside it.
#[test]
fn the_agent_list_and_its_chips_stay_inside_the_smallest_walker_viewport() {
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Preferences(
        Box::new(preferences()),
    )));
    state.update(Action::Preferences(PreferencesAction::NewRunner));
    for action in [
        skit_ui::RunnerEditorAction::SetName("codex".to_owned()),
        skit_ui::RunnerEditorAction::SetCommand("codex {{prompt}}".to_owned()),
        skit_ui::RunnerEditorAction::Submit,
    ] {
        state.update(Action::RunnerEditor(action));
    }
    state.update(Action::Preferences(PreferencesAction::Focus(
        PreferencesControlId::Runners,
    )));

    let mut session = TuiSession::default();
    let mut terminal = Terminal::new(TestBackend::new(24, 6)).unwrap();
    terminal
        .draw(|frame| {
            let _ = render_with_session(frame, &state, Locale::En, &mut session);
        })
        .unwrap();

    let inventory = session.screen_target_inventory(&state).unwrap();
    let viewport = terminal.backend().buffer().area;
    assert!(
        inventory
            .hits
            .iter()
            .all(|hit| hit.rect.right() <= viewport.right()
                && hit.rect.bottom() <= viewport.bottom()),
        "{:?}",
        inventory.hits
    );
    assert!(
        inventory
            .available
            .contains(&skit_tui::ScreenTarget::Runner {
                row: 0,
                name: Some("codex".to_owned()),
            })
    );
}

#[test]
fn the_focused_agent_list_owns_its_keys_at_the_session_level() {
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Preferences(
        Box::new(preferences()),
    )));
    state.update(Action::Preferences(PreferencesAction::NewRunner));
    for action in [
        skit_ui::RunnerEditorAction::SetName("codex".to_owned()),
        skit_ui::RunnerEditorAction::SetCommand("codex {{prompt}}".to_owned()),
        skit_ui::RunnerEditorAction::Submit,
    ] {
        state.update(Action::RunnerEditor(action));
    }
    state.update(Action::Preferences(PreferencesAction::Focus(
        PreferencesControlId::Runners,
    )));

    let mut session = TuiSession::default();
    let mut geometry = Default::default();
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal
        .draw(|frame| {
            geometry = render_with_session(frame, &state, Locale::En, &mut session);
        })
        .unwrap();

    // A row chip is a command chip: it paints in the same pill colors as the shared footer.
    let chip = session
        .screen_target_inventory(&state)
        .unwrap()
        .hits
        .into_iter()
        .find(|hit| matches!(hit.target, skit_tui::ScreenTarget::RunnerChip { .. }))
        .expect("the cursor row paints its chips")
        .rect;
    let footer_chip = geometry
        .hits
        .iter()
        .find(|hit| matches!(hit.action, HitTarget::Command(_)))
        .expect("the frame paints a footer chip")
        .rect;
    let buffer = terminal.backend().buffer();
    let painted = |rect: ratatui_core::layout::Rect| {
        let cell = &buffer[(rect.x + rect.width / 2, rect.y)];
        (cell.fg, cell.bg)
    };
    assert_eq!(painted(chip), painted(footer_chip));

    for (code, expected) in [
        (KeyCode::Up, PreferencesAction::RunnerCursorPrevious),
        (KeyCode::Down, PreferencesAction::RunnerCursorNext),
        (KeyCode::Enter, PreferencesAction::EditRunner),
        (KeyCode::Delete, PreferencesAction::ToggleRunnerRemoval),
        (KeyCode::Backspace, PreferencesAction::ToggleRunnerRemoval),
    ] {
        assert_eq!(
            session.handle_event(
                Event::Key(KeyEvent::new(code, KeyModifiers::NONE)),
                &state,
                &geometry,
            ),
            EventHandling::Action(Action::Preferences(expected)),
            "{code:?} must reach the agent list, not a global command"
        );
    }

    // The footer stops offering Down as "next field" while the list owns vertical navigation.
    assert!(
        session
            .advertised_command_bindings(&state, UiCommand::FocusNext)
            .iter()
            .all(|binding| binding.key != skit_ui::UiKey::Down)
    );
}
