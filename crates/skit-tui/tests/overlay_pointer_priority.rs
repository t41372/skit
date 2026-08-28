use std::{collections::BTreeMap, path::PathBuf};

use ratatui_core::{backend::TestBackend, buffer::Buffer, terminal::Terminal};
use ratatui_crossterm::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use skit_application::{
    AgentScope, AgentTarget, SourcePermissions,
    preferences::{
        AfterRunChoice, InteractiveFormChoice, JavascriptChoice, MirrorConfiguration,
        PreferencesDraft, PreferencesSnapshot,
    },
    tokens::TokenContext,
};
use skit_domain::parameters::{ParamDecl, ParameterType, ParameterValue};
use skit_i18n::Locale;
use skit_tui::{EventHandling, HitTarget, TuiSession, ViewGeometry, render_with_session};
use skit_ui::{
    Action, AddAction, AddWorkflowState, KnownEntryKind, LibraryState, PreferencesAction,
    PreferencesView, ReviewDefaults, ReviewState, RunFormContext, RunFormView, RunPathContext,
    RunnerEditorAction, Screen, SettingsAction, SettingsInputs, SettingsView, SourceSnapshot,
};

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

fn key(code: KeyCode, modifiers: KeyModifiers) -> Event {
    Event::Key(KeyEvent::new(code, modifiers))
}

fn mouse(kind: MouseEventKind, column: u16, row: u16) -> Event {
    Event::Mouse(MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    })
}

fn footer_target(geometry: &ViewGeometry) -> (u16, u16) {
    let area = geometry
        .hits
        .iter()
        .find(|hit| matches!(hit.action, HitTarget::Command(_)))
        .expect("the rendered global footer has a command hit")
        .rect;
    (area.x, area.y)
}

fn assert_footer_is_blocked(
    session: &mut TuiSession,
    state: &LibraryState,
    geometry: &ViewGeometry,
) {
    let (column, row) = footer_target(geometry);
    assert_eq!(
        session.handle_event(
            mouse(MouseEventKind::Down(MouseButton::Left), column, row),
            state,
            geometry,
        ),
        EventHandling::Consumed,
    );
    assert_eq!(
        session.handle_event(
            mouse(MouseEventKind::Up(MouseButton::Left), column, row),
            state,
            geometry,
        ),
        EventHandling::Consumed,
        "a blocking overlay leaked its rendered global-footer target",
    );
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
    panic!("missing rendered text {needle:?}");
}

fn last_buffer_position(buffer: &Buffer, needle: &str) -> (u16, u16) {
    let mut found = None;
    for row in 0..buffer.area.height {
        let line = (0..buffer.area.width)
            .map(|column| buffer[(column, row)].symbol())
            .collect::<String>();
        if let Some(column) = line.find(needle) {
            found = Some((u16::try_from(column).unwrap(), row));
        }
    }
    found.unwrap_or_else(|| panic!("missing rendered text {needle:?}"))
}

fn prompt_add_state() -> LibraryState {
    let source = (0..21)
        .map(|index| format!("{{{{h{index:02}}}}}"))
        .collect::<Vec<_>>()
        .join(" ");
    let review = ReviewState::from_source(
        SourceSnapshot {
            path: "task.prompt.md".into(),
            source_record: "task.prompt.md".to_owned(),
            bytes: source.into_bytes(),
            permissions: SourcePermissions::default(),
            executable: None,
            is_regular: true,
            is_directory: false,
            is_draft: false,
            identity: None,
        },
        KnownEntryKind::Prompt,
        ReviewDefaults::default(),
    );
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Add(Box::new(
        AddWorkflowState::from_review(review),
    ))));
    state
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
        runner_names: Vec::new(),
        mirror: MirrorConfiguration::default(),
    }))
}

