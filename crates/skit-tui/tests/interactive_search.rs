use ratatui_core::{backend::TestBackend, terminal::Terminal};
use ratatui_crossterm::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use skit_application::LibraryScan;
use skit_domain::{EntryKind, EntrySummary, Slug, StorageMode};
use skit_i18n::Locale;
use skit_tui::{EventHandling, HitTarget, TuiSession, ViewGeometry, render_with_session};
use skit_ui::{Action, Effect, HostRequest, InputMode, LibraryState, UiCommand};

fn state() -> LibraryState {
    state_with_names(&["Unicode"])
}

fn state_with_names(names: &[&str]) -> LibraryState {
    LibraryState::from_scan(LibraryScan {
        entries: names
            .iter()
            .map(|name| EntrySummary {
                slug: Slug::parse(name.to_lowercase()).unwrap(),
                name: (*name).to_owned(),
                kind: EntryKind::parse("python").unwrap(),
                mode: StorageMode::Copy,
                description: String::new(),
                target: None,
            })
            .collect(),
        diagnostics: Vec::new(),
    })
}

fn key(code: KeyCode, modifiers: KeyModifiers) -> Event {
    Event::Key(KeyEvent::new(code, modifiers))
}

fn draw(session: &mut TuiSession, state: &LibraryState) -> (Terminal<TestBackend>, ViewGeometry) {
    let mut terminal = Terminal::new(TestBackend::new(80, 20)).unwrap();
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

fn mouse(button: MouseButton, column: u16, row: u16) -> Event {
    mouse_kind(MouseEventKind::Down(button), column, row)
}

fn mouse_kind(kind: MouseEventKind, column: u16, row: u16) -> Event {
    Event::Mouse(MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    })
}

#[test]
fn global_footer_activates_only_after_a_primary_press_and_release() {
    let state = state();
    let mut session = TuiSession::default();
    let (_, geometry) = draw(&mut session, &state);
    let search = geometry
        .hits
        .iter()
        .find(|hit| hit.action == HitTarget::Command(UiCommand::Search))
        .expect("Search is visible in the global footer")
        .rect;

    assert_eq!(
        session.handle_event(
            mouse_kind(MouseEventKind::Down(MouseButton::Left), search.x, search.y,),
            &state,
            &geometry,
        ),
        EventHandling::Consumed,
        "primary Down must arm the footer without activating it"
    );
    assert_eq!(
        session.handle_event(
            mouse_kind(MouseEventKind::Up(MouseButton::Left), search.x, search.y,),
            &state,
            &geometry,
        ),
        EventHandling::Action(Action::BeginSearch)
    );

    let mut cancelled = TuiSession::default();
    let (_, cancel_geometry) = draw(&mut cancelled, &state);
    let search = cancel_geometry
        .hits
        .iter()
        .find(|hit| hit.action == HitTarget::Command(UiCommand::Search))
        .unwrap()
        .rect;
    let outside = (
        cancel_geometry.rows.x.saturating_sub(1),
        cancel_geometry.rows.y,
    );
    assert!(!cancel_geometry.rows.contains(outside.into()));
    assert!(
        cancel_geometry
            .hits
            .iter()
            .all(|hit| !hit.rect.contains(outside.into())),
        "the cancellation probe must not name another top-level target"
    );
    assert_eq!(
        cancelled.handle_event(
            mouse_kind(MouseEventKind::Down(MouseButton::Left), search.x, search.y,),
            &state,
            &cancel_geometry,
        ),
        EventHandling::Consumed
    );
    assert_eq!(
        cancelled.handle_event(
            mouse_kind(MouseEventKind::Up(MouseButton::Left), outside.0, outside.1),
            &state,
            &cancel_geometry,
        ),
        EventHandling::Ignored
    );
    assert_eq!(
        cancelled.handle_event(
            mouse_kind(MouseEventKind::Up(MouseButton::Left), search.x, search.y,),
            &state,
            &cancel_geometry,
        ),
        EventHandling::Ignored,
        "release outside must cancel the armed footer target"
    );
    for button in [MouseButton::Right, MouseButton::Middle] {
        for kind in [MouseEventKind::Down(button), MouseEventKind::Up(button)] {
            assert_eq!(
                cancelled.handle_event(
                    mouse_kind(kind, search.x, search.y),
                    &state,
                    &cancel_geometry,
                ),
                EventHandling::Ignored,
                "{button:?} must never arm or activate the footer"
            );
        }
    }
}

