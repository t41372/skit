//! Provenance, input mirror, and projection failure contracts.

use super::*;

#[test]
fn c2_commit_review_name_requires_exact_live_draft_provenance() {
    fn commit_value(path: &Path, name: &str) -> Value {
        let path = path.display().to_string();
        let source = json!({
            "path": path,
            "source_record": path,
            "bytes": [1, 2, 3],
            "permissions": {"readonly": false, "unix_mode": 384},
            "executable": false,
            "is_regular": true,
            "is_directory": false,
            "is_draft": true,
            "identity": null,
        });
        let mut entry = serde_json::to_value(profile().entries[1].clone()).unwrap();
        entry["name"] = json!(name);
        entry["source"] = json!(path);
        json!({"add": [{"commit": {
            "request": 0,
            "entry": entry,
            "source": source,
        }}]})
    }

    fn project(host: &mut RealWalkerHost, value: Value) -> Result<Value, String> {
        let mut effect: Effect = deserialize_canonical(value, "C2 derived Commit fixture")?;
        host.path_map.project_typed_effect(&mut effect)?;
        serde_json::to_value(effect).map_err(|error| error.to_string())
    }

    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    let first = write_draft_at(&host, "skit-new-first.py", b"first\n", 10);
    let second = write_draft_at(&host, "skit-new-second.py", b"second\n", 20);
    host.path_map.refresh(&host.service).unwrap();
    host.path_map.add_provenance.review_source_path = Some(first.clone());
    set_review_name_projection(&mut host, &first, "skit-new-first", "<draft:0>");

    let valid = commit_value(&first, "skit-new-first");
    assert_eq!(
        project(&mut host, valid.clone()).unwrap()["add"][0]["commit"]["entry"]["name"],
        "<draft:0>"
    );

    host.path_map.add_provenance.review_source_path = None;
    assert!(project(&mut host, valid.clone()).is_err());
    host.path_map.add_provenance.review_source_path = Some(first.clone());
    host.path_map
        .add_provenance
        .review_name
        .as_mut()
        .unwrap()
        .stable_name = "wrong".to_owned();
    assert!(project(&mut host, valid.clone()).is_err());
    host.path_map
        .add_provenance
        .review_name
        .as_mut()
        .unwrap()
        .stable_name = "<draft:0>".to_owned();
    host.path_map.add_provenance.review_source_path =
        Some(PathBuf::from("/unregistered/current.py"));
    assert!(project(&mut host, valid.clone()).is_err());
    host.path_map.add_provenance.review_source_path = Some(first.clone());

    for pointer in [
        "/add/0/commit/entry/name",
        "/add/0/commit/entry/kind",
        "/add/0/commit/entry/mode",
        "/add/0/commit/entry/source",
        "/add/0/commit/source/path",
        "/add/0/commit/source/source_record",
        "/add/0/commit/source/is_draft",
    ] {
        let mut mismatch = valid.clone();
        *mismatch.pointer_mut(pointer).unwrap() = match pointer {
            "/add/0/commit/entry/kind" => json!("shell"),
            "/add/0/commit/entry/mode" => json!("reference"),
            "/add/0/commit/source/is_draft" => json!(false),
            _ => json!("user-owned-lookalike"),
        };
        let mut typed: Effect =
            deserialize_canonical(mismatch, "C2 mismatched derived Commit fixture").unwrap();
        let before = typed.clone();
        assert!(host.path_map.project_typed_effect(&mut typed).is_err());
        assert_eq!(typed, before, "{pointer}");
    }
    let mut missing_source = valid.clone();
    missing_source["add"][0]["commit"]["source"] = Value::Null;
    assert!(project(&mut host, missing_source).is_err());

    host.path_map.add_provenance.review_source_path = Some(second.clone());
    let changed_source = commit_value(&second, "skit-new-first");
    assert_eq!(
        project(&mut host, changed_source).unwrap()["add"][0]["commit"]["entry"]["name"],
        "<draft:0>"
    );

    set_review_name_projection(
        &mut host,
        Path::new("/unregistered/skit-new-first.py"),
        "skit-new-first",
        "<draft:0>",
    );
    assert!(project(&mut host, valid).is_err());
}

