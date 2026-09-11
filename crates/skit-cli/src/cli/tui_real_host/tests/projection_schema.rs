//! Session schema, action surface, and review name projection contracts.

use super::*;

#[test]
fn c2_session_schema_node_and_nested_picker_corruption_refusals() {
    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    let root = Path::new("/unregistered/session");
    let valid = c2_session(root, None, Vec::new());

    for mut invalid in [
        Value::Null,
        json!({}),
        json!({"schema_version": 2}),
        json!({"schema_version": "1"}),
    ] {
        assert!(host.path_map.project_session_value(&mut invalid).is_err());
    }
    for owner in [
        "preferences",
        "add",
        "path_suggestions",
        "run_modal",
        "add_overlay",
        "file_picker_source",
    ] {
        let mut invalid = valid.clone();
        invalid.as_object_mut().unwrap().remove(owner);
        assert!(host.path_map.project_session_value(&mut invalid).is_err());
    }
    for (pointer, replacement) in [
        ("/preferences/kind", Value::Null),
        ("/preferences/kind", json!("wrong")),
        ("/preferences/fields", Value::Null),
    ] {
        let mut invalid = valid.clone();
        *invalid.pointer_mut(pointer).unwrap() = replacement;
        assert!(host.path_map.project_session_value(&mut invalid).is_err());
    }
    let mut missing_kind = valid.clone();
    missing_kind["preferences"]
        .as_object_mut()
        .unwrap()
        .remove("kind");
    assert!(
        host.path_map
            .project_session_value(&mut missing_kind)
            .is_err()
    );
    let mut missing_fields = valid.clone();
    missing_fields["preferences"]
        .as_object_mut()
        .unwrap()
        .remove("fields");
    assert!(
        host.path_map
            .project_session_value(&mut missing_fields)
            .is_err()
    );
    let mut extra = valid.clone();
    extra["preferences"]["extra"] = json!(true);
    assert!(host.path_map.project_session_value(&mut extra).is_err());

    let mut bad_overlay = valid.clone();
    bad_overlay["add_overlay"]["kind"] = json!("unknown_overlay");
    assert!(
        host.path_map
            .project_session_value(&mut bad_overlay)
            .is_err()
    );
    let mut scalar_overlay = valid.clone();
    scalar_overlay["add_overlay"] = json!(7);
    assert!(
        host.path_map
            .project_session_value(&mut scalar_overlay)
            .is_err()
    );
    let mut missing_overlay_session = valid.clone();
    missing_overlay_session["add_overlay"]["fields"] = json!({});
    assert!(
        host.path_map
            .project_session_value(&mut missing_overlay_session)
            .is_err()
    );
    let mut wrong_picker = valid.clone();
    wrong_picker["run_modal"]["fields"]["file"]["kind"] = json!("ordinary");
    assert!(
        host.path_map
            .project_session_value(&mut wrong_picker)
            .is_err()
    );
    let mut missing_entries = valid.clone();
    missing_entries["run_modal"]["fields"]["file"]["fields"]["explorer"] = json!({});
    assert!(
        host.path_map
            .project_session_value(&mut missing_entries)
            .is_err()
    );
    let mut missing_memory = valid.clone();
    missing_memory["run_modal"]["fields"]["file"]["fields"]
        .as_object_mut()
        .unwrap()
        .remove("memory_source");
    assert!(
        host.path_map
            .project_session_value(&mut missing_memory)
            .is_err()
    );
    let mut wrong_memory = valid.clone();
    wrong_memory["file_picker_source"]["kind"] = json!("file_picker");
    assert!(
        host.path_map
            .project_session_value(&mut wrong_memory)
            .is_err()
    );

    let mut null_memory = valid.clone();
    null_memory["run_modal"]["fields"]["file"]["fields"]["memory_source"] = Value::Null;
    assert!(
        host.path_map
            .project_session_value(&mut null_memory)
            .is_err()
    );
    let mut null_owners = valid.clone();
    null_owners["add"]["fields"]["picker_root"] = Value::Null;
    null_owners["run_modal"]["fields"]["file"] = Value::Null;
    null_owners["run_modal"]["fields"]["file_picker_source"] = Value::Null;
    null_owners["add_overlay"] = Value::Null;
    null_owners["file_picker_source"] = Value::Null;
    host.path_map
        .project_session_value(&mut null_owners)
        .unwrap();

    let mut prompt_overlay = valid.clone();
    prompt_overlay["add_overlay"] = json!({
        "kind": "add_prompt_overlay",
        "fields": {
            "session": {"kind": "prompt_candidate_picker", "fields": {}},
            "geometry": null,
        },
    });
    host.path_map
        .project_session_value(&mut prompt_overlay)
        .unwrap();

    for event in [
        json!("open_prompt_candidates"),
        json!("open_runner_editor"),
        json!("changed"),
        json!({"action": "continue"}),
    ] {
        let mut session = valid.clone();
        session["add"]["fields"]["advertised"][0]["event"] = event;
        host.path_map.project_session_value(&mut session).unwrap();
    }
    let mut absolute = valid.clone();
    absolute["add"]["fields"]["advertised"][0]["event"]["open_path_picker"]["output_policy"] =
        json!("absolute");
    absolute["run_modal"]["fields"]["file"]["fields"]["contract"]["output_policy"] =
        json!("absolute");
    host.path_map.project_session_value(&mut absolute).unwrap();
    for signature in [
        Value::Null,
        json!("preset"),
        json!("other"),
        json!({"environment": {"field": 0, "names": []}}),
    ] {
        let mut session = valid.clone();
        session["run_modal"]["fields"]["signature"] = signature;
        host.path_map.project_session_value(&mut session).unwrap();
    }
    let mut nullable = valid.clone();
    nullable["path_suggestions"]["fields"]["expected"]["request"]["context"]["tokens"]["home"] =
        Value::Null;
    nullable["run_modal"]["fields"]["file"]["fields"]["io_error"] = Value::Null;
    let symlink = nullable["run_modal"]["fields"]["file"]["fields"]["explorer"]["entries"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|entry| entry.pointer("/entry_type/symlink").is_some())
        .unwrap();
    symlink["entry_type"]["symlink"]["target"] = Value::Null;
    host.path_map.project_session_value(&mut nullable).unwrap();

    for (pointer, replacement) in [
        ("/preferences/fields/agent_signature", json!(7)),
        ("/preferences/fields/agent_signature/0/base", Value::Null),
        ("/add/fields/picker_root", json!(7)),
        ("/add/fields/advertised", json!({})),
        ("/add/fields/advertised/0/event", json!(7)),
        ("/add/fields/advertised/0/event", json!("unknown")),
        ("/add/fields/advertised/0/event", json!({"unknown": {}})),
        (
            "/add/fields/advertised/0/event",
            json!({"action": "continue", "open_path_picker": {}}),
        ),
        (
            "/add/fields/advertised/0/event/open_path_picker/start_dir",
            Value::Null,
        ),
        (
            "/add/fields/advertised/0/event/open_path_picker/output_policy",
            json!("unknown"),
        ),
        (
            "/add/fields/advertised/0/event/open_path_picker/output_policy",
            json!({"unknown": "/path"}),
        ),
        ("/path_suggestions/fields/expected", json!([])),
        (
            "/path_suggestions/fields/expected/request/context/workdir",
            Value::Null,
        ),
        (
            "/path_suggestions/fields/expected/request/context/tokens",
            json!([]),
        ),
        (
            "/path_suggestions/fields/expected/request/context/tokens/env",
            json!([]),
        ),
        ("/path_suggestions/fields/visible/suggestion", Value::Null),
        ("/run_modal/fields/signature", json!(7)),
        ("/run_modal/fields/signature", json!("unknown")),
        ("/run_modal/fields/signature", json!({"unknown": {}})),
        (
            "/run_modal/fields/signature",
            json!({"file": {}, "token": {}}),
        ),
        (
            "/run_modal/fields/signature/file/context/workdir",
            Value::Null,
        ),
        (
            "/run_modal/fields/file/fields/contract/start_dir",
            Value::Null,
        ),
        (
            "/run_modal/fields/file/fields/contract/output_policy",
            json!([]),
        ),
        (
            "/run_modal/fields/file/fields/contract/output_policy",
            json!({"relative_to": "/path", "extra": true}),
        ),
        (
            "/run_modal/fields/file/fields/explorer/current_dir",
            Value::Null,
        ),
        ("/run_modal/fields/file/fields/explorer/entries", json!({})),
        (
            "/run_modal/fields/file/fields/explorer/entries/0/path",
            Value::Null,
        ),
        (
            "/run_modal/fields/file/fields/explorer/selected_files",
            json!({}),
        ),
        ("/run_modal/fields/file/fields/io_error", json!(7)),
        (
            "/run_modal/fields/file/fields/memory_source/fields/root",
            Value::Null,
        ),
        (
            "/run_modal/fields/file/fields/memory_source/fields/directories",
            json!({}),
        ),
        (
            "/run_modal/fields/file/fields/memory_source/fields/files",
            json!({}),
        ),
    ] {
        let mut session = valid.clone();
        *session.pointer_mut(pointer).unwrap() = replacement;
        assert!(
            host.path_map.project_session_value(&mut session).is_err(),
            "accepted corrupt session pointer {pointer}"
        );
    }

    let mut ordinary = valid;
    ordinary["add"]["fields"]["ordinary"] = json!({
        "kind": "file_picker",
        "fields": {"path": host._sandbox.path()},
    });
    let expected = ordinary["add"]["fields"]["ordinary"].clone();
    host.path_map.project_session_value(&mut ordinary).unwrap();
    assert_eq!(ordinary["add"]["fields"]["ordinary"], expected);

    let draft = write_draft_at(&host, "skit-new-picker-name.py", b"print(1)\n", 10);
    host.path_map.refresh(&host.service).unwrap();
    let mut mismatch = c2_session(host._sandbox.path(), Some(&draft), Vec::new());
    let entry = mismatch["run_modal"]["fields"]["file"]["fields"]["explorer"]["entries"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|entry| entry["path"] == json!(draft))
        .unwrap();
    entry["name"] = json!("wrong.py");
    assert!(host.path_map.project_session_value(&mut mismatch).is_err());
}