#[test]
fn add_file_and_prompt_overlays_block_global_footer_but_keep_their_owner() {
    let mut file_state = LibraryState::default();
    file_state.update(Action::Present(Screen::Add(Box::new(
        AddWorkflowState::new(Vec::new()),
    ))));
    let mut file_session = TuiSession::default();
    let (_, geometry) = draw(&mut file_session, &file_state, 80, 34);
    assert_eq!(
        file_session.handle_event(
            key(KeyCode::Char('o'), KeyModifiers::CONTROL),
            &file_state,
            &geometry,
        ),
        EventHandling::Consumed,
    );
    let (_, geometry) = draw(&mut file_session, &file_state, 80, 34);
    assert!(
        !geometry
            .hits
            .iter()
            .any(|hit| matches!(hit.action, HitTarget::Command(_))),
        "the Add workflow suppresses the global footer while its picker owns the screen",
    );
    assert_eq!(
        file_session.handle_event(
            key(KeyCode::Char('x'), KeyModifiers::NONE),
            &file_state,
            &geometry,
        ),
        EventHandling::Consumed,
        "the file overlay lost its own search input",
    );

    let source = (0..21)
        .map(|index| format!("{{{{field{index:02}}}}}"))
        .collect::<Vec<_>>()
        .join(" ");
    let review = ReviewState::from_source(
        SourceSnapshot {
            path: "task.prompt.md".into(),
            source_record: "task.prompt.md".to_owned(),
            bytes: source.into_bytes(),
            permissions: SourcePermissions::default(),
            executable: None,
            is_regular: true,
            is_directory: false,
            is_draft: false,
            identity: None,
        },
        KnownEntryKind::Prompt,
        ReviewDefaults::default(),
    );
    let mut prompt_state = LibraryState::default();
    prompt_state.update(Action::Present(Screen::Add(Box::new(
        AddWorkflowState::from_review(review),
    ))));
    let mut prompt_session = TuiSession::default();
    let (_, geometry) = draw(&mut prompt_session, &prompt_state, 80, 60);
    assert_eq!(
        prompt_session.handle_event(
            key(KeyCode::Char('o'), KeyModifiers::CONTROL),
            &prompt_state,
            &geometry,
        ),
        EventHandling::Consumed,
    );
    let (_, geometry) = draw(&mut prompt_session, &prompt_state, 80, 60);
    assert!(
        !geometry
            .hits
            .iter()
            .any(|hit| matches!(hit.action, HitTarget::Command(_))),
        "the Add workflow suppresses the global footer while its picker owns the screen",
    );
    assert_eq!(
        prompt_session.handle_event(
            key(KeyCode::Char(' '), KeyModifiers::NONE),
            &prompt_state,
            &geometry,
        ),
        EventHandling::Consumed,
        "the prompt overlay lost its focused choice",
    );
}

#[test]
fn settings_prompt_picker_blocks_global_footer_but_keeps_its_owner() {
    let candidates = (0..=skit_ui::PROMPT_LIST_PREVIEW_LIMIT)
        .map(|index| format!("VALUE_{index}"))
        .collect::<Vec<_>>();
    let settings = SettingsView::from_inputs(&SettingsInputs {
        kind: "prompt".to_owned(),
        name: "Prompt".to_owned(),
        supports_modes: true,
        interpolate: true,
        candidates,
        ..SettingsInputs::default()
    });
    let mut settings_state = LibraryState::default();
    settings_state.update(Action::Present(Screen::Settings(Box::new(settings))));
    let mut settings_session = TuiSession::default();
    let (_, geometry) = draw(&mut settings_session, &settings_state, 90, 32);
    assert_eq!(
        settings_session.handle_event(
            key(KeyCode::Char('o'), KeyModifiers::CONTROL),
            &settings_state,
            &geometry,
        ),
        EventHandling::Consumed,
    );
    let (terminal, geometry) = draw(&mut settings_session, &settings_state, 90, 32);
    assert_footer_is_blocked(&mut settings_session, &settings_state, &geometry);
    let (choice_column, choice_row) = buffer_position(terminal.backend().buffer(), "VALUE_1");
    assert_eq!(
        settings_session.handle_event(
            mouse(
                MouseEventKind::Down(MouseButton::Left),
                choice_column,
                choice_row,
            ),
            &settings_state,
            &geometry,
        ),
        EventHandling::Consumed,
    );
    assert_eq!(
        settings_session.handle_event(
            mouse(
                MouseEventKind::Up(MouseButton::Left),
                choice_column,
                choice_row,
            ),
            &settings_state,
            &geometry,
        ),
        EventHandling::Consumed,
        "the Settings picker lost its selected row",
    );
    assert_eq!(
        settings_session.handle_event(
            key(KeyCode::Char('s'), KeyModifiers::CONTROL),
            &settings_state,
            &geometry,
        ),
        EventHandling::Action(Action::Settings(
            skit_ui::SettingsAction::SetPromptCandidates(vec!["VALUE_1".to_owned()]),
        )),
        "the overlay row did not change the picker's isolated selection",
    );
}

