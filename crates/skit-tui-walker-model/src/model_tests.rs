use proptest::{strategy::Strategy, test_runner::TestRunner};
use ratatui_core::{backend::TestBackend, layout::Rect, layout::Size, terminal::Terminal};
use ratatui_crossterm::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use skit_i18n::Locale;
use skit_tui::{
    HitRegion, HitTarget, LocalActionInventory, LocalActionOutcome, LocalActionTarget,
    LocalAdvertisedAction, ScreenFocusInventory, ScreenTarget, ScreenTargetHit,
    ScreenTargetInventory, TuiSession, ViewGeometry, render_with_session,
};
use skit_ui::{
    Action, Effect, HealthIssue, HealthIssueKind, HealthSnapshot, HealthView, LibraryState,
    MirrorHealth, Screen, UiBinding, UiCommand, UiKey, UiModifiers, UvHealth, command_specs,
};

use crate::model::{
    AdvertisedCommand, KeyKind, LiveInventory, MouseKind, OperationFamily, PASTE_CASES,
    RESIZE_CASES, RandomOperation, RawKey, ResolveRefusal, ResolvedInput, operation_strategy,
    random_walk_profiles, resolve,
};

// --------------------------------------------------------------------------
// fixtures
// --------------------------------------------------------------------------

fn hit(rect: Rect, action: HitTarget) -> HitRegion {
    HitRegion { rect, action }
}

fn live_with_hits(hits: Vec<HitRegion>) -> LiveInventory {
    LiveInventory {
        hits,
        ..LiveInventory::default()
    }
}

fn live_with_size(size: Size) -> LiveInventory {
    LiveInventory {
        size,
        ..LiveInventory::default()
    }
}

fn health_state() -> LibraryState {
    let mut state = LibraryState::default();
    let effect = state.update(Action::Present(Screen::Health(Box::new(HealthView::new(
        HealthSnapshot {
            uv: UvHealth::NotRequired,
            entry_count: 2,
            issues: vec![HealthIssue {
                slug: "missing".to_owned(),
                name: "Missing".to_owned(),
                kind: HealthIssueKind::MissingTarget,
            }],
            invalid_runner_rows: Vec::new(),
            mirror: MirrorHealth::Off,
            library_path: "/data/scripts".to_owned(),
            library_size: "2 KiB".to_owned(),
            diagnostics: Vec::new(),
        },
    )))));
    assert_eq!(effect, Effect::None);
    state
}

fn rendered_frame(state: &LibraryState) -> (ViewGeometry, LocalActionInventory) {
    let mut session = TuiSession::default();
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    let mut geometry = ViewGeometry::default();
    terminal
        .draw(|frame| {
            geometry = render_with_session(frame, state, Locale::En, &mut session);
        })
        .unwrap();
    (geometry, session.local_action_inventory().clone())
}

fn rendered_local_actions(state: &LibraryState) -> LocalActionInventory {
    rendered_frame(state).1
}

fn clipped_action(target: LocalActionTarget) -> LocalAdvertisedAction {
    LocalAdvertisedAction {
        target,
        keys: Vec::new(),
        hit: None,
        outcome: LocalActionOutcome::Consumed,
    }
}

fn runner_target(name: &str) -> ScreenTarget {
    ScreenTarget::Runner {
        name: name.to_owned(),
    }
}

fn ui_binding(key: UiKey, modifiers: UiModifiers) -> UiBinding {
    UiBinding {
        key,
        modifiers,
        hint: "hint",
        compact_hint: "hint",
    }
}

fn advertised(command: UiCommand, bindings: Vec<UiBinding>) -> AdvertisedCommand {
    AdvertisedCommand { command, bindings }
}

fn live_with_commands(commands: Vec<AdvertisedCommand>) -> LiveInventory {
    LiveInventory {
        commands,
        ..LiveInventory::default()
    }
}

// --------------------------------------------------------------------------
// the tables (legacy `strategy.rs:527`, `:545` and `driver.rs:2234`)
// --------------------------------------------------------------------------

