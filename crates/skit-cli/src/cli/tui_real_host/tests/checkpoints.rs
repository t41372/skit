//! Checkpoint capture and typed projection contracts.

use super::*;

#[test]
fn run_and_settings_reducer_checkpoints_replay_without_profile_paths() {
    let mut first = RealWalkerHost::spawn(profile()).unwrap();
    let mut second = RealWalkerHost::spawn(profile()).unwrap();
    let mut first_state = first.initial_state().unwrap();
    let mut second_state = second.initial_state().unwrap();

    for (request, selector) in [
        (HostRequest::Run, "Command"),
        (HostRequest::Settings, "Reference"),
    ] {
        let first_action = first
            .dispatch(Effect::Open {
                request,
                selector: Some(selector.to_owned()),
            })
            .unwrap();
        let second_action = second
            .dispatch(Effect::Open {
                request,
                selector: Some(selector.to_owned()),
            })
            .unwrap();
        let _ = first_state.update(first_action);
        let _ = second_state.update(second_action);

        let first_observation = first.observe(&first_state).unwrap();
        let second_observation = second.observe(&second_state).unwrap();
        assert_eq!(first_observation.state, second_observation.state);
        let encoded = serde_json::to_string(&first_observation.state).unwrap();
        assert!(!encoded.contains(&first._sandbox.path().display().to_string()));
        assert!(!encoded.contains(&second._sandbox.path().display().to_string()));
    }
}

#[test]
fn run_modal_reducer_checkpoints_replay_without_profile_paths() {
    let mut first = RealWalkerHost::spawn(profile()).unwrap();
    let mut second = RealWalkerHost::spawn(profile()).unwrap();
    let mut first_state = first.initial_state().unwrap();
    let mut second_state = second.initial_state().unwrap();

    for (host, state) in [(&first, &mut first_state), (&second, &mut second_state)] {
        let action = host
            .dispatch(Effect::Open {
                request: HostRequest::Run,
                selector: Some("Command".to_owned()),
            })
            .unwrap();
        assert_eq!(state.update(action), Effect::None);
        assert_eq!(state.update(Action::OpenRunTokenMenuFor(1)), Effect::None);
    }

    let first_tokens = first.observe(&first_state).unwrap();
    let second_tokens = second.observe(&second_state).unwrap();
    assert_eq!(first_tokens, second_tokens);
    assert_eq!(
        first_tokens
            .state
            .pointer("/modal/run_token_menu/options/1/fixed_directory/path")
            .and_then(Value::as_str),
        Some("<profile:fixture>/cwd")
    );
    let tokens = serde_json::to_string(&first_tokens).unwrap();
    assert!(!tokens.contains(&first._sandbox.path().display().to_string()));
    assert!(!tokens.contains(&second._sandbox.path().display().to_string()));

    assert_eq!(
        first_state.update(Action::OpenRunFilePicker(1)),
        Effect::None
    );
    assert_eq!(
        second_state.update(Action::OpenRunFilePicker(1)),
        Effect::None
    );
    let first_picker = first.observe(&first_state).unwrap();
    let second_picker = second.observe(&second_state).unwrap();
    assert_eq!(first_picker, second_picker);
    assert_eq!(
        first_picker
            .state
            .pointer("/modal/run_file_picker/context/workdir")
            .and_then(Value::as_str),
        Some("<profile:fixture>/cwd")
    );
    assert_eq!(
        first_picker
            .state
            .pointer("/modal/run_file_picker/context/invoke_cwd")
            .and_then(Value::as_str),
        Some("<profile:fixture>/cwd")
    );
    let picker = serde_json::to_string(&first_picker).unwrap();
    assert!(!picker.contains(&first._sandbox.path().display().to_string()));
    assert!(!picker.contains(&second._sandbox.path().display().to_string()));
}

