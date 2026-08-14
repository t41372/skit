use std::{collections::BTreeMap, fs, path::PathBuf};

use ratatui_core::{backend::TestBackend, buffer::Buffer, terminal::Terminal};
use ratatui_crossterm::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use skit_application::tokens::TokenContext;
use skit_domain::parameters::{ParamDecl, ParameterType};
use skit_i18n::Locale;
use skit_tui::{
    EventHandling, FilePickerEvent, FilePickerGeometry, FilePickerHit, FilePickerSession, HitTarget,
    TuiSession, ViewGeometry, render_file_picker, render_with_session,
};
use skit_ui::{
    Action, LibraryState, PathOutputPolicy, PathPickerState, PathSelectionMode, PickerPurpose,
    RunFormContext, RunFormView, RunPathContext, RunTokenOption, Screen, UiCommand,
};

fn key(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn mouse(column: u16, row: u16) -> Event {
    Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column,
        row,
        modifiers: KeyModifiers::NONE,
    })
}

fn context(workdir: &std::path::Path) -> RunFormContext {
    RunFormContext {
        entry_kind: "python".to_owned(),
        path: Some(RunPathContext {
            workdir: workdir.to_string_lossy().into_owned(),
            invoke_cwd: workdir.to_string_lossy().into_owned(),
        }),
        tokens: TokenContext {
            cwd: workdir.to_string_lossy().into_owned(),
            home: Some(workdir.to_string_lossy().into_owned()),
            env: BTreeMap::new(),
            today: "2026-08-14".to_owned(),
            now: "15-57-00".to_owned(),
        },
    }
}

fn draw(
    session: &mut TuiSession,
    state: &LibraryState,
    width: u16,
    height: u16,
) -> (Terminal<TestBackend>, ViewGeometry) {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
    let mut geometry = ViewGeometry::default();
    terminal
        .draw(|frame| {
            geometry = render_with_session(frame, state, Locale::En, session);
        })
        .expect("draw");
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

fn buffer_text(buffer: &Buffer) -> String {
    buffer.content().iter().map(|cell| cell.symbol()).collect()
}

fn row_text(buffer: &Buffer, needle: &str) -> String {
    (0..buffer.area.height)
        .find_map(|row| {
            let line = (0..buffer.area.width)
                .map(|column| buffer[(column, row)].symbol())
                .collect::<String>();
            line.contains(needle).then_some(line)
        })
        .unwrap_or_else(|| panic!("missing rendered row containing {needle:?}"))
}

#[test]
fn test_picker_esc_cancels_and_up_chip_is_clickable() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    fs::create_dir(root.join("sub")).expect("sub");
    let mut picker = FilePickerSession::new(PathPickerState::new(
        PickerPurpose::Argument,
        root.join("sub"),
        PathSelectionMode::FileOrDirectory,
        PathOutputPolicy::RelativeTo(root.to_path_buf()),
        false,
    ));
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).expect("terminal");
    let mut geometry = FilePickerGeometry::default();
    terminal
        .draw(|frame| {
            geometry = render_file_picker(frame, frame.area(), &mut picker, Locale::En);
        })
        .expect("draw picker");
    let up = geometry
        .hits
        .iter()
        .find(|hit| hit.target == FilePickerHit::Up)
        .expect("the visible Up chip must have a mouse hit target");
    assert_eq!(
        picker.handle_event(mouse(up.area.x, up.area.y), &geometry),
        Some(FilePickerEvent::Changed)
    );
    assert_eq!(picker.current_dir(), root);
    assert_eq!(
        picker.handle_event(key(KeyCode::Esc), &geometry),
        Some(FilePickerEvent::Cancelled)
    );
}

