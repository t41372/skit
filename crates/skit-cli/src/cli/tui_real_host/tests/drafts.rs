//! Draft scan, editor write, and quarantine contracts.

use super::*;

#[test]
fn real_host_refuses_an_unscanned_draft_in_state_and_action() {
    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    let draft = skit_ui::DraftSummary {
        path: host.roots().data.join("unscanned-draft.py"),
        modified: 100,
        identity: None,
        permissions: SourcePermissions::default(),
        content_hash: None,
    };
    let action = Action::Present(skit_ui::Screen::Add(Box::new(AddWorkflowState::new(vec![
        draft,
    ]))));
    assert_eq!(
        host.canonical_action(&action).unwrap_err(),
        "the sorted scan did not assign a rank to this draft modified value: 100"
    );

    let mut state = host.initial_state().unwrap();
    assert_eq!(state.update(action), Effect::None);
    assert_eq!(
        host.observe(&state).unwrap_err(),
        "the sorted scan did not assign a rank to this draft modified value: 100"
    );
}

#[test]
fn real_host_refuses_drafts_with_equal_modified_times() {
    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    write_draft_at(&host, "skit-new-a.py", b"a\n", 10);
    write_draft_at(&host, "skit-new-b.py", b"b\n", 10);

    assert_eq!(
        host.observe(&host.initial_state().unwrap()).map(|_| ()),
        Err("rank scan contains duplicate value 10000000000".to_owned())
    );
}

#[test]
fn real_host_refuses_a_new_draft_with_an_older_modified_time() {
    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    write_draft_at(&host, "skit-new-later.py", b"later\n", 20);
    let state = host.initial_state().unwrap();
    host.observe(&state).unwrap();
    write_draft_at(&host, "skit-new-earlier.py", b"earlier\n", 10);

    assert_eq!(
        host.observe(&state).map(|_| ()),
        Err("rank value 10000000000 does not exceed the current maximum 20000000000".to_owned())
    );
}

#[test]
fn real_host_two_drafts_use_sorted_paths_and_ascending_modified_ranks() {
    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    write_draft_at(&host, "skit-new-z.py", b"z\n", 20);
    write_draft_at(&host, "skit-new-a.py", b"a\n", 10);
    let mut state = host.initial_state().unwrap();

    let observation = host.observe(&state).unwrap();
    assert_eq!(
        observation
            .drafts
            .as_array()
            .unwrap()
            .iter()
            .map(|draft| (&draft["path"], &draft["modified"]))
            .collect::<Vec<_>>(),
        vec![
            (
                &json!("<profile:fixture>/data/.drafts/<draft:0>.py"),
                &json!(0)
            ),
            (
                &json!("<profile:fixture>/data/.drafts/<draft:1>.py"),
                &json!(1)
            ),
        ]
    );

    let action = host
        .dispatch(Effect::Open {
            request: HostRequest::Add,
            selector: None,
        })
        .unwrap();
    assert_eq!(state.update(action), Effect::None);
    let state = host.observe(&state).unwrap().state;
    let drafts = state
        .pointer("/workflow/active/add/source/drafts")
        .and_then(Value::as_array)
        .unwrap();
    assert_eq!(
        drafts
            .iter()
            .map(|draft| (&draft["path"], &draft["modified"]))
            .collect::<Vec<_>>(),
        vec![
            (
                &json!("<profile:fixture>/data/.drafts/<draft:1>.py"),
                &json!(1)
            ),
            (
                &json!("<profile:fixture>/data/.drafts/<draft:0>.py"),
                &json!(0)
            ),
        ]
    );
}