#[test]
fn c2_action_surface_run_health_preferences_and_runner_owner_contract() {
    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    let root = host._sandbox.path().display().to_string();
    let stable = "<profile:fixture>";
    let diagnostic = format!("Diagnostic ({root}).");
    let stable_diagnostic = format!("Diagnostic ({stable}).");
    let mut reload = serde_json::to_value(host.dispatch(Effect::Reload).unwrap()).unwrap();
    let surface = &mut reload["replace_surface"]["surface"];
    surface["scan"]["entries"][0]["target"] = json!(root);
    surface["scan"]["diagnostics"] = json!([{
        "code": "io",
        "message": diagnostic,
    }]);
    let detail_key = surface["details"]
        .as_object()
        .unwrap()
        .keys()
        .next()
        .unwrap()
        .clone();
    surface["details"][&detail_key]["missing_target"] = json!(root);
    surface["details"][&detail_key]["parameters"] = json!([{
        "key": "user",
        "value": root,
        "secret": false,
    }]);
    let surface = surface.clone();
    let scan = surface["scan"].clone();
    let user_message = format!("Created user entry ({root}).");
    let fixtures = [
        (
            json!({"replace": {"scan": scan, "rerunnable": []}}),
            false,
            false,
        ),
        (
            json!({"replace_surface": {"surface": surface, "rerunnable": []}}),
            false,
            true,
        ),
        (
            json!({"complete": {
                "surface": surface,
                "rerunnable": [],
                "message": user_message,
            }}),
            true,
            true,
        ),
        (
            json!({"add_completed": {
                "surface": surface,
                "rerunnable": [],
                "slug": "command",
                "message": user_message,
            }}),
            true,
            true,
        ),
    ];
    for (fixture, has_user_message, has_details) in fixtures {
        let action = host.path_map.normalize_action_json(fixture).unwrap();
        let encoded = serde_json::to_string(&action).unwrap();
        assert!(encoded.contains(stable));
        assert!(encoded.contains(&stable_diagnostic));
        let encoded_message = serde_json::to_string(&user_message).unwrap();
        assert_eq!(
            encoded.contains(&encoded_message[1..encoded_message.len() - 1]),
            has_user_message
        );
        assert_eq!(
            encoded.contains(&format!(
                "\"value\":{}",
                serde_json::to_string(&root).unwrap()
            )),
            has_details
        );
    }

    let mut run = serde_json::to_value(
        host.dispatch(Effect::Open {
            request: HostRequest::Run,
            selector: Some("Command".to_owned()),
        })
        .unwrap(),
    )
    .unwrap()["present"]["run"]
        .clone();
    for pointer in [
        "/context/path/workdir",
        "/context/path/invoke_cwd",
        "/context/tokens/cwd",
        "/context/tokens/home",
    ] {
        *run.pointer_mut(pointer).unwrap() = json!(root);
    }
    run["context"]["tokens"]["env"] = json!({"ROOT": root});
    let text_index = run["fields"]
        .as_array()
        .unwrap()
        .iter()
        .position(|field| field.pointer("/control/text").is_some())
        .unwrap();
    run["fields"][text_index]["feedback"]["expanded"] = json!(diagnostic);
    run["fields"][text_index]["control"]["text"]["value"] = json!(root);
    run["fields"][text_index]["default"] = json!(root);
    run["initial_values"] = json!({"user": root});
    run["hidden_values"] = json!({"user": root});
    run["presets"] = json!({"saved": {"user": root}});
    let action = host
        .path_map
        .normalize_action_json(json!({"prompt_runner_required": {
            "form": run,
            "cancel_status": user_message,
        }}))
        .unwrap();
    let form = &action["prompt_runner_required"]["form"];
    assert_eq!(form["context"]["path"]["workdir"], stable);
    assert_eq!(form["context"]["tokens"]["env"]["ROOT"], stable);
    assert_eq!(
        form["fields"][text_index]["feedback"]["expanded"],
        stable_diagnostic
    );
    assert_eq!(form["fields"][text_index]["control"]["text"]["value"], root);
    assert_eq!(form["fields"][text_index]["default"], root);
    assert_eq!(form["initial_values"]["user"], root);
    assert_eq!(form["hidden_values"]["user"], root);
    assert_eq!(form["presets"]["saved"]["user"], root);
    assert_eq!(
        action["prompt_runner_required"]["cancel_status"],
        user_message
    );

    let snapshot = json!({
        "uv": {"found": root},
        "entry_count": 1,
        "issues": [{
            "slug": "command",
            "name": "Command",
            "kind": {"launch_blocked": {"reason": diagnostic}},
        }],
        "invalid_runner_rows": [],
        "mirror": "off",
        "library_path": root,
        "library_size": root,
        "diagnostics": [diagnostic],
    });
    let action = host
        .path_map
        .normalize_action_json(json!({"health": {"rebuilt": {
            "snapshot": snapshot,
            "outcome": {"entry_count": 1, "problems": [diagnostic]},
        }}}))
        .unwrap();
    let rebuilt = &action["health"]["rebuilt"];
    assert_eq!(rebuilt["snapshot"]["library_path"], stable);
    assert_eq!(rebuilt["snapshot"]["uv"]["found"], stable);
    assert_eq!(rebuilt["snapshot"]["library_size"], root);
    assert_eq!(
        rebuilt["snapshot"]["issues"][0]["kind"]["launch_blocked"]["reason"],
        stable_diagnostic
    );
    assert_eq!(rebuilt["snapshot"]["diagnostics"][0], stable_diagnostic);
    assert_eq!(rebuilt["outcome"]["problems"][0], stable_diagnostic);

    let targets = host
        .path_map
        .normalize_action_json(json!({"preferences": {
            "present_agent_skill_targets": [{
                "name": "codex",
                "scope": "user",
                "base": root,
            }],
        }}))
        .unwrap();
    assert_eq!(
        targets["preferences"]["present_agent_skill_targets"][0]["base"],
        stable
    );
    let mutation = host
        .path_map
        .normalize_action_json(json!({"runners": {
            "mutation_failed": diagnostic,
        }}))
        .unwrap();
    assert_eq!(mutation["runners"]["mutation_failed"], stable_diagnostic);
    let editor = host
        .path_map
        .normalize_action_json(json!({"runner_editor_save_failed": {
            "owner": "add",
            "message": diagnostic,
        }}))
        .unwrap();
    assert_eq!(
        editor["runner_editor_save_failed"]["message"],
        stable_diagnostic
    );

    let mut preferences = serde_json::to_value(
        host.dispatch(Effect::Open {
            request: HostRequest::Preferences,
            selector: None,
        })
        .unwrap(),
    )
    .unwrap()["present"]["preferences"]
        .clone();
    preferences["agent_skill_install"] = json!({
        "targets": [{"name": "codex", "scope": "user", "base": root}],
        "selected": 0,
    });
    for fixture in [
        json!({"present": {"preferences": preferences}}),
        json!({"runner_manager_closed": {"preferences": preferences}}),
    ] {
        let action = host.path_map.normalize_action_json(fixture).unwrap();
        let encoded = serde_json::to_string(&action).unwrap();
        assert!(!encoded.contains(&root));
        assert!(encoded.contains(stable));
    }

    let mut runners = serde_json::to_value(
        host.dispatch(Effect::Open {
            request: HostRequest::Runners,
            selector: None,
        })
        .unwrap(),
    )
    .unwrap()["present"]["runners"]
        .clone();
    runners["status"] = json!(diagnostic);
    runners["overlay"] = json!({"editor": {
        "name": "",
        "command": root,
        "target": "new",
        "focused": "name",
        "error": null,
        "host_error": diagnostic,
    }});
    let action = host
        .path_map
        .normalize_action_json(json!({"present": {"runners": runners}}))
        .unwrap();
    assert_eq!(action["present"]["runners"]["status"], diagnostic);
    assert_eq!(
        action["present"]["runners"]["overlay"]["editor"]["host_error"],
        stable_diagnostic
    );
    assert_eq!(
        action["present"]["runners"]["overlay"]["editor"]["command"],
        root
    );
}