#[test]
fn resize_cases_include_tiny_responsive_and_large_viewports() {
    for required in [
        (1, 1),
        (1, 2),
        (2, 1),
        (24, 6),
        (40, 40),
        (46, 12),
        (80, 24),
        (120, 12),
        (120, 30),
        (300, 100),
    ] {
        assert!(RESIZE_CASES.contains(&required), "missing {required:?}");
    }
    assert_eq!(
        RESIZE_CASES.len(),
        10,
        "a new shape must replace one required tier"
    );
}

#[test]
fn paste_cases_cover_terminal_text_edges() {
    for required in ["", "界", "e\u{301}", "🙂", "one\ntwo", "a\tb", "\0"] {
        assert!(PASTE_CASES.contains(&required), "missing {required:?}");
    }
    assert_eq!(
        PASTE_CASES.len(),
        7,
        "a new payload must replace one required edge"
    );
}

#[test]
fn complete_profile_guarantees_common_shapes_without_a_full_cartesian_product() {
    let profiles = random_walk_profiles();
    for locale in [Locale::En, Locale::ZhCn, Locale::ZhTw, Locale::Pseudo] {
        for size in [Size::new(1, 1), Size::new(24, 6), Size::new(120, 30)] {
            assert!(
                profiles.contains(&(locale, size)),
                "the complete profile must retain locale={} size={size:?}",
                locale.tag()
            );
        }
    }
    for required in [
        (Locale::En, Size::new(80, 24)),
        (Locale::Pseudo, Size::new(120, 12)),
        (Locale::ZhTw, Size::new(40, 40)),
    ] {
        assert!(
            profiles.contains(&required),
            "the complete profile must start from {required:?}"
        );
    }
    assert_eq!(
        profiles.len(),
        15,
        "new shapes must not expand into every locale and size pair"
    );
}

// --------------------------------------------------------------------------
// the strategy (legacy `strategy.rs:449`, `:552`)
// --------------------------------------------------------------------------

#[test]
fn operation_strategy_generates_every_required_event_family() {
    let strategy = operation_strategy();
    let mut runner = TestRunner::deterministic();
    let mut observed = Vec::new();
    for _ in 0..4096 {
        let family = strategy.new_tree(&mut runner).unwrap().current().family();
        if !observed.contains(&family) {
            observed.push(family);
        }
    }
    for expected in OperationFamily::ALL {
        assert!(observed.contains(expected), "missing {expected:?}");
    }
    assert_eq!(observed.len(), OperationFamily::ALL.len());
}

#[test]
fn semantic_traces_have_deterministic_json_replays() {
    let mut trace = vec![
        RandomOperation::Resize {
            width: 24,
            height: 6,
        },
        RandomOperation::Paste {
            value: "界🙂".to_owned(),
        },
        RandomOperation::RawKey {
            key: RawKey::Escape,
            kind: KeyKind::Press,
        },
        RandomOperation::Focus { gained: false },
        RandomOperation::Focus { gained: true },
        RandomOperation::AdvertisedKey {
            command: 1,
            binding: 2,
        },
        RandomOperation::PublicHit { ordinal: 3 },
        RandomOperation::LocalAdvertisedKey {
            action: 4,
            binding: 5,
        },
        RandomOperation::LocalHit { action: 6 },
    ];
    for kind in MouseKind::ALL {
        trace.push(RandomOperation::MouseCell {
            x_fraction: 7,
            y_fraction: 8,
            kind: *kind,
        });
    }
    for key in RawKey::ALL {
        for kind in KeyKind::ALL {
            trace.push(RandomOperation::RawKey {
                key: *key,
                kind: *kind,
            });
        }
    }

    let bytes = serde_json::to_vec(&trace).unwrap();
    let decoded: Vec<RandomOperation> = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(decoded, trace);
    assert_eq!(decoded.clone(), trace);

    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value[0]["operation"], "resize");
    assert_eq!(value[0]["width"], 24);
    assert_eq!(value[2]["operation"], "raw_key");
    assert_eq!(value[2]["key"], "escape");
    assert_eq!(value[2]["kind"], "press");
    assert_eq!(value[3]["gained"], false);
    assert_eq!(value[9]["operation"], "mouse_cell");
    assert_eq!(value[9]["kind"], "left_down");
}

// --------------------------------------------------------------------------
// the late binder (legacy `strategy.rs:486`, `:509`, `:572`, `:595`)
// --------------------------------------------------------------------------