#[test]
fn preferences_target_and_install_reducer_checkpoints_replay_without_profile_paths() {
    let mut first = RealWalkerHost::spawn(profile()).unwrap();
    let mut second = RealWalkerHost::spawn(profile()).unwrap();
    let mut first_state = first.initial_state().unwrap();
    let mut second_state = second.initial_state().unwrap();

    for host in [&first, &second] {
        fs::create_dir_all(host.roots().home.as_ref().unwrap().join(".codex/skills")).unwrap();
    }
    for (host, state) in [(&first, &mut first_state), (&second, &mut second_state)] {
        let open = host
            .dispatch(Effect::Open {
                request: HostRequest::Preferences,
                selector: None,
            })
            .unwrap();
        assert_eq!(state.update(open), Effect::None);
        let present = host
            .dispatch(Effect::Preferences(
                PreferencesEffect::DiscoverAgentSkillTargets,
            ))
            .unwrap();
        assert_eq!(state.update(present), Effect::None);
    }

    let first_targets = first.observe(&first_state).unwrap();
    let second_targets = second.observe(&second_state).unwrap();
    assert_eq!(first_targets, second_targets);
    assert_eq!(
        first_targets
            .state
            .pointer("/workflow/active/preferences/agent_skill_install/targets/0/base",)
            .and_then(Value::as_str),
        Some("<profile:fixture>/home/.codex")
    );
    let targets = serde_json::to_string(&first_targets).unwrap();
    assert!(!targets.contains(&first._sandbox.path().display().to_string()));
    assert!(!targets.contains(&second._sandbox.path().display().to_string()));

    let mut installed_observations = Vec::new();
    for (host, state) in [
        (&mut first, &mut first_state),
        (&mut second, &mut second_state),
    ] {
        let mut request = state.update(Action::Preferences(
            skit_ui::PreferencesAction::ConfirmAgentSkillTarget,
        ));
        assert!(matches!(
            request,
            Effect::Preferences(PreferencesEffect::InstallAgentSkill { .. })
        ));
        let mut response = host.dispatch(request.clone()).unwrap();
        let mut emitted = state.update(response.clone());
        let mut session = c2_session(Path::new("/unregistered/session"), None, Vec::new());
        installed_observations.push(
            host.capture_checkpoint_parts(
                state,
                &mut session,
                CheckpointCauseProjection::Host {
                    request: &mut request,
                    response: &mut response,
                    emitted: &mut emitted,
                },
            )
            .unwrap(),
        );
    }

    let first_installed = &installed_observations[0];
    let second_installed = &installed_observations[1];
    assert_eq!(first_installed, second_installed);
    assert_eq!(
        first_installed.state.get("status").and_then(Value::as_str),
        Some("Installed the skit Agent Skill: <profile:fixture>/home/.codex/skills/skit/SKILL.md")
    );
    let installed = serde_json::to_string(&first_installed).unwrap();
    assert!(!installed.contains(&first._sandbox.path().display().to_string()));
    assert!(!installed.contains(&second._sandbox.path().display().to_string()));
}

#[test]
fn missing_reference_target_replays_without_profile_paths() {
    let mut first = RealWalkerHost::spawn(profile()).unwrap();
    let mut second = RealWalkerHost::spawn(profile()).unwrap();
    fs::remove_file(first.external_root.join("outside.sh")).unwrap();
    fs::remove_file(second.external_root.join("outside.sh")).unwrap();

    let first_observation = first.observe(&first.initial_state().unwrap()).unwrap();
    let second_observation = second.observe(&second.initial_state().unwrap()).unwrap();
    assert_eq!(first_observation, second_observation);
    assert_eq!(
        first_observation
            .state
            .pointer("/details/reference/missing_target")
            .and_then(Value::as_str),
        Some("<profile:fixture>/external/outside.sh")
    );
    let encoded = serde_json::to_string(&first_observation).unwrap();
    assert!(!encoded.contains(&first._sandbox.path().display().to_string()));
    assert!(!encoded.contains(&second._sandbox.path().display().to_string()));
}

#[test]
fn modal_and_install_status_lookalikes_remain_user_text() {
    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    let raw = format!("{}-user", host._sandbox.path().display());
    let status = format!("Installed the skit Agent Skill: {raw}");
    let value = json!({
        "modal": {
            "run_token_menu": {
                "field": 0,
                "options": [{ "fixed_directory": { "path": raw } }],
            },
        },
        "status": status,
    });
    let path_map = &mut host.path_map;
    assert_eq!(
        path_map.normalize_library_json(value.clone()).unwrap(),
        value
    );
}

#[test]
fn real_host_add_screen_with_a_draft_round_trips_through_parse_library_state() {
    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    write_draft_at(&host, "skit-new-round-trip.py", b"print('draft')\n", 10);
    let mut state = host.initial_state().unwrap();
    let action = host
        .dispatch(Effect::Open {
            request: HostRequest::Add,
            selector: None,
        })
        .unwrap();
    assert!(matches!(&action, Action::Present(skit_ui::Screen::Add(_))));
    assert_eq!(state.update(action), Effect::None);

    let value = host.observe(&state).unwrap().state;
    let parsed = crate::cli::tui_real_walker::parse_library_state(&value).unwrap();

    assert_eq!(serde_json::to_value(parsed).unwrap(), value);
}

#[test]
fn real_host_add_screen_with_a_draft_round_trips_through_validate_reducer_action() {
    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    write_draft_at(&host, "skit-new-action.py", b"print('draft')\n", 10);
    let mut state = host.initial_state().unwrap();
    let previous = host.observe(&state).unwrap().state;
    let action = host
        .dispatch(Effect::Open {
            request: HostRequest::Add,
            selector: None,
        })
        .unwrap();
    assert!(matches!(&action, Action::Present(skit_ui::Screen::Add(_))));
    let canonical_action = host.canonical_action(&action).unwrap();
    let emitted = serde_json::to_value(state.update(action)).unwrap();
    let current = host.observe(&state).unwrap().state;

    crate::cli::tui_real_walker::validate_reducer_action(
        &previous,
        &current,
        &canonical_action,
        &emitted,
    )
    .unwrap();
}