#[test]
fn c2_review_name_and_kind_filename_require_draft_provenance() {
    assert_eq!(
        serde_json::to_value(skit_tui::AddTextField::SourcePath).unwrap(),
        "SourcePath"
    );
    assert_eq!(
        serde_json::to_value(skit_tui::AddTextField::ReviewName).unwrap(),
        "ReviewName"
    );

    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    let draft_path = write_draft_at(&host, "skit-new-review-name.py", b"print('review')\n", 10);
    let raw_name = "skit-new-review-name";
    let mut state = host.initial_state().unwrap();
    let open = host
        .dispatch(Effect::Open {
            request: HostRequest::Add,
            selector: None,
        })
        .unwrap();
    assert_eq!(state.update(open), Effect::None);
    assert_eq!(
        state.update(Action::Add(AddAction::SelectDraft(0))),
        Effect::None
    );
    let request = state.update(Action::Add(AddAction::Continue));
    let response = host.dispatch(request.clone()).unwrap();
    let response_value = serde_json::to_value(&response).unwrap();
    let source: SourceSnapshot =
        serde_json::from_value(response_value["add"]["source_inspected"]["result"]["Ok"].clone())
            .unwrap();
    let emitted = state.update(response.clone());
    let mut projected_state = state.clone();
    let mut session = c2_session(
        Path::new("/unregistered/session"),
        None,
        vec![json!({
            "id": "ReviewName",
            "state": {
                "value": raw_name,
                "cursor": raw_name.chars().count(),
                "yank": "",
                "last_was_cut": false,
            },
        })],
    );
    let mut recorded_request = request;
    let mut recorded_response = response;
    let mut recorded_emitted = emitted;
    host.capture_checkpoint_parts(
        &mut projected_state,
        &mut session,
        CheckpointCauseProjection::Host {
            request: &mut recorded_request,
            response: &mut recorded_response,
            emitted: &mut recorded_emitted,
        },
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(&projected_state).unwrap()["workflow"]["active"]["add"]["review"]["name"],
        "<draft:0>"
    );
    assert_eq!(
        session["add"]["fields"]["inputs"][0]["state"]["value"],
        "<draft:0>"
    );

    let projected_before_save = serde_json::to_value(&projected_state).unwrap();
    let mut save_state = state.clone();
    let mut save_action = Action::Add(AddAction::Save);
    let mut save_effect = save_state.update(save_action.clone());
    let mut save_session = c2_session(
        Path::new("/unregistered/session"),
        None,
        vec![json!({
            "id": "ReviewName",
            "state": {
                "value": raw_name,
                "cursor": raw_name.chars().count(),
                "yank": "",
                "last_was_cut": false,
            },
        })],
    );
    let save_observation = host
        .capture_checkpoint_parts(
            &mut save_state,
            &mut save_session,
            CheckpointCauseProjection::Reducer {
                action: &mut save_action,
                emitted: &mut save_effect,
            },
        )
        .unwrap();
    assert_eq!(
        serde_json::to_value(&save_effect).unwrap()["add"][0]["commit"]["entry"]["name"],
        "<draft:0>"
    );
    crate::cli::tui_real_walker::validate_reducer_action(
        &projected_before_save,
        &save_observation.state,
        &serde_json::to_value(save_action).unwrap(),
        &serde_json::to_value(save_effect).unwrap(),
    )
    .unwrap();

    let action = Action::Add(AddAction::SetReviewName(raw_name.to_owned()));
    let emitted = state.update(action.clone());
    let mut user_state = state.clone();
    let mut user_session = c2_session(
        Path::new("/unregistered/session"),
        None,
        vec![json!({
            "id": "ReviewName",
            "state": {
                "value": raw_name,
                "cursor": raw_name.chars().count(),
                "yank": "",
                "last_was_cut": false,
            },
        })],
    );
    let mut recorded_action = action;
    let mut recorded_emitted = emitted;
    let user_observation = host
        .capture_checkpoint_parts(
            &mut user_state,
            &mut user_session,
            CheckpointCauseProjection::Reducer {
                action: &mut recorded_action,
                emitted: &mut recorded_emitted,
            },
        )
        .unwrap();
    assert_eq!(
        serde_json::to_value(user_state).unwrap()["workflow"]["active"]["add"]["review"]["name"],
        raw_name
    );
    assert_eq!(
        user_session["add"]["fields"]["inputs"][0]["state"]["value"],
        raw_name
    );

    let mut manual_save_state = state.clone();
    let mut manual_save_action = Action::Add(AddAction::Save);
    let mut manual_save_effect = manual_save_state.update(manual_save_action.clone());
    let mut manual_save_session = user_session.clone();
    let manual_save_observation = host
        .capture_checkpoint_parts(
            &mut manual_save_state,
            &mut manual_save_session,
            CheckpointCauseProjection::Reducer {
                action: &mut manual_save_action,
                emitted: &mut manual_save_effect,
            },
        )
        .unwrap();
    assert_eq!(
        serde_json::to_value(&manual_save_effect).unwrap()["add"][0]["commit"]["entry"]["name"],
        raw_name
    );
    crate::cli::tui_real_walker::validate_reducer_action(
        &user_observation.state,
        &manual_save_observation.state,
        &serde_json::to_value(manual_save_action).unwrap(),
        &serde_json::to_value(manual_save_effect).unwrap(),
    )
    .unwrap();

    let mut default_host = RealWalkerHost::spawn(profile()).unwrap();
    let default_path = write_draft_at(
        &default_host,
        "skit-new-review-name.py",
        b"print('review default')\n",
        10,
    );
    let drafts = sorted_tui_drafts(&default_host.roots().data);
    let workflow = AddWorkflowState::new(drafts).with_review_defaults(ReviewDefaults {
        name: Some(raw_name.to_owned()),
        ..ReviewDefaults::default()
    });
    let mut default_state = default_host.initial_state().unwrap();
    assert_eq!(
        default_state.update(Action::Present(skit_ui::Screen::Add(Box::new(workflow)))),
        Effect::None
    );
    assert_eq!(
        default_state.update(Action::Add(AddAction::SelectDraft(0))),
        Effect::None
    );
    let mut default_request = default_state.update(Action::Add(AddAction::Continue));
    let mut default_response = default_host.dispatch(default_request.clone()).unwrap();
    let mut default_emitted = default_state.update(default_response.clone());
    let mut session = c2_session(
        Path::new("/unregistered/session"),
        None,
        vec![json!({
            "id": "ReviewName",
            "state": {
                "value": raw_name,
                "cursor": raw_name.chars().count(),
                "yank": "",
                "last_was_cut": false,
            },
        })],
    );
    let observation = default_host
        .capture_checkpoint_parts(
            &mut default_state,
            &mut session,
            CheckpointCauseProjection::Host {
                request: &mut default_request,
                response: &mut default_response,
                emitted: &mut default_emitted,
            },
        )
        .unwrap();
    assert_eq!(
        observation.state["workflow"]["active"]["add"]["review"]["name"],
        raw_name
    );
    assert_eq!(
        session["add"]["fields"]["inputs"][0]["state"]["value"],
        raw_name
    );
    assert_eq!(default_path.file_stem().unwrap(), raw_name);

    let source_value = serde_json::to_value(source).unwrap();
    let mut kind = json!({"add": {
        "pending_source": source_value,
        "kind_picker": {"filename": "skit-new-review-name.py"},
    }});
    host.path_map.normalize_add_screen(&mut kind).unwrap();
    assert_eq!(kind["add"]["kind_picker"]["filename"], "<draft:0>.py");
    let mut corrupt = json!({"add": {
        "pending_source": serde_json::to_value(SourceSnapshot {
            path: draft_path.clone(),
            source_record: draft_path.display().to_string(),
            bytes: Vec::new(),
            permissions: SourcePermissions::default(),
            executable: None,
            is_regular: true,
            is_directory: false,
            is_draft: true,
            identity: None,
        }).unwrap(),
        "kind_picker": {"filename": "wrong.py"},
    }});
    assert!(host.path_map.normalize_add_screen(&mut corrupt).is_err());
}