#[test]
fn c2_add_input_mirror_refuses_cursor_yank_cut_and_shape_drift() {
    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    let draft = write_draft_at(&host, "skit-new-界.py", b"print('unicode')\n", 10);
    host.path_map.refresh(&host.service).unwrap();
    host.path_map.add_provenance.selected_source_path = Some(draft.clone());
    let raw = draft.display().to_string();
    let input = json!({
        "id": "SourcePath",
        "state": {
            "value": raw,
            "cursor": raw.chars().count(),
            "yank": "",
            "last_was_cut": false,
        },
    });
    let valid = c2_session(
        Path::new("/unregistered/session"),
        None,
        vec![input.clone()],
    );
    let mut projected = valid.clone();
    host.path_map.project_session_value(&mut projected).unwrap();
    let stable = "<profile:fixture>/data/.drafts/<draft:0>.py";
    assert_eq!(
        projected["add"]["fields"]["inputs"][0]["state"]["value"],
        stable
    );
    assert_eq!(
        projected["add"]["fields"]["inputs"][0]["state"]["cursor"],
        stable.chars().count()
    );

    let mut invalid = c2_session(Path::new("/unregistered/session"), None, Vec::new());
    assert!(host.path_map.project_session_value(&mut invalid).is_err());
    let mut duplicate = c2_session(
        Path::new("/unregistered/session"),
        None,
        vec![input.clone(), input.clone()],
    );
    assert!(host.path_map.project_session_value(&mut duplicate).is_err());
    for (pointer, replacement) in [
        ("/add/fields/inputs/0/state/value", json!("wrong")),
        ("/add/fields/inputs/0/state/cursor", json!(0)),
        ("/add/fields/inputs/0/state/yank", json!("cut")),
        ("/add/fields/inputs/0/state/yank", json!(7)),
        ("/add/fields/inputs/0/state/last_was_cut", json!(true)),
        ("/add/fields/inputs/0/state", json!([])),
    ] {
        let mut invalid = valid.clone();
        *invalid.pointer_mut(pointer).unwrap() = replacement;
        assert!(host.path_map.project_session_value(&mut invalid).is_err());
    }
    let mut missing_state = valid.clone();
    missing_state["add"]["fields"]["inputs"][0]
        .as_object_mut()
        .unwrap()
        .remove("state");
    assert!(
        host.path_map
            .project_session_value(&mut missing_state)
            .is_err()
    );
    let mut missing_field = valid.clone();
    missing_field["add"]["fields"]["inputs"][0]["state"]
        .as_object_mut()
        .unwrap()
        .remove("yank");
    assert!(
        host.path_map
            .project_session_value(&mut missing_field)
            .is_err()
    );
    let mut extra_field = valid;
    extra_field["add"]["fields"]["inputs"][0]["state"]["extra"] = json!(true);
    assert!(
        host.path_map
            .project_session_value(&mut extra_field)
            .is_err()
    );
}

#[test]
fn c2_add_review_name_yank_keeps_the_cut_derived_name_projected() {
    let review_input = |value: &str, yank: &str, cut: bool| {
        json!({
            "id": "ReviewName",
            "state": {
                "value": value,
                "cursor": value.chars().count(),
                "yank": yank,
                "last_was_cut": cut,
            },
        })
    };
    let project = |host: &mut RealWalkerHost, value: &str, yank: &str, cut: bool| {
        let mut session = c2_session(
            Path::new("/unregistered/session"),
            None,
            vec![review_input(value, yank, cut)],
        );
        host.path_map
            .project_session_value(&mut session)
            .map(|()| session["add"]["fields"]["inputs"][0]["state"].clone())
    };
    let edit = |name: &str| {
        deserialize_canonical::<Action>(
            json!({"add": {"set_review_name": name}}),
            "C2 SetReviewName fixture",
        )
        .unwrap()
    };
    let state = |name: &str| {
        json!({"workflow": {"active": {"add": {
            "review_defaults": {"name": null},
            "review": {"name": name},
        }}}})
    };

    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    let draft = write_draft_at(&host, "skit-new-first.py", b"first\n", 10);
    host.path_map.refresh(&host.service).unwrap();
    host.path_map.add_provenance.review_source_path = Some(draft.clone());
    set_review_name_projection(&mut host, &draft, "skit-new-first", "<draft:0>");

    host.path_map.record_add_projection_cause(&edit(""));
    host.path_map.reconcile_add_provenance(&state("")).unwrap();
    assert!(host.path_map.add_provenance.review_name.is_none());
    let cut = project(&mut host, "", "skit-new-first", true).unwrap();
    assert_eq!(cut["yank"], "<draft:0>");
    assert_eq!(cut["value"], "");
    assert_eq!(cut["cursor"], 0);
    assert_eq!(cut["last_was_cut"], true);

    host.path_map
        .record_add_projection_cause(&edit("Corpus Prompt"));
    host.path_map
        .reconcile_add_provenance(&state("Corpus Prompt"))
        .unwrap();
    let typed = project(&mut host, "Corpus Prompt", "skit-new-first", false).unwrap();
    assert_eq!(typed["yank"], "<draft:0>");
    assert_eq!(typed["value"], "Corpus Prompt");

    let replaced = project(&mut host, "Corpus Prompt", "Corpus", false).unwrap();
    assert_eq!(replaced["yank"], "Corpus");

    let mut manual = RealWalkerHost::spawn(profile()).unwrap();
    let manual_draft = write_draft_at(&manual, "skit-new-first.py", b"first\n", 10);
    manual.path_map.refresh(&manual.service).unwrap();
    manual.path_map.add_provenance.review_source_path = Some(manual_draft);
    manual.path_map.add_provenance.review_name_edited = true;
    let lookalike = project(&mut manual, "typed", "skit-new-first", true).unwrap();
    assert_eq!(lookalike["yank"], "skit-new-first");

    let mut drift = RealWalkerHost::spawn(profile()).unwrap();
    let drift_draft = write_draft_at(&drift, "skit-new-first.py", b"first\n", 10);
    drift.path_map.refresh(&drift.service).unwrap();
    drift.path_map.add_provenance.review_source_path = Some(drift_draft.clone());
    set_review_name_projection(&mut drift, &drift_draft, "skit-new-first", "<draft:0>");
    drift.path_map.add_cause = AddProjectionCause::None;
    drift
        .path_map
        .reconcile_add_provenance(&state("Corpus Prompt"))
        .unwrap();
    assert!(drift.path_map.add_provenance.review_name.is_none());
    let drifted = project(&mut drift, "Corpus Prompt", "skit-new-first", false).unwrap();
    assert_eq!(drifted["yank"], "<draft:0>");
}