fn buffer_position(terminal: &Terminal<TestBackend>, needle: &str) -> (u16, u16) {
    let buffer = terminal.backend().buffer();
    for row in 0..buffer.area.height {
        for column in 0..buffer.area.width {
            let tail = (column..buffer.area.width)
                .map(|x| buffer[(x, row)].symbol())
                .collect::<String>();
            if tail.starts_with(needle) {
                return (column, row);
            }
        }
    }
    panic!("missing {needle:?}");
}

#[test]
fn search_uses_grapheme_editing_paste_navigation_and_a_real_cursor() {
    let mut state = state();
    state.update(Action::BeginSearch);
    state.update(Action::SetSearchQuery("e\u{301}👨‍👩‍👧‍👦x".to_owned()));
    let mut session = TuiSession::default();
    let (_, geometry) = draw(&mut session, &state);

    for event in [
        key(KeyCode::Home, KeyModifiers::NONE),
        key(KeyCode::Right, KeyModifiers::NONE),
        key(KeyCode::Delete, KeyModifiers::NONE),
    ] {
        let _ = drive(&mut session, &mut state, &geometry, event);
    }
    assert_eq!(state.query(), "e\u{301}x");

    let _ = drive(
        &mut session,
        &mut state,
        &geometry,
        key(KeyCode::Char('u'), KeyModifiers::CONTROL),
    );
    assert_eq!(state.query(), "");
    let _ = drive(
        &mut session,
        &mut state,
        &geometry,
        Event::Paste("界q".to_owned()),
    );
    assert_eq!(state.query(), "界q");

    let (terminal, _) = draw(&mut session, &state);
    let cursor = terminal.backend().cursor_position();
    assert!(
        cursor.y < 3,
        "search must expose a real cursor in the header"
    );
    assert!(cursor.x > 1);
    assert_eq!(
        session.handle_event(key(KeyCode::Enter, KeyModifiers::NONE), &state, &geometry,),
        EventHandling::Action(Action::OpenRun)
    );
}

#[test]
fn search_ignores_key_release_events() {
    let mut state = state();
    state.update(Action::BeginSearch);
    state.update(Action::SetSearchQuery("abc".to_owned()));
    let mut session = TuiSession::default();
    let (_, geometry) = draw(&mut session, &state);
    let released = Event::Key(KeyEvent::new_with_kind(
        KeyCode::Esc,
        KeyModifiers::NONE,
        ratatui_crossterm::crossterm::event::KeyEventKind::Release,
    ));

    assert_eq!(
        session.handle_event(released, &state, &geometry),
        EventHandling::Ignored
    );
    assert_eq!(state.query(), "abc");
    assert_eq!(
        session.handle_event(key(KeyCode::Esc, KeyModifiers::NONE), &state, &geometry),
        EventHandling::Action(Action::FinishSearch)
    );
}

#[test]
fn clicking_the_search_header_enters_search_mode() {
    let state = state();
    let mut session = TuiSession::default();
    let (_, geometry) = draw(&mut session, &state);
    assert_eq!(
        session.handle_event(
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 2,
                row: 1,
                modifiers: KeyModifiers::NONE,
            }),
            &state,
            &geometry,
        ),
        EventHandling::Consumed,
        "browse-search Down must only arm the semantic input target"
    );
    assert_eq!(
        session.handle_event(
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Up(MouseButton::Left),
                column: 2,
                row: 1,
                modifiers: KeyModifiers::NONE,
            }),
            &state,
            &geometry,
        ),
        EventHandling::Action(Action::BeginSearch)
    );
}