#[test]
fn agent_picker_blocks_global_footer_but_keeps_its_owner() {
    let mut preferences = preferences();
    preferences.update(PreferencesAction::PresentAgentSkillTargets(vec![
        AgentTarget {
            name: "codex".to_owned(),
            scope: AgentScope::User,
            base: PathBuf::from("/home/demo/.codex"),
        },
    ]));
    let mut preferences_state = LibraryState::default();
    preferences_state.update(Action::Present(Screen::Preferences(Box::new(preferences))));
    let mut preferences_session = TuiSession::default();
    let (terminal, geometry) = draw(&mut preferences_session, &preferences_state, 100, 30);
    assert_footer_is_blocked(&mut preferences_session, &preferences_state, &geometry);
    let (column, row) = buffer_position(terminal.backend().buffer(), "codex (user)");
    assert_eq!(
        preferences_session.handle_event(
            mouse(MouseEventKind::Down(MouseButton::Left), column, row),
            &preferences_state,
            &geometry,
        ),
        EventHandling::Consumed,
    );
    assert_eq!(
        preferences_session.handle_event(
            mouse(MouseEventKind::Up(MouseButton::Left), column, row),
            &preferences_state,
            &geometry,
        ),
        EventHandling::Action(Action::Preferences(
            PreferencesAction::ActivateAgentSkillTarget(0),
        )),
    );
}

#[test]
fn runner_editor_blocks_the_underlying_run_field_command_hit() {
    let mut path = ParamDecl::new("path");
    path.parameter_type = ParameterType::Path;
    path.default = Some(ParameterValue::String("default.txt".to_owned()));
    let form = RunFormView::from_declarations(
        "prompt",
        "Prompt",
        &[path],
        &BTreeMap::new(),
        &["codex".to_owned()],
        "codex",
        &BTreeMap::new(),
        "",
    )
    .with_context(RunFormContext {
        entry_kind: "prompt".to_owned(),
        path: Some(RunPathContext {
            workdir: "/work".to_owned(),
            invoke_cwd: "/invoke".to_owned(),
        }),
        tokens: TokenContext {
            cwd: "/invoke".to_owned(),
            home: None,
            env: BTreeMap::new(),
            today: "2026-08-28".to_owned(),
            now: "12-00-00".to_owned(),
        },
    });
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Run(Box::new(form))));
    state.update(Action::OpenRunRunnerEditor);
    let mut session = TuiSession::default();
    let (_, geometry) = draw(&mut session, &state, 100, 32);
    let underlay = geometry
        .hits
        .iter()
        .find(|hit| matches!(hit.action, HitTarget::RunFieldCommand { .. }))
        .expect("the RunnerEditor render exposed one underlying Run command")
        .rect;
    assert_eq!(
        session.handle_event(
            mouse(
                MouseEventKind::Down(MouseButton::Left),
                underlay.x,
                underlay.y,
            ),
            &state,
            &geometry,
        ),
        EventHandling::Consumed,
    );
    let release = session.handle_event(
        mouse(
            MouseEventKind::Up(MouseButton::Left),
            underlay.x,
            underlay.y,
        ),
        &state,
        &geometry,
    );
    assert!(
        !matches!(
            release,
            EventHandling::Action(
                Action::OpenRunFilePicker(_)
                    | Action::OpenRunTokenMenuFor(_)
                    | Action::ResetRunField(_)
            )
        ),
        "the RunnerEditor leaked an underlying Run field command: {release:?}",
    );
    assert!(matches!(
        session.handle_event(
            key(KeyCode::Char('x'), KeyModifiers::NONE),
            &state,
            &geometry
        ),
        EventHandling::Action(Action::RunnerEditor(_))
    ));
}

