//! Session pointer and native path projection contracts.

use super::*;

fn real_file_picker_session(host: &RealWalkerHost, draft: &Path) -> Value {
    let drafts = sorted_tui_drafts(&host.roots().data);
    let mut state = host.initial_state().unwrap();
    assert_eq!(
        state.update(Action::Present(skit_ui::Screen::Add(Box::new(
            AddWorkflowState::new(drafts),
        )))),
        Effect::None
    );
    let picker_root = draft.parent().unwrap().to_path_buf();
    let mut session = TuiSession::with_file_picker_tree(
        picker_root.clone(),
        BTreeSet::from([picker_root]),
        BTreeSet::from([draft.to_path_buf()]),
    );
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    let mut geometry = ViewGeometry::default();
    terminal
        .draw(|frame| {
            geometry = render_with_session(frame, &state, Locale::En, &mut session);
        })
        .unwrap();
    let binding = session
        .local_action_inventory()
        .actions
        .iter()
        .find(|action| action.target == LocalActionTarget::Add(AddControlId::BrowseSource))
        .unwrap()
        .keys[0];
    assert_eq!(
        session.handle_event(Event::Key(binding.event()), &state, &geometry),
        EventHandling::Consumed
    );
    serde_json::to_value(session.agent_review_snapshot().unwrap()).unwrap()
}

#[test]
fn c2_library_state_and_every_screen_pointer_contract() {
    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    let root = host._sandbox.path().display().to_string();
    let stable = "<profile:fixture>";
    let message = format!("Host failure ({root}).");
    let projected_message = format!("Host failure ({stable}).");
    host.path_map.added_at.insert(
        "volatile-added-at".to_owned(),
        "<added-at:fixture>".to_owned(),
    );
    let mut value = json!({
        "entries": [{"slug": "entry", "kind": "python", "target": root}],
        "diagnostics": [{"message": message}],
        "details": {"entry": {
            "added_at": "volatile-added-at",
            "missing_target": root,
            "last_run": {"at": root},
            "parameters": [{"value": root}],
        }},
        "status": format!("prefix Installed the skit Agent Skill: {root}"),
        "workflow": {
            "active": {"run": {
                "context": {
                    "path": {"workdir": root, "invoke_cwd": root},
                    "tokens": {"cwd": root, "home": root, "env": {"ROOT": root}},
                },
                "fields": [{
                    "control": {"text": {"value": root}},
                    "default": root,
                    "feedback": {"expanded": message},
                }],
                "initial_values": {"path": root},
                "hidden_values": {"path": root},
                "presets": {"saved": {"path": root}},
            }},
            "history": [
                {"health": {
                    "snapshot": {
                        "library_path": root,
                        "uv": {"found": root},
                        "library_size": root,
                        "issues": [{"kind": {"launch_blocked": {"reason": message}}}],
                        "diagnostics": [message],
                    },
                    "rebuilt": {"problems": [message]},
                }},
                {"preferences": {
                    "agent_skill_install": {"targets": [{"base": root}]},
                }},
                {"runners": {
                    "status": message,
                    "overlay": {"editor": {"host_error": message}},
                }},
            ],
        },
        "modal": {
            "run_file_picker": {"context": {"workdir": root, "invoke_cwd": root}},
            "run_token_menu": {"options": [{"fixed_directory": {"path": root}}]},
            "runner_editor": {"view": {"host_error": message}},
        },
    });

    value = host.path_map.normalize_library_json(value).unwrap();

    assert_eq!(value["entries"][0]["target"], stable);
    assert_eq!(value["diagnostics"][0]["message"], projected_message);
    assert_eq!(value["details"]["entry"]["added_at"], "<added-at:fixture>");
    assert_eq!(value["details"]["entry"]["missing_target"], stable);
    assert_eq!(value["details"]["entry"]["last_run"]["at"], root);
    assert_eq!(value["details"]["entry"]["parameters"][0]["value"], root);
    assert_eq!(
        value["status"],
        format!("prefix Installed the skit Agent Skill: {root}")
    );
    let run = &value["workflow"]["active"]["run"];
    for pointer in [
        "/context/path/workdir",
        "/context/path/invoke_cwd",
        "/context/tokens/cwd",
        "/context/tokens/home",
        "/context/tokens/env/ROOT",
    ] {
        assert_eq!(run.pointer(pointer), Some(&json!(stable)));
    }
    assert_eq!(run["fields"][0]["feedback"]["expanded"], projected_message);
    assert_eq!(run["fields"][0]["control"]["text"]["value"], root);
    assert_eq!(run["fields"][0]["default"], root);
    assert_eq!(run["initial_values"]["path"], root);
    assert_eq!(run["hidden_values"]["path"], root);
    assert_eq!(run["presets"]["saved"]["path"], root);
    let health = &value["workflow"]["history"][0]["health"];
    assert_eq!(health["snapshot"]["library_path"], stable);
    assert_eq!(health["snapshot"]["uv"]["found"], stable);
    assert_eq!(health["snapshot"]["library_size"], root);
    assert_eq!(
        health["snapshot"]["issues"][0]["kind"]["launch_blocked"]["reason"],
        projected_message
    );
    assert_eq!(health["snapshot"]["diagnostics"][0], projected_message);
    assert_eq!(health["rebuilt"]["problems"][0], projected_message);
    assert_eq!(
        value["workflow"]["history"][1]["preferences"]["agent_skill_install"]["targets"][0]["base"],
        stable
    );
    assert_eq!(
        value["workflow"]["history"][2]["runners"]["status"],
        message
    );
    assert_eq!(
        value["workflow"]["history"][2]["runners"]["overlay"]["editor"]["host_error"],
        projected_message
    );
    assert_eq!(
        value["modal"]["run_file_picker"]["context"]["workdir"],
        stable
    );
    assert_eq!(
        value["modal"]["run_file_picker"]["context"]["invoke_cwd"],
        stable
    );
    assert_eq!(
        value["modal"]["run_token_menu"]["options"][0]["fixed_directory"]["path"],
        stable
    );
    assert_eq!(
        value["modal"]["runner_editor"]["view"]["host_error"],
        projected_message
    );
}