#[test]
fn a_new_frame_drops_the_previous_search_header_hit() {
    let browse = state();
    let mut session = TuiSession::default();
    let _ = draw(&mut session, &browse);

    let mut help = browse.clone();
    help.update(Action::OpenHelp);
    let (_, geometry) = draw(&mut session, &help);
    for kind in [
        MouseEventKind::Down(MouseButton::Left),
        MouseEventKind::Up(MouseButton::Left),
    ] {
        assert_eq!(
            session.handle_event(mouse_kind(kind, 2, 1), &help, &geometry),
            EventHandling::Ignored,
            "the Library Search hit survived into the Help frame"
        );
    }
}

#[test]
fn global_hits_exclude_their_right_and_bottom_edges() {
    let state = state();
    let mut session = TuiSession::default();
    let (_, geometry) = draw(&mut session, &state);
    let contains = |rect: ratatui_core::layout::Rect, column, row| {
        column >= rect.x && column < rect.right() && row >= rect.y && row < rect.bottom()
    };
    let edge_without_another_hit = |right: bool| {
        geometry.hits.iter().find_map(|hit| {
            let point = if right {
                (hit.rect.right(), hit.rect.y)
            } else {
                (hit.rect.x, hit.rect.bottom())
            };
            (point.0 < 80
                && point.1 < 20
                && !geometry
                    .hits
                    .iter()
                    .any(|candidate| contains(candidate.rect, point.0, point.1)))
            .then_some(point)
        })
    };

    for (label, point) in [
        (
            "right",
            edge_without_another_hit(true).expect("a footer hit has a free right edge"),
        ),
        (
            "bottom",
            edge_without_another_hit(false).expect("a footer hit has a free bottom edge"),
        ),
    ] {
        assert_eq!(
            session.handle_event(
                mouse_kind(MouseEventKind::Down(MouseButton::Left), point.0, point.1),
                &state,
                &geometry,
            ),
            EventHandling::Ignored,
            "the exclusive {label} edge armed a global hit"
        );
    }
}

#[test]
fn clicking_after_b_moves_the_search_cursor_before_typing() {
    let mut state = state();
    state.update(Action::BeginSearch);
    state.update(Action::SetSearchQuery("abcdef".to_owned()));
    let mut session = TuiSession::default();
    let (terminal, geometry) = draw(&mut session, &state);
    let (start, row) = buffer_position(&terminal, "abcdef");

    assert_eq!(
        session.handle_event(mouse(MouseButton::Left, start + 2, row), &state, &geometry),
        EventHandling::Consumed,
        "a click inside the active search input must move its cursor"
    );
    assert_eq!(
        session.handle_event(
            mouse_kind(MouseEventKind::Up(MouseButton::Left), start + 2, row),
            &state,
            &geometry,
        ),
        EventHandling::Consumed
    );
    let _ = drive(
        &mut session,
        &mut state,
        &geometry,
        key(KeyCode::Char('X'), KeyModifiers::NONE),
    );
    assert_eq!(state.query(), "abXcdef");
}

#[test]
fn nonprimary_search_presses_do_not_move_the_caret() {
    for button in [MouseButton::Right, MouseButton::Middle] {
        let mut state = state();
        state.update(Action::BeginSearch);
        state.update(Action::SetSearchQuery("abcdef".to_owned()));
        let mut session = TuiSession::default();
        let (terminal, geometry) = draw(&mut session, &state);
        let (start, row) = buffer_position(&terminal, "abcdef");

        assert_eq!(
            session.handle_event(mouse(button, start + 2, row), &state, &geometry),
            EventHandling::Ignored
        );
        let _ = drive(
            &mut session,
            &mut state,
            &geometry,
            key(KeyCode::Char('X'), KeyModifiers::NONE),
        );
        assert_eq!(state.query(), "abcdefX", "{button:?} moved the caret");
    }
}