#[test]
fn a_global_click_cancels_an_armed_add_control() {
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Add(Box::new(
        AddWorkflowState::new(Vec::new()),
    ))));
    let mut session = TuiSession::default();
    let (terminal, geometry) = draw(&mut session, &state, 90, 34);
    let local = buffer_position(terminal.backend().buffer(), "[Enter] Continue");
    assert_eq!(
        session.handle_event(
            mouse(MouseEventKind::Down(MouseButton::Left), local.0, local.1),
            &state,
            &geometry,
        ),
        EventHandling::Consumed,
    );
    let mut preferences_state = LibraryState::default();
    preferences_state.update(Action::Present(Screen::Preferences(
        Box::new(preferences()),
    )));
    let (_, preferences_geometry) = draw(&mut session, &preferences_state, 90, 34);
    let (footer_column, footer_row) = footer_target(&preferences_geometry);
    assert_eq!(
        session.handle_event(
            mouse(
                MouseEventKind::Down(MouseButton::Left),
                footer_column,
                footer_row,
            ),
            &preferences_state,
            &preferences_geometry,
        ),
        EventHandling::Consumed,
    );
    assert!(matches!(
        session.handle_event(
            mouse(
                MouseEventKind::Up(MouseButton::Left),
                footer_column,
                footer_row,
            ),
            &preferences_state,
            &preferences_geometry,
        ),
        EventHandling::Action(_),
    ));
    let (_, geometry) = draw(&mut session, &state, 90, 34);
    assert_eq!(
        session.handle_event(
            mouse(MouseEventKind::Up(MouseButton::Left), local.0, local.1),
            &state,
            &geometry,
        ),
        EventHandling::Ignored,
        "a late Add release survived a complete global-footer click",
    );
}

#[test]
fn resize_cancels_an_armed_add_file_overlay_row_without_disabling_it() {
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Add(Box::new(
        AddWorkflowState::new(Vec::new()),
    ))));
    let mut session = TuiSession::default();
    let (_, geometry) = draw(&mut session, &state, 80, 34);
    assert_eq!(
        session.handle_event(
            key(KeyCode::Char('o'), KeyModifiers::CONTROL),
            &state,
            &geometry,
        ),
        EventHandling::Consumed,
    );
    for character in "Cargo.toml".chars() {
        assert_eq!(
            session.handle_event(
                key(KeyCode::Char(character), KeyModifiers::NONE),
                &state,
                &geometry,
            ),
            EventHandling::Consumed,
        );
    }
    let (terminal, geometry) = draw(&mut session, &state, 80, 34);
    let target = last_buffer_position(terminal.backend().buffer(), "Cargo.toml");
    assert_eq!(
        session.handle_event(
            mouse(MouseEventKind::Down(MouseButton::Left), target.0, target.1),
            &state,
            &geometry,
        ),
        EventHandling::Consumed,
    );
    assert_eq!(
        session.handle_event(Event::Resize(80, 34), &state, &geometry),
        EventHandling::Ignored,
    );
    let (_, geometry) = draw(&mut session, &state, 80, 34);
    assert_eq!(
        session.handle_event(
            mouse(MouseEventKind::Up(MouseButton::Left), target.0, target.1),
            &state,
            &geometry,
        ),
        EventHandling::Consumed,
        "a file row survived resize as an armed semantic target",
    );
    assert_eq!(
        session.handle_event(
            mouse(MouseEventKind::Down(MouseButton::Left), target.0, target.1),
            &state,
            &geometry,
        ),
        EventHandling::Consumed,
    );
    assert!(matches!(
        session.handle_event(
            mouse(MouseEventKind::Up(MouseButton::Left), target.0, target.1),
            &state,
            &geometry,
        ),
        EventHandling::Action(Action::Add(AddAction::SetSourcePath(path)))
            if path.ends_with("Cargo.toml")
    ));
}