#[test]
fn test_token_rows_still_insert_at_cursor() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut path = ParamDecl::new("src");
    path.parameter_type = ParameterType::Path;
    path.prompt = "Source".to_owned();
    let form = RunFormView::from_declarations(
        "job",
        "job",
        &[path],
        &BTreeMap::from([("src".to_owned(), "out-.csv".to_owned())]),
        &[],
        "",
        &BTreeMap::new(),
        "",
    )
    .with_context(context(temp.path()));
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Run(Box::new(form))));
    let mut session = TuiSession::default();
    let (_, geometry) = draw(&mut session, &state, 100, 28);

    assert_eq!(
        drive(&mut session, &mut state, &geometry, key(KeyCode::Home)),
        EventHandling::Consumed
    );
    for _ in 0..4 {
        assert_eq!(
            drive(&mut session, &mut state, &geometry, key(KeyCode::Right)),
            EventHandling::Consumed
        );
    }
    state.update(Action::OpenRunTokenMenuFor(0));
    let options = match state.modal() {
        Some(skit_ui::ModalState::RunTokenMenu { options, .. }) => options.clone(),
        other => panic!("expected token menu, got {other:?}"),
    };
    let today = options
        .iter()
        .position(|option| option == &RunTokenOption::Today)
        .expect("today token");
    let (_, modal_geometry) = draw(&mut session, &state, 100, 28);
    for _ in 0..today {
        assert_eq!(
            drive(
                &mut session,
                &mut state,
                &modal_geometry,
                key(KeyCode::Down)
            ),
            EventHandling::Consumed
        );
    }
    let handling = drive(
        &mut session,
        &mut state,
        &modal_geometry,
        key(KeyCode::Enter),
    );
    assert!(matches!(handling, EventHandling::Action(_)));
    assert_eq!(
        state.run_form().expect("run form").fields()[0]
            .control
            .value(),
        "out-{today}.csv"
    );
}

#[test]
fn test_browse_link_renders_on_text_fields_only() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut src = ParamDecl::new("src");
    src.prompt = "Source path".to_owned();
    src.parameter_type = ParameterType::Path;
    let mut note = ParamDecl::new("note");
    note.prompt = "Plain note".to_owned();
    let mut count = ParamDecl::new("count");
    count.prompt = "Count value".to_owned();
    count.parameter_type = ParameterType::Int;
    let mut loud = ParamDecl::new("loud");
    loud.prompt = "Loud toggle".to_owned();
    loud.parameter_type = ParameterType::Bool;

    let form = RunFormView::from_declarations(
        "mixed",
        "mixed",
        &[src, note, count, loud],
        &BTreeMap::new(),
        &[],
        "",
        &BTreeMap::new(),
        "",
    )
    .with_context(context(temp.path()));
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Run(Box::new(form))));
    let mut session = TuiSession::default();
    let (terminal, geometry) = draw(&mut session, &state, 120, 44);
    let buffer = terminal.backend().buffer();

    for (field, label) in [(0, "Source path"), (1, "Plain note"), (4, "Extra arguments")] {
        let line = row_text(buffer, label);
        let browse = line.find("browse").expect("browse chip");
        let insert = line.find("insert").expect("insert chip");
        assert!(browse < insert, "Browse must render before Insert on {label}");
        for command in [UiCommand::BrowsePath, UiCommand::InsertValue] {
            assert!(
                geometry.hits.iter().any(|hit| {
                    hit.action == HitTarget::RunFieldCommand { field, command }
                }),
                "{label} lost the typed {command:?} mouse action"
            );
        }
    }

    for (field, label) in [(2, "Count value"), (3, "Loud toggle")] {
        let line = row_text(buffer, label);
        assert!(!line.contains("browse"), "{label} must not advertise Browse");
        assert!(
            !geometry.hits.iter().any(|hit| {
                hit.action == HitTarget::RunFieldCommand {
                    field,
                    command: UiCommand::BrowsePath,
                }
            }),
            "{label} must not have a hidden Browse click target"
        );
    }
    assert!(
        geometry.hits.iter().any(|hit| {
            hit.action == HitTarget::RunFieldCommand {
                field: 2,
                command: UiCommand::InsertValue,
            }
        }),
        "the integer text row keeps the token insertion menu even though Browse is refused"
    );
    let rendered = buffer_text(buffer);
    assert!(rendered.contains("Source path"));
    assert!(rendered.contains("Plain note"));
}
