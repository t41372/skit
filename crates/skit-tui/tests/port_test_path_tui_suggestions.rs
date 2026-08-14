use std::{collections::BTreeMap, fs, path::Path};

use ratatui_core::{backend::TestBackend, buffer::Buffer, terminal::Terminal};
use ratatui_crossterm::crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use skit_application::tokens::TokenContext;
use skit_domain::parameters::{ParamDecl, ParameterType};
use skit_i18n::Locale;
use skit_tui::{EventHandling, TuiSession, ViewGeometry, render_with_session};
use skit_ui::{Action, LibraryState, RunFormContext, RunFormView, RunPathContext, Screen};

fn state_for(
    parameter_type: ParameterType,
    secret: bool,
    multiple: bool,
    workdir: &Path,
    invoke_cwd: &Path,
    home: Option<&Path>,
    env: BTreeMap<String, String>,
) -> LibraryState {
    let mut value = ParamDecl::new("value");
    value.parameter_type = parameter_type;
    value.secret = secret;
    value.multiple = multiple;
    let form = RunFormView::from_declarations(
        "job",
        "job",
        &[value],
        &BTreeMap::new(),
        &[],
        "",
        &BTreeMap::new(),
        "",
    )
    .with_context(RunFormContext {
        entry_kind: "python".to_owned(),
        path: Some(RunPathContext {
            workdir: workdir.to_string_lossy().into_owned(),
            invoke_cwd: invoke_cwd.to_string_lossy().into_owned(),
        }),
        tokens: TokenContext {
            cwd: invoke_cwd.to_string_lossy().into_owned(),
            home: home.map(|path| path.to_string_lossy().into_owned()),
            env,
            today: "2026-08-14".to_owned(),
            now: "15-57-00".to_owned(),
        },
    });
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Run(Box::new(form))));
    state
}

fn draw(session: &mut TuiSession, state: &LibraryState) -> (Terminal<TestBackend>, ViewGeometry) {
    let mut terminal = Terminal::new(TestBackend::new(110, 28)).expect("terminal");
    let mut geometry = ViewGeometry::default();
    terminal
        .draw(|frame| {
            geometry = render_with_session(frame, state, Locale::En, session);
        })
        .expect("draw");
    (terminal, geometry)
}

fn buffer_text(buffer: &Buffer) -> String {
    buffer.content().iter().map(|cell| cell.symbol()).collect()
}

fn type_text(
    session: &mut TuiSession,
    state: &mut LibraryState,
    geometry: &ViewGeometry,
    value: &str,
) {
    for character in value.chars() {
        let handling = session.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE)),
            state,
            geometry,
        );
        match handling {
            EventHandling::Action(action) => {
                state.update(action);
            }
            EventHandling::Consumed => {}
            EventHandling::Ignored => panic!("run input ignored typed character {character:?}"),
        }
    }
}

fn rendered_after_typing(state: &mut LibraryState, value: &str) -> String {
    let mut session = TuiSession::default();
    let (_, geometry) = draw(&mut session, state);
    type_text(&mut session, state, &geometry, value);
    let (terminal, _) = draw(&mut session, state);
    buffer_text(terminal.backend().buffer())
}

#[test]
fn test_path_field_completes_bare_prefix_at_workdir() {
    let temp = tempfile::tempdir().expect("tempdir");
    fs::write(temp.path().join("data-ghost-unique.csv"), b"x").expect("data");
    let invoke = tempfile::tempdir().expect("invoke");
    let mut state = state_for(
        ParameterType::Path,
        false,
        false,
        temp.path(),
        invoke.path(),
        None,
        BTreeMap::new(),
    );
    let rendered = rendered_after_typing(&mut state, "data-gh");
    assert!(
        rendered.contains("data-ghost-unique.csv"),
        "a path field must expose the frozen inline completion, not require opening the picker"
    );
}

