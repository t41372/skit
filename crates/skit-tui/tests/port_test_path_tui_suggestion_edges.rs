use std::{collections::BTreeMap, fs, path::Path};

use ratatui_core::{backend::TestBackend, buffer::Buffer, terminal::Terminal};
use ratatui_crossterm::crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use skit_application::tokens::TokenContext;
use skit_domain::parameters::{ParamDecl, ParameterDelivery, ParameterType};
use skit_i18n::Locale;
use skit_tui::{EventHandling, TuiSession, ViewGeometry, render_with_session};
use skit_ui::{Action, LibraryState, RunFormContext, RunFormView, RunPathContext, Screen};

fn path_form(
    workdir: &Path,
    invoke_cwd: &Path,
    env: BTreeMap<String, String>,
    placeholder: bool,
) -> LibraryState {
    let mut value = ParamDecl::new("value");
    value.parameter_type = ParameterType::Path;
    if placeholder {
        value.delivery = ParameterDelivery::Placeholder;
    }
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
            home: None,
            env,
            today: "2026-08-14".to_owned(),
            now: "15-57-00".to_owned(),
        },
    });
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Run(Box::new(form))));
    state
}

fn extra_args_form(workdir: &Path, invoke_cwd: &Path) -> LibraryState {
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
            workdir: workdir.to_string_lossy().into_owned(),
            invoke_cwd: invoke_cwd.to_string_lossy().into_owned(),
        }),
        tokens: TokenContext {
            cwd: invoke_cwd.to_string_lossy().into_owned(),
            home: None,
            env: BTreeMap::new(),
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

fn text(buffer: &Buffer) -> String {
    buffer.content().iter().map(|cell| cell.symbol()).collect()
}

fn type_text(
    session: &mut TuiSession,
    state: &mut LibraryState,
    geometry: &ViewGeometry,
    value: &str,
) {
    for character in value.chars() {
        match session.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE)),
            state,
            geometry,
        ) {
            EventHandling::Action(action) => {
                state.update(action);
            }
            EventHandling::Consumed => {}
            EventHandling::Ignored => panic!("run form ignored {character:?}"),
        }
    }
}

fn after_typing(state: &mut LibraryState, value: &str) -> String {
    let mut session = TuiSession::default();
    let (_, geometry) = draw(&mut session, state);
    type_text(&mut session, state, &geometry, value);
    let (terminal, _) = draw(&mut session, state);
    text(terminal.backend().buffer())
}

#[test]
fn test_secretless_activation_never_guesses_beyond_prefix() {
    let root = tempfile::tempdir().expect("root");
    fs::write(root.path().join("data-should-not-appear.csv"), b"x").expect("data");
    let invoke = tempfile::tempdir().expect("invoke");
    let mut state = path_form(root.path(), invoke.path(), BTreeMap::new(), false);
    let rendered = after_typing(&mut state, "zzz-no-such-prefix");
    assert!(!rendered.contains("data-should-not-appear.csv"));
}

#[test]
fn test_unset_env_token_is_silence_not_a_traceback() {
    let root = tempfile::tempdir().expect("root");
    fs::write(root.path().join("data-env-unique.csv"), b"x").expect("data");
    let invoke = tempfile::tempdir().expect("invoke");
    let mut state = path_form(root.path(), invoke.path(), BTreeMap::new(), false);
    let rendered = after_typing(&mut state, "{env:SKIT_NO_SUCH_VAR}/da");
    assert!(!rendered.contains("data-env-unique.csv"));
    assert_eq!(
        state.run_form().expect("run form").fields()[0]
            .control
            .value(),
        "{env:SKIT_NO_SUCH_VAR}/da"
    );
}

#[test]
fn test_missing_workdir_silences_bare_completion() {
    let temp = tempfile::tempdir().expect("temp");
    let missing = temp.path().join("missing/workdir");
    let invoke = temp.path().join("invoke");
    fs::create_dir(&invoke).expect("invoke");
    fs::write(invoke.join("data-invoke-only.csv"), b"x").expect("invoke file");
    let mut state = path_form(&missing, &invoke, BTreeMap::new(), false);
    let rendered = after_typing(&mut state, "da");
    assert!(!rendered.contains("data-invoke-only.csv"));
}

