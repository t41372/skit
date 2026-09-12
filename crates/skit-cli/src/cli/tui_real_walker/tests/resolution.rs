//! Screen target resolution, parity, and raw pointer contracts.

use super::*;

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn corpus_parity_after(
    namespace: StableSandboxNamespace,
    operations: &[CorpusOperation],
) -> FrontendParity {
    let mut engine = stable_corpus_engine(namespace);
    let mut sink = skit_tui_walker_support::engine::NoopCheckpointSink;
    engine.start(&mut sink).unwrap();
    for operation in operations {
        engine.run_operation(operation.clone(), &mut sink).unwrap();
    }
    let parity = engine.parity_snapshot().unwrap();
    let (frontend, host) = engine.into_parts();
    drop(frontend);
    host.close().unwrap();
    parity
}

#[test]
fn corpus_screen_picker_uses_the_seeded_tree_and_real_add_chain() {
    let mut engine = corpus_engine();
    let mut sink = skit_tui_walker_support::engine::NoopCheckpointSink;
    engine.start(&mut sink).unwrap();
    let operations = [
        CorpusOperation::CommandKeyboard(UiCommand::Add),
        CorpusOperation::ScreenHit(skit_tui::ScreenTarget::Add(
            skit_tui::AddControlId::BrowseSource,
        )),
        CorpusOperation::ScreenHit(skit_tui::ScreenTarget::FilePickerEntry {
            relative: PathBuf::from("original.sh"),
        }),
        CorpusOperation::ScreenFocus(skit_tui::ScreenTarget::Add(
            skit_tui::AddControlId::Continue,
        )),
        CorpusOperation::RawKey(CorpusKeyEvent::press(
            CorpusKey::Enter,
            CorpusModifiers::NONE,
        )),
    ];
    for operation in operations {
        engine.run_operation(operation, &mut sink).unwrap();
    }

    assert!(
        engine
            .frontend()
            .inner
            .session_value()
            .unwrap()
            .to_string()
            .contains("memory_file_picker_source")
    );
    assert!(matches!(
        engine.frontend().inner.state.screen(),
        Screen::Add(view) if view.stage() == skit_ui::AddStage::Review
    ));
    let (frontend, host) = engine.into_parts();
    drop(frontend);
    host.close().unwrap();
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn corpus_screen_focus_and_hit_reach_the_same_preferences_endpoint() {
    let parent = tempfile::TempDir::new().unwrap();
    let namespace =
        StableSandboxNamespace::explicit(parent.path().join(STABLE_SANDBOX_NAMESPACE)).unwrap();
    let focus = CorpusOperation::ScreenFocus(skit_tui::ScreenTarget::Preferences(
        skit_ui::PreferencesControlId::ManageAgents,
    ));
    let keyboard = corpus_parity_after(
        namespace.clone(),
        &[
            CorpusOperation::CommandKeyboard(UiCommand::Preferences),
            focus.clone(),
            CorpusOperation::RawKey(CorpusKeyEvent::press(
                CorpusKey::Enter,
                CorpusModifiers::NONE,
            )),
        ],
    );
    let mouse = corpus_parity_after(
        namespace,
        &[
            CorpusOperation::CommandKeyboard(UiCommand::Preferences),
            focus,
            CorpusOperation::ScreenHit(skit_tui::ScreenTarget::Preferences(
                skit_ui::PreferencesControlId::ManageAgents,
            )),
        ],
    );

    assert_eq!(keyboard, mouse);
    assert!(matches!(keyboard.state.screen(), Screen::Runners(_)));
}

#[test]
fn corpus_screen_agent_hit_uses_name_and_scope_from_the_live_overlay() {
    let mut factory = smoke_factory();
    factory
        .seed
        .directories
        .push(crate::cli::tui_real_host::WalkerDirectorySeed {
            root: crate::cli::tui_real_host::WalkerDirectoryRoot::Home,
            path: PathBuf::from(".codex/skills"),
        });
    let mut engine = corpus_engine_for_factory(factory);
    let mut sink = skit_tui_walker_support::engine::NoopCheckpointSink;
    engine.start(&mut sink).unwrap();
    for operation in [
        CorpusOperation::CommandKeyboard(UiCommand::Preferences),
        CorpusOperation::ScreenFocus(skit_tui::ScreenTarget::Preferences(
            skit_ui::PreferencesControlId::InstallAgentSkill,
        )),
        CorpusOperation::ScreenHit(skit_tui::ScreenTarget::Preferences(
            skit_ui::PreferencesControlId::InstallAgentSkill,
        )),
        CorpusOperation::ScreenHit(skit_tui::ScreenTarget::AgentSkill {
            name: "codex".to_owned(),
            scope: skit_application::AgentScope::User,
        }),
    ] {
        engine.run_operation(operation, &mut sink).unwrap();
    }

    assert!(matches!(
        engine.frontend().inner.state.screen(),
        Screen::Preferences(view) if view.agent_skill_install().is_none()
    ));
    let (frontend, host) = engine.into_parts();
    drop(frontend);
    host.close().unwrap();
}

#[test]
fn corpus_screen_resolver_has_distinct_total_refusals_and_event_chains() {
    let mut engine = corpus_engine();
    let mut sink = skit_tui_walker_support::engine::NoopCheckpointSink;
    engine.start(&mut sink).unwrap();
    let (mut frontend, host) = engine.into_parts();
    let target = skit_tui::ScreenTarget::Add(skit_tui::AddControlId::RunnerOption(1));
    let operation = CorpusOperation::ScreenHit(target.clone());
    let resolve = |operation: &CorpusOperation, inventory: &skit_tui::ScreenTargetInventory| {
        resolution_parts(
            resolve_screen_operation_with_inventory(&frontend.inner, operation, inventory).unwrap(),
        )
    };
    let assert_refusal = |inventory: skit_tui::ScreenTargetInventory,
                          expected: CorpusNotApplicable| {
        let (resolved, events) = resolve(&operation, &inventory);
        assert!(events.is_empty());
        assert_eq!(resolved.refusal, Some(expected));
    };
    assert_refusal(
        skit_tui::ScreenTargetInventory::default(),
        CorpusNotApplicable::Unavailable,
    );
    assert_refusal(
        skit_tui::ScreenTargetInventory {
            available: vec![target.clone()],
            ..skit_tui::ScreenTargetInventory::default()
        },
        CorpusNotApplicable::Clipped,
    );
    assert_refusal(
        skit_tui::ScreenTargetInventory {
            available: vec![target.clone(), target.clone()],
            ..skit_tui::ScreenTargetInventory::default()
        },
        CorpusNotApplicable::Ambiguous,
    );
    let hit = |rect| skit_tui::ScreenTargetHit {
        target: target.clone(),
        rect,
    };
    assert_refusal(
        skit_tui::ScreenTargetInventory {
            available: vec![target.clone()],
            hits: vec![hit(Rect::new(0, 0, 0, 1))],
            focus: None,
        },
        CorpusNotApplicable::Clipped,
    );
    assert_refusal(
        skit_tui::ScreenTargetInventory {
            available: vec![target.clone()],
            hits: vec![hit(Rect::new(60, 0, 1, 1))],
            focus: None,
        },
        CorpusNotApplicable::Clipped,
    );
    assert_refusal(
        skit_tui::ScreenTargetInventory {
            available: vec![target.clone()],
            hits: vec![hit(Rect::new(1, 1, 1, 1)), hit(Rect::new(2, 1, 1, 1))],
            focus: None,
        },
        CorpusNotApplicable::Ambiguous,
    );
    let inventory_error = |inventory: skit_tui::ScreenTargetInventory| {
        resolve_screen_operation_with_inventory(&frontend.inner, &operation, &inventory)
            .unwrap_err()
    };
    assert!(
        inventory_error(skit_tui::ScreenTargetInventory {
            available: Vec::new(),
            hits: vec![hit(Rect::new(1, 1, 1, 1))],
            focus: None,
        })
        .contains("hit is absent")
    );
    assert!(
        inventory_error(skit_tui::ScreenTargetInventory {
            available: Vec::new(),
            hits: Vec::new(),
            focus: Some(skit_tui::ScreenFocusInventory {
                current: None,
                order: vec![target.clone()],
            }),
        })
        .contains("focus target is absent")
    );
    assert!(
        inventory_error(skit_tui::ScreenTargetInventory {
            available: vec![target.clone()],
            hits: Vec::new(),
            focus: Some(skit_tui::ScreenFocusInventory {
                current: Some(target.clone()),
                order: Vec::new(),
            }),
        })
        .contains("current screen focus is absent")
    );

    let focus_target = skit_tui::ScreenTarget::Add(skit_tui::AddControlId::Draft(0));
    let current = skit_tui::ScreenTarget::Add(skit_tui::AddControlId::Continue);
    let focus_operation = CorpusOperation::ScreenFocus(focus_target.clone());
    let assert_focus_refusal = |inventory: skit_tui::ScreenTargetInventory,
                                expected: CorpusNotApplicable| {
        let (resolved, events) = resolve(&focus_operation, &inventory);
        assert!(events.is_empty());
        assert_eq!(resolved.refusal, Some(expected));
    };
    assert_focus_refusal(
        skit_tui::ScreenTargetInventory::default(),
        CorpusNotApplicable::Unavailable,
    );
    assert_focus_refusal(
        skit_tui::ScreenTargetInventory {
            available: vec![focus_target.clone(), focus_target.clone()],
            ..skit_tui::ScreenTargetInventory::default()
        },
        CorpusNotApplicable::Ambiguous,
    );
    assert_focus_refusal(
        skit_tui::ScreenTargetInventory {
            available: vec![focus_target.clone()],
            ..skit_tui::ScreenTargetInventory::default()
        },
        CorpusNotApplicable::NotKeyboardFocusable,
    );
    assert_focus_refusal(
        skit_tui::ScreenTargetInventory {
            available: vec![focus_target.clone(), current.clone()],
            focus: Some(skit_tui::ScreenFocusInventory {
                current: Some(current.clone()),
                order: vec![current.clone()],
            }),
            hits: Vec::new(),
        },
        CorpusNotApplicable::NotKeyboardFocusable,
    );
    assert_focus_refusal(
        skit_tui::ScreenTargetInventory {
            available: vec![focus_target.clone(), current.clone()],
            focus: Some(skit_tui::ScreenFocusInventory {
                current: Some(current.clone()),
                order: vec![focus_target.clone(), focus_target.clone(), current.clone()],
            }),
            hits: Vec::new(),
        },
        CorpusNotApplicable::Ambiguous,
    );
    assert_focus_refusal(
        skit_tui::ScreenTargetInventory {
            available: vec![focus_target.clone()],
            focus: Some(skit_tui::ScreenFocusInventory {
                current: None,
                order: vec![focus_target.clone()],
            }),
            hits: Vec::new(),
        },
        CorpusNotApplicable::NotKeyboardFocusable,
    );
    assert_focus_refusal(
        skit_tui::ScreenTargetInventory {
            available: vec![focus_target.clone(), current.clone()],
            focus: Some(skit_tui::ScreenFocusInventory {
                current: Some(current.clone()),
                order: vec![focus_target.clone(), current.clone(), current.clone()],
            }),
            hits: Vec::new(),
        },
        CorpusNotApplicable::Ambiguous,
    );
    let focus_inventory = skit_tui::ScreenTargetInventory {
        available: vec![focus_target.clone(), current.clone()],
        focus: Some(skit_tui::ScreenFocusInventory {
            current: Some(current.clone()),
            order: vec![focus_target.clone(), current],
        }),
        hits: Vec::new(),
    };
    let (resolved, events) = resolution_parts(
        resolve_screen_operation_with_inventory(
            &frontend.inner,
            &focus_operation,
            &focus_inventory,
        )
        .unwrap(),
    );
    assert_eq!(resolved.input, CorpusInputKind::EventChain);
    assert_eq!(events.len(), 1);
    assert!(matches!(
        events.as_slice(),
        [CorpusEvent::Key(CorpusKeyEvent {
            code: CorpusKey::BackTab,
            ..
        })]
    ));

    let already = skit_tui::ScreenTargetInventory {
        available: vec![focus_target.clone()],
        focus: Some(skit_tui::ScreenFocusInventory {
            current: Some(focus_target.clone()),
            order: vec![focus_target.clone()],
        }),
        hits: Vec::new(),
    };
    let (resolved, events) = resolution_parts(
        resolve_screen_operation_with_inventory(&frontend.inner, &focus_operation, &already)
            .unwrap(),
    );
    assert!(events.is_empty());
    assert_eq!(resolved.refusal, Some(CorpusNotApplicable::AlreadyFocused));

    let agent = skit_tui::ScreenTarget::AgentSkill {
        name: "codex".to_owned(),
        scope: skit_application::AgentScope::Project,
    };
    let (resolved, events) = resolution_parts(
        resolve_screen_operation_with_inventory(
            &frontend.inner,
            &CorpusOperation::ScreenFocus(agent.clone()),
            &skit_tui::ScreenTargetInventory {
                available: vec![agent],
                focus: None,
                hits: Vec::new(),
            },
        )
        .unwrap(),
    );
    assert!(events.is_empty());
    assert_eq!(
        resolved.refusal,
        Some(CorpusNotApplicable::NotKeyboardFocusable)
    );

    let invalid = CorpusOperation::ScreenHit(skit_tui::ScreenTarget::FilePickerEntry {
        relative: PathBuf::from("../outside"),
    });
    assert!(frontend.resolve(&invalid).is_err());
    assert!(
        resolve_screen_operation_with_inventory(
            &frontend.inner,
            &CorpusOperation::RawKey(CorpusKeyEvent::press(
                CorpusKey::Enter,
                CorpusModifiers::NONE,
            )),
            &skit_tui::ScreenTargetInventory::default(),
        )
        .unwrap_err()
        .contains("non-screen operation")
    );
    let geometry = frontend.inner.geometry.clone();
    let state = frontend.inner.state.clone();
    let _ = frontend
        .inner
        .session
        .handle_event(Event::FocusLost, &state, &geometry);
    assert!(frontend.resolve(&operation).is_err());

    drop(frontend);
    host.close().unwrap();
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn corpus_keyboard_and_mouse_paths_reach_the_same_command_and_local_endpoint() {
    let parent = tempfile::TempDir::new().unwrap();
    let namespace =
        StableSandboxNamespace::explicit(parent.path().join(STABLE_SANDBOX_NAMESPACE)).unwrap();
    let back = LocalActionTarget::Health(HealthAction::Back);
    let keyboard = corpus_parity_after(
        namespace.clone(),
        &[
            CorpusOperation::CommandKeyboard(UiCommand::Health),
            CorpusOperation::LocalKeyboard {
                target: back.clone(),
                key: CorpusKeyEvent::press(CorpusKey::Escape, CorpusModifiers::NONE),
            },
        ],
    );
    let mouse = corpus_parity_after(
        namespace,
        &[
            CorpusOperation::HitTarget(HitTarget::Command(UiCommand::Health)),
            CorpusOperation::LocalHit(back),
        ],
    );

    assert_eq!(keyboard, mouse);
    assert!(matches!(keyboard.state.screen(), Screen::Library));
}

/// Publish one hit that reaches past the right edge of the viewport.
fn out_of_bounds_probe_geometry(geometry: &ViewGeometry) -> ViewGeometry {
    let mut forged = geometry.clone();
    forged.hits.push(skit_tui::HitRegion {
        rect: Rect::new(0, 0, u16::MAX, 1),
        action: HitTarget::Command(UiCommand::Help),
    });
    forged
}

fn corpus_engine_with_probe(probe: fn(&ViewGeometry) -> ViewGeometry) -> CorpusEngine {
    let factory = smoke_factory();
    let host = RealWalkerHost::spawn(factory.seed).unwrap();
    let file_picker_tree = host.file_picker_tree();
    let state = host.initial_state().unwrap();
    let mut frontend =
        CorpusFrontend::new(state, factory.locale, factory.size, file_picker_tree).unwrap();
    frontend.inner.probe_geometry = probe;
    WalkerEngine::new(
        frontend,
        CorpusHostBoundary {
            inner: RealHostBoundary { host },
            leak_sandbox: None,
        },
        EFFECT_LIMIT,
    )
    .unwrap()
}

#[test]
fn a_forged_geometry_poisons_the_engine_with_the_named_parity_rule() {
    let mut engine = corpus_engine_with_probe(out_of_bounds_probe_geometry);
    let mut sink = skit_tui_walker_support::engine::NoopCheckpointSink;

    let error = engine.start(&mut sink).unwrap_err();

    assert!(error.contains("PUBLIC_HIT_BOUNDS"), "{error}");
    assert!(engine.is_poisoned());
    close_corpus_engine(engine).unwrap();
}

#[test]
fn a_consumed_operation_records_a_session_checkpoint_and_empties_the_local_inventory() {
    let mut engine = corpus_engine();
    let mut sink = CorpusRefusalSink::default();
    engine.start(&mut sink).unwrap();
    let before = sink.last;

    engine
        .run_operation(CorpusOperation::Focus { gained: true }, &mut sink)
        .unwrap();

    assert_ne!(sink.last, before);
    assert_eq!(sink.last, Some((EnginePhase::Operations, Some(0))));
    assert_eq!(sink.boundaries.last(), Some(&EngineBoundary::Session));
    assert!(sink.refusals.is_empty());

    engine
        .run_operation(CorpusOperation::CommandKeyboard(UiCommand::Add), &mut sink)
        .unwrap();
    let browse = engine
        .frontend()
        .inner
        .session
        .local_action_inventory()
        .actions
        .iter()
        .find(|advertised| advertised.target == LocalActionTarget::Add(AddControlId::BrowseSource))
        .cloned()
        .expect("the rendered Add footer advertises its path picker");
    engine
        .run_operation(
            CorpusOperation::LocalKeyboard {
                target: browse.target.clone(),
                key: CorpusKeyEvent::from_terminal(browse.keys[0].event()).unwrap(),
            },
            &mut sink,
        )
        .unwrap();

    assert!(
        engine
            .frontend()
            .inner
            .session
            .local_action_inventory()
            .actions
            .is_empty(),
        "the open local overlay leaves the inventory empty"
    );
    assert_eq!(sink.boundaries.last(), Some(&EngineBoundary::Session));
    close_corpus_engine(engine).unwrap();
}

#[test]
fn a_forged_local_descriptor_refuses_with_the_named_endpoint_rule() {
    let mut engine = corpus_engine();
    let mut sink = skit_tui_walker_support::engine::NoopCheckpointSink;
    engine.start(&mut sink).unwrap();
    engine
        .run_operation(CorpusOperation::CommandKeyboard(UiCommand::Add), &mut sink)
        .unwrap();
    let live = engine
        .frontend()
        .inner
        .session
        .local_action_inventory()
        .actions
        .clone();
    let advertised = live
        .iter()
        .find(|advertised| advertised.target == LocalActionTarget::Add(AddControlId::BrowseSource))
        .cloned()
        .expect("the Add screen advertises its source picker");
    let operation = CorpusOperation::LocalKeyboard {
        target: advertised.target.clone(),
        key: CorpusKeyEvent::from_terminal(advertised.keys[0].event()).unwrap(),
    };
    let mut forged = advertised;
    forged.outcome = LocalActionOutcome::Action(Action::Quit);

    assert!(
        resolve_corpus_operation_with_local_actions(&engine.frontend().inner, &operation, &live)
            .is_ok()
    );
    let error = resolve_corpus_operation_with_local_actions(
        &engine.frontend().inner,
        &operation,
        &[forged],
    )
    .unwrap_err();

    assert!(error.contains("LOCAL_ACTION_ENDPOINT"), "{error}");
    assert!(!engine.is_quit());
    close_corpus_engine(engine).unwrap();
}

#[test]
fn random_inventory_reports_a_stale_render_instead_of_empty_targets() {
    let mut engine = corpus_engine();
    let mut sink = skit_tui_walker_support::engine::NoopCheckpointSink;
    engine.start(&mut sink).unwrap();
    let inner = &engine.frontend().inner;
    let mut frontend = CorpusFrontend::from_real(
        RealFrontend::new(inner.state.clone(), Locale::En, Size::new(80, 24)).unwrap(),
        engine.host().inner.host.file_picker_tree(),
    );
    frontend.inner.observe_frontend().unwrap();
    assert!(frontend.live_inventory().is_ok());
    let _ = frontend.inner.session.handle_event(
        Event::FocusLost,
        &frontend.inner.state,
        &frontend.inner.geometry,
    );
    assert_eq!(
        frontend.live_inventory().unwrap_err(),
        "the screen target inventory is stale"
    );
    close_corpus_engine(engine).unwrap();
}

#[test]
fn raw_quit_press_is_dispatched_without_inventing_a_release() {
    let mut engine = corpus_engine();
    let mut sink = skit_tui_walker_support::engine::NoopCheckpointSink;
    engine.start(&mut sink).unwrap();
    let frontend = &engine.frontend().inner;
    let rect = frontend
        .geometry
        .hits
        .iter()
        .find(|hit| hit.action == HitTarget::Command(UiCommand::Quit) && !hit.rect.is_empty())
        .unwrap()
        .rect;
    let operation = CorpusOperation::RawMouse {
        column: rect.x,
        row: rect.y,
        kind: CorpusMouseKind::PrimaryDown,
    };
    let (resolved, events) =
        resolution_parts(resolve_corpus_operation(frontend, &operation).unwrap());
    assert_eq!(resolved.refusal, None);
    assert_eq!(
        events,
        vec![CorpusEvent::mouse(
            rect.x,
            rect.y,
            CorpusMouseKind::PrimaryDown
        )]
    );
    engine.run_operation(operation, &mut sink).unwrap();
    let release = CorpusOperation::RawMouse {
        column: rect.x,
        row: rect.y,
        kind: CorpusMouseKind::PrimaryUp,
    };
    let (resolved, events) =
        resolution_parts(resolve_corpus_operation(&engine.frontend().inner, &release).unwrap());
    assert_eq!(resolved.refusal, Some(CorpusNotApplicable::EventWouldQuit));
    assert!(events.is_empty());
    close_corpus_engine(engine).unwrap();
}

#[test]
fn every_cell_of_the_live_quit_chip_accepts_an_unarmed_raw_event() {
    let mut engine = corpus_engine();
    let mut sink = skit_tui_walker_support::engine::NoopCheckpointSink;
    engine.start(&mut sink).unwrap();
    let frontend = &engine.frontend().inner;
    let quit = frontend
        .geometry
        .hits
        .iter()
        .find(|hit| hit.action == HitTarget::Command(UiCommand::Quit) && !hit.rect.is_empty())
        .cloned()
        .expect("the library footer advertises a visible Quit chip");

    let mut swept = 0_usize;
    for row in quit.rect.rows() {
        for column in quit.rect.columns() {
            for kind in [CorpusMouseKind::PrimaryDown, CorpusMouseKind::PrimaryUp] {
                let operation = CorpusOperation::RawMouse {
                    column: column.x,
                    row: row.y,
                    kind,
                };
                let (resolved, events) =
                    resolution_parts(resolve_corpus_operation(frontend, &operation).unwrap());
                assert_eq!(events, vec![CorpusEvent::mouse(column.x, row.y, kind)]);
                assert_eq!(
                    resolved.refusal, None,
                    "cell {},{} kind {kind:?}",
                    column.x, row.y
                );
                swept = swept.saturating_add(1);
            }
        }
    }
    assert!(swept > 0);

    let outside = CorpusOperation::RawMouse {
        column: 0,
        row: 0,
        kind: CorpusMouseKind::Move,
    };
    let (resolved, events) =
        resolution_parts(resolve_corpus_operation(frontend, &outside).unwrap());
    assert_eq!(resolved.refusal, None);
    assert_eq!(
        events,
        vec![CorpusEvent::mouse(0, 0, CorpusMouseKind::Move)]
    );
    close_corpus_engine(engine).unwrap();
}

#[test]
fn every_pointer_kind_keeps_its_raw_event_shape() {
    for kind in CorpusMouseKind::ALL {
        let event = CorpusEvent::mouse(3, 4, *kind);
        assert_eq!(event.value()["kind"], json!(kind.label()));
        assert!(matches!(event.terminal(), Event::Mouse(mouse) if mouse.kind == kind.terminal()));
        assert_eq!(
            corpus_operation_value(&CorpusOperation::RawMouse {
                column: 3,
                row: 4,
                kind: *kind,
            })
            .unwrap(),
            json!({
                "operation": "raw_mouse",
                "column": 3,
                "row": 4,
                "kind": kind.label(),
            })
        );
    }
    assert_eq!(
        CorpusMouseKind::ALL
            .iter()
            .map(|kind| kind.label())
            .collect::<Vec<_>>(),
        [
            "primary_down",
            "secondary_down",
            "middle_down",
            "primary_up",
            "primary_drag",
            "move",
            "scroll_up",
            "scroll_down",
        ]
    );
    assert_eq!(
        corpus_canvas_size(
            Size::new(10, 10),
            &[CorpusOperation::RawMouse {
                column: 0,
                row: 0,
                kind: CorpusMouseKind::Move,
            }],
        ),
        Size::new(10, 10)
    );
    assert!(
        resolve_screen_operation_with_inventory(
            &RealFrontend::new(LibraryState::default(), Locale::En, Size::new(4, 4)).unwrap(),
            &CorpusOperation::RawMouse {
                column: 0,
                row: 0,
                kind: CorpusMouseKind::Move,
            },
            &ScreenTargetInventory {
                available: Vec::new(),
                focus: None,
                hits: Vec::new(),
            },
        )
        .unwrap_err()
        .contains("non-screen operation")
    );
}

#[test]
fn corpus_resolver_records_typed_refusals_and_real_input_plans() {
    let mut engine = corpus_engine();
    let mut sink = skit_tui_walker_support::engine::NoopCheckpointSink;
    engine.start(&mut sink).unwrap();
    let frontend = engine.frontend();

    for operation in [
        CorpusOperation::CommandKeyboard(UiCommand::Quit),
        CorpusOperation::HitTarget(HitTarget::Command(UiCommand::Quit)),
    ] {
        let (resolved, events) = resolution_parts(frontend.resolve(&operation).unwrap());
        assert!(events.is_empty());
        assert_eq!(resolved.refusal, Some(CorpusNotApplicable::QuitFiltered));
    }

    let raw_quit = CorpusOperation::RawKey(CorpusKeyEvent::press(
        CorpusKey::Character('q'),
        CorpusModifiers::NONE,
    ));
    let (resolved, events) = resolution_parts(frontend.resolve(&raw_quit).unwrap());
    assert!(events.is_empty());
    assert_eq!(resolved.refusal, Some(CorpusNotApplicable::EventWouldQuit));

    let unavailable_command = CorpusOperation::CommandKeyboard(UiCommand::SavePreferences);
    let (resolved, events) = resolution_parts(frontend.resolve(&unavailable_command).unwrap());
    assert!(events.is_empty());
    assert_eq!(resolved.refusal, Some(CorpusNotApplicable::Unavailable));

    let missing = CorpusOperation::HitTarget(HitTarget::FocusField(usize::MAX));
    let (resolved, events) = resolution_parts(frontend.resolve(&missing).unwrap());
    assert!(events.is_empty());
    assert_eq!(resolved.refusal, Some(CorpusNotApplicable::Unavailable));
    assert_eq!(
        corpus_resolution_value(TimelinePhase::Operations, &resolved).unwrap(),
        json!({
            "input": {"kind": "not_applicable"},
            "semantic_target": {"requested": {
                "operation": "hit_target",
                "target": {"focus_field": usize::MAX},
            }},
            "refusal": "unavailable",
        })
    );

    let invalid_resize = CorpusOperation::Resize {
        width: 0,
        height: 24,
    };
    let (resolved, events) = resolution_parts(frontend.resolve(&invalid_resize).unwrap());
    assert!(events.is_empty());
    assert_eq!(resolved.refusal, Some(CorpusNotApplicable::InvalidViewport));

    let target = LocalActionTarget::Health(HealthAction::Back);
    let key = CorpusKeyEvent::press(CorpusKey::Enter, CorpusModifiers::NONE);
    let local_keyboard = CorpusOperation::LocalKeyboard {
        target: target.clone(),
        key,
    };
    let local_hit = CorpusOperation::LocalHit(target.clone());
    let assert_local_refusal = |operation: &CorpusOperation,
                                local_actions: &[LocalAdvertisedAction],
                                expected: CorpusNotApplicable| {
        let (resolved, events) = resolution_parts(
            resolve_corpus_operation_with_local_actions(&frontend.inner, operation, local_actions)
                .unwrap(),
        );
        assert!(events.is_empty());
        assert_eq!(resolved.refusal, Some(expected));
    };
    assert_local_refusal(&local_keyboard, &[], CorpusNotApplicable::Unavailable);
    assert_local_refusal(&local_hit, &[], CorpusNotApplicable::Unavailable);

    let advertised = LocalAdvertisedAction {
        target,
        keys: Vec::new(),
        hit: Some(Rect::new(2, 3, 4, 1)),
        outcome: LocalActionOutcome::Consumed,
    };
    let duplicate_actions = [advertised.clone(), advertised.clone()];
    assert_local_refusal(
        &local_keyboard,
        &duplicate_actions,
        CorpusNotApplicable::Unavailable,
    );
    assert_local_refusal(
        &local_hit,
        &duplicate_actions,
        CorpusNotApplicable::Unavailable,
    );
    assert_local_refusal(
        &local_keyboard,
        std::slice::from_ref(&advertised),
        CorpusNotApplicable::Unavailable,
    );

    let mut clipped = advertised.clone();
    clipped.hit = None;
    assert_local_refusal(
        &local_hit,
        std::slice::from_ref(&clipped),
        CorpusNotApplicable::Clipped,
    );
    clipped.hit = Some(Rect::new(2, 3, 0, 1));
    assert_local_refusal(
        &local_hit,
        std::slice::from_ref(&clipped),
        CorpusNotApplicable::Clipped,
    );

    for operation in [
        CorpusOperation::RawKey(CorpusKeyEvent::press(
            CorpusKey::Function(24),
            CorpusModifiers::NONE,
        )),
        CorpusOperation::Paste("typed".to_owned()),
        CorpusOperation::Focus { gained: false },
        CorpusOperation::Resize {
            width: 72,
            height: 20,
        },
        CorpusOperation::FinalLiveness,
    ] {
        let (resolved, events) = resolution_parts(frontend.resolve(&operation).unwrap());
        assert_eq!(events.len(), 1);
        let phase = if operation == CorpusOperation::FinalLiveness {
            TimelinePhase::FinalLiveness
        } else {
            TimelinePhase::Operations
        };
        assert!(corpus_resolution_value(phase, &resolved).is_ok());
    }

    let (frontend, host) = engine.into_parts();
    drop(frontend);
    host.close().unwrap();
}

#[test]
fn corpus_clipped_hit_is_not_applicable_without_guessing_an_ordinal() {
    let mut engine = corpus_engine();
    let mut sink = skit_tui_walker_support::engine::NoopCheckpointSink;
    engine.start(&mut sink).unwrap();
    let (mut frontend, host) = engine.into_parts();
    let target = HitTarget::FocusField(42);
    frontend.inner.geometry.hits.push(skit_tui::HitRegion {
        rect: Rect::new(3, 4, 0, 1),
        action: target,
    });

    let (resolved, events) = resolution_parts(
        frontend
            .resolve(&CorpusOperation::HitTarget(target))
            .unwrap(),
    );
    assert!(events.is_empty());
    assert_eq!(resolved.refusal, Some(CorpusNotApplicable::Clipped));

    let duplicate_target = HitTarget::FocusField(43);
    for rect in [Rect::new(1, 1, 2, 1), Rect::new(5, 1, 2, 1)] {
        frontend.inner.geometry.hits.push(skit_tui::HitRegion {
            rect,
            action: duplicate_target,
        });
    }
    let (resolved, events) = resolution_parts(
        frontend
            .resolve(&CorpusOperation::HitTarget(duplicate_target))
            .unwrap(),
    );
    assert!(events.is_empty());
    assert_eq!(resolved.refusal, Some(CorpusNotApplicable::Unavailable));

    drop(frontend);
    host.close().unwrap();
}