#[test]
fn resize_cancels_an_armed_add_prompt_row_without_changing_its_selection() {
    let state = prompt_add_state();
    let mut session = TuiSession::default();
    let (_, geometry) = draw(&mut session, &state, 80, 60);
    assert_eq!(
        session.handle_event(
            key(KeyCode::Char('o'), KeyModifiers::CONTROL),
            &state,
            &geometry,
        ),
        EventHandling::Consumed,
    );
    let (terminal, geometry) = draw(&mut session, &state, 80, 60);
    let target = buffer_position(terminal.backend().buffer(), "h01");
    assert_eq!(
        session.handle_event(
            mouse(MouseEventKind::Down(MouseButton::Left), target.0, target.1),
            &state,
            &geometry,
        ),
        EventHandling::Consumed,
    );
    assert_eq!(
        session.handle_event(Event::Resize(80, 60), &state, &geometry),
        EventHandling::Ignored,
    );
    let (_, geometry) = draw(&mut session, &state, 80, 60);
    assert_eq!(
        session.handle_event(
            mouse(MouseEventKind::Up(MouseButton::Left), target.0, target.1),
            &state,
            &geometry,
        ),
        EventHandling::Consumed,
        "a prompt row survived resize as an armed semantic target",
    );
    let expected = (0..21)
        .map(|index| format!("h{index:02}"))
        .collect::<Vec<_>>();
    assert_eq!(
        session.handle_event(
            key(KeyCode::Char('s'), KeyModifiers::CONTROL),
            &state,
            &geometry,
        ),
        EventHandling::Action(Action::Add(AddAction::SetPromptCandidates(expected))),
        "the cancelled release changed the isolated prompt selection",
    );
}

#[test]
fn focus_loss_cancels_agent_and_preferences_arms_without_disabling_the_overlay() {
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Preferences(
        Box::new(preferences()),
    )));
    state.update(Action::Preferences(PreferencesAction::SetEditor(
        "editor-probe".to_owned(),
    )));
    let mut session = TuiSession::default();
    let (terminal, geometry) = draw(&mut session, &state, 100, 30);
    let underlay = buffer_position(terminal.backend().buffer(), "editor-probe");
    assert_eq!(
        session.handle_event(
            mouse(
                MouseEventKind::Down(MouseButton::Left),
                underlay.0,
                underlay.1,
            ),
            &state,
            &geometry,
        ),
        EventHandling::Consumed,
    );

    state.update(Action::Preferences(
        PreferencesAction::PresentAgentSkillTargets(vec![AgentTarget {
            name: "codex".to_owned(),
            scope: AgentScope::User,
            base: PathBuf::from("/home/demo/.codex"),
        }]),
    ));
    let (terminal, geometry) = draw(&mut session, &state, 100, 30);
    let overlay = buffer_position(terminal.backend().buffer(), "codex (user)");
    assert_eq!(
        session.handle_event(
            mouse(
                MouseEventKind::Down(MouseButton::Left),
                overlay.0,
                overlay.1,
            ),
            &state,
            &geometry,
        ),
        EventHandling::Consumed,
    );
    assert_eq!(
        session.handle_event(Event::FocusLost, &state, &geometry),
        EventHandling::Consumed,
    );
    assert_eq!(
        session.handle_event(
            mouse(MouseEventKind::Up(MouseButton::Left), overlay.0, overlay.1,),
            &state,
            &geometry,
        ),
        EventHandling::Consumed,
        "the agent overlay arm survived terminal focus loss",
    );
    assert_eq!(
        session.handle_event(
            mouse(
                MouseEventKind::Down(MouseButton::Left),
                overlay.0,
                overlay.1,
            ),
            &state,
            &geometry,
        ),
        EventHandling::Consumed,
    );
    assert_eq!(
        session.handle_event(
            mouse(MouseEventKind::Up(MouseButton::Left), overlay.0, overlay.1,),
            &state,
            &geometry,
        ),
        EventHandling::Action(Action::Preferences(
            PreferencesAction::ActivateAgentSkillTarget(0),
        )),
        "focus loss disabled the agent overlay's next complete click",
    );
    state.update(Action::Preferences(
        PreferencesAction::CloseAgentSkillTargets,
    ));
    let (_, geometry) = draw(&mut session, &state, 100, 30);
    assert_eq!(
        session.handle_event(
            mouse(
                MouseEventKind::Up(MouseButton::Left),
                underlay.0,
                underlay.1,
            ),
            &state,
            &geometry,
        ),
        EventHandling::Ignored,
        "the Preferences underlay arm survived its blocking overlay",
    );
}