#[test]
fn c2_add_action_result_and_effect_pointer_contract() {
    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    let draft_path = write_draft_at(&host, "skit-new-add-matrix.py", b"print('matrix')\n", 10);
    host.path_map.refresh(&host.service).unwrap();
    let draft = sorted_tui_drafts(&host.roots().data)
        .into_iter()
        .next()
        .unwrap();
    let source = SourceSnapshot {
        path: draft_path.clone(),
        source_record: draft_path.display().to_string(),
        bytes: b"print('matrix')\n".to_vec(),
        permissions: SourcePermissions {
            readonly: true,
            unix_mode: Some(0o640),
        },
        executable: Some(false),
        is_regular: true,
        is_directory: false,
        is_draft: true,
        identity: draft.identity.clone(),
    };
    let source_value = serde_json::to_value(&source).unwrap();
    let draft_value = serde_json::to_value(&draft).unwrap();
    let stable_path = "<profile:fixture>/data/.drafts/<draft:0>.py";
    let root = host._sandbox.path().display().to_string();
    let error = format!("Host error: {root}.");
    let stable_error = "Host error: <profile:fixture>.";

    for (tag, ok) in [
        ("source_inspected", json!({"Ok": source_value.clone()})),
        ("draft_edited", json!({"Ok": source_value.clone()})),
        ("source_edited", json!({"Ok": source_value.clone()})),
    ] {
        let action = host
            .path_map
            .normalize_action_json(json!({"add": {tag: {
                "request": 0,
                "result": ok,
            }}}))
            .unwrap();
        let source = &action["add"][tag]["result"]["Ok"];
        assert_eq!(source["path"], stable_path);
        assert_eq!(source["source_record"], stable_path);
        assert_eq!(source["permissions"]["unix_mode"], 0o640);
    }
    let changed = host
        .path_map
        .normalize_action_json(json!({"add": {"draft_deleted": {
            "request": 0,
            "result": {"Ok": {"changed": draft_value.clone()}},
        }}}))
        .unwrap();
    assert_eq!(
        changed["add"]["draft_deleted"]["result"]["Ok"]["changed"]["path"],
        stable_path
    );
    assert_eq!(
        changed["add"]["draft_deleted"]["result"]["Ok"]["changed"]["modified"],
        0
    );
    for stable_result in [
        json!({"draft_edited": {"request": 0, "result": {"Ok": null}}}),
        json!({"draft_deleted": {"request": 0, "result": {"Ok": "removed"}}}),
        json!({"draft_deleted": {"request": 0, "result": {"Ok": "already_missing"}}}),
        json!({"commit_finished": {"request": 0, "result": {"Ok": "created"}}}),
    ] {
        let expected = stable_result.clone();
        assert_eq!(
            host.path_map
                .normalize_action_json(json!({"add": stable_result}))
                .unwrap()["add"],
            expected
        );
    }

    for tag in [
        "source_inspected",
        "draft_edited",
        "draft_deleted",
        "source_edited",
        "commit_finished",
    ] {
        let action = host
            .path_map
            .normalize_action_json(json!({"add": {tag: {
                "request": 0,
                "result": {"Err": error},
            }}}))
            .unwrap();
        assert_eq!(action["add"][tag]["result"]["Err"], stable_error);
        let lookalike = format!("{root}-user");
        let action = host
            .path_map
            .normalize_action_json(json!({"add": {tag: {
                "request": 0,
                "result": {"Err": lookalike},
            }}}))
            .unwrap();
        assert_eq!(action["add"][tag]["result"]["Err"], lookalike);
    }

    let mut entry = serde_json::to_value(profile().entries[0].clone()).unwrap();
    entry["source"] = json!(draft_path);
    let mut effect: Effect = deserialize_canonical(
        json!({"add": [
            {"inspect_source": {"request": 0, "path": draft_path}},
            {"delete_draft": {"request": 0, "draft": draft_value}},
            {"edit_source": {"request": 0, "path": draft_path}},
            {"commit": {"request": 0, "entry": entry, "source": source_value}},
            {"consume_draft": source},
            {"draft_kept": draft_path},
        ]}),
        "C2 Add Effect fixture",
    )
    .unwrap();
    host.path_map.project_typed_effect(&mut effect).unwrap();
    let effect = serde_json::to_value(effect).unwrap();
    assert_eq!(effect["add"][0]["inspect_source"]["path"], stable_path);
    assert_eq!(
        effect["add"][1]["delete_draft"]["draft"]["path"],
        stable_path
    );
    assert_eq!(effect["add"][2]["edit_source"]["path"], stable_path);
    assert_eq!(
        effect["add"][3]["commit"]["source"]["source_record"],
        stable_path
    );
    assert_eq!(effect["add"][3]["commit"]["entry"]["source"], stable_path);
    assert_eq!(
        effect["add"][3]["commit"]["source"]["permissions"]["unix_mode"],
        0o640
    );
    assert_eq!(effect["add"][4]["consume_draft"]["path"], stable_path);
    assert_eq!(effect["add"][5]["draft_kept"], stable_path);

    let mismatch = format!("{}-user", draft_path.display());
    let mut entry = serde_json::to_value(profile().entries[0].clone()).unwrap();
    entry["source"] = json!(mismatch);
    let mut effect: Effect = deserialize_canonical(
        json!({"add": [{"commit": {
            "request": 0,
            "entry": entry,
            "source": serde_json::to_value(&source).unwrap(),
        }}]}),
        "C2 mismatched Commit fixture",
    )
    .unwrap();
    host.path_map.project_typed_effect(&mut effect).unwrap();
    assert_eq!(
        serde_json::to_value(effect).unwrap()["add"][0]["commit"]["entry"]["source"],
        mismatch
    );

    for raw in [
        json!({"count_run_glob": {
            "selector": "entry",
            "field": 0,
            "value": "*",
            "request": {"cwd": root, "pieces": ["*"]},
        }}),
        json!({"preferences": {"install_agent_skill": {"skills_dir": root}}}),
    ] {
        let mut effect: Effect = deserialize_canonical(raw, "C2 Effect fixture").unwrap();
        host.path_map.project_typed_effect(&mut effect).unwrap();
        let encoded = serde_json::to_string(&effect).unwrap();
        assert!(!encoded.contains(&root));
        assert!(encoded.contains("<profile:fixture>"));
    }
}