#[test]
fn checkpoint_capture_projects_the_real_add_delete_chain_in_every_effect_position() {
    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    let draft_path = write_draft_at(
        &host,
        "skit-new-checkpoint-delete.py",
        b"print('draft')\n",
        10,
    );
    let mut state = host.initial_state().unwrap();
    let open = host
        .dispatch(Effect::Open {
            request: HostRequest::Add,
            selector: None,
        })
        .unwrap();
    assert_eq!(state.update(open), Effect::None);
    assert_eq!(
        state.update(Action::Add(AddAction::HighlightDraft(0))),
        Effect::None
    );
    assert_eq!(
        state.update(Action::Add(AddAction::DeleteSelectedDraft)),
        Effect::None
    );

    let previous = host.observe(&state).unwrap().state;
    let raw_action = Action::Add(AddAction::ConfirmDraftDelete(true));
    let raw_effect = state.update(raw_action.clone());
    assert!(
        matches!(&raw_effect, Effect::Add(effects) if matches!(effects.as_slice(), [skit_ui::AddEffect::DeleteDraft { .. }]))
    );
    let raw_draft =
        serde_json::to_value(&raw_effect).unwrap()["add"][0]["delete_draft"]["draft"].clone();
    let mut recorded_action = raw_action;
    let mut reducer_emitted = raw_effect.clone();
    let mut reducer_state = state.clone();
    let expected_session = c2_session(Path::new("/unregistered/session"), None, Vec::new());
    let mut reducer_session = expected_session.clone();
    let reducer = host
        .capture_checkpoint_parts(
            &mut reducer_state,
            &mut reducer_session,
            CheckpointCauseProjection::Reducer {
                action: &mut recorded_action,
                emitted: &mut reducer_emitted,
            },
        )
        .unwrap();
    assert_eq!(reducer_session, expected_session);
    crate::cli::tui_real_walker::validate_reducer_action(
        &previous,
        &reducer.state,
        &serde_json::to_value(&recorded_action).unwrap(),
        &serde_json::to_value(&reducer_emitted).unwrap(),
    )
    .unwrap();

    let mut direct_request = Effect::None;
    let mut direct_response = Action::ClearStatus;
    let mut direct_emitted = raw_effect.clone();
    let mut direct_state = state.clone();
    let mut direct_session = c2_session(Path::new("/unregistered/session"), None, Vec::new());
    host.capture_checkpoint_parts(
        &mut direct_state,
        &mut direct_session,
        CheckpointCauseProjection::Host {
            request: &mut direct_request,
            response: &mut direct_response,
            emitted: &mut direct_emitted,
        },
    )
    .unwrap();
    assert_eq!(direct_emitted, reducer_emitted);

    let raw_response = host.dispatch(raw_effect.clone()).unwrap();
    let raw_host_emitted = state.update(raw_response.clone());
    let mut host_request = raw_effect;
    let mut host_response = raw_response;
    let mut host_emitted = raw_host_emitted;
    let mut host_state = state.clone();
    let mut host_session = c2_session(Path::new("/unregistered/session"), None, Vec::new());
    let host_checkpoint = host
        .capture_checkpoint_parts(
            &mut host_state,
            &mut host_session,
            CheckpointCauseProjection::Host {
                request: &mut host_request,
                response: &mut host_response,
                emitted: &mut host_emitted,
            },
        )
        .unwrap();
    assert_eq!(reducer_emitted, host_request);
    for projected in [&reducer_emitted, &host_request, &direct_emitted] {
        let projected_draft =
            &serde_json::to_value(projected).unwrap()["add"][0]["delete_draft"]["draft"];
        assert_ne!(projected_draft["path"], raw_draft["path"]);
        assert_ne!(projected_draft["modified"], raw_draft["modified"]);
        assert_ne!(projected_draft["identity"], raw_draft["identity"]);
        assert!(
            !projected_draft
                .to_string()
                .contains(&draft_path.display().to_string())
        );
    }
    crate::cli::tui_real_walker::validate_reducer_action(
        &reducer.state,
        &host_checkpoint.state,
        &serde_json::to_value(&host_response).unwrap(),
        &serde_json::to_value(&host_emitted).unwrap(),
    )
    .unwrap();
}

#[test]
fn checkpoint_projection_failure_preserves_transcript_until_one_successful_drain() {
    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    host.clear_transcript();
    host.adapters.push(PortEvent::Output {
        stream: "plain".to_owned(),
        text: "pending checkpoint output".to_owned(),
    });
    let draft = skit_ui::DraftSummary {
        path: host.roots().data.join("unscanned-checkpoint-draft.py"),
        modified: 100,
        identity: None,
        permissions: SourcePermissions::default(),
        content_hash: None,
    };
    let mut workflow = AddWorkflowState::new(vec![draft]);
    assert!(workflow.reduce(AddAction::HighlightDraft(0)).is_empty());
    assert!(workflow.reduce(AddAction::DeleteSelectedDraft).is_empty());
    let mut emitted = Effect::Add(workflow.reduce(AddAction::ConfirmDraftDelete(true)));
    let mut action = Action::Add(AddAction::ConfirmDraftDelete(true));
    let mut state = host.initial_state().unwrap();
    let mut session = json!({"keep": "byte-exact"});

    assert_eq!(
        host.capture_checkpoint_parts(
            &mut state,
            &mut session,
            CheckpointCauseProjection::Reducer {
                action: &mut action,
                emitted: &mut emitted,
            },
        )
        .unwrap_err(),
        "the sorted scan did not assign a rank to this draft modified value: 100"
    );
    assert_eq!(session, json!({"keep": "byte-exact"}));

    let first = host.observe(&host.initial_state().unwrap()).unwrap();
    assert_eq!(
        events_with_keys(&first.transcript, &["output"]),
        vec![json!({
            "output": "plain",
            "text": "pending checkpoint output",
        })]
    );
    let second = host.observe(&host.initial_state().unwrap()).unwrap();
    assert!(events_with_keys(&second.transcript, &["output"]).is_empty());
}