#[test]
fn add_runner_editor_cancels_its_armed_underlay_and_remains_operable() {
    let mut state = prompt_add_state();
    let mut session = TuiSession::default();
    let (terminal, geometry) = draw(&mut session, &state, 80, 60);
    let underlay = buffer_position(terminal.backend().buffer(), "Add Runner");
    assert_eq!(
        session.handle_event(
            mouse(
                MouseEventKind::Down(MouseButton::Left),
                underlay.0,
                underlay.1,
            ),
            &state,
            &geometry,
        ),
        EventHandling::Consumed,
    );
    state.update(Action::OpenAddRunnerEditor);
    let (_, geometry) = draw(&mut session, &state, 80, 60);
    let edit = session.handle_event(
        key(KeyCode::Char('x'), KeyModifiers::NONE),
        &state,
        &geometry,
    );
    assert_eq!(
        edit,
        EventHandling::Action(Action::RunnerEditor(RunnerEditorAction::SetName(
            "x".to_owned(),
        ))),
        "the Add-owned RunnerEditor lost its own input",
    );
    if let EventHandling::Action(action) = edit {
        state.update(action);
    }
    state.update(Action::RunnerEditor(RunnerEditorAction::Cancel));
    let (_, geometry) = draw(&mut session, &state, 80, 60);
    assert_eq!(
        session.handle_event(
            mouse(
                MouseEventKind::Up(MouseButton::Left),
                underlay.0,
                underlay.1,
            ),
            &state,
            &geometry,
        ),
        EventHandling::Ignored,
        "the Add underlay activated after its RunnerEditor closed",
    );
}

#[test]
fn settings_runner_editor_cancels_its_armed_underlay_and_remains_operable() {
    let settings = SettingsView::from_inputs(&SettingsInputs {
        selector: "prompt-entry".to_owned(),
        kind: "prompt".to_owned(),
        name: "Prompt".to_owned(),
        runner: "codex".to_owned(),
        interpolate: true,
        configured_runners: vec!["codex".to_owned()],
        ..SettingsInputs::default()
    });
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Settings(Box::new(settings))));
    let mut session = TuiSession::default();
    let (terminal, geometry) = draw(&mut session, &state, 100, 50);
    let underlay = buffer_position(terminal.backend().buffer(), "New agent");
    assert_eq!(
        session.handle_event(
            mouse(
                MouseEventKind::Down(MouseButton::Left),
                underlay.0,
                underlay.1,
            ),
            &state,
            &geometry,
        ),
        EventHandling::Consumed,
    );
    let open = session.handle_event(
        key(KeyCode::Char('n'), KeyModifiers::CONTROL),
        &state,
        &geometry,
    );
    assert_eq!(
        open,
        EventHandling::Action(Action::Settings(SettingsAction::NewRunner)),
        "the advertised Settings Ctrl+N did not route to its Settings-owned editor",
    );
    if let EventHandling::Action(action) = open {
        state.update(action);
    }
    let (_, geometry) = draw(&mut session, &state, 100, 50);
    let edit = session.handle_event(
        key(KeyCode::Char('x'), KeyModifiers::NONE),
        &state,
        &geometry,
    );
    assert_eq!(
        edit,
        EventHandling::Action(Action::RunnerEditor(RunnerEditorAction::SetName(
            "x".to_owned(),
        ))),
        "the Settings-owned RunnerEditor lost its own input",
    );
    if let EventHandling::Action(action) = edit {
        state.update(action);
    }
    state.update(Action::RunnerEditor(RunnerEditorAction::Cancel));
    let (_, geometry) = draw(&mut session, &state, 100, 50);
    assert_eq!(
        session.handle_event(
            mouse(
                MouseEventKind::Up(MouseButton::Left),
                underlay.0,
                underlay.1,
            ),
            &state,
            &geometry,
        ),
        EventHandling::Ignored,
        "the Settings underlay activated after its RunnerEditor closed",
    );
    let footer = geometry
        .hits
        .iter()
        .find(|hit| hit.action == HitTarget::Command(skit_ui::UiCommand::NewRunner))
        .expect("the advertised Settings Ctrl+N command also has a footer hit")
        .rect;
    assert_eq!(
        session.handle_event(
            mouse(MouseEventKind::Down(MouseButton::Left), footer.x, footer.y,),
            &state,
            &geometry,
        ),
        EventHandling::Consumed,
    );
    assert_eq!(
        session.handle_event(
            mouse(MouseEventKind::Up(MouseButton::Left), footer.x, footer.y,),
            &state,
            &geometry,
        ),
        EventHandling::Action(Action::Settings(SettingsAction::NewRunner)),
        "the Settings New agent footer hit used the Run-only action",
    );
}