#[test]
fn public_hits_resolve_against_the_latest_geometry() {
    let operation = RandomOperation::PublicHit { ordinal: 0 };
    let first = live_with_hits(vec![hit(
        Rect::new(1, 2, 3, 2),
        HitTarget::Command(UiCommand::Help),
    )]);
    let second = live_with_hits(vec![hit(Rect::new(10, 7, 5, 3), HitTarget::FocusField(2))]);

    assert_eq!(
        resolve(&operation, &first),
        ResolvedInput::Hit(HitTarget::Command(UiCommand::Help))
    );
    assert_eq!(
        resolve(&operation, &second),
        ResolvedInput::Hit(HitTarget::FocusField(2))
    );
}

#[test]
fn public_hits_skip_a_clipped_rectangle() {
    let live = live_with_hits(vec![
        hit(Rect::new(1, 1, 0, 1), HitTarget::FocusField(0)),
        hit(Rect::new(1, 1, 4, 0), HitTarget::FocusField(1)),
    ]);
    assert_eq!(
        resolve(&RandomOperation::PublicHit { ordinal: 3 }, &live),
        ResolvedInput::NotApplicable(ResolveRefusal::Unavailable)
    );
}

#[test]
fn arbitrary_mouse_cells_stay_inside_the_current_viewport() {
    let last = RandomOperation::MouseCell {
        x_fraction: u8::MAX,
        y_fraction: u8::MAX,
        kind: MouseKind::LeftDown,
    };
    assert_eq!(
        resolve(&last, &live_with_size(Size::new(24, 6))),
        ResolvedInput::RawMouse {
            column: 23,
            row: 5,
            kind: MouseKind::LeftDown,
        }
    );

    let middle = RandomOperation::MouseCell {
        x_fraction: 128,
        y_fraction: 128,
        kind: MouseKind::ScrollDown,
    };
    assert_eq!(
        resolve(&middle, &live_with_size(Size::new(24, 6))),
        ResolvedInput::RawMouse {
            column: 11,
            row: 2,
            kind: MouseKind::ScrollDown,
        }
    );

    for size in [Size::new(1, 1), Size::new(0, 0)] {
        assert_eq!(
            resolve(&last, &live_with_size(size)),
            ResolvedInput::RawMouse {
                column: 0,
                row: 0,
                kind: MouseKind::LeftDown,
            }
        );
    }
}

#[test]
fn raw_key_inventory_excludes_the_session_clock_dependent_ctrl_c_chord() {
    let live = LiveInventory::default();
    let mut observed = Vec::new();
    for key in RawKey::ALL {
        for kind in KeyKind::ALL {
            let ResolvedInput::RawKey(event) = resolve(
                &RandomOperation::RawKey {
                    key: *key,
                    kind: *kind,
                },
                &live,
            ) else {
                panic!("the raw key family must resolve to one key event");
            };
            assert_ne!(
                (event.code, event.modifiers),
                (KeyCode::Char('c'), KeyModifiers::CONTROL)
            );
            observed.push((event.code, event.modifiers, event.kind));
        }
    }
    assert_eq!(observed.len(), 57);
    assert!(observed.contains(&(KeyCode::Char('x'), KeyModifiers::ALT, KeyEventKind::Release)));
    assert!(observed.contains(&(KeyCode::F(2), KeyModifiers::NONE, KeyEventKind::Repeat)));
    assert!(observed.contains(&(KeyCode::BackTab, KeyModifiers::SHIFT, KeyEventKind::Press)));
}

