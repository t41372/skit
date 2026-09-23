//! Draft, picker, and add session projection contracts.

use super::*;

fn picker_provenance_profile() -> WalkerSeedSpec {
    let mut spec = profile();
    spec.external.extend([
        WalkerExternalSeed::File {
            path: PathBuf::from("nested/picked.py"),
            bytes: b"print('picked')\n".to_vec(),
            readonly: false,
            unix_mode: 0o640,
        },
        WalkerExternalSeed::Symlink {
            path: PathBuf::from("linked.py"),
            target: PathBuf::from("nested/picked.py"),
        },
    ]);
    spec
}

fn render_real_session_snapshot(state: &LibraryState, session: &mut TuiSession) -> Value {
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal
        .draw(|frame| {
            let _ = render_with_session(frame, state, Locale::En, session);
        })
        .unwrap();
    serde_json::to_value(session.agent_review_snapshot().unwrap()).unwrap()
}

fn capture_real_reducer_checkpoint(
    host: &mut RealWalkerHost,
    state: &LibraryState,
    session: &mut TuiSession,
    action: Action,
    emitted: Effect,
) -> (HostObservation, Action, Effect, Value) {
    let mut captured_state = state.clone();
    let mut captured_session = render_real_session_snapshot(state, session);
    let mut recorded_action = action;
    let mut recorded_emitted = emitted;
    let observation = host
        .capture_checkpoint_parts(
            &mut captured_state,
            &mut captured_session,
            CheckpointCauseProjection::Reducer {
                action: &mut recorded_action,
                emitted: &mut recorded_emitted,
            },
        )
        .unwrap();
    (
        observation,
        recorded_action,
        recorded_emitted,
        captured_session,
    )
}

fn picker_add_state(host: &RealWalkerHost) -> LibraryState {
    let mut state = host.initial_state().unwrap();
    let open = host
        .dispatch(Effect::Open {
            request: HostRequest::Add,
            selector: None,
        })
        .unwrap();
    assert_eq!(state.update(open), Effect::None);
    state
}

fn picker_tui_session(host: &RealWalkerHost) -> TuiSession {
    let tree = host.file_picker_tree();
    TuiSession::with_file_picker_tree(tree.root, tree.directories, tree.files)
}

fn source_input_value(session: &Value) -> &str {
    session
        .pointer("/add/fields/inputs")
        .and_then(Value::as_array)
        .and_then(|inputs| inputs.iter().find(|input| input["id"] == "SourcePath"))
        .and_then(|input| input.pointer("/state/value"))
        .and_then(Value::as_str)
        .expect("the source stage owns one source-path input")
}

fn capture_real_host_checkpoint(
    host: &mut RealWalkerHost,
    state: &LibraryState,
    session: &mut TuiSession,
    request: Effect,
    response: Action,
    emitted: Effect,
) -> (HostObservation, Action, Effect, Value) {
    let mut captured_state = state.clone();
    let mut captured_session = render_real_session_snapshot(state, session);
    let mut recorded_request = request;
    let mut recorded_response = response;
    let mut recorded_emitted = emitted;
    let observation = host
        .capture_checkpoint_parts(
            &mut captured_state,
            &mut captured_session,
            CheckpointCauseProjection::Host {
                request: &mut recorded_request,
                response: &mut recorded_response,
                emitted: &mut recorded_emitted,
            },
        )
        .unwrap();
    (
        observation,
        recorded_response,
        recorded_emitted,
        captured_session,
    )
}

fn assert_real_draft_transition_projects_and_replays(
    name: &str,
    bytes: &[u8],
    expected_stage: &str,
    derived_pointer: &str,
    raw_derived: &str,
    stable_path: &str,
    stable_derived: &str,
) {
    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    write_unicode_draft_at(&host, name, bytes);
    host.path_map.refresh(&host.service).unwrap();
    let mut state = host.initial_state().unwrap();
    assert_eq!(
        state.update(Action::Present(skit_ui::Screen::Add(Box::new(
            AddWorkflowState::new(sorted_tui_drafts(&host.roots().data)),
        )))),
        Effect::None
    );
    let mut session = TuiSession::default();

    let select = Action::Add(AddAction::SelectDraft(0));
    let select_emitted = state.update(select.clone());
    let _ =
        capture_real_reducer_checkpoint(&mut host, &state, &mut session, select, select_emitted);
    let continue_action = Action::Add(AddAction::Continue);
    let inspect = state.update(continue_action.clone());
    let (inspecting, _, _, _) = capture_real_reducer_checkpoint(
        &mut host,
        &state,
        &mut session,
        continue_action,
        inspect.clone(),
    );
    let response = host.dispatch(inspect.clone()).unwrap();
    let emitted = state.update(response.clone());
    let raw = serde_json::to_value(&state).unwrap();
    assert_eq!(
        raw.pointer("/workflow/active/add/stage"),
        Some(&json!(expected_stage))
    );
    assert_eq!(raw.pointer(derived_pointer), Some(&json!(raw_derived)));

    let (projected, recorded_response, recorded_emitted, _) =
        capture_real_host_checkpoint(&mut host, &state, &mut session, inspect, response, emitted);
    assert_eq!(
        projected
            .state
            .pointer("/workflow/active/add/pending_source/path")
            .filter(|value| !value.is_null())
            .or_else(|| {
                projected
                    .state
                    .pointer("/workflow/active/add/review/source/path")
            }),
        Some(&json!(stable_path))
    );
    assert_eq!(
        projected.state.pointer(derived_pointer),
        Some(&json!(stable_derived))
    );
    crate::cli::tui_real_walker::validate_reducer_action(
        &inspecting.state,
        &projected.state,
        &serde_json::to_value(recorded_response).unwrap(),
        &serde_json::to_value(recorded_emitted).unwrap(),
    )
    .unwrap();
}