#[test]
fn c2_runner_failure_status_provenance_replays_and_success_values_stay_user_owned() {
    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    let mut state = host.initial_state().unwrap();
    let open = host
        .dispatch(Effect::Open {
            request: HostRequest::Runners,
            selector: None,
        })
        .unwrap();
    assert_eq!(state.update(open), Effect::None);
    let previous = host.observe(&state).unwrap().state;
    let root = host._sandbox.path().display().to_string();
    let failure = format!("Runner store failure ({root}).");
    let stable_failure = "Runner store failure (<profile:fixture>).";
    let action = Action::Runners(skit_ui::RunnerManagerAction::MutationFailed(failure));
    let emitted = state.update(action.clone());
    let mut projected_state = state.clone();
    let mut session = c2_session(Path::new("/unregistered/session"), None, Vec::new());
    let mut recorded_action = action;
    let mut recorded_emitted = emitted;
    let observation = host
        .capture_checkpoint_parts(
            &mut projected_state,
            &mut session,
            CheckpointCauseProjection::Reducer {
                action: &mut recorded_action,
                emitted: &mut recorded_emitted,
            },
        )
        .unwrap();
    assert_eq!(
        observation.state["workflow"]["active"]["runners"]["status"],
        stable_failure
    );
    assert_eq!(
        serde_json::to_value(&recorded_action).unwrap()["runners"]["mutation_failed"],
        stable_failure
    );
    crate::cli::tui_real_walker::validate_reducer_action(
        &previous,
        &observation.state,
        &serde_json::to_value(recorded_action).unwrap(),
        &serde_json::to_value(recorded_emitted).unwrap(),
    )
    .unwrap();

    let success: Action = deserialize_canonical(
        json!({"runners": {"mutation_succeeded": {
            "rows": [{
                "identity": {"index": 0, "snapshot_token": root},
                "name": root,
                "argv": [root],
                "reason": null,
                "descriptor": root,
                "key_identities": [],
                "pinned_count": 0,
            }],
            "selected_name": root,
            "message": format!("Saved runner ({root})."),
        }}}),
        "C2 runner success fixture",
    )
    .unwrap();
    let emitted = state.update(success.clone());
    let mut session = c2_session(Path::new("/unregistered/session"), None, Vec::new());
    let mut recorded_action = success;
    let mut recorded_emitted = emitted;
    let observation = host
        .capture_checkpoint_parts(
            &mut state,
            &mut session,
            CheckpointCauseProjection::Reducer {
                action: &mut recorded_action,
                emitted: &mut recorded_emitted,
            },
        )
        .unwrap();
    let action = serde_json::to_value(recorded_action).unwrap();
    assert_eq!(
        observation.state["workflow"]["active"]["runners"]["status"],
        format!("Saved runner ({root}).")
    );
    let row = &action["runners"]["mutation_succeeded"]["rows"][0];
    assert_eq!(row["name"], root);
    assert_eq!(row["argv"][0], root);
    assert_eq!(row["descriptor"], root);
    assert_eq!(row["identity"]["snapshot_token"], root);
}

#[test]
fn c2_external_skit_new_editor_path_never_gains_draft_provenance() {
    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    let path = host.external_root.join("skit-new-user.py");
    fs::write(&path, b"print('user')\n").unwrap();

    host.path_map.register_event_artifacts(&PortEvent::Editor {
        argv: vec!["editor".to_owned()],
        path: path.clone(),
        outcome: json!({"ok": true}),
    });

    assert!(!host.path_map.draft_paths.contains_key(&path));
    assert_eq!(
        host.path_map.normalize_path(&path),
        "<profile:fixture>/external/skit-new-user.py"
    );
    assert_eq!(
        host.path_map
            .text_artifacts
            .get(&path.display().to_string()),
        Some(&"<profile:fixture>/external/skit-new-user.py".to_owned())
    );
}