#[test]
fn model_operations_do_not_select_quit_from_shared_keys_or_hits() {
    let live = LiveInventory::new(
        &LibraryState::default(),
        &ViewGeometry::default(),
        &LocalActionInventory::default(),
        None,
        Size::new(80, 24),
    );
    assert!(
        live.commands
            .iter()
            .any(|entry| entry.command == UiCommand::Quit)
    );
    for command in 0..=u8::MAX {
        for binding in 0..=u8::MAX {
            let resolved = resolve(&RandomOperation::AdvertisedKey { command, binding }, &live);
            assert!(
                !matches!(
                    resolved,
                    ResolvedInput::CommandKeyboard {
                        command: UiCommand::Quit,
                        ..
                    }
                ),
                "command ordinal {command} and binding ordinal {binding} selected Quit"
            );
        }
    }

    let quit_only = live_with_hits(vec![hit(
        Rect::new(1, 1, 8, 1),
        HitTarget::Command(UiCommand::Quit),
    )]);
    assert_eq!(
        resolve(&RandomOperation::PublicHit { ordinal: 0 }, &quit_only),
        ResolvedInput::NotApplicable(ResolveRefusal::QuitFiltered)
    );

    let quit_command = live_with_commands(vec![advertised(
        UiCommand::Quit,
        vec![ui_binding(UiKey::Character('q'), UiModifiers::NONE)],
    )]);
    assert_eq!(
        resolve(
            &RandomOperation::AdvertisedKey {
                command: 0,
                binding: 0
            },
            &quit_command
        ),
        ResolvedInput::NotApplicable(ResolveRefusal::QuitFiltered)
    );
}

// --------------------------------------------------------------------------
// one deterministic case per family
// --------------------------------------------------------------------------

#[test]
fn advertised_key_binds_one_enabled_command() {
    let live = live_with_commands(vec![
        advertised(
            UiCommand::Help,
            vec![ui_binding(UiKey::Character('?'), UiModifiers::NONE)],
        ),
        advertised(
            UiCommand::FocusNext,
            vec![ui_binding(UiKey::Tab, UiModifiers::NONE)],
        ),
    ]);
    assert_eq!(
        resolve(
            &RandomOperation::AdvertisedKey {
                command: 3,
                binding: 9
            },
            &live
        ),
        ResolvedInput::CommandKeyboard {
            command: UiCommand::FocusNext,
            key: KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
        }
    );
    assert_eq!(
        resolve(
            &RandomOperation::AdvertisedKey {
                command: 0,
                binding: 0
            },
            &LiveInventory::default()
        ),
        ResolvedInput::NotApplicable(ResolveRefusal::Unavailable)
    );
}

#[test]
fn the_binding_ordinal_reaches_every_chord_of_one_command() {
    let live = live_with_commands(vec![advertised(
        UiCommand::Run,
        vec![
            ui_binding(UiKey::Enter, UiModifiers::NONE),
            ui_binding(UiKey::Character('r'), UiModifiers::CONTROL),
        ],
    )]);
    let first = ResolvedInput::CommandKeyboard {
        command: UiCommand::Run,
        key: KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    };
    let second = ResolvedInput::CommandKeyboard {
        command: UiCommand::Run,
        key: KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL),
    };

    assert_ne!(first, second);
    for (binding, expected) in [(0, &first), (1, &second), (2, &first), (3, &second)] {
        assert_eq!(
            resolve(
                &RandomOperation::AdvertisedKey {
                    command: 0,
                    binding
                },
                &live
            ),
            *expected,
            "binding ordinal {binding} chose the other chord"
        );
    }
}

#[test]
fn one_chord_answers_every_binding_ordinal() {
    let live = live_with_commands(vec![advertised(
        UiCommand::Help,
        vec![ui_binding(UiKey::Function(1), UiModifiers::NONE)],
    )]);
    let expected = ResolvedInput::CommandKeyboard {
        command: UiCommand::Help,
        key: KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE),
    };
    for binding in [0, 1, 2, u8::MAX] {
        assert_eq!(
            resolve(
                &RandomOperation::AdvertisedKey {
                    command: 0,
                    binding
                },
                &live
            ),
            expected,
            "binding ordinal {binding} left the only chord"
        );
    }
}

#[test]
fn a_command_without_a_chord_is_not_applicable() {
    let live = live_with_commands(vec![advertised(UiCommand::Help, Vec::new())]);
    for binding in [0, 1, u8::MAX] {
        assert_eq!(
            resolve(
                &RandomOperation::AdvertisedKey {
                    command: 0,
                    binding
                },
                &live
            ),
            ResolvedInput::NotApplicable(ResolveRefusal::Unavailable)
        );
    }
}

