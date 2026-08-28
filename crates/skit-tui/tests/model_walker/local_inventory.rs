use ratatui_core::{backend::TestBackend, layout::Size, terminal::Terminal};
use ratatui_crossterm::crossterm::event::{
    Event, KeyEvent, MouseButton, MouseEvent, MouseEventKind,
};
use skit_i18n::Locale;
use skit_tui::{
    EventHandling, HitTarget, LocalActionInventory, LocalActionOutcome, TuiSession, ViewGeometry,
    render_with_session,
};
use skit_ui::{
    Action, AddWorkflowState, Effect, LibraryState, RunnerManagerAction, Screen, UiCommand,
};

use super::fake_host::FakeHost;
use super::strategy::{KeyKind, MouseKind, RawKey, ResolvedOperation, WalkerOperation, resolve};

fn answer(host: &mut FakeHost, state: &mut LibraryState, mut effect: Effect) {
    for _ in 0..16 {
        match effect {
            Effect::None | Effect::Quit => return,
            request => {
                let action = host.serve(request).unwrap();
                effect = state.update(action);
            }
        }
    }
    panic!("fixture effect chain did not stop");
}

fn open(host: &mut FakeHost, state: &mut LibraryState, action: Action) {
    let effect = state.update(action);
    answer(host, state, effect);
}

fn render(
    state: &LibraryState,
    session: &mut TuiSession,
    width: u16,
    height: u16,
) -> LocalActionInventory {
    render_locale(state, session, width, height, Locale::En)
}

fn render_locale(
    state: &LibraryState,
    session: &mut TuiSession,
    width: u16,
    height: u16,
    locale: Locale,
) -> LocalActionInventory {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| {
            let _ = render_with_session(frame, state, locale, session);
        })
        .unwrap();
    session.local_action_inventory().clone()
}

fn handling_for_key(state: &LibraryState, key: KeyEvent, width: u16, height: u16) -> EventHandling {
    handling_for_key_locale(state, key, width, height, Locale::En)
}

fn handling_for_key_locale(
    state: &LibraryState,
    key: KeyEvent,
    width: u16,
    height: u16,
    locale: Locale,
) -> EventHandling {
    let mut session = TuiSession::default();
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    let mut geometry = ViewGeometry::default();
    terminal
        .draw(|frame| {
            geometry = render_with_session(frame, state, locale, &mut session);
        })
        .unwrap();
    session.handle_event(Event::Key(key), state, &geometry)
}

fn handling_for_hit(
    state: &LibraryState,
    rect: ratatui_core::layout::Rect,
    width: u16,
    height: u16,
) -> EventHandling {
    handling_for_hit_locale(state, rect, width, height, Locale::En)
}

fn handling_for_hit_locale(
    state: &LibraryState,
    rect: ratatui_core::layout::Rect,
    width: u16,
    height: u16,
    locale: Locale,
) -> EventHandling {
    let mut session = TuiSession::default();
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    let mut geometry = ViewGeometry::default();
    terminal
        .draw(|frame| {
            geometry = render_with_session(frame, state, locale, &mut session);
        })
        .unwrap();
    session.handle_event(
        Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: rect.x.saturating_add(rect.width / 2),
            row: rect.y.saturating_add(rect.height / 2),
            modifiers: ratatui_crossterm::crossterm::event::KeyModifiers::NONE,
        }),
        state,
        &geometry,
    )
}

fn assert_inventory_parity(state: &LibraryState, expected_context: &str) {
    let (width, height) = (120, 30);
    let inventory = render(state, &mut TuiSession::default(), width, height);
    assert!(
        !inventory.actions.is_empty(),
        "{expected_context} must expose its visible local advertised actions"
    );
    for advertised in &inventory.actions {
        let rect = advertised.hit.unwrap_or_else(|| {
            panic!("{expected_context} has a visible key without a mouse hit: {advertised:?}")
        });
        assert!(
            rect.width > 0 && rect.height > 0,
            "clipped hit leaked: {advertised:?}"
        );
        assert!(
            !advertised.keys.is_empty(),
            "{expected_context} has a visible mouse action without an advertised key: {advertised:?}"
        );
        let mouse = handling_for_hit(state, rect, width, height);
        assert_outcome(&advertised.outcome, &mouse, expected_context, advertised);
        for key in &advertised.keys {
            let handling = handling_for_key(state, key.event(), width, height);
            assert_outcome(&advertised.outcome, &handling, expected_context, advertised);
            assert_eq!(
                handling, mouse,
                "{expected_context} local action has different key and mouse endpoints: {advertised:?}",
            );
        }
    }
}