#[test]
fn real_host_copy_mode_settings_normalizes_the_source_path() {
    let mut spec = profile();
    let mut request = spec.external_references[0].request.clone();
    request.name = "Copied source".to_owned();
    request.mode = StorageMode::Copy;
    spec.external_references.push(WalkerExternalReferenceSeed {
        request,
        source: PathBuf::from("outside.sh"),
    });
    let mut host = RealWalkerHost::spawn(spec).unwrap();
    let mut state = host.initial_state().unwrap();
    let action = host
        .dispatch(Effect::Open {
            request: HostRequest::Settings,
            selector: Some("Copied source".to_owned()),
        })
        .unwrap();
    assert!(matches!(
        &action,
        Action::Present(skit_ui::Screen::Settings(_))
    ));
    assert_eq!(state.update(action), Effect::None);

    let value = host.observe(&state).unwrap().state;
    let encoded = serde_json::to_string(&value).unwrap();
    assert!(!encoded.contains(&host.external_root.display().to_string()));
    assert!(encoded.contains("<profile:fixture>/external/outside.sh"));
    let parsed = crate::cli::tui_real_walker::parse_library_state(&value).unwrap();
    assert_eq!(serde_json::to_value(parsed).unwrap(), value);
}

#[test]
fn draft_paths_timestamps_and_source_identities_are_artifact_only_values() {
    let mut first = RealWalkerHost::spawn(profile()).unwrap();
    let mut second = RealWalkerHost::spawn(profile()).unwrap();
    for (host, name) in [
        (&first, "skit-new-random-a.py"),
        (&second, "skit-new-random-b.py"),
    ] {
        let drafts = crate::cli::create_owned_drafts_dir(&host.roots().data).unwrap();
        fs::write(drafts.join(name), b"print('draft')\n").unwrap();
    }

    let first_observation = first.observe(&first.initial_state().unwrap()).unwrap();
    let second_observation = second.observe(&second.initial_state().unwrap()).unwrap();

    assert_eq!(first_observation.drafts, second_observation.drafts);
    assert_eq!(
        first_observation
            .tree
            .iter()
            .filter(|row| row.path.contains("<draft:"))
            .collect::<Vec<_>>(),
        second_observation
            .tree
            .iter()
            .filter(|row| row.path.contains("<draft:"))
            .collect::<Vec<_>>()
    );
    assert_identity_sentinel(&first_observation.drafts[0]["identity"], 0, 0);
    assert_eq!(first_observation.drafts[0]["modified"], 0);
    let encoded = first_observation.drafts.to_string();
    assert!(encoded.contains("<draft:0>.py"));
    assert!(!encoded.contains("skit-new-random-a"));
    let raw_draft = sorted_tui_drafts(first.service.repository().data_dir())
        .into_iter()
        .next()
        .unwrap();
    let tree_draft = first_observation
        .tree
        .iter()
        .find(|row| row.path == "data/.drafts/<draft:0>.py")
        .unwrap();
    assert_eq!(
        tree_draft.mode,
        portable_mode(&fs::symlink_metadata(raw_draft.path).unwrap())
    );
}

#[test]
fn removed_authored_draft_keeps_a_stable_editor_transcript_path() {
    let mut first = RealWalkerHost::spawn(profile()).unwrap();
    let mut second = RealWalkerHost::spawn(profile()).unwrap();
    let author = |host: &mut RealWalkerHost| {
        let mut workflow = AddWorkflowState::new(Vec::new());
        host.clear_transcript();
        let action = host
            .dispatch(Effect::Add(
                workflow.reduce(AddAction::NewDraft(DraftKind::Script)),
            ))
            .unwrap();
        assert!(matches!(
            action,
            Action::Add(AddAction::DraftEdited {
                result: Ok(None),
                ..
            })
        ));
        host.observe(&host.initial_state().unwrap()).unwrap()
    };

    let first_observation = author(&mut first);
    let second_observation = author(&mut second);
    assert_eq!(first_observation, second_observation);
    let encoded = serde_json::to_string(&first_observation.transcript).unwrap();
    assert!(encoded.contains("<draft:0>.py"));
    assert!(!encoded.contains("skit-new-"));
    assert!(!encoded.contains(&first._sandbox.path().display().to_string()));
    assert!(!encoded.contains(&second._sandbox.path().display().to_string()));
}