#[test]
fn every_enabled_command_carries_the_chords_of_the_registry() {
    let state = LibraryState::default();
    let live = LiveInventory::new(
        &state,
        &ViewGeometry::default(),
        &LocalActionInventory::default(),
        None,
        Size::new(80, 24),
    );

    for spec in
        command_specs(state.command_context()).filter(|spec| state.command_enabled(spec.command))
    {
        let entry = live
            .commands
            .iter()
            .find(|entry| entry.command == spec.command)
            .expect("the live inventory keeps every enabled command");
        assert_eq!(entry.bindings, spec.bindings.to_vec());
        assert!(
            !entry.bindings.is_empty(),
            "{:?} advertises no chord",
            spec.command
        );
    }
}

#[test]
fn local_advertised_key_binds_one_live_chord() {
    let inventory = rendered_local_actions(&health_state());
    let action = inventory
        .actions
        .iter()
        .find(|action| !action.keys.is_empty())
        .expect("the health screen advertises one local chord")
        .clone();
    let live = LiveInventory {
        local_actions: vec![action.clone()],
        ..LiveInventory::default()
    };
    assert_eq!(
        resolve(
            &RandomOperation::LocalAdvertisedKey {
                action: 0,
                binding: 0
            },
            &live
        ),
        ResolvedInput::LocalKeyboard {
            target: action.target.clone(),
            key: action.keys[0].event(),
        }
    );
}

#[test]
fn local_advertised_key_without_a_chord_is_not_applicable() {
    let live = LiveInventory {
        local_actions: vec![clipped_action(LocalActionTarget::Health(
            skit_ui::HealthAction::Rebuild,
        ))],
        ..LiveInventory::default()
    };
    assert_eq!(
        resolve(
            &RandomOperation::LocalAdvertisedKey {
                action: 0,
                binding: 0
            },
            &live
        ),
        ResolvedInput::NotApplicable(ResolveRefusal::Unavailable)
    );
    assert_eq!(
        resolve(
            &RandomOperation::LocalAdvertisedKey {
                action: 0,
                binding: 0
            },
            &LiveInventory::default()
        ),
        ResolvedInput::NotApplicable(ResolveRefusal::Unavailable)
    );
}

#[test]
fn local_advertised_key_beyond_the_local_actions_moves_the_screen_focus() {
    let live = LiveInventory {
        local_actions: vec![clipped_action(LocalActionTarget::Health(
            skit_ui::HealthAction::Rebuild,
        ))],
        screen_focus: vec![runner_target("codex"), runner_target("claude")],
        ..LiveInventory::default()
    };
    assert_eq!(
        resolve(
            &RandomOperation::LocalAdvertisedKey {
                action: 2,
                binding: 0
            },
            &live
        ),
        ResolvedInput::ScreenFocus(runner_target("claude"))
    );
    assert_eq!(
        resolve(
            &RandomOperation::LocalAdvertisedKey {
                action: 1,
                binding: 0
            },
            &live
        ),
        ResolvedInput::ScreenFocus(runner_target("codex"))
    );
}

#[test]
fn local_hit_binds_the_visible_chip_and_refuses_a_clipped_chip() {
    let visible = LocalAdvertisedAction {
        hit: Some(Rect::new(2, 3, 4, 1)),
        ..clipped_action(LocalActionTarget::Health(skit_ui::HealthAction::Rebuild))
    };
    let live = LiveInventory {
        local_actions: vec![visible.clone()],
        ..LiveInventory::default()
    };
    assert_eq!(
        resolve(&RandomOperation::LocalHit { action: 0 }, &live),
        ResolvedInput::LocalHit(visible.target.clone())
    );

    let clipped = LiveInventory {
        local_actions: vec![clipped_action(LocalActionTarget::Health(
            skit_ui::HealthAction::Rebuild,
        ))],
        ..LiveInventory::default()
    };
    assert_eq!(
        resolve(&RandomOperation::LocalHit { action: 0 }, &clipped),
        ResolvedInput::NotApplicable(ResolveRefusal::Clipped)
    );
    assert_eq!(
        resolve(
            &RandomOperation::LocalHit { action: 0 },
            &LiveInventory::default()
        ),
        ResolvedInput::NotApplicable(ResolveRefusal::Unavailable)
    );
}