#[test]
fn resize_cancels_active_search_arm_but_keeps_the_placed_caret() {
    let mut state = state();
    state.update(Action::BeginSearch);
    state.update(Action::SetSearchQuery("abcdef".to_owned()));
    let mut session = TuiSession::default();
    let (terminal, geometry) = draw(&mut session, &state);
    let (start, row) = buffer_position(&terminal, "abcdef");

    assert_eq!(
        session.handle_event(mouse(MouseButton::Left, start + 2, row), &state, &geometry),
        EventHandling::Consumed
    );
    assert_eq!(
        session.handle_event(Event::Resize(79, 19), &state, &geometry),
        EventHandling::Ignored
    );
    let (terminal, resized) = draw(&mut session, &state);
    let (start, row) = buffer_position(&terminal, "abcdef");
    assert_eq!(
        session.handle_event(
            mouse_kind(MouseEventKind::Up(MouseButton::Left), start + 2, row),
            &state,
            &resized,
        ),
        EventHandling::Ignored,
        "a resize must cancel the Search arm from the old geometry"
    );
    let handling = drive(
        &mut session,
        &mut state,
        &resized,
        key(KeyCode::Char('X'), KeyModifiers::NONE),
    );
    assert_eq!(
        handling,
        EventHandling::Action(Action::SetSearchQuery("abXcdef".to_owned()))
    );
    assert_eq!(state.query(), "abXcdef");
}

#[test]
fn clicking_browse_search_places_the_cursor_before_entering_search() {
    let mut state = state();
    state.update(Action::BeginSearch);
    state.update(Action::SetSearchQuery("abcdef".to_owned()));
    state.update(Action::FinishSearch);
    let mut session = TuiSession::default();
    let (terminal, geometry) = draw(&mut session, &state);
    let (start, row) = buffer_position(&terminal, "abcdef");

    assert_eq!(
        session.handle_event(mouse(MouseButton::Left, start + 2, row), &state, &geometry),
        EventHandling::Consumed
    );
    let handling = session.handle_event(
        mouse_kind(MouseEventKind::Up(MouseButton::Left), start + 2, row),
        &state,
        &geometry,
    );
    assert_eq!(handling, EventHandling::Action(Action::BeginSearch));
    let EventHandling::Action(action) = handling else {
        unreachable!();
    };
    state.update(action);
    let _ = drive(
        &mut session,
        &mut state,
        &geometry,
        key(KeyCode::Char('X'), KeyModifiers::NONE),
    );
    assert_eq!(state.query(), "abXcdef");
}

#[test]
fn invalid_row_selection_does_not_leave_search() {
    let mut state = state_with_names(&["Alpha", "Beta"]);
    state.update(Action::BeginSearch);
    state.update(Action::SelectVisible(usize::MAX));
    assert_eq!(state.input_mode(), InputMode::Search);
}

#[test]
fn library_rows_select_then_activate_and_leave_search_mode() {
    let mut state = state_with_names(&["Alpha", "Beta", "Gamma"]);
    state.update(Action::BeginSearch);
    let mut session = TuiSession::default();
    let (_, geometry) = draw(&mut session, &state);
    let second_row = geometry.rows.y.saturating_add(1);
    assert_eq!(
        session.handle_event(
            mouse_kind(
                MouseEventKind::Down(MouseButton::Left),
                geometry.rows.x,
                second_row,
            ),
            &state,
            &geometry,
        ),
        EventHandling::Consumed
    );
    let select = session.handle_event(
        mouse_kind(
            MouseEventKind::Up(MouseButton::Left),
            geometry.rows.x,
            second_row,
        ),
        &state,
        &geometry,
    );
    assert_eq!(select, EventHandling::Action(Action::SelectVisible(1)));
    let EventHandling::Action(select) = select else {
        unreachable!();
    };
    assert_eq!(state.update(select), Effect::None);
    assert_eq!(
        state.selected().map(|entry| entry.name.as_str()),
        Some("Beta")
    );
    assert_eq!(
        state.input_mode(),
        InputMode::Browse,
        "a row click must leave Search so the next click can activate it"
    );

    let (_, geometry) = draw(&mut session, &state);
    assert_eq!(
        session.handle_event(
            mouse_kind(
                MouseEventKind::Down(MouseButton::Left),
                geometry.rows.x,
                second_row,
            ),
            &state,
            &geometry,
        ),
        EventHandling::Consumed
    );
    let open = session.handle_event(
        mouse_kind(
            MouseEventKind::Up(MouseButton::Left),
            geometry.rows.x,
            second_row,
        ),
        &state,
        &geometry,
    );
    assert_eq!(open, EventHandling::Action(Action::OpenRun));
    let EventHandling::Action(open) = open else {
        unreachable!();
    };
    assert_eq!(
        state.update(open),
        Effect::Open {
            request: HostRequest::Run,
            selector: Some("beta".to_owned()),
        }
    );

    let boot = state_with_names(&["Alpha", "Beta"]);
    let mut boot_session = TuiSession::default();
    let (_, geometry) = draw(&mut boot_session, &boot);
    assert_eq!(
        boot_session.handle_event(
            mouse_kind(
                MouseEventKind::Down(MouseButton::Left),
                geometry.rows.x,
                geometry.rows.y,
            ),
            &boot,
            &geometry,
        ),
        EventHandling::Consumed
    );
    assert_eq!(
        boot_session.handle_event(
            mouse_kind(
                MouseEventKind::Up(MouseButton::Left),
                geometry.rows.x,
                geometry.rows.y,
            ),
            &boot,
            &geometry,
        ),
        EventHandling::Action(Action::OpenRun),
        "the boot-selected row must activate on its first click"
    );
}