fn source_edited_response(request: &Effect, source: Value) -> Action {
    let request =
        serde_json::to_value(request).unwrap()["add"][0]["edit_source"]["request"].clone();
    deserialize_canonical(
        json!({"add": {"source_edited": {
            "request": request,
            "result": {"Ok": source},
        }}}),
        "C2 real SourceEdited response",
    )
    .unwrap()
}

#[test]
fn c2_host_text_boundary_grammar_and_root_registration_contract() {
    let host = RealWalkerHost::spawn(profile()).unwrap();
    let root = host._sandbox.path().display().to_string();
    let stable = "<profile:fixture>";

    assert_eq!(host.path_map.normalize_host_text(&root), stable);
    for (left, right) in [
        (" ", " "),
        ("'", "'"),
        ("\"", "\""),
        ("(", ")"),
        ("[", "]"),
        ("{", "}"),
        (":", ","),
        ("=", "/"),
        ("/", "\\"),
        ("，", "。"),
        ("；", "："),
        ("！", "？"),
        ("、", "（"),
        ("）", "【"),
        ("】", "「"),
        ("」", "『"),
        ("』", "《"),
        ("《", "》"),
    ] {
        assert_eq!(
            host.path_map
                .normalize_host_text(&format!("{left}{root}{right}")),
            format!("{left}{stable}{right}")
        );
    }
    assert_eq!(
        host.path_map.normalize_host_text(&format!("{root}.")),
        format!("{stable}.")
    );
    assert_eq!(
        host.path_map.normalize_host_text(&format!("{root}~~⟧")),
        format!("{stable}~~⟧")
    );
    for lookalike in [
        format!("x{root}"),
        format!("{root}x"),
        format!("_{root}"),
        format!("{root}_"),
        format!("-{root}"),
        format!("{root}-suffix"),
        format!("{root}.py"),
        format!("{root}. trailing"),
        format!(".{root}"),
        format!("{root}~~⟧ trailing"),
    ] {
        assert_eq!(host.path_map.normalize_host_text(&lookalike), lookalike);
    }
    let unregistered = host
        ._sandbox
        .path()
        .parent()
        .unwrap()
        .join("unregistered-profile")
        .display()
        .to_string();
    assert_eq!(
        host.path_map.normalize_host_text(&unregistered),
        unregistered
    );
}

#[test]
fn c2_exact_localized_agent_install_templates_contract() {
    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    let root = host._sandbox.path().display().to_string();
    for locale in [Locale::En, Locale::ZhCn, Locale::ZhTw, Locale::Pseudo] {
        let raw = format_text(locale, "Installed the skit Agent Skill: {}", &[&root]);
        let stable = format_text(
            locale,
            "Installed the skit Agent Skill: {}",
            &[&"<profile:fixture>"],
        );
        let mut action = Action::Preferences(PreferencesAction::AgentSkillInstalled {
            message: raw.clone(),
        });
        host.path_map.project_typed_action(&mut action).unwrap();
        assert_eq!(
            serde_json::to_value(action).unwrap()["preferences"]["agent_skill_installed"]["message"],
            stable
        );

        let state = host
            .path_map
            .normalize_library_json(json!({"status": raw.clone()}))
            .unwrap();
        assert_eq!(state["status"], raw);
        let unregistered = format_text(
            locale,
            "Installed the skit Agent Skill: {}",
            &[&"/unregistered/skills/skit/SKILL.md"],
        );
        let action = host
            .path_map
            .normalize_action_json(json!({
                "preferences": {
                    "agent_skill_installed": {"message": unregistered.clone()},
                },
            }))
            .unwrap();
        assert_eq!(
            action["preferences"]["agent_skill_installed"]["message"],
            unregistered
        );
        for partial in [format!("prefix {raw}"), format!("{raw} suffix")] {
            assert!(
                host.path_map
                    .normalize_action_json(json!({
                        "preferences": {"agent_skill_installed": {"message": partial.clone()}},
                    }))
                    .is_err()
            );
            assert_eq!(
                host.path_map
                    .normalize_library_json(json!({"status": partial}))
                    .unwrap()["status"],
                partial
            );
        }
    }
}