#[test]
fn c2_session_projection_failure_retains_then_drains_the_event_prefix_once() {
    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    host.clear_transcript();
    host.adapters.push(PortEvent::Output {
        stream: "diagnostic".to_owned(),
        text: "retained session event".to_owned(),
    });
    let state = host.initial_state().unwrap();
    let mut failed_state = state.clone();
    let mut invalid_session = json!({"schema_version": 2});

    assert!(
        host.capture_checkpoint_parts(
            &mut failed_state,
            &mut invalid_session,
            CheckpointCauseProjection::Observation,
        )
        .is_err()
    );
    assert!(host.adapters.pending_events().iter().any(|event| {
        matches!(event, PortEvent::Output { text, .. } if text == "retained session event")
    }));

    let mut valid_state = state.clone();
    let mut valid_session = c2_session(Path::new("/unregistered/session"), None, Vec::new());
    let observation = host
        .capture_checkpoint_parts(
            &mut valid_state,
            &mut valid_session,
            CheckpointCauseProjection::Observation,
        )
        .unwrap();
    assert_eq!(
        events_with_keys(&observation.transcript, &["output"]),
        vec![json!({
            "output": "diagnostic",
            "text": "retained session event",
        })]
    );
    assert!(host.adapters.pending_events().is_empty());

    let mut final_state = state;
    let mut final_session = c2_session(Path::new("/unregistered/session"), None, Vec::new());
    let final_observation = host
        .capture_checkpoint_parts(
            &mut final_state,
            &mut final_session,
            CheckpointCauseProjection::Observation,
        )
        .unwrap();
    assert!(events_with_keys(&final_observation.transcript, &["output"]).is_empty());
}

#[test]
fn c2_every_add_effect_path_projects_in_all_checkpoint_cause_positions() {
    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    let draft_path = write_draft_at(
        &host,
        "skit-new-effect-cause.py",
        b"print('effect cause')\n",
        10,
    );
    host.path_map.refresh(&host.service).unwrap();
    let draft = serde_json::to_value(
        sorted_tui_drafts(&host.roots().data)
            .into_iter()
            .next()
            .unwrap(),
    )
    .unwrap();
    let source = json!({
        "path": draft_path,
        "source_record": draft_path,
        "bytes": [1, 2, 3],
        "permissions": {"readonly": false, "unix_mode": 384},
        "executable": false,
        "is_regular": true,
        "is_directory": false,
        "is_draft": true,
        "identity": null,
    });
    let mut entry = serde_json::to_value(profile().entries[0].clone()).unwrap();
    entry["source"] = json!(draft_path);
    let raw: Effect = deserialize_canonical(
        json!({"add": [
            {"inspect_source": {"request": 0, "path": draft_path}},
            {"delete_draft": {"request": 0, "draft": draft}},
            {"edit_source": {"request": 0, "path": draft_path}},
            {"commit": {"request": 0, "entry": entry, "source": source}},
            {"consume_draft": source},
            {"draft_kept": draft_path},
        ]}),
        "C2 checkpoint Add Effect fixture",
    )
    .unwrap();
    let raw_text = serde_json::to_string(&draft_path).unwrap();
    let stable = "<profile:fixture>/data/.drafts/<draft:0>.py";
    let assert_projected = |effect: &Effect| {
        let value = serde_json::to_value(effect).unwrap();
        for pointer in [
            "/add/0/inspect_source/path",
            "/add/1/delete_draft/draft/path",
            "/add/2/edit_source/path",
            "/add/3/commit/entry/source",
            "/add/3/commit/source/path",
            "/add/3/commit/source/source_record",
            "/add/4/consume_draft/path",
            "/add/4/consume_draft/source_record",
            "/add/5/draft_kept",
        ] {
            assert_eq!(value.pointer(pointer), Some(&json!(stable)), "{pointer}");
        }
        assert!(!serde_json::to_string(&value).unwrap().contains(&raw_text));
    };

    let mut state = host.initial_state().unwrap();
    let mut action = Action::ClearStatus;
    let mut reducer_emitted = raw.clone();
    let mut session = c2_session(Path::new("/unregistered/session"), None, Vec::new());
    host.capture_checkpoint_parts(
        &mut state,
        &mut session,
        CheckpointCauseProjection::Reducer {
            action: &mut action,
            emitted: &mut reducer_emitted,
        },
    )
    .unwrap();
    assert_projected(&reducer_emitted);

    let mut state = host.initial_state().unwrap();
    let mut request = raw.clone();
    let mut response = Action::ClearStatus;
    let mut host_emitted = raw.clone();
    let mut session = c2_session(Path::new("/unregistered/session"), None, Vec::new());
    host.capture_checkpoint_parts(
        &mut state,
        &mut session,
        CheckpointCauseProjection::Host {
            request: &mut request,
            response: &mut response,
            emitted: &mut host_emitted,
        },
    )
    .unwrap();
    for projected in [&request, &host_emitted] {
        assert_projected(projected);
    }
    assert!(serde_json::to_string(&raw).unwrap().contains(&raw_text));
}