#[test]
fn c2_agent_skill_status_provenance_replays_and_user_status_stays_byte_exact() {
    for locale in [Locale::En, Locale::ZhCn, Locale::ZhTw, Locale::Pseudo] {
        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        let mut state = host.initial_state().unwrap();
        let open = host
            .dispatch(Effect::Open {
                request: HostRequest::Preferences,
                selector: None,
            })
            .unwrap();
        assert_eq!(state.update(open), Effect::None);
        let previous = host.observe(&state).unwrap().state;
        let raw = format_text(
            locale,
            "Installed the skit Agent Skill: {}",
            &[&host._sandbox.path().display()],
        );
        let stable = format_text(
            locale,
            "Installed the skit Agent Skill: {}",
            &[&"<profile:fixture>"],
        );
        let action = Action::Preferences(PreferencesAction::AgentSkillInstalled { message: raw });
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
        assert_eq!(observation.state["status"], stable);
        assert_eq!(
            serde_json::to_value(&recorded_action).unwrap()["preferences"]["agent_skill_installed"]
                ["message"],
            stable
        );
        crate::cli::tui_real_walker::validate_reducer_action(
            &previous,
            &observation.state,
            &serde_json::to_value(recorded_action).unwrap(),
            &serde_json::to_value(recorded_emitted).unwrap(),
        )
        .unwrap();
    }

    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    let mut state = host.initial_state().unwrap();
    let root = host._sandbox.path().display().to_string();
    let user_message = format!("User entry ({root}).");
    let action = Action::SetStatus(user_message.clone());
    let emitted = state.update(action.clone());
    let mut session = c2_session(Path::new("/unregistered/session"), None, Vec::new());
    let mut recorded_action = action;
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
    assert_eq!(observation.state["status"], user_message);
    assert_eq!(
        serde_json::to_value(recorded_action).unwrap()["set_status"],
        user_message
    );
}