#[test]
fn c2_session_owner_and_file_picker_pointer_contract() {
    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    let draft = write_draft_at(&host, "skit-new-session.py", b"print('session')\n", 10);
    host.path_map.refresh(&host.service).unwrap();
    let root = host._sandbox.path().to_path_buf();
    let root_text = root.display().to_string();
    let draft_text = serde_json::to_string(&draft).unwrap();
    let query = format!("{root_text}-query");
    let mut session = c2_session(&root, Some(&draft), Vec::new());

    host.path_map.project_session_value(&mut session).unwrap();

    let encoded = serde_json::to_string(&session).unwrap();
    assert!(!encoded.contains(&draft_text));
    assert!(encoded.contains("<profile:fixture>"));
    assert!(encoded.contains("<draft:0>.py"));

    assert_eq!(
        session["preferences"]["fields"]["agent_signature"][0]["base"],
        "<profile:fixture>"
    );
    assert_eq!(session["add"]["fields"]["picker_root"], "<profile:fixture>");
    assert_eq!(
        session["run_modal"]["fields"]["file"]["fields"]["contract"]["query"],
        query
    );
    assert_eq!(
        session["run_modal"]["fields"]["file"]["fields"]["explorer"]["entries"][1]["name"],
        "<draft:0>.py"
    );
    assert_eq!(
        session["run_modal"]["fields"]["file"]["fields"]["io_error"],
        "Could not read <profile:fixture>."
    );
    assert_eq!(
        session["run_modal"]["fields"]["file"]["fields"]["explorer"]["entries"][2]["entry_type"]["symlink"]
            ["target"],
        "<profile:fixture>/target.txt"
    );

    let mut token_session = c2_session(&root, None, Vec::new());
    token_session["run_modal"]["fields"]["signature"] = json!({
        "token": {
            "field": 0,
            "options": ["environment", {"fixed_directory": {"path": root}}],
        },
    });
    host.path_map
        .project_session_value(&mut token_session)
        .unwrap();
    assert_eq!(
        token_session["run_modal"]["fields"]["signature"]["token"]["options"][1]["fixed_directory"]
            ["path"],
        "<profile:fixture>"
    );
}