#[test]
fn test_missing_workdir_silences_relative_token_lookup() {
    let temp = tempfile::tempdir().expect("temp");
    let missing = temp.path().join("missing/workdir");
    let invoke = temp.path().join("invoke");
    fs::create_dir(&invoke).expect("invoke");
    fs::create_dir(invoke.join("sub")).expect("invoke sub");
    fs::write(invoke.join("sub/data-invoke-only.csv"), b"x").expect("invoke file");
    let mut state = path_form(
        &missing,
        &invoke,
        BTreeMap::from([("REL".to_owned(), "sub".to_owned())]),
        false,
    );
    let rendered = after_typing(&mut state, "{env:REL}/da");
    assert!(!rendered.contains("data-invoke-only.csv"));
}

#[test]
fn test_brace_escapes_on_a_normal_field_halves_doubled_braces() {
    let root = tempfile::tempdir().expect("root");
    fs::create_dir(root.path().join("{x}")).expect("brace dir");
    fs::write(root.path().join("{x}/data-brace-unique.csv"), b"x").expect("data");
    let invoke = tempfile::tempdir().expect("invoke");
    let mut state = path_form(root.path(), invoke.path(), BTreeMap::new(), false);
    let rendered = after_typing(&mut state, "{{x}}/data-br");
    assert!(
        rendered.contains("{{x}}/data-brace-unique.csv"),
        "normal fields halve doubled braces for lookup but preserve the user's doubled spelling"
    );
}

#[test]
fn test_brace_escapes_off_on_a_placeholder_field_keeps_doubled_braces() {
    let root = tempfile::tempdir().expect("root");
    fs::create_dir(root.path().join("{x}")).expect("brace dir");
    fs::write(root.path().join("{x}/data-brace-unique.csv"), b"x").expect("data");
    let invoke = tempfile::tempdir().expect("invoke");
    let mut state = path_form(root.path(), invoke.path(), BTreeMap::new(), true);
    let rendered = after_typing(&mut state, "{{x}}/data-br");
    assert!(!rendered.contains("{{x}}/data-brace-unique.csv"));
}

#[test]
fn test_shlexy_trailing_piece_refuses_either_quote() {
    let root = tempfile::tempdir().expect("root");
    fs::write(root.path().join("draft-quote-unique.txt"), b"x").expect("draft");
    fs::write(root.path().join("'q.txt"), b"x").expect("single quote");
    #[cfg(not(windows))]
    fs::write(root.path().join("\"q.txt"), b"x").expect("double quote");
    let invoke = tempfile::tempdir().expect("invoke");

    for value in ["done.txt 'q", "done.txt \"q"] {
        let mut state = extra_args_form(root.path(), invoke.path());
        let rendered = after_typing(&mut state, value);
        assert!(!rendered.contains("'q.txt"));
        assert!(!rendered.contains("\"q.txt"));
    }

    let mut clean = extra_args_form(root.path(), invoke.path());
    assert!(
        after_typing(&mut clean, "done.txt draft-q").contains("done.txt draft-quote-unique.txt"),
        "quote refusal must not disable clean trailing-piece completion"
    );
}

#[test]
fn test_bare_token_prefix_without_separator_is_silent() {
    let root = tempfile::tempdir().expect("root");
    fs::write(root.path().join("~data-token-unique.txt"), b"x").expect("tilde file");
    fs::write(root.path().join("{data-token-unique.txt"), b"x").expect("brace file");
    let invoke = tempfile::tempdir().expect("invoke");

    for value in ["~da", "{da"] {
        let mut state = path_form(root.path(), invoke.path(), BTreeMap::new(), false);
        let rendered = after_typing(&mut state, value);
        assert!(!rendered.contains("data-token-unique.txt"));
    }
}