#[test]
fn c2_source_edited_keeps_derived_review_provenance_when_the_name_is_unchanged() {
    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    let first = write_draft_at(&host, "skit-new-first.py", b"first\n", 10);
    let second = write_draft_at(&host, "skit-new-second.py", b"second\n", 20);
    host.path_map.refresh(&host.service).unwrap();
    host.path_map.add_provenance.review_source_path = Some(first.clone());
    set_review_name_projection(&mut host, &first, "skit-new-first", "<draft:0>");
    let source = |path: &Path| {
        json!({
            "path": path,
            "source_record": path,
            "bytes": [],
            "permissions": {"readonly": false, "unix_mode": null},
            "executable": false,
            "is_regular": true,
            "is_directory": false,
            "is_draft": true,
            "identity": null,
        })
    };
    let action = |path: &Path| {
        deserialize_canonical::<Action>(
            json!({"add": {"source_edited": {
                "request": 0,
                "result": {"Ok": source(path)},
            }}}),
            "C2 SourceEdited fixture",
        )
        .unwrap()
    };
    let state = |path: &Path, name: &str| {
        json!({"workflow": {"active": {"add": {
            "review_defaults": {"name": null},
            "review": {"name": name, "source": source(path)},
        }}}})
    };

    host.path_map.record_add_projection_cause(&action(&first));
    host.path_map
        .reconcile_add_provenance(&state(&first, "skit-new-first"))
        .unwrap();
    assert_eq!(
        host.path_map
            .add_provenance
            .review_name
            .as_ref()
            .map(|projection| &projection.source_path),
        Some(&first)
    );
    let mut same_screen = json!({"add": state(&first, "skit-new-first")
            ["workflow"]["active"]["add"].clone()});
    host.path_map
        .normalize_add_screen(&mut same_screen)
        .unwrap();
    assert_eq!(same_screen["add"]["review"]["name"], "<draft:0>");
    let mut same_session = c2_session(
        Path::new("/unregistered/session"),
        None,
        vec![json!({
            "id": "ReviewName",
            "state": {
                "value": "skit-new-first",
                "cursor": "skit-new-first".chars().count(),
                "yank": "",
                "last_was_cut": false,
            },
        })],
    );
    host.path_map
        .project_session_value(&mut same_session)
        .unwrap();
    assert_eq!(
        same_session["add"]["fields"]["inputs"][0]["state"]["value"],
        "<draft:0>"
    );

    host.path_map.record_add_projection_cause(&action(&second));
    host.path_map
        .reconcile_add_provenance(&state(&second, "skit-new-first"))
        .unwrap();
    assert_eq!(
        host.path_map.add_provenance.review_source_path,
        Some(second.clone())
    );
    assert_eq!(
        host.path_map
            .add_provenance
            .review_name
            .as_ref()
            .map(|projection| &projection.source_path),
        Some(&first)
    );
    assert!(!host.path_map.add_provenance.review_name_edited);
    let mut changed_screen = json!({"add": state(&second, "skit-new-first")
            ["workflow"]["active"]["add"].clone()});
    host.path_map
        .normalize_add_screen(&mut changed_screen)
        .unwrap();
    assert_eq!(changed_screen["add"]["review"]["name"], "<draft:0>");
    let mut changed_session = c2_session(
        Path::new("/unregistered/session"),
        None,
        vec![json!({
            "id": "ReviewName",
            "state": {
                "value": "skit-new-first",
                "cursor": "skit-new-first".chars().count(),
                "yank": "",
                "last_was_cut": false,
            },
        })],
    );
    host.path_map
        .project_session_value(&mut changed_session)
        .unwrap();
    assert_eq!(
        changed_session["add"]["fields"]["inputs"][0]["state"]["value"],
        "<draft:0>"
    );

    host.path_map.add_cause = AddProjectionCause::ReviewSource(Some(first.clone()));
    let pending = json!({"workflow": {"active": {"add": {
        "review_defaults": {"name": null},
        "pending_source": source(&first),
        "kind_picker": {"filename": "skit-new-first.py"},
    }}}});
    host.path_map.reconcile_add_provenance(&pending).unwrap();
    let mut pending_screen = json!({"add": pending["workflow"]["active"]["add"].clone()});
    host.path_map
        .normalize_add_screen(&mut pending_screen)
        .unwrap();
    assert_eq!(
        pending_screen["add"]["kind_picker"]["filename"],
        "<draft:0>.py"
    );
    host.path_map.add_cause = AddProjectionCause::None;
    let review = state(&first, "skit-new-first");
    host.path_map.reconcile_add_provenance(&review).unwrap();
    let mut review_screen = json!({"add": review["workflow"]["active"]["add"].clone()});
    host.path_map
        .normalize_add_screen(&mut review_screen)
        .unwrap();
    assert_eq!(review_screen["add"]["review"]["name"], "<draft:0>");

    host.path_map
        .record_add_projection_cause(&Action::Present(skit_ui::Screen::Add(Box::new(
            AddWorkflowState::new(Vec::new()),
        ))));
    let leaving = json!({"workflow": {"active": {"add": {
        "source": {"path": first, "selected_draft": null, "drafts": []},
    }}}});
    host.path_map.reconcile_add_provenance(&leaving).unwrap();
    let mut leaving_screen = json!({"add": leaving["workflow"]["active"]["add"].clone()});
    host.path_map
        .normalize_add_screen(&mut leaving_screen)
        .unwrap();
    assert_eq!(
        leaving_screen["add"]["source"]["path"],
        first.display().to_string()
    );
    let mut leaving_session = c2_session(
        Path::new("/unregistered/session"),
        None,
        vec![json!({
            "id": "SourcePath",
            "state": {
                "value": first.display().to_string(),
                "cursor": first.display().to_string().chars().count(),
                "yank": "",
                "last_was_cut": false,
            },
        })],
    );
    host.path_map
        .project_session_value(&mut leaving_session)
        .unwrap();
    assert_eq!(
        leaving_session["add"]["fields"]["inputs"][0]["state"]["value"],
        first.display().to_string()
    );
}