#[test]
fn queued_editor_write_records_its_filesystem_error() {
    let host = RealWalkerHost::spawn(profile()).unwrap();
    host.adapters
        .editor_write_queue
        .borrow_mut()
        .push_back(b"edited".to_vec());
    host.clear_transcript();
    let result = crate::cli::tui_host::EditorLauncher::launch(
        &host.adapters,
        &["editor".to_owned()],
        host._sandbox.path(),
    );
    let error = result.unwrap_err();
    let events = host.adapters.events.borrow();
    let expected = serde_json::json!({
        "error": {"kind": format!("{:?}", error.kind()), "reason": error.to_string()}
    });
    assert!(matches!(
        events.last(),
        Some(PortEvent::Editor { outcome, .. }) if outcome == &expected
    ));
}

#[test]
fn authored_draft_editor_write_keeps_the_deterministic_private_file() {
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt as _;

    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    host.adapters
        .editor_script
        .replace(EditorScript::Write(b"print('kept')\n".to_vec()));
    let mut workflow = AddWorkflowState::new(Vec::new());
    host.clear_transcript();

    let action = host
        .dispatch(Effect::Add(
            workflow.reduce(AddAction::NewDraft(DraftKind::Script)),
        ))
        .unwrap();
    let source = expect_authored_draft_source(action);

    assert_eq!(source.path.file_name().unwrap(), "skit-new-000000.py");
    assert_eq!(fs::read(&source.path).unwrap(), b"print('kept')\n");
    #[cfg(unix)]
    assert_eq!(
        fs::metadata(&source.path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let observation = host.observe(&host.initial_state().unwrap()).unwrap();
    let draft = observation
        .tree
        .iter()
        .find(|row| row.path == "data/.drafts/<draft:0>.py")
        .unwrap();
    #[cfg(unix)]
    assert_eq!(draft.mode, Some(0o600));
    #[cfg(not(unix))]
    assert_eq!(draft.mode, None);
    let mut ordered = Vec::new();
    for event in &observation.transcript {
        if event.get("allocation").is_some() || event.get("editor").is_some() {
            ordered.push(event);
        }
    }
    assert_eq!(ordered.len(), 2);
    assert_eq!(
        ordered[0]["allocation"]["path"],
        "<profile:fixture>/data/.drafts/<draft:0>.py"
    );
    assert_eq!(
        ordered[1]["path"],
        "<profile:fixture>/data/.drafts/<draft:0>.py"
    );
}

#[test]
fn changed_draft_restore_removes_its_deterministic_quarantine() {
    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    host.adapters
        .editor_script
        .replace(EditorScript::Write(b"print('before')\n".to_vec()));
    let mut workflow = AddWorkflowState::new(Vec::new());
    let action = host
        .dispatch(Effect::Add(
            workflow.reduce(AddAction::NewDraft(DraftKind::Script)),
        ))
        .unwrap();
    let source = expect_authored_draft_source(action);
    fs::write(&source.path, b"print('changed')\n").unwrap();
    host.clear_transcript();

    let outcome = crate::cli::consume_owned_draft_with(
        &host.roots().data,
        &source,
        &host.adapters,
        |_, _| {},
    )
    .unwrap();

    assert_eq!(outcome, crate::cli::DraftConsumeOutcome::Changed);
    assert_eq!(fs::read(&source.path).unwrap(), b"print('changed')\n");
    let drafts = source.path.parent().unwrap();
    assert!(fs::read_dir(drafts).unwrap().all(|item| {
        !item
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".skit-quarantine-")
    }));
    let transcript = host.adapters.transcript(&mut host.path_map);
    assert_eq!(
        events_with_keys(&transcript, &["allocation"]),
        serde_json::from_str::<Vec<Value>>(
            r#"[{
                    "allocation": {
                        "purpose": "draft_quarantine",
                        "attempt": 0,
                        "location": {
                            "kind": "directory",
                            "path": "<profile:fixture>/data/drafts"
                        },
                        "path": "<profile:fixture>/data/drafts/<quarantine:0>",
                        "outcome": {"accepted": true}
                    }
                }]"#,
        )
        .unwrap()
    );
}