#[test]
fn test_str_field_needs_pathy_text() {
    let temp = tempfile::tempdir().expect("tempdir");
    fs::write(temp.path().join("data-ghost-unique.csv"), b"x").expect("data");
    let invoke = tempfile::tempdir().expect("invoke");

    let mut bare = state_for(
        ParameterType::Str,
        false,
        false,
        temp.path(),
        invoke.path(),
        None,
        BTreeMap::new(),
    );
    assert!(!rendered_after_typing(&mut bare, "data-gh").contains("data-ghost-unique.csv"));

    let mut pathy = state_for(
        ParameterType::Str,
        false,
        false,
        temp.path(),
        invoke.path(),
        None,
        BTreeMap::new(),
    );
    assert!(
        rendered_after_typing(&mut pathy, "./data-gh").contains("./data-ghost-unique.csv"),
        "a plain string becomes path-completable only after path syntax is explicit"
    );
}

#[test]
fn test_hidden_entries_only_behind_a_dot_prefix() {
    let temp = tempfile::tempdir().expect("tempdir");
    fs::write(temp.path().join(".hidden-ghost-unique"), b"x").expect("hidden");
    let invoke = tempfile::tempdir().expect("invoke");

    let mut plain = state_for(
        ParameterType::Path,
        false,
        false,
        temp.path(),
        invoke.path(),
        None,
        BTreeMap::new(),
    );
    assert!(!rendered_after_typing(&mut plain, "hid").contains(".hidden-ghost-unique"));

    let mut dotted = state_for(
        ParameterType::Path,
        false,
        false,
        temp.path(),
        invoke.path(),
        None,
        BTreeMap::new(),
    );
    assert!(rendered_after_typing(&mut dotted, ".hid").contains(".hidden-ghost-unique"));
}

#[test]
fn test_cwd_token_completes_at_invoke_cwd_not_workdir() {
    let workdir = tempfile::tempdir().expect("workdir");
    fs::write(workdir.path().join("work-note-unique.txt"), b"x").expect("work note");
    let invoke = tempfile::tempdir().expect("invoke");
    fs::write(invoke.path().join("invoke-note-unique.txt"), b"x").expect("invoke note");
    let mut state = state_for(
        ParameterType::Path,
        false,
        false,
        workdir.path(),
        invoke.path(),
        None,
        BTreeMap::new(),
    );
    let rendered = rendered_after_typing(&mut state, "{cwd}/invoke-no");
    assert!(rendered.contains("{cwd}/invoke-note-unique.txt"));
    assert!(!rendered.contains("work-note-unique.txt"));
}

#[test]
fn test_relative_env_token_falls_back_to_the_workdir_rule() {
    let workdir = tempfile::tempdir().expect("workdir");
    fs::create_dir(workdir.path().join("sub")).expect("sub");
    fs::write(workdir.path().join("sub/inner-ghost-unique.txt"), b"x").expect("inner");
    let invoke = tempfile::tempdir().expect("invoke");
    let mut state = state_for(
        ParameterType::Path,
        false,
        false,
        workdir.path(),
        invoke.path(),
        None,
        BTreeMap::from([("REL".to_owned(), "sub".to_owned())]),
    );
    assert!(
        rendered_after_typing(&mut state, "{env:REL}/inner-gh")
            .contains("{env:REL}/inner-ghost-unique.txt")
    );
}

#[test]
fn test_home_prefix_completes_inside_home() {
    let workdir = tempfile::tempdir().expect("workdir");
    let invoke = tempfile::tempdir().expect("invoke");
    let home = tempfile::tempdir().expect("home");
    fs::write(home.path().join("home-note-ghost-unique.txt"), b"x").expect("home note");
    let mut state = state_for(
        ParameterType::Path,
        false,
        false,
        workdir.path(),
        invoke.path(),
        Some(home.path()),
        BTreeMap::new(),
    );
    assert!(
        rendered_after_typing(&mut state, "~/home-note-gh")
            .contains("~/home-note-ghost-unique.txt")
    );
}