#[test]
fn checkpoint_registers_pending_event_artifacts_before_projecting_the_cause() {
    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    host.clear_transcript();
    let allocated = host
        .adapters
        .temporary_file(
            TemporaryFilePurpose::InjectedSource,
            TempLocation::System,
            ".py",
        )
        .unwrap();
    let path = allocated.path().to_path_buf();
    drop(allocated);
    assert!(!host.path_map.transient_paths.contains_key(&path));
    let draft = skit_ui::DraftSummary {
        path: host.roots().data.join("unscanned-order-draft.py"),
        modified: 100,
        identity: None,
        permissions: SourcePermissions::default(),
        content_hash: None,
    };
    let mut workflow = AddWorkflowState::new(vec![draft]);
    assert!(workflow.reduce(AddAction::HighlightDraft(0)).is_empty());
    assert!(workflow.reduce(AddAction::DeleteSelectedDraft).is_empty());
    let mut action = Action::Add(AddAction::ConfirmDraftDelete(true));
    let mut emitted = Effect::Add(workflow.reduce(AddAction::ConfirmDraftDelete(true)));
    let mut state = host.initial_state().unwrap();
    let mut session = Value::Null;

    assert_eq!(
        host.capture_checkpoint_parts(
            &mut state,
            &mut session,
            CheckpointCauseProjection::Reducer {
                action: &mut action,
                emitted: &mut emitted,
            },
        )
        .unwrap_err(),
        "the sorted scan did not assign a rank to this draft modified value: 100"
    );
    assert_eq!(
        host.path_map.transient_paths.get(&path).map(String::as_str),
        Some("<profile:fixture>/system-temp/<injected-source:0>.py")
    );
    let events = host.adapters.events.borrow();
    assert!(matches!(events.first(), Some(PortEvent::Allocation { .. })));
    assert!(
        events
            .iter()
            .any(|event| matches!(event, PortEvent::Clock(_)))
    );
}

#[test]
fn strict_projection_refuses_noncanonical_outer_nested_and_screen_tags() {
    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    for malformed in [
        json!({}),
        json!({"quit": null}),
        json!({"quit": null, "reload": null}),
        json!({"future_action": null}),
        json!({"set_status": "status", "extra": true}),
        json!({"add": "future_add_action"}),
        json!({"settings": {"action": "future_settings_action"}}),
    ] {
        assert!(host.path_map.normalize_action_json(malformed).is_err());
    }
    assert_eq!(
        host.path_map
            .normalize_action_json(json!({
                "toggle_detail": {"currently_visible": true, "extra": false},
            }))
            .unwrap_err(),
        "a serialized Action is not canonical"
    );
    assert_eq!(
        host.path_map
            .normalize_library_json(json!({
                "workflow": {"active": {"future_screen": null}, "history": []},
            }))
            .unwrap_err(),
        "unknown Screen tag: future_screen"
    );
    let raw_path = host.roots().data.join("draft-lookalike.py");
    let raw_path = raw_path.display().to_string();
    let normalized = host
        .path_map
        .normalize_library_json(json!({
            "workflow": {
                "active": {"add": {
                    "notice": {"draft_deleted": raw_path.clone()},
                    "user_text": raw_path,
                }},
                "history": [],
            },
        }))
        .unwrap();
    assert_eq!(
        normalized["workflow"]["active"]["add"]["notice"]["draft_deleted"],
        "<profile:fixture>/data/draft-lookalike.py"
    );
    assert_eq!(
        normalized["workflow"]["active"]["add"]["user_text"],
        raw_path
    );

    for (mut screen, expected) in [
        (json!({}), "a serialized external tag object is empty"),
        (
            json!({"library": null, "run": null}),
            "a serialized external tag object has more than one member",
        ),
        (
            json!({"library": null}),
            "serialized tag library must not have a payload",
        ),
        (json!("run"), "serialized tag run must have one payload"),
        (json!(7), "a serialized external tag has a malformed shape"),
        (
            json!({"future_screen": null}),
            "unknown Screen tag: future_screen",
        ),
    ] {
        assert_eq!(
            host.path_map
                .project_serialized_screen(&mut screen)
                .unwrap_err(),
            expected
        );
    }
    assert_eq!(
        require_internal_tag(&json!({"action": "future"}), "action", "focus_next",).unwrap_err(),
        "expected serialized action tag focus_next, but found future"
    );
    assert_eq!(
        require_internal_tag(&json!({"action": 7}), "action", "focus_next").unwrap_err(),
        "serialized action tag focus_next is not text"
    );
    assert_eq!(
        require_internal_tag(&json!({}), "action", "focus_next").unwrap_err(),
        "serialized action tag focus_next is missing"
    );
    assert_eq!(
        require_external_unit(&json!("cancel"), "continue").unwrap_err(),
        "expected serialized tag continue, but found cancel"
    );
    assert_eq!(
        require_external_unit(&json!({"continue": null, "cancel": null}), "continue",).unwrap_err(),
        "serialized tag continue must have exactly one object member"
    );
    assert_eq!(
        require_external_unit(&json!(7), "continue").unwrap_err(),
        "serialized tag continue has a malformed shape"
    );
    let mut nested_multi = json!({"continue": null, "cancel": null});
    assert_eq!(
        external_payload_mut(&mut nested_multi, "continue").unwrap_err(),
        "serialized tag continue must have exactly one object member"
    );
    assert_eq!(
        host.adapters.drain_event_prefix(usize::MAX).unwrap_err(),
        "the walker port transcript changed during checkpoint capture"
    );
    let mut unrelated = json!("library");
    host.path_map.normalize_run_screen(&mut unrelated).unwrap();
    host.path_map.normalize_settings_screen(&mut unrelated);
    host.path_map.normalize_add_screen(&mut unrelated).unwrap();
    let raw_note_path = host.roots().data.join("note-source.py");
    let mut settings_note = json!({"settings": {"sections": [{"items": [{
        "item": "note",
        "text": "Linked to the original: {}",
        "arguments": [raw_note_path],
    }]}]}});
    host.path_map.normalize_settings_screen(&mut settings_note);
    assert_eq!(
        settings_note["settings"]["sections"][0]["items"][0]["arguments"][0],
        "<profile:fixture>/data/note-source.py"
    );
    let mut note_without_arguments = json!({"settings": {"sections": [{"items": [{
        "item": "note",
        "text": "Linked to the original: {}",
    }]}]}});
    host.path_map
        .normalize_settings_screen(&mut note_without_arguments);

    let nested_failures = [
        (Action::Add(AddAction::Continue), json!({"add": "cancel"})),
        (
            Action::Health(skit_ui::HealthAction::Previous),
            json!({"health": "next"}),
        ),
        (
            Action::Runners(skit_ui::RunnerManagerAction::Previous),
            json!({"runners": "next"}),
        ),
        (
            Action::RunnerEditor(skit_ui::RunnerEditorAction::FocusNext),
            json!({"runner_editor": "cancel"}),
        ),
        (
            Action::Preferences(skit_ui::PreferencesAction::Previous),
            json!({"preferences": "next"}),
        ),
        (
            Action::Settings(skit_ui::SettingsAction::FocusNext),
            json!({"settings": {"action": "close"}}),
        ),
    ];
    for (action, mut raw) in nested_failures {
        assert!(
            host.path_map
                .project_typed_action_value(&action, &mut raw)
                .is_err()
        );
    }
    let mut wrong_screen = json!("run");
    assert_eq!(
        host.path_map
            .project_typed_screen(&skit_ui::Screen::Library, &mut wrong_screen)
            .unwrap_err(),
        "expected serialized Screen tag library, but found run"
    );
    let mut wrong_preferences = json!({"preferences": "close"});
    assert!(
        host.path_map
            .project_typed_effect_value(
                &Effect::Preferences(PreferencesEffect::None),
                &mut wrong_preferences,
            )
            .is_err()
    );
    let mut wrong_add_length = json!({"add": ["cancel"]});
    assert_eq!(
        host.path_map
            .project_typed_effect_value(&Effect::Add(Vec::new()), &mut wrong_add_length)
            .unwrap_err(),
        "an Add Effect payload changed its length"
    );
}