#[test]
fn failed_changed_draft_restore_retains_the_deterministic_quarantine() {
    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    host.adapters
        .editor_script
        .replace(EditorScript::Write(b"print('before')\n".to_vec()));
    let mut workflow = AddWorkflowState::new(Vec::new());
    let action = host
        .dispatch(Effect::Add(
            workflow.reduce(AddAction::NewDraft(DraftKind::Script)),
        ))
        .unwrap();
    let source = expect_authored_draft_source(action);
    fs::write(&source.path, b"print('changed')\n").unwrap();
    host.clear_transcript();

    let error = crate::cli::consume_owned_draft_with(
        &host.roots().data,
        &source,
        &host.adapters,
        |point, _| {
            if point == crate::cli::DraftConsumeTestPoint::BeforeRestore {
                fs::write(&source.path, b"replacement").unwrap();
            }
        },
    )
    .unwrap_err();
    let quarantine = source
        .path
        .parent()
        .unwrap()
        .join(".skit-quarantine-000000/draft");

    assert!(
        error
            .to_string()
            .contains("could not restore quarantined draft")
    );
    assert_eq!(fs::read(&source.path).unwrap(), b"replacement");
    assert_eq!(fs::read(quarantine).unwrap(), b"print('changed')\n");
    let transcript = host.adapters.transcript(&mut host.path_map);
    assert_eq!(
        events_with_keys(&transcript, &["allocation"]),
        vec![json!({ "allocation": {
            "purpose": "draft_quarantine",
            "attempt": 0,
            "location": {
                "kind": "directory",
                "path": "<profile:fixture>/data/drafts",
            },
            "path": "<profile:fixture>/data/drafts/<quarantine:0>",
            "outcome": { "accepted": true },
        }})]
    );
}

/// Each accepted allocator boundary enters the path map with one typed sentinel.
///
/// The three boundaries are the authored draft, the draft quarantine directory, and the
/// injected source. The leak oracle refuses every raw allocator
/// name, so a boundary without a sentinel cannot reach an installed corpus.
#[test]
fn every_accepted_allocation_purpose_projects_one_typed_sentinel() {
    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    let data = host.roots().data.clone();
    let boundaries = [
        (
            AllocationPurpose::TemporaryFile(TemporaryFilePurpose::AuthoredDraft),
            data.join("drafts/skit-new-0000ab.py"),
            "<profile:fixture>/data/.drafts/<draft:0>.py",
        ),
        (
            AllocationPurpose::PrivateDirectory(PrivateDirectoryPurpose::DraftQuarantine),
            data.join("drafts/.skit-quarantine-0000cd"),
            "<profile:fixture>/data/drafts/<quarantine:0>",
        ),
        (
            AllocationPurpose::TemporaryFile(TemporaryFilePurpose::InjectedSource),
            host.roots().state.join(".injected-0000ef.sh"),
            "<profile:fixture>/state/<injected-source:1>.sh",
        ),
    ];

    for (purpose, path, expected) in &boundaries {
        host.path_map
            .register_event_artifacts(&PortEvent::Allocation {
                purpose: *purpose,
                attempt: 0,
                location: AllocationLocation::Directory(path.parent().unwrap().to_path_buf()),
                path: Some(path.clone()),
                outcome: AllocationOutcome::Accepted,
            });
        assert_eq!(host.path_map.normalize_path(path), *expected);
        assert_eq!(
            host.path_map
                .normalize_host_text(&path.display().to_string()),
            *expected
        );
    }
}