#[test]
fn c2_provenance_refuses_mismatched_state_and_covers_optional_session_shapes() {
    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    let draft = write_draft_at(&host, "skit-new-refusal.py", b"refusal\n", 10);
    host.path_map.refresh(&host.service).unwrap();
    let draft_row = serde_json::to_value(
        sorted_tui_drafts(&host.roots().data)
            .into_iter()
            .next()
            .unwrap(),
    )
    .unwrap();
    let add_state = |path: &Path, selected: Value, source_path: &str| {
        json!({"workflow": {"active": {"add": {
            "source": {
                "drafts": [{"path": path}],
                "selected_draft": selected,
                "path": source_path,
            },
        }}}})
    };

    host.path_map.add_cause = AddProjectionCause::SelectDraft(0);
    assert!(
        host.path_map
            .reconcile_add_provenance(&add_state(
                Path::new("/outside/unregistered.py"),
                json!(0),
                "/outside/unregistered.py",
            ))
            .is_err()
    );
    host.path_map.add_cause = AddProjectionCause::SelectDraft(0);
    assert!(
        host.path_map
            .reconcile_add_provenance(&add_state(&draft, json!(1), &draft.display().to_string()))
            .is_err()
    );
    host.path_map.add_cause = AddProjectionCause::SelectDraft(0);
    assert!(
        host.path_map
            .reconcile_add_provenance(&add_state(&draft, json!(0), "different"))
            .is_err()
    );
    host.path_map.add_provenance.selected_source_path = Some(draft.clone());
    host.path_map.add_cause = AddProjectionCause::None;
    host.path_map
        .reconcile_add_provenance(&add_state(
            &draft,
            Value::Null,
            &draft.display().to_string(),
        ))
        .unwrap();
    assert_eq!(
        host.path_map.add_provenance.selected_source_path,
        Some(draft.clone())
    );
    host.path_map.add_cause = AddProjectionCause::None;
    host.path_map
        .reconcile_add_provenance(&add_state(&draft, Value::Null, "different"))
        .unwrap();
    assert!(host.path_map.add_provenance.selected_source_path.is_none());

    host.path_map.add_provenance.review_source_path = Some(draft.clone());
    set_review_name_projection(&mut host, &draft, "skit-new-refusal", "<draft:0>");
    host.path_map.add_cause = AddProjectionCause::None;
    host.path_map
        .reconcile_add_provenance(&json!({"workflow": {"active": {"add": {}}}}))
        .unwrap();
    assert!(host.path_map.add_provenance.review_source_path.is_none());

    host.path_map.add_provenance.review_source_path = Some(draft.clone());
    set_review_name_projection(&mut host, &draft, "skit-new-refusal", "<draft:0>");
    host.path_map.add_provenance.review_name_edited = false;
    host.path_map.add_cause = AddProjectionCause::None;
    host.path_map
        .reconcile_add_provenance(&json!({"workflow": {"active": {"add": {
            "review_defaults": {"name": null},
            "review": {
                "name": "user changed the derived name",
                "source": {"path": draft},
            },
        }}}}))
        .unwrap();
    assert!(host.path_map.add_provenance.review_name.is_none());
    assert!(host.path_map.add_provenance.review_name_edited);

    host.path_map.add_provenance.review_source_path = None;
    host.path_map.add_provenance.review_name = None;
    host.path_map.add_provenance.review_name_edited = false;
    host.path_map.add_cause = AddProjectionCause::None;
    host.path_map
        .reconcile_add_provenance(&json!({"workflow": {"active": {"add": {
            "review_defaults": {"name": null},
            "review": {"name": "user", "source": {"path": draft}},
        }}}}))
        .unwrap();
    assert!(host.path_map.add_provenance.review_name.is_none());
    assert!(!host.path_map.add_provenance.review_name_edited);

    host.path_map.add_provenance.review_source_path = Some(draft.clone());
    host.path_map.add_provenance.review_name = None;
    host.path_map.add_provenance.review_name_edited = false;
    host.path_map.add_cause = AddProjectionCause::None;
    host.path_map
        .reconcile_add_provenance(&json!({"workflow": {"active": {"add": {
            "review_defaults": {"name": null},
            "review": {
                "name": "skit-new-refusal",
                "source": {"path": "/outside/different.py"},
            },
        }}}}))
        .unwrap();
    assert!(host.path_map.add_provenance.review_source_path.is_none());
    assert!(host.path_map.add_provenance.review_name_edited);

    let unregistered = host.roots().data.join("not-a-draft.py");
    host.path_map.add_provenance.review_source_path = Some(unregistered.clone());
    host.path_map.add_provenance.review_name = None;
    host.path_map.add_provenance.review_name_edited = false;
    host.path_map.add_cause = AddProjectionCause::None;
    let error = host
        .path_map
        .reconcile_add_provenance(&json!({"workflow": {"active": {"add": {
            "review_defaults": {"name": null},
            "review": {
                "name": "not-a-draft",
                "source": {"path": unregistered},
            },
        }}}}))
        .unwrap_err();
    assert_eq!(
        error,
        "an auto-derived Add review source path is not registered"
    );
    assert!(host.path_map.add_provenance.review_name.is_none());

    host.path_map.add_provenance.review_source_path = Some(draft.clone());
    host.path_map.add_provenance.review_name_edited = false;
    host.path_map.add_cause = AddProjectionCause::None;
    host.path_map
        .reconcile_add_provenance(&json!({"workflow": {"active": {"add": {
            "review_defaults": {"name": null},
            "review": {"name": "wrong", "source": {"path": draft}},
        }}}}))
        .unwrap();
    assert!(host.path_map.add_provenance.review_name_edited);
    set_review_name_projection(&mut host, &draft, "skit-new-refusal", "<draft:0>");
    let mut wrong_review = json!({"add": {"review": {"name": "wrong"}}});
    assert!(
        host.path_map
            .normalize_add_screen(&mut wrong_review)
            .is_err()
    );
    set_review_name_projection(&mut host, &unregistered, "not-a-draft", "<not-a-draft>");
    let mut unregistered_review = json!({"add": {"review": {"name": "not-a-draft"}}});
    assert!(
        host.path_map
            .normalize_add_screen(&mut unregistered_review)
            .is_err()
    );

    host.path_map.status_cause = StatusProjectionCause::AgentSkillInstalled("expected".to_owned());
    assert!(
        host.path_map
            .reconcile_status_provenance(&json!({"status": "different"}))
            .is_err()
    );
    host.path_map.status_provenance = Some("expected".to_owned());
    assert!(
        host.path_map
            .normalize_library_json(json!({"status": "different"}))
            .is_err()
    );
    host.path_map.status_provenance = Some("not a template".to_owned());
    assert!(
        host.path_map
            .normalize_library_json(json!({"status": "not a template"}))
            .is_err()
    );
    host.path_map.status_provenance = None;
    let mut not_text = Value::Null;
    assert!(
        !host
            .path_map
            .normalize_agent_skill_install_text(&mut not_text)
            .unwrap()
    );

    let source = SourceSnapshot {
        path: draft.clone(),
        source_record: draft.display().to_string(),
        bytes: Vec::new(),
        permissions: SourcePermissions::default(),
        executable: None,
        is_regular: true,
        is_directory: false,
        is_draft: true,
        identity: None,
    };
    for action in [
        deserialize_canonical::<AddAction>(
            json!({"draft_edited": {"request": 0, "result": {"Ok": source}}}),
            "DraftEdited Some cause",
        )
        .unwrap(),
        deserialize_canonical::<AddAction>(
            json!({"draft_edited": {"request": 0, "result": {"Ok": null}}}),
            "DraftEdited None cause",
        )
        .unwrap(),
        deserialize_canonical::<AddAction>(
            json!({"draft_edited": {"request": 0, "result": {"Err": "failed"}}}),
            "DraftEdited Err cause",
        )
        .unwrap(),
    ] {
        let _ = add_projection_cause(&action);
    }

    host.path_map.add_provenance = AddProjectionProvenance::default();
    let mut optional = c2_session(
        Path::new("/unregistered/session"),
        None,
        vec![json!({
            "id": "Other",
            "state": {},
        })],
    );
    optional["preferences"]["fields"]["agent_signature"] = Value::Null;
    optional["path_suggestions"]["fields"]["expected"] = Value::Null;
    optional["path_suggestions"]["fields"]["visible"] = Value::Null;
    optional["add"]["fields"]["signature"] = Value::Null;
    host.path_map.project_session_value(&mut optional).unwrap();

    set_review_name_projection(&mut host, &draft, "skit-new-refusal", "<draft:0>");
    let mut missing = c2_session(Path::new("/unregistered/session"), None, Vec::new());
    missing["add"]["fields"]["signature"]["stage"] = json!("review");
    assert!(host.path_map.project_session_value(&mut missing).is_err());
    let review = json!({
        "id": "ReviewName",
        "state": {
            "value": "skit-new-refusal",
            "cursor": "skit-new-refusal".chars().count(),
            "yank": "",
            "last_was_cut": false,
        },
    });
    let mut duplicate = c2_session(
        Path::new("/unregistered/session"),
        None,
        vec![review.clone(), review],
    );
    assert!(host.path_map.project_session_value(&mut duplicate).is_err());
    let mut scalar_inputs = c2_session(Path::new("/unregistered/session"), None, Vec::new());
    scalar_inputs["add"]["fields"]["inputs"] = json!({});
    assert!(
        host.path_map
            .project_session_value(&mut scalar_inputs)
            .is_err()
    );

    let mut unrelated = json!({});
    host.path_map
        .normalize_preferences_screen(&mut unrelated)
        .unwrap();
    assert_eq!(draft_row["path"], draft.display().to_string());
}