#[test]
fn right_and_middle_clicks_do_not_activate_search_rows_or_global_footer() {
    let state = state_with_names(&["Alpha", "Beta"]);
    let mut session = TuiSession::default();
    let (_, geometry) = draw(&mut session, &state);
    let footer = geometry
        .hits
        .iter()
        .find(|hit| matches!(hit.action, HitTarget::Command(_)))
        .expect("the global footer has a visible command")
        .rect;
    for button in [MouseButton::Right, MouseButton::Middle] {
        for (column, row, owner) in [
            (2, 1, "search"),
            (geometry.rows.x, geometry.rows.y, "Library row"),
            (footer.x, footer.y, "global footer"),
        ] {
            assert_eq!(
                session.handle_event(mouse(button, column, row), &state, &geometry),
                EventHandling::Ignored,
                "{owner} accepted {button:?}"
            );
        }
    }
}

#[test]
fn library_rows_cancel_mismatched_outside_and_nonprimary_click_sequences() {
    let state = state_with_names(&["Alpha", "Beta"]);
    let mut session = TuiSession::default();
    let (_, geometry) = draw(&mut session, &state);
    let first = (geometry.rows.x, geometry.rows.y);
    let second = (geometry.rows.x, geometry.rows.y.saturating_add(1));

    for (release, label) in [(second, "a different row"), ((0, 0), "outside the rows")] {
        assert_eq!(
            session.handle_event(
                mouse_kind(MouseEventKind::Down(MouseButton::Left), first.0, first.1),
                &state,
                &geometry,
            ),
            EventHandling::Consumed
        );
        assert_eq!(
            session.handle_event(
                mouse_kind(MouseEventKind::Up(MouseButton::Left), release.0, release.1,),
                &state,
                &geometry,
            ),
            EventHandling::Ignored,
            "release over {label} activated the pressed row"
        );
        assert_eq!(
            session.handle_event(
                mouse_kind(MouseEventKind::Up(MouseButton::Left), first.0, first.1),
                &state,
                &geometry,
            ),
            EventHandling::Ignored,
            "a cancelled Library row accepted a later release"
        );
    }

    for button in [MouseButton::Right, MouseButton::Middle] {
        assert_eq!(
            session.handle_event(
                mouse_kind(MouseEventKind::Down(MouseButton::Left), first.0, first.1),
                &state,
                &geometry,
            ),
            EventHandling::Consumed
        );
        assert_eq!(
            session.handle_event(
                mouse_kind(MouseEventKind::Down(button), first.0, first.1),
                &state,
                &geometry,
            ),
            EventHandling::Ignored
        );
        assert_eq!(
            session.handle_event(
                mouse_kind(MouseEventKind::Up(MouseButton::Left), first.0, first.1),
                &state,
                &geometry,
            ),
            EventHandling::Ignored,
            "{button:?} did not cancel the armed Library row"
        );
    }

    assert_eq!(
        session.handle_event(
            mouse_kind(MouseEventKind::Down(MouseButton::Left), first.0, first.1),
            &state,
            &geometry,
        ),
        EventHandling::Consumed
    );
    assert_eq!(
        session.handle_event(
            mouse_kind(MouseEventKind::Up(MouseButton::Left), first.0, first.1),
            &state,
            &geometry,
        ),
        EventHandling::Action(Action::OpenRun)
    );
}