#[test]
fn c2_select_draft_provenance_projects_state_and_untouched_input_mirror() {
    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    let draft = write_draft_at(
        &host,
        "skit-new-provenance.py",
        b"print('provenance')\n",
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
    let action = Action::Add(AddAction::SelectDraft(0));
    let emitted = state.update(action.clone());
    let raw = draft.display().to_string();
    let mut projected_state = state.clone();
    let mut session = c2_session(
        host._sandbox.path(),
        Some(&draft),
        vec![json!({
            "id": "SourcePath",
            "state": {
                "value": raw,
                "cursor": draft.display().to_string().chars().count(),
                "yank": "",
                "last_was_cut": false,
            },
        })],
    );
    let mut recorded_action = action;
    let mut recorded_emitted = emitted;

    host.capture_checkpoint_parts(
        &mut projected_state,
        &mut session,
        CheckpointCauseProjection::Reducer {
            action: &mut recorded_action,
            emitted: &mut recorded_emitted,
        },
    )
    .unwrap();

    let projected = serde_json::to_value(projected_state).unwrap();
    let expected = "<profile:fixture>/data/.drafts/<draft:0>.py";
    assert_eq!(
        projected.pointer("/workflow/active/add/source/path"),
        Some(&json!(expected))
    );
    assert_eq!(
        session["add"]["fields"]["inputs"][0]["state"]["value"],
        expected
    );
    assert_eq!(
        session["add"]["fields"]["inputs"][0]["state"]["cursor"],
        expected.chars().count()
    );

    let user_action = Action::Add(AddAction::SetSourcePath(draft.display().to_string()));
    let user_emitted = state.update(user_action.clone());
    let mut user_state = state.clone();
    let mut user_session = c2_session(
        host._sandbox.path(),
        Some(&draft),
        vec![json!({
            "id": "SourcePath",
            "state": {
                "value": draft.display().to_string(),
                "cursor": draft.display().to_string().chars().count(),
                "yank": "",
                "last_was_cut": false,
            },
        })],
    );
    let mut recorded_user_action = user_action;
    let mut recorded_user_emitted = user_emitted;
    host.capture_checkpoint_parts(
        &mut user_state,
        &mut user_session,
        CheckpointCauseProjection::Reducer {
            action: &mut recorded_user_action,
            emitted: &mut recorded_user_emitted,
        },
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(user_state).unwrap()["workflow"]["active"]["add"]["source"]["path"],
        draft.display().to_string()
    );
    assert_eq!(
        user_session["add"]["fields"]["inputs"][0]["state"]["value"],
        draft.display().to_string()
    );
}

#[test]
fn g2c_picked_source_projects_action_state_session_and_replayed_inspection() {
    let exercise = |host: &mut RealWalkerHost| {
        let raw = host
            .external_root
            .join("nested/picked.py")
            .display()
            .to_string();
        let stable = "<profile:fixture>/external/nested/picked.py";
        let mut state = picker_add_state(host);
        let mut session = picker_tui_session(host);

        let picked_action = Action::Add(AddAction::PickedSourcePath(raw.clone()));
        let picked_effect = state.update(picked_action.clone());
        assert_eq!(picked_effect, Effect::None);
        let (picked, recorded_pick, recorded_pick_effect, projected_session) =
            capture_real_reducer_checkpoint(
                host,
                &state,
                &mut session,
                picked_action,
                picked_effect,
            );
        let recorded_pick = serde_json::to_value(recorded_pick).unwrap();
        assert_eq!(recorded_pick["add"]["picked_source_path"], stable);
        assert_eq!(
            picked.state.pointer("/workflow/active/add/source/path"),
            Some(&json!(stable))
        );
        assert_eq!(source_input_value(&projected_session), stable);
        assert!(
            !serde_json::to_string(&recorded_pick)
                .unwrap()
                .contains(&raw)
        );
        assert!(!serde_json::to_string(&picked.state).unwrap().contains(&raw));
        assert!(
            !serde_json::to_string(&projected_session)
                .unwrap()
                .contains(&raw)
        );
        assert_eq!(recorded_pick_effect, Effect::None);

        let continue_action = Action::Add(AddAction::Continue);
        let inspect = state.update(continue_action.clone());
        let (inspecting, recorded_continue, recorded_inspect, _) =
            capture_real_reducer_checkpoint(host, &state, &mut session, continue_action, inspect);
        let recorded_inspect_value = serde_json::to_value(&recorded_inspect).unwrap();
        assert_eq!(
            recorded_inspect_value["add"][0]["inspect_source"]["path"],
            stable
        );
        crate::cli::tui_real_walker::validate_reducer_action(
            &picked.state,
            &inspecting.state,
            &serde_json::to_value(&recorded_continue).unwrap(),
            &recorded_inspect_value,
        )
        .unwrap();

        json!({
            "pick": recorded_pick,
            "state": picked.state,
            "session": projected_session,
            "continue": recorded_continue,
            "inspect": recorded_inspect,
        })
    };

    let spec = picker_provenance_profile();
    let mut main = RealWalkerHost::spawn(spec.clone()).unwrap();
    let mut replay = RealWalkerHost::spawn(spec).unwrap();
    assert_eq!(exercise(&mut main), exercise(&mut replay));
}

#[test]
fn g2c_picked_source_refuses_every_non_file_and_state_action_mismatch() {
    let spec = picker_provenance_profile();
    let mut host = RealWalkerHost::spawn(spec.clone()).unwrap();
    let other = RealWalkerHost::spawn(spec).unwrap();
    let invalid = [
        host.external_root.join("unseeded.py"),
        host.external_root.join("nested"),
        host.external_root.join("linked.py"),
        host._sandbox.path().join("outside-root.py"),
        other.external_root.join("nested/picked.py"),
        PathBuf::from(format!(
            "{}/nested//picked.py",
            host.external_root.display()
        )),
        host.external_root.join("nested/./picked.py"),
    ];
    for path in invalid {
        let mut action = Action::Add(AddAction::PickedSourcePath(path.display().to_string()));
        assert!(
            host.path_map.project_typed_action(&mut action).is_err(),
            "accepted non-file picker provenance"
        );
    }

    let raw = host
        .external_root
        .join("nested/picked.py")
        .display()
        .to_string();
    let mut mismatched_value = json!({"picked_source_path": host.external_root.join("outside.sh")});
    assert!(
        host.path_map
            .project_typed_add_action(
                &AddAction::PickedSourcePath(raw.clone()),
                &mut mismatched_value,
            )
            .unwrap_err()
            .contains("does not match its typed action")
    );
    let mismatch = host.external_root.join("outside.sh").display().to_string();
    let mut state = picker_add_state(&host);
    let picked = Action::Add(AddAction::PickedSourcePath(raw));
    assert_eq!(state.update(picked.clone()), Effect::None);
    assert_eq!(
        state.update(Action::Add(AddAction::SetSourcePath(mismatch))),
        Effect::None
    );
    let mut session = picker_tui_session(&host);
    let mut projected_state = state.clone();
    let mut projected_session = render_real_session_snapshot(&state, &mut session);
    let mut recorded_pick = picked;
    let mut recorded_effect = Effect::None;
    let error = host
        .capture_checkpoint_parts(
            &mut projected_state,
            &mut projected_session,
            CheckpointCauseProjection::Reducer {
                action: &mut recorded_pick,
                emitted: &mut recorded_effect,
            },
        )
        .unwrap_err();
    assert!(error.contains("does not match the raw state"), "{error}");
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn g2c_picked_source_refuses_without_an_active_add_and_cleans_the_checkpoint() {
    for (profile_id, request) in [
        ("picked-on-library", None),
        ("picked-on-preferences", Some(HostRequest::Preferences)),
    ] {
        let parent = tempfile::TempDir::new().unwrap();
        let namespace_root = parent.path().join(STABLE_SANDBOX_NAMESPACE);
        let mut host = RealWalkerHost::spawn_stable_in(
            picker_provenance_profile(),
            safe_profile(profile_id),
            StableSandboxNamespace::explicit(namespace_root).unwrap(),
        )
        .unwrap();
        let sandbox_root = host.sandbox_root().to_path_buf();
        let mut state = host.initial_state().unwrap();
        if let Some(request) = request {
            let presented = host
                .dispatch(Effect::Open {
                    request,
                    selector: None,
                })
                .unwrap();
            assert_eq!(state.update(presented), Effect::None);
        }
        let before_state = state.clone();
        let mut session = json!({"unpublished": profile_id});
        let before_session = session.clone();
        let raw = host
            .external_root
            .join("nested/picked.py")
            .display()
            .to_string();
        let mut action = Action::Add(AddAction::PickedSourcePath(raw));
        let mut emitted = Effect::None;

        let error = host
            .capture_checkpoint_parts(
                &mut state,
                &mut session,
                CheckpointCauseProjection::Reducer {
                    action: &mut action,
                    emitted: &mut emitted,
                },
            )
            .unwrap_err();

        assert!(error.contains("has no active Add state"), "{error}");
        assert_eq!(state, before_state);
        assert_eq!(session, before_session);
        assert_eq!(emitted, Effect::None);
        assert_eq!(
            host.path_map.add_provenance,
            AddProjectionProvenance::default()
        );
        assert!(!host.adapters.pending_events().is_empty());
        let _ = host.observe(&state).unwrap();
        assert!(host.adapters.pending_events().is_empty());
        host.close().unwrap();
        assert!(!sandbox_root.exists());
    }
}

#[test]
fn g2c_manual_draft_new_add_and_exit_keep_picker_provenance_disjoint() {
    let mut host = RealWalkerHost::spawn(picker_provenance_profile()).unwrap();
    let draft = write_draft_at(&host, "skit-new-picker.py", b"print('draft')\n", 10);
    let raw = host
        .external_root
        .join("nested/picked.py")
        .display()
        .to_string();
    let mut state = picker_add_state(&host);
    let mut session = picker_tui_session(&host);

    let picked = Action::Add(AddAction::PickedSourcePath(raw.clone()));
    let picked_effect = state.update(picked.clone());
    let _ = capture_real_reducer_checkpoint(&mut host, &state, &mut session, picked, picked_effect);
    assert_eq!(
        host.path_map.add_provenance.picked_source_path,
        Some(raw.clone())
    );
    assert!(host.path_map.add_provenance.selected_source_path.is_none());

    let manual = Action::Add(AddAction::SetSourcePath(raw.clone()));
    let manual_effect = state.update(manual.clone());
    let (manual_checkpoint, recorded_manual, _, manual_session) =
        capture_real_reducer_checkpoint(&mut host, &state, &mut session, manual, manual_effect);
    assert_eq!(
        serde_json::to_value(recorded_manual).unwrap()["add"]["set_source_path"],
        raw
    );
    assert_eq!(
        manual_checkpoint
            .state
            .pointer("/workflow/active/add/source/path"),
        Some(&json!(raw))
    );
    assert_eq!(source_input_value(&manual_session), raw);
    assert!(host.path_map.add_provenance.picked_source_path.is_none());

    let lookalike = format!("{raw}-user-text");
    let manual = Action::Add(AddAction::SetSourcePath(lookalike.clone()));
    let manual_effect = state.update(manual.clone());
    let (lookalike_checkpoint, recorded_manual, _, lookalike_session) =
        capture_real_reducer_checkpoint(&mut host, &state, &mut session, manual, manual_effect);
    assert_eq!(
        serde_json::to_value(recorded_manual).unwrap()["add"]["set_source_path"],
        lookalike
    );
    assert_eq!(
        lookalike_checkpoint
            .state
            .pointer("/workflow/active/add/source/path"),
        Some(&json!(lookalike))
    );
    assert_eq!(source_input_value(&lookalike_session), lookalike);

    let picked = Action::Add(AddAction::PickedSourcePath(raw.clone()));
    let picked_effect = state.update(picked.clone());
    let _ = capture_real_reducer_checkpoint(&mut host, &state, &mut session, picked, picked_effect);
    let stale = format!("{raw}-stale-without-cause");
    assert_eq!(
        state.update(Action::Add(AddAction::SetSourcePath(stale.clone()))),
        Effect::None
    );
    let stale_observation = host.observe(&state).unwrap();
    assert_eq!(
        stale_observation
            .state
            .pointer("/workflow/active/add/source/path"),
        Some(&json!(stale))
    );
    assert!(host.path_map.add_provenance.picked_source_path.is_none());

    let picked = Action::Add(AddAction::PickedSourcePath(raw.clone()));
    let picked_effect = state.update(picked.clone());
    let _ = capture_real_reducer_checkpoint(&mut host, &state, &mut session, picked, picked_effect);
    let select = Action::Add(AddAction::SelectDraft(0));
    let select_effect = state.update(select.clone());
    let (selected, _, _, _) =
        capture_real_reducer_checkpoint(&mut host, &state, &mut session, select, select_effect);
    assert!(host.path_map.add_provenance.picked_source_path.is_none());
    assert_eq!(
        host.path_map.add_provenance.selected_source_path,
        Some(draft.clone())
    );
    assert_eq!(
        selected.state.pointer("/workflow/active/add/source/path"),
        Some(&json!("<profile:fixture>/data/.drafts/<draft:0>.py"))
    );

    let picked = Action::Add(AddAction::PickedSourcePath(raw.clone()));
    let picked_effect = state.update(picked.clone());
    let _ = capture_real_reducer_checkpoint(&mut host, &state, &mut session, picked, picked_effect);
    let new_add = Action::Present(skit_ui::Screen::Add(Box::new(AddWorkflowState::new(
        Vec::new(),
    ))));
    let new_add_effect = state.update(new_add.clone());
    let _ =
        capture_real_reducer_checkpoint(&mut host, &state, &mut session, new_add, new_add_effect);
    assert_eq!(
        host.path_map.add_provenance,
        AddProjectionProvenance::default()
    );

    let picked = Action::Add(AddAction::PickedSourcePath(raw));
    let picked_effect = state.update(picked.clone());
    let _ = capture_real_reducer_checkpoint(&mut host, &state, &mut session, picked, picked_effect);
    let leave = Action::Present(skit_ui::Screen::Library);
    let leave_effect = state.update(leave.clone());
    let _ = capture_real_reducer_checkpoint(&mut host, &state, &mut session, leave, leave_effect);
    assert_eq!(
        host.path_map.add_provenance,
        AddProjectionProvenance::default()
    );
}

#[test]
fn c2_real_add_session_requires_only_the_mirror_owned_by_its_stage() {
    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    let draft = write_draft_at(
        &host,
        "skit-new-stage-mirror.py",
        b"print('stage mirror')\n",
        10,
    );
    host.path_map.refresh(&host.service).unwrap();
    let mut state = host.initial_state().unwrap();
    assert_eq!(
        state.update(Action::Present(skit_ui::Screen::Add(Box::new(
            AddWorkflowState::new(sorted_tui_drafts(&host.roots().data)),
        )))),
        Effect::None
    );
    let mut tui_session = TuiSession::default();

    let select = Action::Add(AddAction::SelectDraft(0));
    let select_emitted = state.update(select.clone());
    let mut selected_state = state.clone();
    let mut selected_session = render_real_session_snapshot(&state, &mut tui_session);
    let mut recorded_select = select;
    let mut recorded_select_emitted = select_emitted;
    let selected = host
        .capture_checkpoint_parts(
            &mut selected_state,
            &mut selected_session,
            CheckpointCauseProjection::Reducer {
                action: &mut recorded_select,
                emitted: &mut recorded_select_emitted,
            },
        )
        .unwrap();
    assert_eq!(
        selected_session.pointer("/add/fields/signature/stage"),
        Some(&json!("source"))
    );
    assert_eq!(
        selected_session.pointer("/add/fields/inputs/0/id"),
        Some(&json!("SourcePath"))
    );

    let continue_action = Action::Add(AddAction::Continue);
    let inspect = state.update(continue_action.clone());
    let mut inspecting_state = state.clone();
    let mut inspecting_session = render_real_session_snapshot(&state, &mut tui_session);
    let mut recorded_continue = continue_action;
    let mut recorded_inspect = inspect.clone();
    let inspecting = host
        .capture_checkpoint_parts(
            &mut inspecting_state,
            &mut inspecting_session,
            CheckpointCauseProjection::Reducer {
                action: &mut recorded_continue,
                emitted: &mut recorded_inspect,
            },
        )
        .unwrap();
    crate::cli::tui_real_walker::validate_reducer_action(
        &selected.state,
        &inspecting.state,
        &serde_json::to_value(recorded_continue).unwrap(),
        &serde_json::to_value(recorded_inspect).unwrap(),
    )
    .unwrap();

    let response = host.dispatch(inspect.clone()).unwrap();
    let response_emitted = state.update(response.clone());
    let mut review_state = state.clone();
    let mut review_session = render_real_session_snapshot(&state, &mut tui_session);
    let review_inputs = review_session
        .pointer("/add/fields/inputs")
        .and_then(Value::as_array)
        .unwrap();
    assert_eq!(
        review_session.pointer("/add/fields/signature/stage"),
        Some(&json!("review"))
    );
    assert!(
        review_inputs
            .iter()
            .any(|input| input["id"] == "ReviewName")
    );
    assert!(
        review_inputs
            .iter()
            .all(|input| input["id"] != "SourcePath")
    );
    let mut recorded_request = inspect;
    let mut recorded_response = response;
    let mut recorded_response_emitted = response_emitted;
    let review = host
        .capture_checkpoint_parts(
            &mut review_state,
            &mut review_session,
            CheckpointCauseProjection::Host {
                request: &mut recorded_request,
                response: &mut recorded_response,
                emitted: &mut recorded_response_emitted,
            },
        )
        .unwrap();
    let stable = "<profile:fixture>/data/.drafts/<draft:0>.py";
    assert_eq!(
        review.state.pointer("/workflow/active/add/source/path"),
        Some(&json!(stable))
    );
    assert_eq!(
        review.state.pointer("/workflow/active/add/review/name"),
        Some(&json!("<draft:0>"))
    );
    let review_name = review_session
        .pointer("/add/fields/inputs")
        .and_then(Value::as_array)
        .unwrap()
        .iter()
        .find(|input| input["id"] == "ReviewName")
        .unwrap();
    assert_eq!(review_name["state"]["value"], "<draft:0>");
    crate::cli::tui_real_walker::validate_reducer_action(
        &inspecting.state,
        &review.state,
        &serde_json::to_value(recorded_response).unwrap(),
        &serde_json::to_value(recorded_response_emitted).unwrap(),
    )
    .unwrap();

    let save = Action::Add(AddAction::Save);
    let commit = state.update(save.clone());
    let mut saving_state = state.clone();
    let mut saving_session = render_real_session_snapshot(&state, &mut tui_session);
    let mut recorded_save = save;
    let mut recorded_commit = commit.clone();
    let saving = host
        .capture_checkpoint_parts(
            &mut saving_state,
            &mut saving_session,
            CheckpointCauseProjection::Reducer {
                action: &mut recorded_save,
                emitted: &mut recorded_commit,
            },
        )
        .unwrap();
    let committed = host.dispatch(commit.clone()).unwrap();
    let completion_effects = state.update(committed.clone());
    let mut complete_state = state.clone();
    let mut complete_session = render_real_session_snapshot(&state, &mut tui_session);
    assert_eq!(
        complete_session.pointer("/add/fields/signature/stage"),
        Some(&json!("complete"))
    );
    assert_eq!(
        complete_session.pointer("/add/fields/inputs"),
        Some(&json!([]))
    );
    let mut recorded_commit_request = commit;
    let mut recorded_committed = committed;
    let mut recorded_completion_effects = completion_effects;
    let complete = host
        .capture_checkpoint_parts(
            &mut complete_state,
            &mut complete_session,
            CheckpointCauseProjection::Host {
                request: &mut recorded_commit_request,
                response: &mut recorded_committed,
                emitted: &mut recorded_completion_effects,
            },
        )
        .unwrap();
    crate::cli::tui_real_walker::validate_reducer_action(
        &saving.state,
        &complete.state,
        &serde_json::to_value(recorded_committed).unwrap(),
        &serde_json::to_value(recorded_completion_effects).unwrap(),
    )
    .unwrap();
    assert_eq!(draft.file_name().unwrap(), "skit-new-stage-mirror.py");
}

#[test]
fn c2_real_unicode_kind_and_review_names_project_and_replay() {
    assert_real_draft_transition_projects_and_replays(
        "skit-new-界.unknown",
        b"ambiguous unicode source\n",
        "kind",
        "/workflow/active/add/kind_picker/filename",
        "skit-new-界.unknown",
        "<profile:fixture>/data/.drafts/<draft:0>.unknown",
        "<draft:0>.unknown",
    );
    assert_real_draft_transition_projects_and_replays(
        "skit-new-界.py",
        b"print('unicode review')\n",
        "review",
        "/workflow/active/add/review/name",
        "skit-new-界",
        "<profile:fixture>/data/.drafts/<draft:0>.py",
        "<draft:0>",
    );
    assert_real_draft_transition_projects_and_replays(
        "skit-x.prompt.py",
        b"print('python with prompt in its stem')\n",
        "review",
        "/workflow/active/add/review/name",
        "skit-x.prompt",
        "<profile:fixture>/data/.drafts/<draft:0>.py",
        "<draft:0>",
    );
}

#[test]
fn c2_real_prompt_suffixes_preserve_kind_and_reducer_replay() {
    for (name, stable_path) in [
        (
            "skit-new-multisuffix.prompt.md",
            "<profile:fixture>/data/.drafts/<draft:0>.prompt.md",
        ),
        (
            "skit-new-single.prompt",
            "<profile:fixture>/data/.drafts/<draft:0>.prompt",
        ),
    ] {
        let raw_name = name
            .strip_suffix(".prompt.md")
            .or_else(|| name.strip_suffix(".prompt"))
            .unwrap();
        assert_real_draft_transition_projects_and_replays(
            name,
            b"Hello {{name}}\n",
            "review",
            "/workflow/active/add/review/name",
            raw_name,
            stable_path,
            "<draft:0>",
        );
    }
}

#[test]
fn c2_highlight_draft_keeps_the_derived_source_projection_and_replays() {
    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    write_draft_at(&host, "skit-new-first.py", b"a first\n", 10);
    write_draft_at(&host, "skit-new-second.py", b"z second\n", 20);
    host.path_map.refresh(&host.service).unwrap();
    let mut state = host.initial_state().unwrap();
    assert_eq!(
        state.update(Action::Present(skit_ui::Screen::Add(Box::new(
            AddWorkflowState::new(sorted_tui_drafts(&host.roots().data)),
        )))),
        Effect::None
    );
    let mut session = TuiSession::default();
    let select = Action::Add(AddAction::SelectDraft(0));
    let select_emitted = state.update(select.clone());
    let (selected, _, _, _) =
        capture_real_reducer_checkpoint(&mut host, &state, &mut session, select, select_emitted);
    let raw_path = serde_json::to_value(&state)
        .unwrap()
        .pointer("/workflow/active/add/source/path")
        .and_then(Value::as_str)
        .unwrap()
        .to_owned();
    let stable_path = selected
        .state
        .pointer("/workflow/active/add/source/path")
        .and_then(Value::as_str)
        .unwrap()
        .to_owned();
    assert_ne!(stable_path, raw_path);

    let highlight = Action::Add(AddAction::HighlightDraft(1));
    let highlight_emitted = state.update(highlight.clone());
    let raw = serde_json::to_value(&state).unwrap();
    assert_eq!(
        raw.pointer("/workflow/active/add/source/selected_draft"),
        Some(&json!(1))
    );
    assert_eq!(
        raw.pointer("/workflow/active/add/source/path"),
        Some(&json!(raw_path))
    );
    let (highlighted, recorded_highlight, recorded_emitted, highlighted_session) =
        capture_real_reducer_checkpoint(
            &mut host,
            &state,
            &mut session,
            highlight,
            highlight_emitted,
        );

    assert_eq!(
        highlighted
            .state
            .pointer("/workflow/active/add/source/path"),
        Some(&json!(stable_path))
    );
    let source_input = highlighted_session
        .pointer("/add/fields/inputs")
        .and_then(Value::as_array)
        .unwrap()
        .iter()
        .find(|input| input["id"] == "SourcePath")
        .unwrap();
    assert_eq!(source_input["state"]["value"], stable_path);
    crate::cli::tui_real_walker::validate_reducer_action(
        &selected.state,
        &highlighted.state,
        &serde_json::to_value(recorded_highlight).unwrap(),
        &serde_json::to_value(recorded_emitted).unwrap(),
    )
    .unwrap();
}

#[test]
fn c2_source_edited_keeps_its_derived_review_name_across_source_changes() {
    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    let first = write_draft_at(&host, "skit-new-first.py", b"a first\n", 10);
    let second = write_draft_at(&host, "skit-new-second.py", b"z second\n", 20);
    host.path_map.refresh(&host.service).unwrap();
    let mut state = host.initial_state().unwrap();
    assert_eq!(
        state.update(Action::Present(skit_ui::Screen::Add(Box::new(
            AddWorkflowState::new(sorted_tui_drafts(&host.roots().data)),
        )))),
        Effect::None
    );
    let mut session = TuiSession::default();
    let first_stable_path = host.path_map.draft_paths[&first].stable_path.clone();
    let first_stable_name = review_default_name(Path::new(&first_stable_path), "python");
    let second_stable_path = host.path_map.draft_paths[&second].stable_path.clone();
    let select = Action::Add(AddAction::SelectDraft(1));
    let selected_effect = state.update(select.clone());
    let _ =
        capture_real_reducer_checkpoint(&mut host, &state, &mut session, select, selected_effect);
    let continue_action = Action::Add(AddAction::Continue);
    let inspect = state.update(continue_action.clone());
    let _ = capture_real_reducer_checkpoint(
        &mut host,
        &state,
        &mut session,
        continue_action,
        inspect.clone(),
    );
    let inspected = host.dispatch(inspect.clone()).unwrap();
    let inspected_effect = state.update(inspected.clone());
    let (review, _, _, _) = capture_real_host_checkpoint(
        &mut host,
        &state,
        &mut session,
        inspect,
        inspected,
        inspected_effect,
    );
    assert_eq!(
        review.state.pointer("/workflow/active/add/review/name"),
        Some(&json!(first_stable_name))
    );

    let edit = Action::Add(AddAction::EditSource);
    let edit_request = state.update(edit.clone());
    let (editing, _, _, _) = capture_real_reducer_checkpoint(
        &mut host,
        &state,
        &mut session,
        edit,
        edit_request.clone(),
    );
    let first_source = serde_json::to_value(&state)
        .unwrap()
        .pointer("/workflow/active/add/review/source")
        .unwrap()
        .clone();
    let same_response = source_edited_response(&edit_request, first_source.clone());
    let same_emitted = state.update(same_response.clone());
    let (same, recorded_same, recorded_same_emitted, same_session) = capture_real_host_checkpoint(
        &mut host,
        &state,
        &mut session,
        edit_request,
        same_response,
        same_emitted,
    );
    assert_eq!(
        same.state.pointer("/workflow/active/add/review/name"),
        Some(&json!(first_stable_name))
    );
    assert!(
        same_session
            .pointer("/add/fields/inputs")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .any(
                |input| input["id"] == "ReviewName" && input["state"]["value"] == first_stable_name
            )
    );
    crate::cli::tui_real_walker::validate_reducer_action(
        &editing.state,
        &same.state,
        &serde_json::to_value(recorded_same).unwrap(),
        &serde_json::to_value(recorded_same_emitted).unwrap(),
    )
    .unwrap();

    let edit = Action::Add(AddAction::EditSource);
    let edit_request = state.update(edit.clone());
    let (editing, _, _, _) = capture_real_reducer_checkpoint(
        &mut host,
        &state,
        &mut session,
        edit,
        edit_request.clone(),
    );
    let mut changed_source = first_source;
    changed_source["path"] = json!(second);
    changed_source["source_record"] = json!(second);
    changed_source["bytes"] = json!(b"z second\n");
    let changed_response = source_edited_response(&edit_request, changed_source);
    let changed_emitted = state.update(changed_response.clone());
    let raw = serde_json::to_value(&state).unwrap();
    assert_eq!(
        raw.pointer("/workflow/active/add/review/source/path"),
        Some(&json!(second))
    );
    assert_eq!(
        raw.pointer("/workflow/active/add/review/name"),
        Some(&json!("skit-new-first"))
    );
    let (changed, recorded_changed, recorded_changed_emitted, changed_session) =
        capture_real_host_checkpoint(
            &mut host,
            &state,
            &mut session,
            edit_request,
            changed_response,
            changed_emitted,
        );
    assert_eq!(
        changed
            .state
            .pointer("/workflow/active/add/review/source/path"),
        Some(&json!(second_stable_path))
    );
    assert_eq!(
        changed.state.pointer("/workflow/active/add/review/name"),
        Some(&json!(first_stable_name))
    );
    assert!(
        changed_session
            .pointer("/add/fields/inputs")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .any(
                |input| input["id"] == "ReviewName" && input["state"]["value"] == first_stable_name
            )
    );
    crate::cli::tui_real_walker::validate_reducer_action(
        &editing.state,
        &changed.state,
        &serde_json::to_value(recorded_changed).unwrap(),
        &serde_json::to_value(recorded_changed_emitted).unwrap(),
    )
    .unwrap();
    assert_eq!(first.file_name().unwrap(), "skit-new-first.py");
}