#[test]
fn local_hit_beyond_the_local_actions_clicks_a_screen_target() {
    let live = LiveInventory {
        screen_hits: vec![ScreenTargetHit {
            target: runner_target("codex"),
            rect: Rect::new(1, 1, 6, 1),
        }],
        ..LiveInventory::default()
    };
    assert_eq!(
        resolve(&RandomOperation::LocalHit { action: 5 }, &live),
        ResolvedInput::ScreenHit(runner_target("codex"))
    );

    let mixed = LiveInventory {
        local_actions: vec![clipped_action(LocalActionTarget::Health(
            skit_ui::HealthAction::Rebuild,
        ))],
        screen_hits: vec![
            ScreenTargetHit {
                target: runner_target("codex"),
                rect: Rect::new(1, 1, 6, 1),
            },
            ScreenTargetHit {
                target: runner_target("claude"),
                rect: Rect::new(1, 2, 6, 1),
            },
        ],
        ..LiveInventory::default()
    };
    assert_eq!(
        resolve(&RandomOperation::LocalHit { action: 1 }, &mixed),
        ResolvedInput::ScreenHit(runner_target("codex"))
    );
    assert_eq!(
        resolve(&RandomOperation::LocalHit { action: 2 }, &mixed),
        ResolvedInput::ScreenHit(runner_target("claude"))
    );
}

#[test]
fn resize_paste_and_focus_keep_their_drawn_payload() {
    let live = LiveInventory::default();
    assert_eq!(
        resolve(
            &RandomOperation::Resize {
                width: 46,
                height: 12
            },
            &live
        ),
        ResolvedInput::Resize {
            width: 46,
            height: 12
        }
    );
    assert_eq!(
        resolve(
            &RandomOperation::Paste {
                value: "one\ntwo".to_owned()
            },
            &live
        ),
        ResolvedInput::Paste("one\ntwo".to_owned())
    );
    for gained in [true, false] {
        assert_eq!(
            resolve(&RandomOperation::Focus { gained }, &live),
            ResolvedInput::Focus { gained }
        );
    }
}

// --------------------------------------------------------------------------
// the live inventory
// --------------------------------------------------------------------------

#[test]
fn live_inventory_reads_one_rendered_frame() {
    let state = LibraryState::default();
    let (geometry, local_actions) = rendered_frame(&state);
    assert!(!geometry.hits.is_empty());
    let screen_targets = ScreenTargetInventory {
        available: vec![runner_target("codex")],
        focus: Some(ScreenFocusInventory {
            current: Some(runner_target("codex")),
            order: vec![runner_target("codex"), runner_target("claude")],
        }),
        hits: vec![ScreenTargetHit {
            target: runner_target("codex"),
            rect: Rect::new(0, 0, 5, 1),
        }],
    };

    let live = LiveInventory::new(
        &state,
        &geometry,
        &local_actions,
        Some(&screen_targets),
        Size::new(80, 24),
    );
    assert_eq!(live.hits, geometry.hits);
    assert_eq!(live.local_actions, local_actions.actions);
    assert_eq!(
        live.screen_focus,
        vec![runner_target("codex"), runner_target("claude")]
    );
    assert_eq!(live.screen_hits, screen_targets.hits);
    assert_eq!(live.size, Size::new(80, 24));
    let commands = live
        .commands
        .iter()
        .map(|entry| entry.command)
        .collect::<Vec<_>>();
    assert!(commands.contains(&UiCommand::Quit));
    assert!(
        !commands.contains(&UiCommand::Run),
        "an empty library disables Run"
    );
    assert!(
        commands
            .iter()
            .all(|command| state.command_enabled(*command))
    );

    let health_actions = rendered_local_actions(&health_state());
    assert!(!health_actions.actions.is_empty());
    let without_screen =
        LiveInventory::new(&state, &geometry, &health_actions, None, Size::new(24, 6));
    assert_eq!(without_screen.local_actions, health_actions.actions);
    assert_eq!(without_screen.size, Size::new(24, 6));
    assert!(without_screen.screen_focus.is_empty());
    assert!(without_screen.screen_hits.is_empty());

    let without_focus = LiveInventory::new(
        &state,
        &geometry,
        &local_actions,
        Some(&ScreenTargetInventory::default()),
        Size::new(24, 6),
    );
    assert!(without_focus.screen_focus.is_empty());
}