#[test]
fn library_wheel_is_contained_and_rows_require_matching_up() {
    let state = state_with_names(&["Alpha", "Beta"]);
    let mut session = TuiSession::default();
    let (_, geometry) = draw(&mut session, &state);
    assert_eq!(
        session.handle_event(
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::ScrollDown,
                column: 0,
                row: 0,
                modifiers: KeyModifiers::NONE,
            }),
            &state,
            &geometry,
        ),
        EventHandling::Ignored,
        "a wheel outside the Library rows escaped its owner"
    );
    assert_eq!(
        session.handle_event(
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::ScrollDown,
                column: geometry.rows.x,
                row: geometry.rows.y,
                modifiers: KeyModifiers::NONE,
            }),
            &state,
            &geometry,
        ),
        EventHandling::Action(Action::Next)
    );

    let second_row = geometry.rows.y.saturating_add(1);
    assert_eq!(
        session.handle_event(
            mouse(MouseButton::Left, geometry.rows.x, second_row),
            &state,
            &geometry,
        ),
        EventHandling::Consumed,
        "Down must arm the Library row without selecting it"
    );
    assert_eq!(
        session.handle_event(
            mouse_kind(
                MouseEventKind::Up(MouseButton::Left),
                geometry.rows.x,
                second_row,
            ),
            &state,
            &geometry,
        ),
        EventHandling::Action(Action::SelectVisible(1)),
        "matching Up must select the armed Library row"
    );
}

#[test]
fn library_wheel_cancels_an_armed_row_before_it_moves_selection() {
    let mut state = state_with_names(&["Alpha", "Beta"]);
    let mut session = TuiSession::default();
    let (_, geometry) = draw(&mut session, &state);
    let first = (geometry.rows.x, geometry.rows.y);

    assert_eq!(
        session.handle_event(
            mouse_kind(MouseEventKind::Down(MouseButton::Left), first.0, first.1),
            &state,
            &geometry,
        ),
        EventHandling::Consumed
    );
    assert_eq!(
        drive(
            &mut session,
            &mut state,
            &geometry,
            mouse_kind(MouseEventKind::ScrollDown, first.0, first.1),
        ),
        EventHandling::Action(Action::Next)
    );
    assert_eq!(
        session.handle_event(
            mouse_kind(MouseEventKind::Up(MouseButton::Left), first.0, first.1),
            &state,
            &geometry,
        ),
        EventHandling::Ignored,
        "wheel input must cancel the earlier row press"
    );
}

#[test]
fn ctrl_c_requires_a_second_press_and_uses_a_transient_mature_toast() {
    for searching in [false, true] {
        let mut state = state();
        if searching {
            state.update(Action::BeginSearch);
        }
        let mut session = TuiSession::default();
        let (_, geometry) = draw(&mut session, &state);
        let ctrl_c = || key(KeyCode::Char('c'), KeyModifiers::CONTROL);

        assert_eq!(
            session.handle_event(ctrl_c(), &state, &geometry),
            EventHandling::Consumed,
            "the first Ctrl+C must arm quit without closing either Library focus mode"
        );
        let (terminal, _) = draw(&mut session, &state);
        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(
            rendered.contains("Press Ctrl+C again to quit"),
            "the mature transient notice must make the second chord discoverable: {rendered}"
        );
        assert_eq!(
            session.handle_event(ctrl_c(), &state, &geometry),
            EventHandling::Action(Action::Quit),
            "the second Ctrl+C inside the main two-second window must quit"
        );
    }
}