#[test]
fn serialized_screen_dispatcher_covers_the_typed_nine_variant_vocabulary() {
    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    let present = |host: &RealWalkerHost, request, selector: Option<&str>| {
        let action = host
            .dispatch(Effect::Open {
                request,
                selector: selector.map(str::to_owned),
            })
            .unwrap();
        let value = serde_json::to_value(action).unwrap();
        serde_json::from_value::<skit_ui::Screen>(value["present"].clone()).unwrap()
    };
    let fixtures = [
        skit_ui::Screen::Library,
        present(&host, HostRequest::Run, Some("Command")),
        present(&host, HostRequest::Preferences, None),
        present(&host, HostRequest::Add, None),
        present(&host, HostRequest::Health, None),
        present(&host, HostRequest::Runners, None),
        present(&host, HostRequest::Settings, Some("Reference")),
        present(&host, HostRequest::Rename, Some("Reference")),
        skit_ui::Screen::Report(skit_ui::ReportView {
            title: "Fixture report".to_owned(),
            items: Vec::new(),
        }),
    ];
    let expected = [
        "library",
        "run",
        "preferences",
        "add",
        "health",
        "runners",
        "settings",
        "form",
        "report",
    ];
    for (screen, expected) in fixtures.iter().zip(expected) {
        assert_eq!(screen_tag(screen), expected);
        let mut fixture = serde_json::to_value(screen).unwrap();
        assert_eq!(external_tag(&fixture).unwrap(), expected);
        host.path_map
            .project_typed_screen(screen, &mut fixture)
            .unwrap();
        let round_trip: skit_ui::Screen = deserialize_canonical(fixture, "Screen fixture").unwrap();
        assert_eq!(screen_tag(&round_trip), expected);
    }
}