#[test]
fn c2_add_screen_artifact_problem_notice_and_user_field_contract() {
    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    let draft_path = write_draft_at(&host, "skit-new-screen.py", b"print('screen')\n", 10);
    host.path_map.refresh(&host.service).unwrap();
    let draft = serde_json::to_value(
        sorted_tui_drafts(&host.roots().data)
            .into_iter()
            .next()
            .unwrap(),
    )
    .unwrap();
    let source = json!({
        "path": draft_path,
        "source_record": draft_path,
        "bytes": [1, 2, 3],
        "permissions": {"readonly": true, "unix_mode": 416},
        "executable": false,
        "is_regular": true,
        "is_directory": false,
        "is_draft": true,
        "identity": null,
    });
    let root = host._sandbox.path().display().to_string();
    let message = format!("Add failure ({root}).");
    let stable_message = "Add failure (<profile:fixture>).";
    let mut screen = json!({"add": {
        "source": {"path": root, "drafts": [draft]},
        "pending_source": source,
        "review": {"name": root, "source": source},
        "pending_delete": [0, draft],
        "delete_candidate": draft,
        "problem": {"source_unavailable": {"path": root, "reason": message}},
        "notice": {"draft_kept": draft_path},
    }});
    host.path_map.normalize_add_screen(&mut screen).unwrap();
    let add = &screen["add"];
    assert_eq!(add["source"]["path"], root);
    assert_eq!(add["review"]["name"], root);
    for pointer in [
        "/source/drafts/0/path",
        "/pending_source/path",
        "/pending_source/source_record",
        "/review/source/path",
        "/review/source/source_record",
        "/pending_delete/1/path",
        "/delete_candidate/path",
        "/notice/draft_kept",
    ] {
        assert_eq!(
            add.pointer(pointer),
            Some(&json!("<profile:fixture>/data/.drafts/<draft:0>.py"))
        );
    }
    assert_eq!(
        add["problem"]["source_unavailable"]["path"],
        "<profile:fixture>"
    );
    assert_eq!(
        add["problem"]["source_unavailable"]["reason"],
        stable_message
    );
    assert_eq!(add["pending_source"]["permissions"]["unix_mode"], 416);

    for (problem, pointer) in [
        (
            json!({"draft_changed": {"path": draft_path}}),
            "/draft_changed/path",
        ),
        (
            json!({"source_edit": {"reason": message}}),
            "/source_edit/reason",
        ),
        (
            json!({"commit_failed": {"reason": message}}),
            "/commit_failed/reason",
        ),
        (
            json!({"edit_failed": {"reason": message}}),
            "/edit_failed/reason",
        ),
        (
            json!({"draft_delete_failed": {"reason": message}}),
            "/draft_delete_failed/reason",
        ),
    ] {
        let mut screen = json!({"add": {"problem": problem}});
        host.path_map.normalize_add_screen(&mut screen).unwrap();
        let expected = if pointer.ends_with("/path") {
            "<profile:fixture>/data/.drafts/<draft:0>.py"
        } else {
            stable_message
        };
        assert_eq!(
            screen["add"]["problem"].pointer(pointer),
            Some(&json!(expected))
        );
    }
    for notice in [
        json!({"draft_kept": draft_path}),
        json!({"draft_deleted": draft_path}),
    ] {
        let mut screen = json!({"add": {"notice": notice}});
        host.path_map.normalize_add_screen(&mut screen).unwrap();
        assert!(
            serde_json::to_string(&screen)
                .unwrap()
                .contains("<profile:fixture>/data/.drafts/<draft:0>.py")
        );
    }
    for problem in [
        json!({"invalid_dependency": {"value": root}}),
        json!({"invalid_python_constraint": {"value": root}}),
    ] {
        let mut screen = json!({"add": {"problem": problem}});
        let before = screen.clone();
        host.path_map.normalize_add_screen(&mut screen).unwrap();
        assert_eq!(screen, before);
    }
}
