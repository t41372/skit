use std::{collections::BTreeMap, fs, path::Path};

use ratatui_core::{backend::TestBackend, terminal::Terminal};
use ratatui_crossterm::crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use skit_application::tokens::TokenContext;
use skit_domain::parameters::{ParamDecl, ParameterType};
use skit_i18n::Locale;
use skit_tui::{EventHandling, TuiSession, ViewGeometry, render_with_session};
use skit_ui::{
    Action, LibraryState, RunFormContext, RunFormView, RunPathContext, Screen,
};

fn key(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn picker_state(workdir: &Path, invoke_cwd: &Path) -> LibraryState {
    let mut path = ParamDecl::new("path");
    path.parameter_type = ParameterType::Path;
    let form = RunFormView::from_declarations(
        "job",
        "job",
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
    state.update(Action::OpenFocusedRunFilePicker);
    state
}

fn render_state(session: &mut TuiSession, state: &LibraryState) -> (String, ViewGeometry) {
    let mut terminal = Terminal::new(TestBackend::new(110, 32)).expect("terminal");
    let mut geometry = ViewGeometry::default();
    terminal
        .draw(|frame| {
            geometry = render_with_session(frame, state, Locale::En, session);
        })
        .expect("draw");
    let text = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    (text, geometry)
}

#[test]
fn test_picker_start_last_resort_is_the_invoke_cwd() {
    let temp = tempfile::tempdir().expect("tempdir");
    let invoke = temp.path().join("invoke");
    fs::create_dir(&invoke).expect("invoke dir");
    let workdir = std::path::PathBuf::from("missing-relative-root/leaf");
    let state = picker_state(&workdir, &invoke);
    let mut session = TuiSession::default();
    let (text, _) = render_state(&mut session, &state);
    assert!(
        text.contains(&invoke.to_string_lossy().into_owned()),
        "when no workdir ancestor exists, the picker must start at invoke_cwd"
    );
}

#[test]
fn test_picker_start_degrades_to_nearest_existing_ancestor() {
    let temp = tempfile::tempdir().expect("tempdir");
    let existing = temp.path().join("root/existing");
    fs::create_dir_all(&existing).expect("existing ancestor");
    let invoke = temp.path().join("invoke");
    fs::create_dir(&invoke).expect("invoke dir");
    let missing = existing.join("gone/deeper");
    let state = picker_state(&missing, &invoke);
    let mut session = TuiSession::default();
    let (text, _) = render_state(&mut session, &state);
    assert!(
        text.contains(&existing.to_string_lossy().into_owned()),
        "the picker must start at the nearest existing workdir ancestor"
    );
}

#[test]
fn test_picker_missing_workdir_opens_at_ancestor_with_notice() {
    let temp = tempfile::tempdir().expect("tempdir");
    let existing = temp.path().join("root/existing");
    fs::create_dir_all(&existing).expect("existing ancestor");
    let missing = existing.join("gone/deeper");
    let state = picker_state(&missing, temp.path());
    let mut session = TuiSession::default();
    let (text, _) = render_state(&mut session, &state);
    assert!(text.contains(&existing.to_string_lossy().into_owned()));
    assert!(
        text.contains("The entry's working directory is missing — starting here instead."),
        "the degraded root is user-visible, not a silent fallback"
    );
}

#[test]
fn test_picker_use_this_directory_row_by_real_keys() {
    let temp = tempfile::tempdir().expect("tempdir");
    fs::write(temp.path().join("real.txt"), b"x").expect("file");
    let mut state = picker_state(temp.path(), temp.path());
    let mut session = TuiSession::default();
    let (_text, geometry) = render_state(&mut session, &state);

    assert_eq!(
        session.handle_event(key(KeyCode::Up), &state, &geometry),
        EventHandling::Consumed
    );
    assert_eq!(
        session.handle_event(key(KeyCode::Enter), &state, &geometry),
        EventHandling::Action(Action::SetRunPickedPathAndCloseModal {
            field: 0,
            path: ".".to_owned(),
        })
    );

    // The reducer must accept the same action shape without a test-only shortcut.
    state.update(Action::SetRunPickedPathAndCloseModal {
        field: 0,
        path: ".".to_owned(),
    });
    assert_eq!(
        state.run_form().expect("run form").fields()[0]
            .control
            .value(),
        "."
    );
}

#[test]
fn test_picker_empty_directory_highlights_the_pinned_row() {
    let temp = tempfile::tempdir().expect("tempdir");
    let empty = temp.path().join("empty");
    fs::create_dir(&empty).expect("empty dir");
    let state = picker_state(&empty, temp.path());
    let mut session = TuiSession::default();
    let (_text, geometry) = render_state(&mut session, &state);

    assert_eq!(
        session.handle_event(key(KeyCode::Enter), &state, &geometry),
        EventHandling::Action(Action::SetRunPickedPathAndCloseModal {
            field: 0,
            path: ".".to_owned(),
        }),
        "with no real files the pinned current-directory row must own the highlight"
    );
}