#[test]
fn typed_action_projection_executes_all_seventy_five_outer_variants() {
    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    let reload = serde_json::to_value(host.dispatch(Effect::Reload).unwrap()).unwrap();
    let surface = reload["replace_surface"]["surface"].clone();
    let scan = surface["scan"].clone();
    let run = serde_json::to_value(
        host.dispatch(Effect::Open {
            request: HostRequest::Run,
            selector: Some("Command".to_owned()),
        })
        .unwrap(),
    )
    .unwrap();
    let form = run["present"]["run"].clone();
    let preferences = serde_json::to_value(
        host.dispatch(Effect::Open {
            request: HostRequest::Preferences,
            selector: None,
        })
        .unwrap(),
    )
    .unwrap()["present"]["preferences"]
        .clone();
    let fixtures = vec![
        json!("previous"),
        json!("next"),
        json!("page_previous"),
        json!("page_next"),
        json!("home"),
        json!("end"),
        json!({"select_visible": 0}),
        json!("begin_search"),
        json!("finish_search"),
        json!({"input": "x"}),
        json!("backspace"),
        json!({"paste": "paste"}),
        json!({"set_search_query": "query"}),
        json!("clear_search"),
        json!({"replace": {"scan": scan, "rerunnable": []}}),
        reload,
        json!({"replace_rerunnable": []}),
        json!("reload"),
        json!("rerun"),
        json!("open_run"),
        json!("open_add"),
        json!("open_settings"),
        json!("open_preferences"),
        json!("open_health"),
        json!("open_runners"),
        json!("open_presets"),
        json!("open_rename"),
        json!("edit"),
        json!("ask_remove"),
        json!("open_help"),
        json!({"toggle_detail": {"currently_visible": true}}),
        json!("focus_next"),
        json!("focus_previous"),
        json!({"focus_field": 0}),
        json!({"set_field_value": {"field": 0, "value": "value"}}),
        json!({"set_run_glob_count": {"field": 0, "value": "*", "count": 1}}),
        json!("open_run_preset_save"),
        json!("open_run_token_menu"),
        json!({"open_run_token_menu_for": 0}),
        json!({"open_run_environment_picker": 0}),
        json!({"set_run_environment_query": "PATH"}),
        json!({"open_run_file_picker": 0}),
        json!("open_focused_run_file_picker"),
        json!({"set_run_field_value_and_close_modal": {"field": 0, "value": "value"}}),
        json!({"set_run_picked_path_and_close_modal": {"field": 0, "path": "path"}}),
        json!("reset_focused_run_field"),
        json!("open_run_runner_editor"),
        json!({"add": "continue"}),
        json!("open_add_runner_editor"),
        json!({"health": "previous"}),
        json!({"runners": "previous"}),
        json!({"runner_editor": "focus_next"}),
        json!({"runner_editor_saved": {"owner": "add", "name": "runner", "message": "saved"}}),
        json!({"runner_editor_save_failed": {"owner": "add", "message": "failed"}}),
        json!({"runner_manager_closed": {"preferences": preferences}}),
        json!({"preferences": "previous"}),
        json!({"settings": {"action": "focus_next"}}),
        json!({"preferences_saved": {"locale": "en", "message": "saved"}}),
        json!("keep_editing"),
        json!("discard_changes"),
        json!({"set_modal_input": "value"}),
        json!({"run_preset_saved": {"name": "preset", "presets": {}, "message": "saved"}}),
        json!({"toggle_field": 0}),
        json!({"select_field_option": {"field": 0, "value": "choice"}}),
        json!({"reset_run_field": 0}),
        json!("submit"),
        json!("back"),
        json!({"present": "library"}),
        json!({"prompt_runner_required": {"form": form, "cancel_status": "cancelled"}}),
        json!({"add_completed": {"surface": surface, "rerunnable": [], "slug": "created", "message": "added"}}),
        json!("add_cancelled"),
        json!({"complete": {"surface": null, "rerunnable": null, "message": "complete"}}),
        json!({"set_status": "status"}),
        json!("clear_status"),
        json!("quit"),
    ];
    assert_eq!(fixtures.len(), 75);
    for fixture in fixtures {
        let projected = host.path_map.normalize_action_json(fixture).unwrap();
        let _: Action = deserialize_canonical(projected, "Action fixture").unwrap();
    }
}