#[test]
fn prompt_edit_drives_editor_terminal_output_and_raw_editor_failure() {
    let mut spec = profile();
    spec.settings
        .insert("editor".to_owned(), "walker-editor --wait".to_owned());
    spec.external.push(WalkerExternalSeed::File {
        path: PathBuf::from("prompt.md"),
        bytes: b"Review {{fresh}}\n".to_vec(),
        readonly: false,
        unix_mode: 0o640,
    });
    spec.external_references.push(WalkerExternalReferenceSeed {
        request: CreateEntry {
            name: "Editable prompt".to_owned(),
            kind: EntryKind::parse("prompt").unwrap(),
            mode: StorageMode::Reference,
            source: String::new(),
            workdir: "origin".to_owned(),
            description: String::new(),
            payload: Some(EntryPayload {
                bytes: b"Review {{fresh}}\n".to_vec(),
                stored_name: None,
                permissions: SourcePermissions::default(),
            }),
            settings: EntrySettings {
                interpolate: true,
                ..EntrySettings::default()
            },
        },
        source: PathBuf::from("prompt.md"),
    });
    let mut success = RealWalkerHost::spawn(spec.clone()).unwrap();
    success.adapters.stdin_terminal.set(true);
    success.adapters.stdout_terminal.set(false);
    success.clear_transcript();

    assert!(matches!(
        success
            .dispatch(Effect::Edit {
                selector: "Editable prompt".to_owned(),
            })
            .unwrap(),
        Action::Complete { .. }
    ));
    let observation = success.observe(&success.initial_state().unwrap()).unwrap();
    assert_eq!(
        events_with_keys(
            &observation.transcript,
            &["environment_variable", "output", "editor", "terminal"]
        ),
        vec![
            json!({ "environment_variable": "VISUAL", "result": null }),
            json!({ "environment_variable": "EDITOR", "result": null }),
            json!({
                "output": "detail",
                "text": "Editing the original file (reference mode): <profile:fixture>/external/prompt.md",
            }),
            json!({
                "editor": ["walker-editor", "--wait"],
                "path": "<profile:fixture>/external/prompt.md",
                "outcome": { "ok": true },
            }),
            json!({ "output": "success", "text": "Saved Editable prompt." }),
            json!({ "terminal": "stdin", "result": true }),
            json!({ "terminal": "stdout", "result": false }),
            json!({
                "output": "plain",
                "text": "Detected but not yet managed: fresh (use --add to manage them)",
            }),
        ]
    );

    let mut failure = RealWalkerHost::spawn(spec).unwrap();
    let source = failure.external_root.join("prompt.md");
    let before = fs::read(&source).unwrap();
    failure
        .adapters
        .editor_script
        .replace(EditorScript::Failure {
            kind: io::ErrorKind::PermissionDenied,
            reason: "walker editor denied".to_owned(),
        });
    failure.clear_transcript();
    assert!(
        failure
            .dispatch(Effect::Edit {
                selector: "Editable prompt".to_owned(),
            })
            .unwrap_err()
            .to_string()
            .contains("walker editor denied")
    );
    assert_eq!(fs::read(&source).unwrap(), before);
    let failed = failure.observe(&failure.initial_state().unwrap()).unwrap();
    assert_eq!(
        events_with_keys(
            &failed.transcript,
            &["environment_variable", "output", "editor", "terminal"]
        ),
        vec![
            json!({ "environment_variable": "VISUAL", "result": null }),
            json!({ "environment_variable": "EDITOR", "result": null }),
            json!({
                "output": "detail",
                "text": "Editing the original file (reference mode): <profile:fixture>/external/prompt.md",
            }),
            json!({
                "editor": ["walker-editor", "--wait"],
                "path": "<profile:fixture>/external/prompt.md",
                "outcome": { "error": {
                    "kind": "PermissionDenied",
                    "reason": "walker editor denied",
                }},
            }),
        ]
    );
}