#[test]
fn test_shlexy_field_completes_only_the_trailing_piece() {
    let workdir = tempfile::tempdir().expect("workdir");
    fs::write(workdir.path().join("draft-ghost-unique.txt"), b"x").expect("draft");
    let invoke = tempfile::tempdir().expect("invoke");
    let form = RunFormView::from_declarations(
        "job",
        "job",
        &[],
        &BTreeMap::new(),
        &[],
        "",
        &BTreeMap::new(),
        "",
    )
    .with_context(RunFormContext {
        entry_kind: "python".to_owned(),
        path: Some(RunPathContext {
            workdir: workdir.path().to_string_lossy().into_owned(),
            invoke_cwd: invoke.path().to_string_lossy().into_owned(),
        }),
        tokens: TokenContext {
            cwd: invoke.path().to_string_lossy().into_owned(),
            home: None,
            env: BTreeMap::new(),
            today: "2026-08-14".to_owned(),
            now: "15-57-00".to_owned(),
        },
    });
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Run(Box::new(form))));
    assert!(
        rendered_after_typing(&mut state, "first.txt draft-gh")
            .contains("first.txt draft-ghost-unique.txt")
    );
}

#[test]
fn test_path_fields_render_hint_and_suggester() {
    let workdir = tempfile::tempdir().expect("workdir");
    fs::write(workdir.path().join("hint-ghost-unique.txt"), b"x").expect("hint file");
    let invoke = tempfile::tempdir().expect("invoke");
    let mut state = state_for(
        ParameterType::Path,
        false,
        false,
        workdir.path(),
        invoke.path(),
        None,
        BTreeMap::new(),
    );
    let mut session = TuiSession::default();
    let (terminal, geometry) = draw(&mut session, &state);
    let before = buffer_text(terminal.backend().buffer());
    assert!(before.contains("browse"), "path fields must render the browse affordance");
    type_text(&mut session, &mut state, &geometry, "hint-gh");
    let (terminal, _) = draw(&mut session, &state);
    assert!(buffer_text(terminal.backend().buffer()).contains("hint-ghost-unique.txt"));
}

#[test]
fn test_secret_field_never_gets_a_suggester() {
    let workdir = tempfile::tempdir().expect("workdir");
    fs::write(workdir.path().join("secret-ghost-unique.txt"), b"x").expect("secret file");
    let invoke = tempfile::tempdir().expect("invoke");
    let mut state = state_for(
        ParameterType::Str,
        true,
        false,
        workdir.path(),
        invoke.path(),
        None,
        BTreeMap::new(),
    );
    let rendered = rendered_after_typing(&mut state, "./secret-gh");
    assert!(!rendered.contains("secret-ghost-unique.txt"));
}

#[test]
fn test_suggester_is_case_sensitive_query_not_casefolded() {
    let workdir = tempfile::tempdir().expect("workdir");
    fs::write(workdir.path().join("DATA-GHOST-UNIQUE.csv"), b"x").expect("data");
    let invoke = tempfile::tempdir().expect("invoke");
    let mut state = state_for(
        ParameterType::Path,
        false,
        false,
        workdir.path(),
        invoke.path(),
        None,
        BTreeMap::new(),
    );
    let rendered = rendered_after_typing(&mut state, "DA");
    assert!(rendered.contains("DATA-GHOST-UNIQUE.csv"));
    assert!(!rendered.contains("data-ghost-unique.csv"));
}

#[test]
fn test_suggester_does_not_cache_stale_results() {
    let workdir = tempfile::tempdir().expect("workdir");
    let target = workdir.path().join("stale-ghost-unique.txt");
    fs::write(&target, b"x").expect("target");
    let invoke = tempfile::tempdir().expect("invoke");
    let mut state = state_for(
        ParameterType::Path,
        false,
        false,
        workdir.path(),
        invoke.path(),
        None,
        BTreeMap::new(),
    );
    let mut session = TuiSession::default();
    let (_, geometry) = draw(&mut session, &state);
    type_text(&mut session, &mut state, &geometry, "stale-gh");
    let (terminal, _) = draw(&mut session, &state);
    assert!(buffer_text(terminal.backend().buffer()).contains("stale-ghost-unique.txt"));

    fs::remove_file(target).expect("remove target");
    let (terminal, _) = draw(&mut session, &state);
    assert!(
        !buffer_text(terminal.backend().buffer()).contains("stale-ghost-unique.txt"),
        "the same query must re-read the filesystem rather than return a cached completion"
    );
}