#[test]
fn typed_nested_action_projection_executes_every_recorded_variant() {
    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    let mut add = vec![
        json!({"set_source_path": "source"}),
        json!({"picked_source_path": host.external_root.join("outside.sh").display().to_string()}),
        json!({"set_command_template": "echo {value}"}),
        json!({"set_command_name": "command"}),
        json!({"set_command_description": "description"}),
        json!({"select_draft": 0}),
        json!({"highlight_draft": 0}),
        json!("continue"),
        json!({"source_inspected": {"request": 0, "result": {"Err": "failed"}}}),
        json!({"pick_kind": null}),
        json!({"new_draft": "script"}),
        json!({"draft_edited": {"request": 0, "result": {"Err": "failed"}}}),
        json!("delete_selected_draft"),
        json!({"confirm_draft_delete": true}),
        json!({"draft_deleted": {"request": 0, "result": {"Err": "failed"}}}),
        json!({"set_review_name": "name"}),
        json!({"set_review_description": "description"}),
        json!({"set_review_storage": "copy"}),
        json!({"set_review_dependencies": "dep"}),
        json!({"set_review_python": ">=3.12"}),
        json!({"set_review_candidate": {"name": "candidate", "selected": true}}),
        json!({"set_prompt_interpolation": true}),
        json!({"set_prompt_candidate": {"name": "value", "selected": true}}),
        json!({"set_prompt_candidates": ["value"]}),
        json!({"set_prompt_runner": {"name": "runner", "picked": true}}),
        json!({"prompt_runner_added": "runner"}),
        json!("edit_source"),
        json!({"source_edited": {"request": 0, "result": {"Err": "failed"}}}),
        json!("save"),
        json!({"commit_finished": {"request": 0, "result": {"Err": "failed"}}}),
        json!("cancel"),
    ];
    assert_eq!(add.len(), 31);
    let health_rebuilt = serde_json::to_value(host.dispatch(Effect::HealthRebuild).unwrap())
        .unwrap()["health"]
        .clone();
    let health = vec![
        json!("previous"),
        json!("next"),
        json!({"page_previous": 2}),
        json!({"page_next": 2}),
        json!("home"),
        json!("end"),
        json!({"select_issue": 0}),
        json!("jump"),
        json!({"activate_issue": 0}),
        json!("rebuild"),
        health_rebuilt,
        json!("back"),
    ];
    assert_eq!(health.len(), 12);
    let preferences = vec![
        json!({"set_language": "en"}),
        json!({"set_editor": "editor"}),
        json!({"set_interactive_form": "tui"}),
        json!({"set_after_run": "stay"}),
        json!({"set_javascript": "automatic"}),
        json!({"set_bash_path": "bash"}),
        json!({"set_mirror_master": true}),
        json!({"choose_mirror": {"field": "pypi_mirror", "choice": "off"}}),
        json!({"set_mirror_url": {"field": "pypi_mirror", "value": "https://example.invalid"}}),
        json!({"focus": "language"}),
        json!("previous"),
        json!("next"),
        json!("save"),
        json!("close"),
        json!("manage_agents"),
        json!("install_agent_skill"),
        json!({"present_agent_skill_targets": [{"name": "codex", "scope": "user", "base": "/user/base"}]}),
        json!({"select_agent_skill_target": 0}),
        json!({"activate_agent_skill_target": 0}),
        json!("confirm_agent_skill_target"),
        json!("close_agent_skill_targets"),
        json!({"agent_skill_installed": {"message": "installed"}}),
        json!({"validation_failed": {"bash_path_missing": {"path": "missing"}}}),
    ];
    assert_eq!(preferences.len(), 23);
    let runner_editor = vec![
        json!({"set_name": "name"}),
        json!({"set_command": "runner --flag"}),
        json!({"focus": "name"}),
        json!("focus_next"),
        json!("focus_previous"),
        json!("submit"),
        json!("cancel"),
        json!({"mutation_failed": "failed"}),
    ];
    assert_eq!(runner_editor.len(), 8);
    let runners = vec![
        json!("previous"),
        json!("next"),
        json!({"page_previous": 2}),
        json!({"page_next": 2}),
        json!("home"),
        json!("end"),
        json!({"select": 0}),
        json!("activate_selected"),
        json!({"activate_row": 0}),
        json!("new"),
        json!("edit_selected"),
        json!("remove_selected"),
        json!("close_actions"),
        json!({"editor": "focus_next"}),
        json!("cancel_editor"),
        json!("confirm_remove"),
        json!("cancel_remove"),
        json!({"mutation_succeeded": {"rows": [], "selected_name": null, "message": "saved"}}),
        json!({"mutation_failed": "failed"}),
        json!("back"),
    ];
    assert_eq!(runners.len(), 20);
    let settings = vec![
        json!({"action": "set_field", "key": "name", "value": {"state": "inherit"}}),
        json!({"action": "focus", "key": "name"}),
        json!({"action": "focus_next"}),
        json!({"action": "focus_previous"}),
        json!({"action": "resync"}),
        json!({"action": "set_prompt_candidates", "selected": ["value"]}),
        json!({"action": "save"}),
        json!({"action": "close"}),
        json!({"action": "new_runner"}),
    ];
    assert_eq!(settings.len(), 9);

    for nested in add.drain(..) {
        host.path_map
            .normalize_action_json(json!({"add": nested}))
            .unwrap();
    }
    for nested in health {
        host.path_map
            .normalize_action_json(json!({"health": nested}))
            .unwrap();
    }
    for nested in preferences {
        host.path_map
            .normalize_action_json(json!({"preferences": nested}))
            .unwrap();
    }
    for nested in runner_editor {
        host.path_map
            .normalize_action_json(json!({"runner_editor": nested}))
            .unwrap();
    }
    for nested in runners {
        host.path_map
            .normalize_action_json(json!({"runners": nested}))
            .unwrap();
    }
    for nested in settings {
        host.path_map
            .normalize_action_json(json!({"settings": nested}))
            .unwrap();
    }
    let source = json!({
        "path": "outside-source.py",
        "source_record": "outside-source.py",
        "bytes": [],
        "permissions": serde_json::to_value(SourcePermissions::default()).unwrap(),
        "executable": null,
        "is_regular": true,
        "is_directory": false,
        "is_draft": false,
        "identity": null,
    });
    let draft = json!({
        "path": "outside-draft.py",
        "modified": 1,
        "identity": null,
        "permissions": serde_json::to_value(SourcePermissions::default()).unwrap(),
        "content_hash": null,
    });
    for nested in [
        json!({"source_inspected": {"request": 0, "result": {"Ok": source.clone()}}}),
        json!({"draft_edited": {"request": 0, "result": {"Ok": source.clone()}}}),
        json!({"draft_deleted": {"request": 0, "result": {"Ok": {"changed": draft}}}}),
        json!({"source_edited": {"request": 0, "result": {"Ok": source.clone()}}}),
    ] {
        host.path_map
            .normalize_action_json(json!({"add": nested}))
            .unwrap();
    }
}