fn assert_inventory_surface(
    state: &LibraryState,
    context: &str,
    locale: Locale,
    width: u16,
    height: u16,
) {
    let inventory = render_locale(state, &mut TuiSession::default(), width, height, locale);
    for (index, advertised) in inventory.actions.iter().enumerate() {
        let rect = advertised.hit.expect("visible action has a hit");
        assert!(
            !rect.is_empty() && rect.right() <= width && rect.bottom() <= height,
            "{context} local rect is outside {width}x{height}: {advertised:?}",
        );
        for other in inventory.actions.iter().skip(index + 1) {
            let other_rect = other.hit.expect("visible action has a hit");
            assert!(
                rect.intersection(other_rect).is_empty() || advertised.target == other.target,
                "{context} has ambiguous local hits: {advertised:?} and {other:?}",
            );
        }
        let mouse = handling_for_hit_locale(state, rect, width, height, locale);
        assert_outcome(&advertised.outcome, &mouse, context, advertised);
        for key in &advertised.keys {
            let handling = handling_for_key_locale(state, key.event(), width, height, locale);
            assert_outcome(&advertised.outcome, &handling, context, advertised);
            assert_eq!(handling, mouse, "{context} alias differs: {advertised:?}");
        }
    }
}

fn assert_outcome(
    expected: &LocalActionOutcome,
    actual: &EventHandling,
    context: &str,
    advertised: &skit_tui::LocalAdvertisedAction,
) {
    let matches = match (expected, actual) {
        (LocalActionOutcome::Action(expected), EventHandling::Action(actual)) => expected == actual,
        (LocalActionOutcome::Consumed, EventHandling::Consumed) => true,
        _ => false,
    };
    assert!(
        matches,
        "{context} descriptor does not match its live endpoint: advertised={advertised:?} actual={actual:?}",
    );
}

#[test]
fn registry_empty_local_contexts_export_live_key_and_mouse_semantics() {
    let mut host = FakeHost::new();

    let mut add = LibraryState::default();
    let _ = add.update(Action::Present(Screen::Add(Box::new(
        AddWorkflowState::new(Vec::new()),
    ))));
    assert!(matches!(add.screen(), Screen::Add(_)));
    assert_inventory_parity(&add, "Add");

    let mut health = host.initial_state();
    open(&mut host, &mut health, Action::OpenHealth);
    assert!(matches!(health.screen(), Screen::Health(_)));
    assert_inventory_parity(&health, "Health");

    let mut runners = host.initial_state();
    open(&mut host, &mut runners, Action::OpenRunners);
    assert!(matches!(runners.screen(), Screen::Runners(_)));
    assert_inventory_parity(&runners, "Runners");

    let effect = runners.update(Action::Runners(RunnerManagerAction::New));
    assert_eq!(effect, Effect::None);
    assert_inventory_parity(&runners, "Runners editor");

    let mut editor = host.initial_state();
    for _ in 0..32 {
        if editor
            .selected()
            .is_some_and(|entry| entry.kind.as_str() == "prompt")
        {
            break;
        }
        let _ = editor.update(Action::Next);
    }
    assert!(
        editor
            .selected()
            .is_some_and(|entry| entry.kind.as_str() == "prompt"),
        "fixture has a reachable prompt entry",
    );
    open(&mut host, &mut editor, Action::OpenRun);
    let effect = editor.update(Action::OpenRunRunnerEditor);
    assert_eq!(effect, Effect::None);
    assert_inventory_parity(&editor, "RunnerEditor");
}