#[cfg(target_os = "linux")]
#[test]
fn c2_session_native_paths_project_only_canonical_registered_objects() {
    use std::os::unix::ffi::{OsStrExt as _, OsStringExt as _};

    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    let registered = host
        ._sandbox
        .path()
        .join(std::ffi::OsString::from_vec(b"native-\xff".to_vec()));
    fs::write(&registered, b"native").unwrap();
    let registered_value = json!({"unix_bytes": registered.as_os_str().as_bytes()});
    let outside = PathBuf::from(std::ffi::OsString::from_vec(
        b"/outside/native-\xff".to_vec(),
    ));
    let outside_value = json!({"unix_bytes": outside.as_os_str().as_bytes()});
    let mut session = c2_session(host._sandbox.path(), None, Vec::new());
    session["preferences"]["fields"]["agent_signature"][0]["base"] = registered_value;
    session["file_picker_source"]["fields"]["files"][0] = outside_value.clone();

    host.path_map.project_session_value(&mut session).unwrap();

    assert_eq!(
        session["preferences"]["fields"]["agent_signature"][0]["base"],
        "<profile:fixture>/native-\\xff"
    );
    assert_eq!(
        session["file_picker_source"]["fields"]["files"][0],
        outside_value
    );
}

#[cfg(unix)]
#[test]
fn c2_session_native_unix_path_values_refuse_malformed_shapes() {
    let host = RealWalkerHost::spawn(profile()).unwrap();

    for malformed in [
        json!({"windows_wide": [65]}),
        json!({"other": []}),
        json!({"unix_bytes": "not-an-array"}),
        json!({"unix_bytes": [65], "extra": true}),
        json!({"unix_bytes": [256]}),
        json!({"unix_bytes": ["x"]}),
        json!({"unix_bytes": []}),
        json!({"unix_bytes": [97]}),
        json!(7),
    ] {
        let mut malformed = malformed;
        assert!(
            host.path_map
                .normalize_session_path_value(&mut malformed)
                .is_err()
        );
    }
}