#[test]
fn typed_effect_projection_executes_all_outer_and_recorded_nested_variants() {
    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    let outer = vec![
        json!("none"),
        json!("reload"),
        json!("quit"),
        json!({"rerun": {"selector": "entry"}}),
        json!({"open": {"request": "run", "selector": "entry"}}),
        json!({"submit": {"purpose": "run", "selector": "entry", "values": {}}}),
        json!({"count_run_glob": {"selector": "entry", "field": 0, "value": "*", "request": {"cwd": "/cwd", "pieces": ["*"]}}}),
        json!({"save_run_preset": {"selector": "entry", "name": "preset", "values": {}, "secret_names": []}}),
        json!({"add": ["cancel"]}),
        json!("health_rebuild"),
        json!({"save_runner": {"request": {"name": "runner", "argv": ["runner"], "target": "new"}, "owner": "manager"}}),
        json!({"remove_runner": {"named": {"name": "runner", "expected": [], "expected_pinned_count": 0}}}),
        json!("refresh_preferences_after_runners"),
        json!({"preferences": "none"}),
        json!({"edit": {"selector": "entry"}}),
        json!({"remove": {"selector": "entry"}}),
    ];
    assert_eq!(outer.len(), 16);
    for raw in outer {
        let mut effect: Effect = deserialize_canonical(raw, "Effect fixture").unwrap();
        host.path_map.project_typed_effect(&mut effect).unwrap();
        let projected = serde_json::to_value(&effect).unwrap();
        let _: Effect = deserialize_canonical(projected, "Effect fixture").unwrap();
    }

    for nested in [
        json!("none"),
        json!({"save": {"settings": {}}}),
        json!("close"),
        json!("confirm_discard"),
        json!("manage_agents"),
        json!("discover_agent_skill_targets"),
        json!({"install_agent_skill": {"skills_dir": "/skills"}}),
    ] {
        let raw = json!({"preferences": nested});
        let mut effect: Effect = deserialize_canonical(raw, "Effect fixture").unwrap();
        host.path_map.project_typed_effect(&mut effect).unwrap();
    }

    let draft_path = write_draft_at(
        &host,
        "skit-new-effect-fixture.py",
        b"print('effect')\n",
        10,
    );
    let state = host.initial_state().unwrap();
    host.observe(&state).unwrap();
    let draft = sorted_tui_drafts(&host.roots().data)
        .into_iter()
        .next()
        .unwrap();
    let mut workflow = AddWorkflowState::new(vec![draft]);
    assert!(workflow.reduce(AddAction::HighlightDraft(0)).is_empty());
    assert!(workflow.reduce(AddAction::DeleteSelectedDraft).is_empty());
    let delete = workflow
        .reduce(AddAction::ConfirmDraftDelete(true))
        .pop()
        .unwrap();
    let source = SourceSnapshot {
        path: PathBuf::from("source.py"),
        source_record: "source.py".to_owned(),
        bytes: b"print('source')\n".to_vec(),
        permissions: SourcePermissions::default(),
        executable: Some(false),
        is_regular: true,
        is_directory: false,
        is_draft: false,
        identity: None,
    };
    let entry = serde_json::to_value(profile().entries[0].clone()).unwrap();
    let source_json = serde_json::to_value(&source).unwrap();
    let mut nested = vec![
        deserialize_canonical::<skit_ui::AddEffect>(
            json!({"inspect_source": {"request": 0, "path": "source.py"}}),
            "AddEffect fixture",
        )
        .unwrap(),
        deserialize_canonical::<skit_ui::AddEffect>(
            json!({"author_draft": {"request": 0, "kind": "script"}}),
            "AddEffect fixture",
        )
        .unwrap(),
        delete,
        deserialize_canonical::<skit_ui::AddEffect>(
            json!({"edit_source": {"request": 0, "path": "source.py"}}),
            "AddEffect fixture",
        )
        .unwrap(),
        deserialize_canonical::<skit_ui::AddEffect>(
            json!({"commit": {"request": 0, "entry": entry, "source": null}}),
            "AddEffect fixture",
        )
        .unwrap(),
        skit_ui::AddEffect::ConsumeDraft(source),
        skit_ui::AddEffect::DraftKept(PathBuf::from("draft.py")),
        skit_ui::AddEffect::RememberRunner("runner".to_owned()),
        skit_ui::AddEffect::Complete("created".to_owned()),
        skit_ui::AddEffect::Cancel,
    ];
    assert_eq!(nested.len(), 10);
    let mut effect = Effect::Add(std::mem::take(&mut nested));
    host.path_map.project_typed_effect(&mut effect).unwrap();
    let projected = serde_json::to_value(&effect).unwrap();
    assert_ne!(
        projected["add"][2]["delete_draft"]["draft"]["path"],
        draft_path.display().to_string()
    );
    assert_eq!(
        serde_json::to_value(&effect).unwrap()["add"][5]["consume_draft"],
        source_json
    );
}