#[test]
fn local_inventory_is_empty_for_hidden_overlays_and_clipped_cells() {
    let mut host = FakeHost::new();
    let mut state = host.initial_state();
    open(&mut host, &mut state, Action::OpenAdd);

    let mut session = TuiSession::default();
    let inventory = render(&state, &mut session, 1, 1);
    assert!(
        inventory.actions.is_empty(),
        "a fully clipped one-cell Add screen must not invent a local action",
    );
    let inventory = render(&state, &mut session, 24, 6);
    assert!(!inventory.actions.is_empty());
    assert!(inventory.actions.iter().all(|action| {
        action
            .hit
            .is_some_and(|rect| rect.width > 0 && rect.height > 0)
    }));

    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    let mut geometry = ViewGeometry::default();
    terminal
        .draw(|frame| {
            geometry = render_with_session(frame, &state, Locale::En, &mut session);
        })
        .unwrap();
    let open_picker = ratatui_crossterm::crossterm::event::KeyEvent::new(
        ratatui_crossterm::crossterm::event::KeyCode::Char('o'),
        ratatui_crossterm::crossterm::event::KeyModifiers::CONTROL,
    );
    assert_eq!(
        session.handle_event(Event::Key(open_picker), &state, &geometry),
        EventHandling::Consumed,
    );
    assert!(render(&state, &mut session, 80, 24).actions.is_empty());
}

#[test]
fn every_advertised_alias_is_a_positive_session_path() {
    let mut host = FakeHost::new();
    let mut state = host.initial_state();
    open(&mut host, &mut state, Action::OpenRunners);
    let _ = state.update(Action::Runners(RunnerManagerAction::New));
    let inventory = render(&state, &mut TuiSession::default(), 120, 30);
    let alias = inventory
        .actions
        .iter()
        .find(|action| action.keys.len() > 1)
        .expect("the runner editor advertises both Tab/down or BackTab/up");
    let expected = handling_for_hit(&state, alias.hit.unwrap(), 120, 30);
    for key in &alias.keys {
        assert_eq!(handling_for_key(&state, key.event(), 120, 30), expected);
    }
}

#[test]
fn inventory_snapshot_is_tied_to_the_last_rendered_local_context() {
    let mut host = FakeHost::new();
    let mut state = host.initial_state();
    let mut session = TuiSession::default();
    assert!(render(&state, &mut session, 80, 24).actions.is_empty());
    open(&mut host, &mut state, Action::OpenHealth);
    assert!(!render(&state, &mut session, 80, 24).actions.is_empty());
}

#[test]
fn preferences_shared_focus_hits_return_typed_session_actions() {
    let mut host = FakeHost::new();
    let mut state = host.initial_state();
    open(&mut host, &mut state, Action::OpenPreferences);
    let _ = state.update(Action::Preferences(skit_ui::PreferencesAction::SetEditor(
        "micro".to_owned(),
    )));
    let mut session = TuiSession::default();
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    let mut geometry = ViewGeometry::default();
    terminal
        .draw(|frame| {
            geometry = render_with_session(frame, &state, Locale::En, &mut session);
        })
        .unwrap();
    for hit in geometry.hits.iter().filter(|hit| {
        matches!(
            hit.action,
            HitTarget::Command(UiCommand::FocusNext | UiCommand::FocusPrevious)
        )
    }) {
        let handling = session.handle_event(
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: hit.rect.x,
                row: hit.rect.y,
                modifiers: ratatui_crossterm::crossterm::event::KeyModifiers::NONE,
            }),
            &state,
            &geometry,
        );
        assert!(
            matches!(handling, EventHandling::Action(Action::Preferences(_))),
            "{hit:?} returned {handling:?}",
        );
    }
}