#[test]
fn c2_wide_unit_escape_is_injective() {
    assert_eq!(escaped_wide_units(&[0xd800]), "\\ud800");
    assert_eq!(escaped_wide_units(&[0xd801]), "\\ud801");
    assert_ne!(escaped_wide_units(&[0xd800]), escaped_wide_units(&[0xd801]));
}

#[cfg(target_os = "linux")]
#[test]
fn c2_real_non_utf_file_picker_entry_projects_its_lossy_name_and_native_path() {
    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    let draft = write_non_utf_draft_at(&host);
    host.path_map.refresh(&host.service).unwrap();
    let mut session = real_file_picker_session(&host, &draft);
    let lossy = draft.file_name().unwrap().to_string_lossy();
    let raw_entry = session
        .pointer("/add_overlay/fields/session/fields/explorer/entries")
        .and_then(Value::as_array)
        .unwrap()
        .iter()
        .find(|entry| entry["name"] == lossy.as_ref())
        .unwrap();
    assert!(raw_entry["path"].get("unix_bytes").is_some());

    host.path_map.project_session_value(&mut session).unwrap();

    let projected_entry = session
        .pointer("/add_overlay/fields/session/fields/explorer/entries")
        .and_then(Value::as_array)
        .unwrap()
        .iter()
        .find(|entry| entry["name"] == "<draft:0>.unknown")
        .unwrap();
    assert_eq!(
        projected_entry["path"],
        "<profile:fixture>/data/.drafts/<draft:0>.unknown"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn c2_real_non_utf_file_picker_entry_refuses_a_lossy_name_lookalike() {
    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    let draft = write_non_utf_draft_at(&host);
    host.path_map.refresh(&host.service).unwrap();
    let mut session = real_file_picker_session(&host, &draft);
    let lossy = draft.file_name().unwrap().to_string_lossy();
    let entry = session
        .pointer_mut("/add_overlay/fields/session/fields/explorer/entries")
        .and_then(Value::as_array_mut)
        .unwrap()
        .iter_mut()
        .find(|entry| entry["name"] == lossy.as_ref())
        .unwrap();
    entry["name"] = json!(format!("{lossy}-lookalike"));

    assert!(host.path_map.project_session_value(&mut session).is_err());
}

#[test]
fn c2_real_unicode_file_picker_entry_projects_and_refuses_a_name_mismatch() {
    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    let draft = write_unicode_draft_at(&host, "skit-new-界.py", b"print('unicode picker')\n");
    host.path_map.refresh(&host.service).unwrap();
    let mut session = real_file_picker_session(&host, &draft);
    let raw_entry = session
        .pointer("/add_overlay/fields/session/fields/explorer/entries")
        .and_then(Value::as_array)
        .unwrap()
        .iter()
        .find(|entry| entry["name"] == "skit-new-界.py")
        .unwrap();
    assert_eq!(raw_entry["path"], draft.display().to_string());

    host.path_map.project_session_value(&mut session).unwrap();

    let projected_entry = session
        .pointer("/add_overlay/fields/session/fields/explorer/entries")
        .and_then(Value::as_array)
        .unwrap()
        .iter()
        .find(|entry| entry["name"] == "<draft:0>.py")
        .unwrap();
    assert_eq!(
        projected_entry["path"],
        "<profile:fixture>/data/.drafts/<draft:0>.py"
    );

    let mut mismatch = real_file_picker_session(&host, &draft);
    let entry = mismatch
        .pointer_mut("/add_overlay/fields/session/fields/explorer/entries")
        .and_then(Value::as_array_mut)
        .unwrap()
        .iter_mut()
        .find(|entry| entry["name"] == "skit-new-界.py")
        .unwrap();
    entry["name"] = json!("skit-new-\\xe7\\x95\\x8c.py");
    assert!(host.path_map.project_session_value(&mut mismatch).is_err());
}

#[cfg(target_os = "linux")]
#[test]
fn c2_non_utf_add_kind_and_review_stop_at_the_typed_serde_boundary() {
    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    let draft = write_non_utf_draft_at(&host);
    host.path_map.refresh(&host.service).unwrap();
    let mut workflow = AddWorkflowState::new(Vec::new());
    assert!(
        workflow
            .reduce(AddAction::SetSourcePath("placeholder".to_owned()))
            .is_empty()
    );
    let mut effects = workflow.reduce(AddAction::Continue);
    assert!(matches!(
        effects.as_slice(),
        [skit_ui::AddEffect::InspectSource { .. }]
    ));
    if let skit_ui::AddEffect::InspectSource { path, .. } = &mut effects[0] {
        *path = draft;
    }
    let response = host.dispatch(Effect::Add(effects)).unwrap();
    assert!(matches!(
        &response,
        Action::Add(AddAction::SourceInspected { result: Ok(_), .. })
    ));
    let action = expect_add_action(response.clone());
    assert!(workflow.reduce(action).is_empty());
    assert_eq!(workflow.kind_picker().unwrap().filename(), "");

    let mut projected = response;
    let error = host
        .path_map
        .project_typed_action(&mut projected)
        .unwrap_err();
    assert!(error.contains("path contains invalid UTF-8 characters"));

    assert!(
        workflow
            .reduce(AddAction::PickKind(Some(KnownEntryKind::Python)))
            .is_empty()
    );
    assert_eq!(workflow.review().unwrap().name(), "entry");
    assert!(serde_json::to_value(workflow).is_err());
}

#[cfg(windows)]
#[test]
fn c2_windows_session_native_paths_project_only_canonical_registered_objects() {
    use std::os::windows::ffi::{OsStrExt as _, OsStringExt as _};

    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    let registered = host
        ._sandbox
        .path()
        .join(std::ffi::OsString::from_wide(&[0xd800]));
    let registered_value = json!({
        "windows_wide": registered.as_os_str().encode_wide().collect::<Vec<_>>(),
    });
    let outside = PathBuf::from(std::ffi::OsString::from_wide(&[0xd801]));
    let outside_value = json!({
        "windows_wide": outside.as_os_str().encode_wide().collect::<Vec<_>>(),
    });
    let mut session = c2_session(host._sandbox.path(), None, Vec::new());
    session["preferences"]["fields"]["agent_signature"][0]["base"] = registered_value;
    session["file_picker_source"]["fields"]["files"][0] = outside_value.clone();

    host.path_map.project_session_value(&mut session).unwrap();

    assert_eq!(
        session["preferences"]["fields"]["agent_signature"][0]["base"],
        format!(
            "<profile:fixture>/{}",
            escaped_os(registered.file_name().unwrap())
        )
    );
    assert_eq!(
        session["file_picker_source"]["fields"]["files"][0],
        outside_value
    );
    let sibling = host
        ._sandbox
        .path()
        .join(std::ffi::OsString::from_wide(&[0xd801]));
    let mut first = json!({
        "windows_wide": registered.as_os_str().encode_wide().collect::<Vec<_>>(),
    });
    let mut second = json!({
        "windows_wide": sibling.as_os_str().encode_wide().collect::<Vec<_>>(),
    });
    host.path_map
        .normalize_session_path_value(&mut first)
        .unwrap();
    host.path_map
        .normalize_session_path_value(&mut second)
        .unwrap();
    assert_ne!(first, second);
    for malformed in [
        json!({"unix_bytes": [65]}),
        json!({"other": []}),
        json!({"windows_wide": "not-an-array"}),
        json!({"windows_wide": [65], "extra": true}),
        json!({"windows_wide": [65536]}),
        json!({"windows_wide": ["x"]}),
        json!({"windows_wide": []}),
        json!({"windows_wide": [65]}),
        json!(7),
    ] {
        let mut malformed = malformed;
        assert!(
            host.path_map
                .normalize_session_path_value(&mut malformed)
                .is_err()
        );
    }
}
