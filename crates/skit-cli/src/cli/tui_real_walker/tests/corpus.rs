//! Canonical corpus, replay, resize, and operation shape contracts.

use super::*;

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn record_stable_corpus_prefix(
    namespace: StableSandboxNamespace,
    operations: &[CorpusOperation],
) -> RecordedRealTrace {
    let factory = smoke_factory();
    record_corpus_profile(&factory, namespace, operations, StablePairPhase::Main).unwrap()
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn canonical_corpus_has_exact_operation_count_and_required_profile_order() {
    let operations = canonical_corpus_operations();
    assert_eq!(operations.len(), 100);
    assert_eq!(
        operations[98],
        CorpusOperation::Resize {
            width: 24,
            height: 6,
        }
    );
    assert_eq!(operations[99], CorpusOperation::Focus { gained: true });
    assert!(!operations.iter().any(|operation| matches!(
        operation,
        CorpusOperation::CommandKeyboard(UiCommand::Quit)
            | CorpusOperation::HitTarget(HitTarget::Command(UiCommand::Quit))
            | CorpusOperation::FinalLiveness
    )));
    assert!(!operations.iter().any(|operation| matches!(
        operation,
        CorpusOperation::RawKey(CorpusKeyEvent {
            code: CorpusKey::Enter,
            ..
        })
    )));
    let values = operations
        .iter()
        .map(corpus_operation_value)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let bytes = canonical_json_bytes(&Value::Array(values)).unwrap();
    assert_eq!(bytes.len(), 7_839);
    assert_eq!(
        sha256_hex(&bytes),
        "824a42732e08119f3c57286f636d3f12936e3a6b0aa99947c8da17db92d0e922"
    );

    let factories = required_corpus_factories().unwrap();
    assert_eq!(factories.len(), 4);
    for (factory, required) in factories
        .iter()
        .zip(skit_tui_walker_support::required_review_profiles())
    {
        assert_eq!(factory.review_profile.as_str(), required.id);
        assert_eq!(factory.locale.tag(), required.locale);
        assert_eq!(factory.size.width, required.viewport.width);
        assert_eq!(factory.size.height, required.viewport.height);
        assert_eq!(corpus_canvas_size(factory.size, &operations), factory.size);
    }
}

#[test]
fn corpus_refusal_sink_records_a_real_not_applicable_operation() {
    let mut engine = corpus_engine();
    let mut sink = CorpusRefusalSink::default();
    engine.start(&mut sink).unwrap();
    engine
        .run_operation(
            CorpusOperation::HitTarget(HitTarget::FocusField(usize::MAX)),
            &mut sink,
        )
        .unwrap();
    let (frontend, host) = engine.into_parts();
    drop(frontend);
    host.close().unwrap();

    assert_eq!(sink.refusals, [(0, CorpusNotApplicable::Unavailable)]);
    assert_eq!(sink.last, Some((EnginePhase::Operations, Some(0))));
}

#[test]
fn local_keyboard_uses_the_exact_key_when_one_add_target_has_two_chords() {
    let mut engine = corpus_engine_for_factory(required_corpus_factories().unwrap().remove(0));
    let mut sink = skit_tui_walker_support::engine::NoopCheckpointSink;
    engine.start(&mut sink).unwrap();
    for operation in [
        CorpusOperation::CommandKeyboard(UiCommand::Add),
        CorpusOperation::ScreenFocus(ScreenTarget::Add(AddControlId::NewScript)),
    ] {
        engine.run_operation(operation, &mut sink).unwrap();
    }
    let operation = CorpusOperation::LocalKeyboard {
        target: LocalActionTarget::Add(AddControlId::NewScript),
        key: CorpusKeyEvent::press(CorpusKey::Enter, CorpusModifiers::NONE),
    };
    let actions = engine
        .frontend()
        .inner
        .session
        .local_action_inventory()
        .actions
        .clone();
    assert_eq!(
        actions
            .iter()
            .filter(
                |advertised| advertised.target == LocalActionTarget::Add(AddControlId::NewScript)
            )
            .count(),
        2
    );
    let enter = actions
        .iter()
        .find(|advertised| {
            advertised.keys.iter().any(|binding| {
                CorpusKeyEvent::from_terminal(binding.event())
                    == Some(CorpusKeyEvent::press(
                        CorpusKey::Enter,
                        CorpusModifiers::NONE,
                    ))
            })
        })
        .unwrap()
        .clone();
    let (duplicate, events) = resolution_parts(
        resolve_corpus_operation_with_local_actions(
            &engine.frontend().inner,
            &operation,
            &[enter.clone(), enter],
        )
        .unwrap(),
    );
    assert!(events.is_empty());
    assert_eq!(duplicate.refusal, Some(CorpusNotApplicable::Unavailable));
    engine.run_operation(operation, &mut sink).unwrap();
    let review = matches!(
        engine.frontend().inner.state.screen(),
        Screen::Add(view) if view.stage() == skit_ui::AddStage::Review
    );
    let (frontend, host) = engine.into_parts();
    drop(frontend);
    host.close().unwrap();

    assert!(review);
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn canonical_corpus_preflight_returns_the_real_host_to_the_library() {
    let operations = canonical_corpus_operations();
    let mut refusals = Vec::new();
    for factory in required_corpus_factories().unwrap() {
        let profile = factory.review_profile.clone();
        let parent = tempfile::TempDir::new().unwrap();
        let namespace =
            StableSandboxNamespace::explicit(parent.path().join(STABLE_SANDBOX_NAMESPACE)).unwrap();
        let mut engine = stable_corpus_engine_for_factory(factory, namespace);
        let mut sink = CorpusRefusalSink::default();
        let result = (|| {
            engine.start(&mut sink)?;
            let mut contexts = Vec::new();
            for (index, operation) in operations.iter().enumerate() {
                let operation_context = format!(
                    "profile {profile} operation {} {operation:?} failed",
                    index.saturating_add(1)
                );
                engine
                    .run_operation(operation.clone(), &mut sink)
                    .expect(&operation_context);
                contexts.push((
                    index.saturating_add(1),
                    engine.frontend().inner.state.command_context(),
                ));
            }
            engine
                .run_final_liveness(CorpusOperation::FinalLiveness, &mut sink)
                .map_err(|error| format!("profile {profile} final liveness failed: {error}"))?;
            let successful_operations = engine.successful_operations().len();
            let quit = engine.is_quit();
            let library = matches!(engine.frontend().inner.state.screen(), Screen::Library)
                && engine.frontend().inner.state.modal().is_none();
            let final_cause = sink.last == Some((EnginePhase::FinalLiveness, None));
            Ok::<_, String>((
                library,
                successful_operations,
                quit,
                final_cause,
                sink.refusals.clone(),
                contexts,
            ))
        })();
        let (frontend, host) = engine.into_parts();
        drop(frontend);
        host.close().unwrap();
        let (library, successful_operations, quit, final_cause, profile_refusals, contexts) =
            result.unwrap();
        let final_contexts = &contexts[contexts.len().saturating_sub(20)..];

        assert!(
            library,
            "profile {profile} refusals {profile_refusals:?}; final contexts are {:?}",
            final_contexts
        );
        assert_eq!(successful_operations, 100, "profile {profile}");
        assert!(!quit, "profile {profile} requested quit");
        assert!(final_cause, "profile {profile} has a bad final cause");
        refusals.push((profile, profile_refusals));
    }
    assert!(
        refusals
            .iter()
            .all(|(_, profile_refusals)| profile_refusals.is_empty()),
        "canonical refusals: {refusals:?}"
    );
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn short_stable_corpus_prefix_replays_the_complete_real_trace() {
    let parent = tempfile::TempDir::new().unwrap();
    let namespace =
        StableSandboxNamespace::explicit(parent.path().join(STABLE_SANDBOX_NAMESPACE)).unwrap();
    let missing = CorpusOperation::HitTarget(HitTarget::FocusField(usize::MAX));
    let operations = vec![
        CorpusOperation::CommandKeyboard(UiCommand::Add),
        CorpusOperation::ScreenHit(ScreenTarget::Add(AddControlId::BrowseSource)),
        CorpusOperation::ScreenHit(ScreenTarget::FilePickerEntry {
            relative: PathBuf::from("original.sh"),
        }),
        CorpusOperation::ScreenFocus(ScreenTarget::Add(AddControlId::Continue)),
        CorpusOperation::RawKey(CorpusKeyEvent::press(
            CorpusKey::Enter,
            CorpusModifiers::NONE,
        )),
        CorpusOperation::RawKey(CorpusKeyEvent::press(
            CorpusKey::Escape,
            CorpusModifiers::NONE,
        )),
        CorpusOperation::RawKey(CorpusKeyEvent::press(
            CorpusKey::Escape,
            CorpusModifiers::NONE,
        )),
        missing.clone(),
    ];

    let main = record_stable_corpus_prefix(namespace.clone(), &operations);
    let replay = record_stable_corpus_prefix(namespace, &operations);

    assert_eq!(main.trace, replay.trace);
    assert_eq!(main.sandbox, replay.sandbox);
    assert_eq!(main.operations.len(), operations.len());
    assert_eq!(
        main.operations,
        operations
            .iter()
            .map(corpus_operation_value)
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    );
    assert_eq!(
        main.final_liveness_requested,
        corpus_operation_value(&CorpusOperation::FinalLiveness).unwrap()
    );
    let missing_value = corpus_operation_value(&missing).unwrap();
    assert!(main.rows.iter().any(|row| {
        matches!(
            &row.cause,
            TransitionCause::Session {
                requested,
                handling,
                ..
            } if requested == &missing_value && handling == "not_applicable"
        )
    }));
    assert_eq!(
        main.rows.last().unwrap().phase,
        TimelinePhase::FinalLiveness
    );
    assert_eq!(
        main.rows.last().unwrap().liveness,
        Some(LivenessResult::Passed)
    );
    validate_effect_chain_termination(&main.rows).unwrap();
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn diagnostic_stable_corpus_resize_replays_backend_terminal_event_and_viewport_bytes() {
    let parent = tempfile::TempDir::new().unwrap();
    let namespace =
        StableSandboxNamespace::explicit(parent.path().join(STABLE_SANDBOX_NAMESPACE)).unwrap();
    let resize = CorpusOperation::Resize {
        width: 48,
        height: 18,
    };
    let operations = [resize.clone()];

    let main = record_stable_corpus_prefix(namespace.clone(), &operations);
    let replay = record_stable_corpus_prefix(namespace, &operations);
    let resize_value = corpus_operation_value(&resize).unwrap();

    assert_eq!(main.trace, replay.trace);
    assert_eq!(main.sandbox, replay.sandbox);
    assert_eq!(
        main.operations.as_slice(),
        std::slice::from_ref(&resize_value)
    );
    let (row, resolved, event, handling) = main
        .rows
        .iter()
        .find_map(|row| match &row.cause {
            TransitionCause::Session {
                requested,
                resolved,
                event,
                handling,
            } if requested == &resize_value => Some((row, resolved, event, handling)),
            TransitionCause::Initial
            | TransitionCause::Session { .. }
            | TransitionCause::Reducer { .. }
            | TransitionCause::Host { .. } => None,
        })
        .unwrap();
    let expected_event = json!({
        "type": "resize",
        "width": 48,
        "height": 18,
    });
    assert_eq!(event, &expected_event);
    assert_eq!(
        resolved,
        &json!({
            "input": {
                "kind": "resize",
                "event": expected_event,
            },
            "semantic_target": {"requested": {
                "operation": "resize",
                "width": 48,
                "height": 18,
            }},
        })
    );
    assert_eq!(handling, "ignored");
    assert_eq!(
        row.viewport,
        RectSnapshot {
            x: 0,
            y: 0,
            width: 48,
            height: 18,
        }
    );
    let frame_bytes = &main.objects[&(row.styled_frame.kind, row.styled_frame.sha256.clone())];
    let frame: StyledFrameSnapshot =
        serde_json::from_value(validate_object(&row.styled_frame, frame_bytes).unwrap()).unwrap();
    assert_eq!(frame.area, row.viewport);
    assert_eq!(main.rows.last().unwrap().viewport, row.viewport);

    let cast_header: Value =
        serde_json::from_slice(main.cast.split(|byte| *byte == b'\n').next().unwrap()).unwrap();
    assert_eq!(
        cast_header,
        json!({"version": 3, "term": {"cols": 60, "rows": 24}})
    );
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn stable_corpus_grow_resize_uses_the_timeline_maximum_canvas() {
    let parent = tempfile::TempDir::new().unwrap();
    let namespace =
        StableSandboxNamespace::explicit(parent.path().join(STABLE_SANDBOX_NAMESPACE)).unwrap();
    let operations = [CorpusOperation::Resize {
        width: 72,
        height: 30,
    }];
    let recorded = record_stable_corpus_prefix(namespace, &operations);

    assert_eq!(bundle::cast_canvas(&recorded.rows).unwrap(), (72, 30));
    let cast_header: Value =
        serde_json::from_slice(recorded.cast.split(|byte| *byte == b'\n').next().unwrap()).unwrap();
    assert_eq!(
        cast_header,
        json!({"version": 3, "term": {"cols": 72, "rows": 30}})
    );
    let rebuilt = bundle::rebuild_presented_cast(&recorded.rows, |reference| {
        let bytes = &recorded.objects[&(reference.kind, reference.sha256.clone())];
        let value = validate_object(reference, bytes).map_err(|error| error.to_string())?;
        serde_json::from_value(value).map_err(|error| error.to_string())
    })
    .unwrap();
    assert_eq!(recorded.cast, rebuilt);
}

#[test]
fn corpus_canvas_forecast_ignores_each_zero_dimension_resize() {
    let initial = Size::new(60, 24);
    let operations = [
        CorpusOperation::Resize {
            width: 0,
            height: 100,
        },
        CorpusOperation::Resize {
            width: 100,
            height: 0,
        },
        CorpusOperation::Resize {
            width: 72,
            height: 12,
        },
        CorpusOperation::Resize {
            width: 40,
            height: 30,
        },
        CorpusOperation::FinalLiveness,
    ];
    assert_eq!(corpus_canvas_size(initial, &operations), Size::new(72, 30));
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn stable_corpus_zero_dimension_resizes_do_not_expand_the_canvas() {
    let parent = tempfile::TempDir::new().unwrap();
    let namespace =
        StableSandboxNamespace::explicit(parent.path().join(STABLE_SANDBOX_NAMESPACE)).unwrap();
    let operations = [
        CorpusOperation::Resize {
            width: 0,
            height: 100,
        },
        CorpusOperation::Resize {
            width: 100,
            height: 0,
        },
    ];
    let recorded = record_stable_corpus_prefix(namespace, &operations);

    assert_eq!(bundle::cast_canvas(&recorded.rows).unwrap(), (60, 24));
    let requested = operations
        .iter()
        .map(corpus_operation_value)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let refusals = recorded
        .rows
        .iter()
        .filter_map(|row| match &row.cause {
            TransitionCause::Session {
                requested,
                resolved,
                handling,
                ..
            } if handling == "not_applicable" => Some((requested, resolved)),
            TransitionCause::Initial
            | TransitionCause::Session { .. }
            | TransitionCause::Reducer { .. }
            | TransitionCause::Host { .. } => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(refusals.len(), 2);
    for ((actual, resolved), expected) in refusals.into_iter().zip(&requested) {
        assert_eq!(actual, expected);
        assert_eq!(resolved["refusal"], "invalid_viewport");
    }
}

#[test]
fn corpus_machine_mappings_are_total_and_stable() {
    let key_cases = [
        (CorpusKey::Character('界'), json!({"character": "界"})),
        (CorpusKey::Enter, json!("enter")),
        (CorpusKey::Escape, json!("escape")),
        (CorpusKey::Delete, json!("delete")),
        (CorpusKey::Backspace, json!("backspace")),
        (CorpusKey::Tab, json!("tab")),
        (CorpusKey::BackTab, json!("back_tab")),
        (CorpusKey::Up, json!("up")),
        (CorpusKey::Down, json!("down")),
        (CorpusKey::Left, json!("left")),
        (CorpusKey::Right, json!("right")),
        (CorpusKey::PageUp, json!("page_up")),
        (CorpusKey::PageDown, json!("page_down")),
        (CorpusKey::Home, json!("home")),
        (CorpusKey::End, json!("end")),
        (CorpusKey::Function(2), json!({"function": 2})),
    ];
    for (key, value) in key_cases {
        assert_eq!(key.value(), value);
        assert_eq!(CorpusKey::from_terminal(key.terminal()), Some(key));
    }
    assert_eq!(CorpusKey::from_terminal(KeyCode::Insert), None);

    for (key, expected) in [
        (UiKey::Character('x'), CorpusKey::Character('x')),
        (UiKey::Enter, CorpusKey::Enter),
        (UiKey::Escape, CorpusKey::Escape),
        (UiKey::Delete, CorpusKey::Delete),
        (UiKey::Backspace, CorpusKey::Backspace),
        (UiKey::Tab, CorpusKey::Tab),
        (UiKey::BackTab, CorpusKey::BackTab),
        (UiKey::Up, CorpusKey::Up),
        (UiKey::Down, CorpusKey::Down),
        (UiKey::PageUp, CorpusKey::PageUp),
        (UiKey::PageDown, CorpusKey::PageDown),
        (UiKey::Home, CorpusKey::Home),
        (UiKey::End, CorpusKey::End),
        (UiKey::Function(12), CorpusKey::Function(12)),
    ] {
        assert_eq!(CorpusKey::from_ui(key), expected);
    }

    let modifiers = CorpusModifiers {
        control: true,
        alt: true,
        shift: true,
    };
    assert_eq!(
        CorpusModifiers::from_terminal(modifiers.terminal()),
        modifiers
    );
    for kind in [
        CorpusKeyKind::Press,
        CorpusKeyKind::Repeat,
        CorpusKeyKind::Release,
    ] {
        let key = CorpusKeyEvent {
            code: CorpusKey::Character('x'),
            modifiers,
            kind,
        };
        assert_eq!(CorpusKeyEvent::from_terminal(key.terminal()), Some(key));
        assert_eq!(key.value()["kind"], kind.label());
    }

    for (target, expected) in [
        (
            HitTarget::Command(UiCommand::Health),
            json!({"command": "health"}),
        ),
        (
            HitTarget::RunFieldCommand {
                field: 1,
                command: RunFieldCommand::BrowsePath,
            },
            json!({"run_field_command": {"field": 1, "command": "browse_path"}}),
        ),
        (
            HitTarget::RunFieldCommand {
                field: 2,
                command: RunFieldCommand::InsertValue,
            },
            json!({"run_field_command": {"field": 2, "command": "insert_value"}}),
        ),
        (
            HitTarget::RunFieldCommand {
                field: 3,
                command: RunFieldCommand::ResetDefault,
            },
            json!({"run_field_command": {"field": 3, "command": "reset_default"}}),
        ),
        (HitTarget::FocusField(4), json!({"focus_field": 4})),
        (HitTarget::ToggleField(5), json!({"toggle_field": 5})),
        (
            HitTarget::SelectFieldOption {
                field: 6,
                option: 7,
            },
            json!({"select_field_option": {"field": 6, "option": 7}}),
        ),
    ] {
        assert_eq!(corpus_hit_target_value(target), expected);
    }

    for (target, key) in [
        (
            LocalActionTarget::Add(skit_tui::AddControlId::Cancel),
            "add",
        ),
        (LocalActionTarget::Health(HealthAction::Back), "health"),
        (
            LocalActionTarget::Runners(RunnerManagerAction::Back),
            "runners",
        ),
        (
            LocalActionTarget::RunnerEditor(RunnerEditorAction::Cancel),
            "runner_editor",
        ),
    ] {
        assert!(corpus_local_target_value(&target).get(key).is_some());
    }
    for (field, expected) in [
        (AddTextField::SourcePath, "source_path"),
        (AddTextField::CommandTemplate, "command_template"),
        (AddTextField::CommandName, "command_name"),
        (AddTextField::CommandDescription, "command_description"),
        (AddTextField::ReviewName, "review_name"),
        (AddTextField::ReviewDescription, "review_description"),
        (AddTextField::Dependencies, "dependencies"),
        (AddTextField::PythonConstraint, "python_constraint"),
    ] {
        assert_eq!(corpus_add_text_field_value(field), json!(expected));
    }
    for (target, expected) in [
        (
            AddControlId::Text(AddTextField::SourcePath),
            json!({"text": "source_path"}),
        ),
        (AddControlId::BrowseSource, json!("browse_source")),
        (AddControlId::Draft(2), json!({"draft": 2})),
        (AddControlId::NewScript, json!("new_script")),
        (AddControlId::NewPrompt, json!("new_prompt")),
        (AddControlId::DeleteDraft, json!("delete_draft")),
        (AddControlId::Continue, json!("continue")),
        (AddControlId::Kind(3), json!({"kind": 3})),
        (AddControlId::PickFocusedKind, json!("pick_focused_kind")),
        (AddControlId::Storage, json!("storage")),
        (AddControlId::StorageOption(1), json!({"storage_option": 1})),
        (
            AddControlId::Candidate("python".to_owned()),
            json!({"candidate": "python"}),
        ),
        (AddControlId::Interpolate, json!("interpolate")),
        (
            AddControlId::PromptCandidate("name".to_owned()),
            json!({"prompt_candidate": "name"}),
        ),
        (AddControlId::Runner, json!("runner")),
        (AddControlId::RunnerOption(4), json!({"runner_option": 4})),
        (AddControlId::NewRunner, json!("new_runner")),
        (AddControlId::EditSource, json!("edit_source")),
        (AddControlId::Save, json!("save")),
        (AddControlId::ToggleFocused, json!("toggle_focused")),
        (AddControlId::NextField, json!("next_field")),
        (AddControlId::PreviousField, json!("previous_field")),
        (AddControlId::Cancel, json!("cancel")),
    ] {
        assert_eq!(corpus_add_control_value(&target), expected);
    }
    for (target, expected) in [
        (PreferencesControlId::Language, "language"),
        (PreferencesControlId::Editor, "editor"),
        (PreferencesControlId::InteractiveForm, "interactive_form"),
        (PreferencesControlId::AfterRun, "after_run"),
        (PreferencesControlId::Javascript, "javascript"),
        (PreferencesControlId::BashPath, "bash_path"),
        (PreferencesControlId::ManageAgents, "manage_agents"),
        (
            PreferencesControlId::InstallAgentSkill,
            "install_agent_skill",
        ),
        (PreferencesControlId::MirrorMaster, "mirror_master"),
        (PreferencesControlId::PypiChoice, "pypi_choice"),
        (PreferencesControlId::PypiUrl, "pypi_url"),
        (PreferencesControlId::GithubChoice, "github_choice"),
        (PreferencesControlId::GithubUrl, "github_url"),
        (PreferencesControlId::NpmChoice, "npm_choice"),
        (PreferencesControlId::NpmUrl, "npm_url"),
    ] {
        assert_eq!(corpus_preferences_control_value(target), json!(expected));
    }
    assert_eq!(
        [AgentScope::User, AgentScope::Project].map(corpus_agent_scope_label),
        ["user", "project"]
    );
    for (target, expected) in [
        (
            ScreenTarget::Preferences(PreferencesControlId::Language),
            json!({"preferences": "language"}),
        ),
        (
            ScreenTarget::AgentSkill {
                name: "codex".to_owned(),
                scope: AgentScope::Project,
            },
            json!({"agent_skill": {"name": "codex", "scope": "project"}}),
        ),
        (
            ScreenTarget::Runner {
                name: "seed-agent".to_owned(),
            },
            json!({"runner": {"name": "seed-agent"}}),
        ),
        (
            ScreenTarget::FilePickerEntry {
                relative: PathBuf::from("nested/picked.py"),
            },
            json!({"file_picker_entry": {"components": ["nested", "picked.py"]}}),
        ),
    ] {
        assert_eq!(corpus_screen_target_value(&target).unwrap(), expected);
    }
    assert_eq!(
        corpus_local_outcome_value(&LocalActionOutcome::Consumed),
        json!("consumed")
    );
    assert_eq!(
        corpus_local_outcome_value(&LocalActionOutcome::Action(Action::Back)),
        json!({"action": "back"})
    );
    assert_eq!(
        corpus_semantic_target_value(&CorpusSemanticTarget::Hit {
            target: HitTarget::FocusField(7),
            rect: Rect::new(2, 3, 4, 5),
        })
        .unwrap(),
        json!({
            "hit_target": {"focus_field": 7},
            "rect": {"x": 2, "y": 3, "width": 4, "height": 5},
        })
    );
    let local = corpus_semantic_target_value(&CorpusSemanticTarget::Local {
        target: LocalActionTarget::Health(HealthAction::Back),
        keys: vec![CorpusKeyEvent::press(
            CorpusKey::Escape,
            CorpusModifiers::NONE,
        )],
        hit: Some(Rect::new(1, 2, 3, 4)),
        outcome: LocalActionOutcome::Consumed,
    })
    .unwrap();
    assert_eq!(local["local_target"], json!({"health": "back"}));
    assert_eq!(
        local["hit"],
        json!({"x": 1, "y": 2, "width": 3, "height": 4})
    );

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
    for kind in CorpusMouseKind::ALL {
        let event = CorpusEvent::mouse(2, 3, *kind);
        assert_eq!(
            event.value(),
            json!({
                "type": "mouse",
                "kind": kind.label(),
                "column": 2,
                "row": 3,
                "modifiers": {"alt": false, "control": false, "shift": false},
            })
        );
        assert!(matches!(event.terminal(), Event::Mouse(mouse) if mouse.kind == kind.terminal()));
    }

    let events = [
        CorpusEvent::Key(CorpusKeyEvent::press(
            CorpusKey::Enter,
            CorpusModifiers::NONE,
        )),
        CorpusEvent::mouse(2, 3, CorpusMouseKind::PrimaryDown),
        CorpusEvent::mouse(2, 3, CorpusMouseKind::PrimaryUp),
        CorpusEvent::Paste("value".to_owned()),
        CorpusEvent::Focus { gained: true },
        CorpusEvent::Focus { gained: false },
        CorpusEvent::Resize {
            width: 80,
            height: 24,
        },
    ];
    for event in events {
        assert!(event.value()["type"].is_string());
        let _ = event.terminal();
    }

    assert_eq!(
        [
            CorpusInputKind::NotApplicable,
            CorpusInputKind::Event,
            CorpusInputKind::EventChain,
            CorpusInputKind::PrimaryClick,
            CorpusInputKind::LocalEvent,
            CorpusInputKind::LocalPrimaryClick,
            CorpusInputKind::Resize,
        ]
        .map(CorpusInputKind::label),
        [
            "not_applicable",
            "event",
            "event_chain",
            "primary_click",
            "local_event",
            "local_primary_click",
            "resize",
        ]
    );
    assert_eq!(
        [
            CorpusNotApplicable::Unavailable,
            CorpusNotApplicable::Clipped,
            CorpusNotApplicable::Ambiguous,
            CorpusNotApplicable::AlreadyFocused,
            CorpusNotApplicable::NotKeyboardFocusable,
            CorpusNotApplicable::InvalidViewport,
            CorpusNotApplicable::QuitFiltered,
            CorpusNotApplicable::EventWouldQuit,
        ]
        .map(CorpusNotApplicable::label),
        [
            "unavailable",
            "clipped",
            "ambiguous",
            "already_focused",
            "not_keyboard_focusable",
            "invalid_viewport",
            "quit_filtered",
            "event_would_quit",
        ]
    );

    let target = CorpusSemanticTarget::Requested(CorpusOperation::Focus { gained: true });
    let invalid_single = CorpusResolution {
        input: CorpusInputKind::Event,
        events: Vec::new(),
        target: target.clone(),
        refusal: None,
    };
    assert!(
        corpus_resolution_value(TimelinePhase::Operations, &invalid_single)
            .unwrap_err()
            .contains("one event")
    );
    assert!(
        corpus_resolution_value(TimelinePhase::FinalLiveness, &invalid_single)
            .unwrap_err()
            .contains("one real event")
    );
    let invalid_click = CorpusResolution {
        input: CorpusInputKind::PrimaryClick,
        events: vec![CorpusEvent::Focus { gained: true }],
        target,
        refusal: None,
    };
    assert!(
        corpus_resolution_value(TimelinePhase::Operations, &invalid_click)
            .unwrap_err()
            .contains("press and release")
    );
    let invalid_chain = CorpusResolution {
        input: CorpusInputKind::EventChain,
        events: Vec::new(),
        target: CorpusSemanticTarget::Requested(CorpusOperation::Focus { gained: true }),
        refusal: None,
    };
    assert!(
        corpus_resolution_value(TimelinePhase::Operations, &invalid_chain)
            .unwrap_err()
            .contains("at least one event")
    );
    assert_eq!(
        [
            ScreenTargetError::ScreenUnavailable,
            ScreenTargetError::StaleSession,
            ScreenTargetError::RealFilesystemPicker,
            ScreenTargetError::InvalidMemoryEntry,
        ]
        .map(screen_target_error_message),
        [
            "the screen target inventory is unavailable",
            "the screen target inventory is stale",
            "the screen target inventory uses the real filesystem picker",
            "the screen target inventory contains an invalid memory entry",
        ]
    );
}

#[test]
fn corpus_file_targets_require_strict_utf8_relative_components() {
    let empty_runner = ScreenTarget::Runner {
        name: String::new(),
    };
    assert!(validate_screen_target(&empty_runner).is_err());
    assert!(corpus_operation_value(&CorpusOperation::ScreenHit(empty_runner)).is_err());
    let assert_invalid = |relative: PathBuf| {
        let target = ScreenTarget::FilePickerEntry { relative };
        assert!(validate_screen_target(&target).is_err());
        assert!(corpus_operation_value(&CorpusOperation::ScreenHit(target)).is_err());
    };
    for spelling in [
        "",
        ".",
        "..",
        "/absolute",
        "nested//original.sh",
        "./nested/original.sh",
        "nested/./original.sh",
        "nested/../original.sh",
        "nested/",
    ] {
        assert_invalid(PathBuf::from(spelling));
    }
    #[cfg(windows)]
    for spelling in [
        r"C:original.sh",
        r"C:\nested\original.sh",
        r"\\server\share\original.sh",
    ] {
        assert_invalid(PathBuf::from(spelling));
    }
    #[cfg(unix)]
    {
        use std::{ffi::OsStr, os::unix::ffi::OsStrExt as _};
        assert_invalid(PathBuf::from(OsStr::from_bytes(b"original-\xff.sh")));
    }
    #[cfg(windows)]
    {
        use std::{ffi::OsString, os::windows::ffi::OsStringExt as _};
        assert_invalid(PathBuf::from(OsString::from_wide(&[
            u16::from(b'o'),
            0xd800,
            u16::from(b's'),
        ])));
    }
    assert_eq!(
        corpus_relative_component_values(Path::new("nested/original.sh")).unwrap(),
        [json!("nested"), json!("original.sh")]
    );
}

#[test]
fn corpus_screen_operations_and_resolutions_have_exact_outer_shapes() {
    let focus_target = ScreenTarget::Preferences(PreferencesControlId::ManageAgents);
    let focus_current = ScreenTarget::Preferences(PreferencesControlId::Language);
    let focus_order = vec![focus_current.clone(), focus_target.clone()];
    let focus_operation = CorpusOperation::ScreenFocus(focus_target.clone());
    let focus_operation_value = json!({
        "operation": "screen_focus",
        "target": {"preferences": "manage_agents"},
    });
    assert_eq!(
        corpus_operation_value(&focus_operation).unwrap(),
        focus_operation_value
    );
    let focus_resolution = CorpusResolution {
        input: CorpusInputKind::EventChain,
        events: vec![CorpusEvent::Key(CorpusKeyEvent::press(
            CorpusKey::Tab,
            CorpusModifiers::NONE,
        ))],
        target: CorpusSemanticTarget::ScreenFocus {
            target: focus_target,
            current: focus_current,
            order: focus_order,
        },
        refusal: None,
    };
    assert_eq!(
        corpus_resolution_value(TimelinePhase::Operations, &focus_resolution).unwrap(),
        json!({
            "input": {
                "kind": "event_chain",
                "events": [{
                    "type": "key",
                    "key": {
                        "code": "tab",
                        "kind": "press",
                        "modifiers": {"alt": false, "control": false, "shift": false},
                    },
                }],
            },
            "semantic_target": {"screen_focus": {
                "target": {"preferences": "manage_agents"},
                "current": {"preferences": "language"},
                "order": [
                    {"preferences": "language"},
                    {"preferences": "manage_agents"},
                ],
            }},
        })
    );

    for (target, target_value) in [
        (
            ScreenTarget::Add(AddControlId::BrowseSource),
            json!({"add": "browse_source"}),
        ),
        (
            ScreenTarget::AgentSkill {
                name: "codex".to_owned(),
                scope: AgentScope::Project,
            },
            json!({"agent_skill": {"name": "codex", "scope": "project"}}),
        ),
        (
            ScreenTarget::Runner {
                name: "seed-agent".to_owned(),
            },
            json!({"runner": {"name": "seed-agent"}}),
        ),
        (
            ScreenTarget::FilePickerEntry {
                relative: PathBuf::from("nested/original.sh"),
            },
            json!({"file_picker_entry": {"components": ["nested", "original.sh"]}}),
        ),
    ] {
        let operation = CorpusOperation::ScreenHit(target.clone());
        let operation_value = corpus_operation_value(&operation).unwrap();
        assert_eq!(
            operation_value,
            json!({"operation": "screen_hit", "target": target_value.clone()})
        );
        let resolution = CorpusResolution {
            input: CorpusInputKind::PrimaryClick,
            events: vec![
                CorpusEvent::mouse(4, 3, CorpusMouseKind::PrimaryDown),
                CorpusEvent::mouse(4, 3, CorpusMouseKind::PrimaryUp),
            ],
            target: CorpusSemanticTarget::ScreenHit {
                target,
                rect: Rect::new(2, 3, 5, 1),
            },
            refusal: None,
        };
        let resolution_value =
            corpus_resolution_value(TimelinePhase::Operations, &resolution).unwrap();
        assert_eq!(
            resolution_value,
            json!({
                "input": {
                    "kind": "primary_click",
                    "events": [
                        {
                            "type": "mouse",
                            "kind": "primary_down",
                            "column": 4,
                            "row": 3,
                            "modifiers": {"alt": false, "control": false, "shift": false},
                        },
                        {
                            "type": "mouse",
                            "kind": "primary_up",
                            "column": 4,
                            "row": 3,
                            "modifiers": {"alt": false, "control": false, "shift": false},
                        },
                    ],
                },
                "semantic_target": {"screen_hit": {
                    "target": target_value,
                    "rect": {"x": 2, "y": 3, "width": 5, "height": 1},
                }},
            })
        );
        let machine_bytes = format!("{operation_value}{resolution_value}");
        for forbidden in ["/nested", "\\nested", "\"base\"", "skills_dir"] {
            assert!(!machine_bytes.contains(forbidden), "found {forbidden}");
        }
    }
}

#[test]
fn corpus_operations_have_stable_source_owned_shapes() {
    let raw = CorpusKeyEvent::press(
        CorpusKey::Character('*'),
        CorpusModifiers {
            control: true,
            alt: false,
            shift: true,
        },
    );
    let local = LocalActionTarget::Health(HealthAction::Back);
    let cases = [
        (
            CorpusOperation::CommandKeyboard(UiCommand::Health),
            json!({"operation": "command_keyboard", "command": "health"}),
        ),
        (
            CorpusOperation::HitTarget(skit_tui::HitTarget::Command(UiCommand::Health)),
            json!({
                "operation": "hit_target",
                "target": {"command": "health"},
            }),
        ),
        (
            CorpusOperation::RawKey(raw),
            json!({
                "operation": "raw_key",
                "key": {
                    "code": {"character": "*"},
                    "kind": "press",
                    "modifiers": {"alt": false, "control": true, "shift": true},
                },
            }),
        ),
        (
            CorpusOperation::LocalKeyboard {
                target: local.clone(),
                key: CorpusKeyEvent::press(CorpusKey::Escape, CorpusModifiers::NONE),
            },
            json!({
                "operation": "local_keyboard",
                "target": {"health": "back"},
                "key": {
                    "code": "escape",
                    "kind": "press",
                    "modifiers": {"alt": false, "control": false, "shift": false},
                },
            }),
        ),
        (
            CorpusOperation::LocalHit(local),
            json!({"operation": "local_hit", "target": {"health": "back"}}),
        ),
        (
            CorpusOperation::RawMouse {
                column: 3,
                row: 4,
                kind: CorpusMouseKind::ScrollUp,
            },
            json!({
                "operation": "raw_mouse",
                "column": 3,
                "row": 4,
                "kind": "scroll_up",
            }),
        ),
        (
            CorpusOperation::Paste("value".to_owned()),
            json!({"operation": "paste", "value": "value"}),
        ),
        (
            CorpusOperation::Focus { gained: false },
            json!({"operation": "focus", "gained": false}),
        ),
        (
            CorpusOperation::Resize {
                width: 80,
                height: 24,
            },
            json!({"operation": "resize", "width": 80, "height": 24}),
        ),
        (
            CorpusOperation::FinalLiveness,
            json!({"synthetic": "final_liveness"}),
        ),
    ];

    for (operation, expected) in cases {
        assert_eq!(corpus_operation_value(&operation).unwrap(), expected);
    }
}