#[test]
fn local_operations_resolve_from_the_latest_rendered_session_inventory() {
    let mut state = LibraryState::default();
    let _ = state.update(Action::Present(Screen::Add(Box::new(
        AddWorkflowState::new(Vec::new()),
    ))));
    let mut session = TuiSession::default();
    let inventory = render(&state, &mut session, 80, 24);
    let advertised = inventory.actions.first().expect("Add has a local action");

    let key = resolve(
        &WalkerOperation::LocalAdvertisedKey {
            action: 0,
            binding: 0,
        },
        &state,
        &ViewGeometry::default(),
        Size::new(80, 24),
        &inventory,
    );
    assert!(matches!(
        key,
        ResolvedOperation::LocalEvent {
            event: Event::Key(event),
            advertised: ref descriptor,
        } if event == advertised.keys[0].event() && descriptor.as_ref() == advertised
    ));

    let rect = advertised.hit.expect("visible local action has a hit");
    let hit = resolve(
        &WalkerOperation::LocalHit { action: 0 },
        &state,
        &ViewGeometry::default(),
        Size::new(80, 24),
        &inventory,
    );
    assert!(matches!(
        hit,
        ResolvedOperation::LocalEvent {
            event: Event::Mouse(MouseEvent {
            column,
            row,
            ..
            }),
            advertised: descriptor,
        } if descriptor.as_ref() == advertised
            && column == rect.x.saturating_add(rect.width / 2)
            && row == rect.y.saturating_add(rect.height / 2)
    ));
}

#[test]
fn raw_and_random_pointer_events_cannot_select_the_live_quit_target() {
    let host = FakeHost::new();
    let state = host.initial_state();
    let mut session = TuiSession::default();
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    let mut geometry = ViewGeometry::default();
    terminal
        .draw(|frame| {
            geometry = render_with_session(frame, &state, Locale::En, &mut session);
        })
        .unwrap();
    assert!(
        geometry.hits.iter().any(|hit| {
            hit.action == HitTarget::Command(UiCommand::Quit) && !hit.rect.is_empty()
        })
    );
    assert_eq!(
        resolve(
            &WalkerOperation::RawKey {
                key: RawKey::Escape,
                kind: KeyKind::Press,
            },
            &state,
            &geometry,
            Size::new(80, 24),
            session.local_action_inventory(),
        ),
        ResolvedOperation::Noop,
    );

    let mut reached_quit = false;
    for x_fraction in 0..=u8::MAX {
        for y_fraction in 0..=u8::MAX {
            let operation = WalkerOperation::MouseCell {
                x_fraction,
                y_fraction,
                kind: MouseKind::LeftDown,
            };
            let resolved = resolve(
                &operation,
                &state,
                &geometry,
                Size::new(80, 24),
                session.local_action_inventory(),
            );
            let x = u16::try_from(u32::from(x_fraction) * 79 / 255).unwrap();
            let y = u16::try_from(u32::from(y_fraction) * 23 / 255).unwrap();
            if geometry.hits.iter().any(|hit| {
                hit.action == HitTarget::Command(UiCommand::Quit)
                    && hit.rect.contains((x, y).into())
            }) {
                reached_quit = true;
                assert_eq!(resolved, ResolvedOperation::Noop);
            }
        }
    }
    assert!(reached_quit, "the random pointer grid never covered Quit");
}

#[test]
fn runner_local_surfaces_are_bounded_and_unambiguous_in_every_tier_and_locale() {
    let mut host = FakeHost::new();
    let mut manager = host.initial_state();
    open(&mut host, &mut manager, Action::OpenRunners);

    let mut actions = manager.clone();
    let _ = actions.update(Action::Runners(RunnerManagerAction::ActivateSelected));
    let mut removal = actions.clone();
    let _ = removal.update(Action::Runners(RunnerManagerAction::RemoveSelected));
    let mut nested_editor = manager.clone();
    let _ = nested_editor.update(Action::Runners(RunnerManagerAction::New));

    let mut standalone_editor = host.initial_state();
    for _ in 0..32 {
        if standalone_editor
            .selected()
            .is_some_and(|entry| entry.kind.as_str() == "prompt")
        {
            break;
        }
        let _ = standalone_editor.update(Action::Next);
    }
    open(&mut host, &mut standalone_editor, Action::OpenRun);
    let _ = standalone_editor.update(Action::OpenRunRunnerEditor);

    for (context, state) in [
        ("Runners", &manager),
        ("Runners actions", &actions),
        ("Runners removal", &removal),
        ("Runners editor", &nested_editor),
        ("RunnerEditor", &standalone_editor),
    ] {
        for locale in [Locale::En, Locale::ZhCn, Locale::ZhTw, Locale::Pseudo] {
            for (width, height) in [(1, 1), (24, 6), (120, 30)] {
                assert_inventory_surface(state, context, locale, width, height);
            }
        }
    }
}
