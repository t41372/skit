//! Recorded port transcript contracts: preferences, launch, dependencies, injection, and uv.

use super::*;

fn events_with_parsed_launch_displays(transcript: &[Value], keys: &[&str]) -> Vec<Value> {
    let mut events = events_with_keys(transcript, keys);
    for event in &mut events {
        if let Some(launch) = event.get_mut("launch") {
            let display = launch["display"].as_str().unwrap();
            launch["display"] = json!(shlex::split(display).unwrap());
        }
    }
    events
}

#[test]
fn preferences_drive_locale_file_discovery_install_and_raw_checkpoint_ports() {
    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    host.adapters.variables.remove("LANG");
    host.adapters.system_locale.set(Locale::ZhCn);
    let home = host.roots().home.as_ref().unwrap();
    fs::write(home.join("bash"), b"#!/bin/sh\n").unwrap();
    fs::create_dir_all(home.join(".codex/skills")).unwrap();
    fs::create_dir_all(host.roots().cwd.join(".agents/skills")).unwrap();
    host.clear_transcript();

    assert!(matches!(
        host.dispatch(Effect::Preferences(PreferencesEffect::Save(
            PreferencesChangeSet {
                settings: BTreeMap::from([("shell.bash_path".to_owned(), "~/bash".to_owned(),)]),
            },
        )))
        .unwrap(),
        Action::PreferencesSaved { .. }
    ));
    let saved = host.observe(&host.initial_state().unwrap()).unwrap();
    assert_eq!(
        events_with_keys(
            &saved.transcript,
            &[
                "environment_variable",
                "system_locale",
                "platform",
                "preference"
            ]
        ),
        vec![
            json!({ "environment_variable": "SKIT_LANG", "result": null }),
            json!({ "environment_variable": "LC_ALL", "result": null }),
            json!({ "environment_variable": "LC_MESSAGES", "result": null }),
            json!({ "environment_variable": "LANG", "result": null }),
            json!({ "system_locale": "ZhCn" }),
            json!({ "platform": "Other" }),
            json!({
                "preference": "is_file",
                "path": "<profile:fixture>/home/bash",
                "outcome": true,
            }),
        ]
    );
    let config_before_refusal = fs::read(host.roots().config.join("config.toml")).unwrap();
    assert!(matches!(
        host.dispatch(Effect::Preferences(PreferencesEffect::Save(
            PreferencesChangeSet {
                settings: BTreeMap::from([("shell.bash_path".to_owned(), "~/.codex".to_owned(),)]),
            },
        )))
        .unwrap(),
        Action::Preferences(_)
    ));
    assert_eq!(
        fs::read(host.roots().config.join("config.toml")).unwrap(),
        config_before_refusal
    );
    let refused = host.observe(&host.initial_state().unwrap()).unwrap();
    assert_eq!(
        events_with_keys(
            &refused.transcript,
            &[
                "environment_variable",
                "system_locale",
                "platform",
                "preference"
            ]
        ),
        vec![
            json!({ "environment_variable": "SKIT_LANG", "result": null }),
            json!({ "environment_variable": "LC_ALL", "result": null }),
            json!({ "environment_variable": "LC_MESSAGES", "result": null }),
            json!({ "environment_variable": "LANG", "result": null }),
            json!({ "system_locale": "ZhCn" }),
            json!({ "platform": "Other" }),
            json!({
                "preference": "is_file",
                "path": "<profile:fixture>/home/.codex",
                "outcome": false,
            }),
        ]
    );
    assert!(matches!(
        host.dispatch(Effect::Preferences(
            PreferencesEffect::DiscoverAgentSkillTargets,
        ))
        .unwrap(),
        Action::Preferences(skit_ui::PreferencesAction::PresentAgentSkillTargets(_))
    ));
    let discovered = host.observe(&host.initial_state().unwrap()).unwrap();
    assert_eq!(
        events_with_keys(
            &discovered.transcript,
            &[
                "environment_variable",
                "system_locale",
                "platform",
                "preference"
            ]
        ),
        vec![
            json!({ "environment_variable": "SKIT_LANG", "result": null }),
            json!({ "environment_variable": "LC_ALL", "result": null }),
            json!({ "environment_variable": "LC_MESSAGES", "result": null }),
            json!({ "environment_variable": "LANG", "result": null }),
            json!({ "system_locale": "ZhCn" }),
            json!({ "platform": "Other" }),
            json!({ "preference": "is_dir", "path": "<profile:fixture>/home/.claude", "outcome": false }),
            json!({ "preference": "is_dir", "path": "<profile:fixture>/home/.codex", "outcome": true }),
            json!({ "preference": "is_dir", "path": "<profile:fixture>/cwd/.claude", "outcome": false }),
            json!({ "preference": "is_dir", "path": "<profile:fixture>/cwd/.codex", "outcome": false }),
            json!({ "preference": "is_dir", "path": "<profile:fixture>/cwd/.agents", "outcome": true }),
        ]
    );
    let install = host.roots().cwd.join(".agents/skills");
    assert!(matches!(
        host.dispatch(Effect::Preferences(PreferencesEffect::InstallAgentSkill {
            skills_dir: install.clone(),
        },))
            .unwrap(),
        Action::Preferences(skit_ui::PreferencesAction::AgentSkillInstalled { .. })
    ));
    assert_eq!(
        fs::read(install.join("skit/SKILL.md")).unwrap(),
        include_bytes!("../../../../../../skills/skit/SKILL.md")
    );
    let installed = host.observe(&host.initial_state().unwrap()).unwrap();
    assert_eq!(
        events_with_keys(
            &installed.transcript,
            &[
                "environment_variable",
                "system_locale",
                "platform",
                "preference"
            ]
        ),
        vec![
            json!({ "environment_variable": "SKIT_LANG", "result": null }),
            json!({ "environment_variable": "LC_ALL", "result": null }),
            json!({ "environment_variable": "LC_MESSAGES", "result": null }),
            json!({ "environment_variable": "LANG", "result": null }),
            json!({ "system_locale": "ZhCn" }),
            json!({ "platform": "Other" }),
            json!({ "preference": "agent_skill::Inspect", "path": "<profile:fixture>/cwd/.agents/skills/skit/SKILL.md", "outcome": { "ok": true } }),
            json!({ "preference": "agent_skill::BeforeCreateDirectory", "path": "<profile:fixture>/cwd/.agents/skills/skit", "outcome": { "ok": true } }),
            json!({ "preference": "agent_skill::BeforeReplace", "path": "<profile:fixture>/cwd/.agents/skills/skit/SKILL.md", "outcome": { "ok": true } }),
        ]
    );
    let before = fs::read(install.join("skit/SKILL.md")).unwrap();
    host.adapters
        .preference_failure
        .replace(Some(PreferenceFailure {
            point: AgentSkillInstallPoint::BeforeReplace,
            kind: io::ErrorKind::PermissionDenied,
            reason: "walker replace denied".to_owned(),
        }));
    let failed = host
        .dispatch(Effect::Preferences(PreferencesEffect::InstallAgentSkill {
            skills_dir: install.clone(),
        }))
        .unwrap();
    assert!(
        matches!(failed, Action::SetStatus(ref status) if status.contains("walker replace denied"))
    );
    assert_eq!(fs::read(install.join("skit/SKILL.md")).unwrap(), before);

    let failed = host.observe(&host.initial_state().unwrap()).unwrap();
    let failure = json!({
        "preference": "agent_skill::BeforeReplace",
        "path": "<profile:fixture>/cwd/.agents/skills/skit/SKILL.md",
        "outcome": { "error": {
            "kind": "PermissionDenied",
            "reason": "walker replace denied",
        }},
    });
    let mut expected = vec![
        json!({ "environment_variable": "SKIT_LANG", "result": null }),
        json!({ "environment_variable": "LC_ALL", "result": null }),
        json!({ "environment_variable": "LC_MESSAGES", "result": null }),
        json!({ "environment_variable": "LANG", "result": null }),
        json!({ "system_locale": "ZhCn" }),
        json!({ "platform": "Other" }),
        json!({ "preference": "agent_skill::Inspect", "path": "<profile:fixture>/cwd/.agents/skills/skit/SKILL.md", "outcome": { "ok": true } }),
    ];
    expected.extend(std::iter::repeat_n(failure, 8));
    assert_eq!(
        events_with_keys(
            &failed.transcript,
            &[
                "environment_variable",
                "system_locale",
                "platform",
                "preference"
            ]
        ),
        expected
    );
}

#[test]
fn launch_transcript_reads_the_program_bytes_at_the_call_boundary() {
    let spec = WalkerSeedSpec {
        profile: "executable".to_owned(),
        settings: BTreeMap::from([
            ("after_run".to_owned(), "stay".to_owned()),
            ("lang".to_owned(), "en".to_owned()),
        ]),
        external: vec![WalkerExternalSeed::File {
            path: PathBuf::from("tool"),
            bytes: b"executable fixture\n".to_vec(),
            readonly: false,
            unix_mode: 0o751,
        }],
        external_references: vec![WalkerExternalReferenceSeed {
            request: CreateEntry {
                name: "Tool".to_owned(),
                kind: EntryKind::parse("exe").unwrap(),
                mode: StorageMode::Reference,
                source: String::new(),
                workdir: "origin".to_owned(),
                description: String::new(),
                payload: Some(EntryPayload {
                    bytes: b"executable fixture\n".to_vec(),
                    stored_name: None,
                    permissions: SourcePermissions {
                        readonly: false,
                        unix_mode: Some(0o751),
                    },
                }),
                settings: EntrySettings::default(),
            },
            source: PathBuf::from("tool"),
        }],
        ..WalkerSeedSpec::default()
    };
    let mut first = RealWalkerHost::spawn(spec.clone()).unwrap();
    let mut second = RealWalkerHost::spawn(spec).unwrap();
    let run = |host: &mut RealWalkerHost| {
        let mut state = host.initial_state().unwrap();
        host.clear_transcript();
        let action = host
            .dispatch(Effect::Submit {
                purpose: FormPurpose::Run,
                selector: Some("Tool".to_owned()),
                values: BTreeMap::new(),
            })
            .unwrap();
        assert!(matches!(action, Action::Complete { .. }), "{action:?}");
        let _ = state.update(action);
        host.observe(&state).unwrap()
    };
    let first_observation = run(&mut first);
    let second_observation = run(&mut second);
    assert_eq!(first_observation, second_observation);
    let encoded = serde_json::to_string(&first_observation).unwrap();
    assert!(!encoded.contains(&first._sandbox.path().display().to_string()));
    assert!(!encoded.contains(&second._sandbox.path().display().to_string()));
    let launch = first_observation
        .transcript
        .iter()
        .find_map(|event| event.get("launch"))
        .unwrap();

    assert_eq!(
        launch["readable_files"],
        json!([{
            "path": "<profile:executable>/external/tool",
            "content": {
                "encoding": "utf8",
                "data": "executable fixture\n",
            },
        }])
    );
}

#[test]
fn trusted_dot_prefixed_external_files_never_become_owned_transient_tokens() {
    let mut spec = WalkerSeedSpec {
        profile: "trusted-dot-files".to_owned(),
        settings: BTreeMap::from([
            ("after_run".to_owned(), "stay".to_owned()),
            ("lang".to_owned(), "en".to_owned()),
        ]),
        ..WalkerSeedSpec::default()
    };
    for (name, path) in [
        ("Trusted run", ".run-report.sh"),
        ("Trusted injected", ".injected-notes"),
    ] {
        spec.external.push(WalkerExternalSeed::File {
            path: PathBuf::from(path),
            bytes: format!("{name}\n").into_bytes(),
            readonly: false,
            unix_mode: 0o751,
        });
        spec.external_references.push(WalkerExternalReferenceSeed {
            request: CreateEntry {
                name: name.to_owned(),
                kind: EntryKind::parse("exe").unwrap(),
                mode: StorageMode::Reference,
                source: String::new(),
                workdir: "origin".to_owned(),
                description: String::new(),
                payload: None,
                settings: EntrySettings::default(),
            },
            source: PathBuf::from(path),
        });
    }
    let mut host = RealWalkerHost::spawn(spec).unwrap();
    host.clear_transcript();
    for selector in ["Trusted run", "Trusted injected"] {
        assert!(matches!(
            host.dispatch(Effect::Submit {
                purpose: FormPurpose::Run,
                selector: Some(selector.to_owned()),
                values: BTreeMap::new(),
            })
            .unwrap(),
            Action::Complete { .. }
        ));
    }

    let observation = host.observe(&host.initial_state().unwrap()).unwrap();
    let encoded = serde_json::to_string(&observation.transcript).unwrap();
    assert!(encoded.contains("external/.run-report.sh"));
    assert!(encoded.contains("external/.injected-notes"));
    assert!(!encoded.contains("<run-snapshot:"));
    assert!(!encoded.contains("<injected-source:"));
}

#[cfg(unix)]
#[test]
fn external_executable_probe_matches_system_execute_permission_semantics() {
    let mut spec = WalkerSeedSpec {
        profile: "execute-bits".to_owned(),
        settings: BTreeMap::from([
            ("after_run".to_owned(), "stay".to_owned()),
            ("lang".to_owned(), "en".to_owned()),
        ]),
        ..WalkerSeedSpec::default()
    };
    for (name, path, mode) in [
        ("Not executable", "plain", 0o640),
        ("Executable", "ready", 0o751),
    ] {
        spec.external.push(WalkerExternalSeed::File {
            path: PathBuf::from(path),
            bytes: format!("{name}\n").into_bytes(),
            readonly: false,
            unix_mode: mode,
        });
        spec.external_references.push(WalkerExternalReferenceSeed {
            request: CreateEntry {
                name: name.to_owned(),
                kind: EntryKind::parse("exe").unwrap(),
                mode: StorageMode::Reference,
                source: String::new(),
                workdir: "origin".to_owned(),
                description: String::new(),
                payload: None,
                settings: EntrySettings::default(),
            },
            source: PathBuf::from(path),
        });
    }
    let mut host = RealWalkerHost::spawn(spec).unwrap();
    host.clear_transcript();
    assert!(
        host.dispatch(Effect::Submit {
            purpose: FormPurpose::Run,
            selector: Some("Not executable".to_owned()),
            values: BTreeMap::new(),
        })
        .is_err()
    );
    assert!(matches!(
        host.dispatch(Effect::Submit {
            purpose: FormPurpose::Run,
            selector: Some("Executable".to_owned()),
            values: BTreeMap::new(),
        })
        .unwrap(),
        Action::Complete { .. }
    ));
    let observation = host.observe(&host.initial_state().unwrap()).unwrap();
    let executable_probes = observation
        .transcript
        .iter()
        .filter(|event| event["probe"] == "is_executable")
        .map(|event| (event["path"].clone(), event["result"].clone()))
        .collect::<Vec<_>>();
    assert_eq!(
        executable_probes,
        vec![
            (json!("<profile:execute-bits>/external/plain"), json!(false)),
            (json!("<profile:execute-bits>/external/ready"), json!(true)),
            (json!("<profile:execute-bits>/external/ready"), json!(true)),
        ]
    );
    assert_eq!(
        observation
            .transcript
            .iter()
            .filter(|event| event.get("launch").is_some())
            .count(),
        1
    );
}

#[test]
fn observations_keep_full_ui_state_unknown_config_and_port_transcripts() {
    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    host.adapters.programs.remove("npm");
    let config_path = host.roots().config.join("config.toml");
    let mut config = fs::read_to_string(&config_path).unwrap();
    config.push_str("\n[future]\nanswer = 42\n");
    fs::write(&config_path, config).unwrap();

    let mut state = host.initial_state().unwrap();
    let dependency_effect = Effect::Open {
        request: HostRequest::Run,
        selector: Some("JavaScript".to_owned()),
    };
    assert!(matches!(
        host.dispatch(dependency_effect).unwrap(),
        Action::SetStatus(ref message)
            if message.starts_with("Error:") && message.contains("dependencies")
    ));
    let action = host
        .dispatch(Effect::Open {
            request: HostRequest::Run,
            selector: Some("Command".to_owned()),
        })
        .unwrap();
    let _ = state.update(action);
    let completed = host
        .dispatch(Effect::Submit {
            purpose: FormPurpose::Run,
            selector: Some("Command".to_owned()),
            values: BTreeMap::from([("value".to_owned(), FieldValue::text("walked"))]),
        })
        .unwrap();
    let _ = state.update(completed);
    assert!(matches!(
        host.dispatch(Effect::None).unwrap(),
        Action::ClearStatus
    ));
    assert!(matches!(
        host.dispatch(Effect::Quit).unwrap(),
        Action::ClearStatus
    ));
    let reload = host.dispatch(Effect::Reload).unwrap();
    let _ = state.update(reload);
    host.dispatch(Effect::Preferences(PreferencesEffect::Save(
        skit_application::preferences::PreferencesChangeSet {
            settings: BTreeMap::from([("after_run".to_owned(), "exit".to_owned())]),
        },
    )))
    .unwrap();
    assert!(
        fs::read_to_string(&config_path)
            .unwrap()
            .contains("answer = 42")
    );

    let observation = host.observe(&state).unwrap();
    let expected_state = PathMap::new(
        &host.profile,
        host._sandbox.path(),
        &host.service,
        &host.file_picker_tree.files,
    )
    .unwrap()
    .normalize_library_json(serde_json::to_value(&state).unwrap())
    .unwrap();
    assert_eq!(observation.state, expected_state);
    assert!(
        observation
            .transcript
            .iter()
            .any(|event| event.get("launch").is_some())
    );
    assert!(
        observation
            .transcript
            .iter()
            .any(|event| event.get("environment_snapshot").is_some())
    );
    assert!(observation.transcript.iter().any(|event| {
        event
            == &json!({
                "probe": "find_program",
                "path": "npm",
                "result": null,
            })
    }));
    assert_eq!(observation.prompt_runner, "walker");
    assert!(observation.tree.iter().any(|row| {
        row.path == "config/config.toml"
            && matches!(
                &row.content,
                Some(ByteView::Utf8(text)) if text.contains("answer = 42")
            )
    }));
    assert_eq!(
        FileConfigStore::new(&host.roots().config)
            .get("after_run")
            .unwrap(),
        "exit"
    );
}

#[test]
fn command_submit_records_the_exact_low_level_request_and_outcome_sequence() {
    let host = RealWalkerHost::spawn(profile()).unwrap();
    host.clear_transcript();

    assert!(matches!(
        host.dispatch(Effect::Submit {
            purpose: FormPurpose::Run,
            selector: Some("Command".to_owned()),
            values: BTreeMap::from([("value".to_owned(), FieldValue::text("walked"))]),
        })
        .unwrap(),
        Action::Complete { .. }
    ));
    let mut paths = PathMap::new(
        &host.profile,
        host._sandbox.path(),
        &host.service,
        &host.file_picker_tree.files,
    )
    .unwrap();
    let transcript = host.adapters.transcript(&mut paths);

    assert_eq!(
        transcript,
        vec![
            json!({ "clock": "2026-08-28T12:34:56+00:00" }),
            json!({ "local_offset_seconds": 0 }),
            json!({ "environment_snapshot": {
                "HOME": "/explicit/home",
                "LANG": "en_US.UTF-8",
                "SENTINEL": "walker",
            }}),
            json!({ "environment_snapshot": {
                "HOME": "/explicit/home",
                "LANG": "en_US.UTF-8",
                "SENTINEL": "walker",
            }}),
            json!({ "platform": "Other" }),
            json!({ "environment_variable": "COMSPEC", "result": null }),
            json!({ "environment_variable": "SystemRoot", "result": null }),
            json!({ "clock": "2026-08-28T12:34:56+00:00" }),
            json!({ "platform": "Other" }),
            json!({ "output": "diagnostic", "text": "Reusing your last arguments: tail" }),
            json!({ "probe": "is_dir", "path": "<profile:fixture>/cwd", "result": true }),
            json!({ "probe": "find_program", "path": "sh", "result": "/virtual/bin/sh" }),
            json!({ "clock": "2026-08-28T12:34:56+00:00" }),
            json!({ "probe": "is_dir", "path": "<profile:fixture>/cwd", "result": true }),
            json!({ "probe": "find_program", "path": "sh", "result": "/virtual/bin/sh" }),
            json!({ "output": "stdout", "text": "→ /virtual/bin/sh -c 'printf remembered tail'" }),
            json!({ "launch": {
                "program": "/virtual/bin/sh",
                "args": ["-c", "printf remembered tail"],
                "environment": {},
                "cwd": "<profile:fixture>/cwd",
                "display": "/virtual/bin/sh -c 'printf remembered tail'",
                "warnings": [],
                "readable_files": [],
                "outcome": { "ok": { "exit_code": 0, "signal": null } },
            }}),
            json!({ "clock": "2026-08-28T12:34:56+00:00" }),
            json!({ "clock": "2026-08-28T12:34:56+00:00" }),
        ]
    );
}

#[test]
fn javascript_run_drives_dependency_and_syntax_success_refusal_and_raw_failure() {
    let submit = |selector: &str| Effect::Submit {
        purpose: FormPurpose::Run,
        selector: Some(selector.to_owned()),
        values: if selector == "Injected JavaScript" {
            BTreeMap::from([("value:TOKEN".to_owned(), FieldValue::text("after"))])
        } else {
            BTreeMap::new()
        },
    };
    let mut success = RealWalkerHost::spawn(profile()).unwrap();
    success
        .adapters
        .dependency_script
        .replace(DependencyScript::Completed(DependencyCommandOutput {
            success: true,
            exit_code: Some(0),
            stderr: Vec::new(),
        }));
    success.clear_transcript();
    assert!(matches!(
        success.dispatch(submit("JavaScript")).unwrap(),
        Action::Complete { .. }
    ));
    let successful = success.observe(&success.initial_state().unwrap()).unwrap();
    assert_eq!(
        events_with_parsed_launch_displays(&successful.transcript, &["dependency", "launch"]),
        vec![
            json!({ "dependency": {
                "program": "/virtual/bin/npm",
                "args": ["install", "--no-audit", "--no-fund", "--ignore-scripts"],
                "cwd": "<profile:fixture>/data/scripts/javascript",
                "environment": {},
                "outcome": { "ok": {
                    "success": true,
                    "exit_code": 0,
                    "stderr": { "encoding": "utf8", "data": "" },
                }},
            }}),
            json!({ "launch": {
                "program": "/virtual/bin/node",
                "args": ["<profile:fixture>/data/scripts/javascript/script.js"],
                "environment": {},
                "cwd": "<profile:fixture>/data/scripts/javascript",
                "display": ["/virtual/bin/node", "<profile:fixture>/data/scripts/javascript/script.js"],
                "warnings": [],
                "readable_files": [{
                    "path": "<profile:fixture>/data/scripts/javascript/script.js",
                    "content": { "encoding": "utf8", "data": "console.log('ok');\n" },
                }],
                "outcome": { "ok": { "exit_code": 0, "signal": null }},
            }}),
        ]
    );

    let default_success = RealWalkerHost::spawn(profile()).unwrap();
    assert!(matches!(
        default_success.dispatch(submit("JavaScript")).unwrap(),
        Action::Complete { .. }
    ));
    let dependency_event = |outcome: Value| {
        json!({ "dependency": {
            "program": "/virtual/bin/npm",
            "args": ["install", "--no-audit", "--no-fund", "--ignore-scripts"],
            "cwd": "<profile:fixture>/data/scripts/javascript",
            "environment": {},
            "outcome": outcome,
        }})
    };

    let mut rejected_dependency = RealWalkerHost::spawn(profile()).unwrap();
    rejected_dependency
        .adapters
        .dependency_script
        .replace(DependencyScript::Completed(DependencyCommandOutput {
            success: false,
            exit_code: Some(9),
            stderr: b"offline \xff".to_vec(),
        }));
    rejected_dependency.clear_transcript();
    assert!(rejected_dependency.dispatch(submit("JavaScript")).is_err());
    let rejected = rejected_dependency
        .observe(&rejected_dependency.initial_state().unwrap())
        .unwrap();
    assert_eq!(
        events_with_keys(&rejected.transcript, &["dependency", "launch"]),
        vec![dependency_event(json!({ "ok": {
            "success": false,
            "exit_code": 9,
            "stderr": { "encoding": "hex", "data": "6f66666c696e6520ff" },
        }}))]
    );

    let mut raw_failure = RealWalkerHost::spawn(profile()).unwrap();
    raw_failure
        .adapters
        .dependency_script
        .replace(DependencyScript::Failure {
            kind: io::ErrorKind::NotFound,
            reason: "walker npm missing".to_owned(),
        });
    raw_failure.clear_transcript();
    assert!(raw_failure.dispatch(submit("JavaScript")).is_err());
    let raw = raw_failure
        .observe(&raw_failure.initial_state().unwrap())
        .unwrap();
    assert_eq!(
        events_with_keys(&raw.transcript, &["dependency", "launch"]),
        vec![dependency_event(json!({ "error": {
            "kind": "NotFound",
            "reason": "walker npm missing",
        }}))]
    );

    let mut gate_spec = profile();
    gate_spec.entries.push(injected_script(
        "js",
        "Injected JavaScript",
        "const TOKEN = 'before';\nconsole.log(TOKEN);\n",
        "script.js",
    ));

    let mut successful_gate = RealWalkerHost::spawn(gate_spec.clone()).unwrap();
    successful_gate.clear_transcript();
    assert!(matches!(
        successful_gate
            .dispatch(submit("Injected JavaScript"))
            .unwrap(),
        Action::Complete { .. }
    ));
    let successful_gate = successful_gate
        .observe(&successful_gate.initial_state().unwrap())
        .unwrap();
    let expected_source = "// /// script\n// [tool.skit]\n// schema = 1\n//\n// [[tool.skit.params]]\n// default = \"before\"\n// kind = \"const\"\n// name = \"TOKEN\"\n// type = \"str\"\n// ///\nconst TOKEN = \"after\";\nconsole.log(TOKEN);\n";
    assert_eq!(
        events_with_parsed_launch_displays(
            &successful_gate.transcript,
            &["javascript_gate", "launch"]
        ),
        vec![
            json!({ "javascript_gate": {
                "program": "/virtual/bin/node",
                "source": "<profile:fixture>/system-temp/<injected-source:0>.js",
                "timeout_ms": 30_000,
                "content": { "encoding": "utf8", "data": expected_source },
                "outcome": { "ok": {
                    "success": true,
                    "stderr": { "encoding": "utf8", "data": "" },
                }},
            }}),
            json!({ "launch": {
                "program": "/virtual/bin/node",
                "args": ["<profile:fixture>/system-temp/<injected-source:0>.js"],
                "environment": {},
                "cwd": "<profile:fixture>/cwd",
                "display": ["/virtual/bin/node", "<profile:fixture>/system-temp/<injected-source:0>.js"],
                "warnings": [],
                "readable_files": [{
                    "path": "<profile:fixture>/system-temp/<injected-source:0>.js",
                    "content": { "encoding": "utf8", "data": expected_source },
                }],
                "outcome": { "ok": { "exit_code": 0, "signal": null }},
            }}),
        ]
    );

    let mut rejected_gate = RealWalkerHost::spawn(gate_spec.clone()).unwrap();
    rejected_gate
        .adapters
        .javascript_gate_script
        .replace(JavaScriptGateScript::Completed(
            JavaScriptSyntaxGateOutput {
                success: false,
                stderr: b"syntax rejected".to_vec(),
            },
        ));
    rejected_gate.clear_transcript();
    assert!(
        rejected_gate
            .dispatch(submit("Injected JavaScript"))
            .is_err()
    );
    let gate = rejected_gate
        .observe(&rejected_gate.initial_state().unwrap())
        .unwrap();
    assert_eq!(
        events_with_keys(&gate.transcript, &["javascript_gate", "launch"]),
        vec![json!({ "javascript_gate": {
            "program": "/virtual/bin/node",
            "source": "<profile:fixture>/system-temp/<injected-source:0>.js",
            "timeout_ms": 30_000,
            "content": { "encoding": "utf8", "data": expected_source },
            "outcome": { "ok": {
                "success": false,
                "stderr": { "encoding": "utf8", "data": "syntax rejected" },
            }},
        }})]
    );

    let mut unavailable_gate = RealWalkerHost::spawn(gate_spec).unwrap();
    unavailable_gate
        .adapters
        .javascript_gate_script
        .replace(JavaScriptGateScript::Unavailable(
            JavaScriptSyntaxGateUnavailable::Spawn {
                reason: "walker node gate unavailable".to_owned(),
            },
        ));
    unavailable_gate.clear_transcript();
    assert!(matches!(
        unavailable_gate
            .dispatch(submit("Injected JavaScript"))
            .unwrap(),
        Action::Complete { .. }
    ));
    let unavailable = unavailable_gate
        .observe(&unavailable_gate.initial_state().unwrap())
        .unwrap();
    assert_eq!(
        events_with_parsed_launch_displays(&unavailable.transcript, &["javascript_gate", "launch"]),
        vec![
            json!({ "javascript_gate": {
            "program": "/virtual/bin/node",
            "source": "<profile:fixture>/system-temp/<injected-source:0>.js",
            "timeout_ms": 30_000,
            "content": { "encoding": "utf8", "data": expected_source },
            "outcome": {
                "unavailable": "could not run node syntax check: walker node gate unavailable"
            },
            }}),
            json!({ "launch": {
                "program": "/virtual/bin/node",
                "args": ["<profile:fixture>/system-temp/<injected-source:0>.js"],
                "environment": {},
                "cwd": "<profile:fixture>/cwd",
                "display": ["/virtual/bin/node", "<profile:fixture>/system-temp/<injected-source:0>.js"],
                "warnings": [],
                "readable_files": [{
                    "path": "<profile:fixture>/system-temp/<injected-source:0>.js",
                    "content": { "encoding": "utf8", "data": expected_source },
                }],
                "outcome": { "ok": { "exit_code": 0, "signal": null }},
            }}),
        ]
    );
}

#[test]
fn shell_injection_drives_success_rejection_and_unavailable_gate_outcomes() {
    let mut spec = profile();
    spec.entries.push(injected_script(
        "shell",
        "Injected shell",
        "TOKEN=before\nprintf '%s\\n' \"$TOKEN\"\n",
        "script.sh",
    ));
    let submit = || Effect::Submit {
        purpose: FormPurpose::Run,
        selector: Some("Injected shell".to_owned()),
        values: BTreeMap::from([("value:TOKEN".to_owned(), FieldValue::text("after"))]),
    };

    let mut success = RealWalkerHost::spawn(spec.clone()).unwrap();
    success.clear_transcript();
    assert!(matches!(
        success.dispatch(submit()).unwrap(),
        Action::Complete { .. }
    ));
    let successful = success.observe(&success.initial_state().unwrap()).unwrap();
    let expected_source = "# /// script\n# [tool.skit]\n# schema = 1\n#\n# [[tool.skit.params]]\n# default = \"before\"\n# kind = \"const\"\n# name = \"TOKEN\"\n# type = \"str\"\n# ///\nTOKEN='after'\nprintf '%s\\n' \"$TOKEN\"\n";
    assert_eq!(
        events_with_parsed_launch_displays(&successful.transcript, &["injected", "launch"]),
        vec![
            json!({ "injected": {
                "program": "/virtual/bin/bash",
                "args": ["-n", "<profile:fixture>/system-temp/<injected-source:0>.sh"],
                "timeout_ms": 30_000,
                "readable_files": [{
                    "path": "<profile:fixture>/system-temp/<injected-source:0>.sh",
                    "content": { "encoding": "utf8", "data": expected_source },
                }],
                "outcome": { "ok": {
                    "success": true,
                    "stderr": { "encoding": "utf8", "data": "" },
                }},
            }}),
            json!({ "launch": {
                "program": "/virtual/bin/bash",
                "args": ["<profile:fixture>/system-temp/<injected-source:0>.sh"],
                "environment": {},
                "cwd": "<profile:fixture>/cwd",
                "display": ["/virtual/bin/bash", "<profile:fixture>/system-temp/<injected-source:0>.sh"],
                "warnings": [],
                "readable_files": [{
                    "path": "<profile:fixture>/system-temp/<injected-source:0>.sh",
                    "content": { "encoding": "utf8", "data": expected_source },
                }],
                "outcome": { "ok": { "exit_code": 0, "signal": null }},
            }}),
        ]
    );
    let encoded = serde_json::to_string(&successful).unwrap();
    assert!(!encoded.contains("/tmp/.injected-"), "{encoded}");
    assert_system_temp_is_empty(&success);

    let mut rejected = RealWalkerHost::spawn(spec.clone()).unwrap();
    rejected
        .adapters
        .injected_script
        .replace(InjectedScript::Completed(InjectedCommandOutput {
            success: false,
            stderr: b"shell rejected".to_vec(),
        }));
    rejected.clear_transcript();
    assert!(rejected.dispatch(submit()).is_err());
    let rejected_observation = rejected
        .observe(&rejected.initial_state().unwrap())
        .unwrap();
    assert_eq!(
        events_with_keys(&rejected_observation.transcript, &["injected", "launch"],),
        vec![json!({ "injected": {
            "program": "/virtual/bin/bash",
            "args": ["-n", "<profile:fixture>/system-temp/<injected-source:0>.sh"],
            "timeout_ms": 30_000,
            "readable_files": [{
                "path": "<profile:fixture>/system-temp/<injected-source:0>.sh",
                "content": { "encoding": "utf8", "data": expected_source },
            }],
            "outcome": { "ok": {
                "success": false,
                "stderr": { "encoding": "utf8", "data": "shell rejected" },
            }},
        }})]
    );
    assert_system_temp_is_empty(&rejected);

    let mut unavailable = RealWalkerHost::spawn(spec).unwrap();
    unavailable
        .adapters
        .injected_script
        .replace(InjectedScript::Unavailable(
            InjectedCommandUnavailable::Spawn {
                reason: "walker shell gate unavailable".to_owned(),
            },
        ));
    unavailable.clear_transcript();
    assert!(matches!(
        unavailable.dispatch(submit()).unwrap(),
        Action::Complete { .. }
    ));
    let unavailable_observation = unavailable
        .observe(&unavailable.initial_state().unwrap())
        .unwrap();
    assert_eq!(
        events_with_parsed_launch_displays(
            &unavailable_observation.transcript,
            &["injected", "launch"]
        ),
        vec![
            json!({ "injected": {
            "program": "/virtual/bin/bash",
            "args": ["-n", "<profile:fixture>/system-temp/<injected-source:0>.sh"],
            "timeout_ms": 30_000,
            "readable_files": [{
                "path": "<profile:fixture>/system-temp/<injected-source:0>.sh",
                "content": { "encoding": "utf8", "data": expected_source },
            }],
            "outcome": {
                "unavailable": "could not run the injected-source check: walker shell gate unavailable"
            },
            }}),
            json!({ "launch": {
                "program": "/virtual/bin/bash",
                "args": ["<profile:fixture>/system-temp/<injected-source:0>.sh"],
                "environment": {},
                "cwd": "<profile:fixture>/cwd",
                "display": ["/virtual/bin/bash", "<profile:fixture>/system-temp/<injected-source:0>.sh"],
                "warnings": [],
                "readable_files": [{
                    "path": "<profile:fixture>/system-temp/<injected-source:0>.sh",
                    "content": { "encoding": "utf8", "data": expected_source },
                }],
                "outcome": { "ok": { "exit_code": 0, "signal": null }},
            }}),
        ]
    );
    assert_system_temp_is_empty(&unavailable);
}

#[test]
fn injected_stage_write_failure_removes_the_partial_system_temp_file() {
    let mut spec = profile();
    spec.entries.push(injected_script(
        "shell",
        "Injected shell",
        "TOKEN=before\nprintf '%s\\n' \"$TOKEN\"\n",
        "script.sh",
    ));
    let mut host = RealWalkerHost::spawn(spec).unwrap();
    let fault = StageWriteFaultGuard::for_current_thread();
    host.clear_transcript();

    let result = host.dispatch(Effect::Submit {
        purpose: FormPurpose::Run,
        selector: Some("Injected shell".to_owned()),
        values: BTreeMap::from([("value:TOKEN".to_owned(), FieldValue::text("after"))]),
    });
    drop(fault);

    assert!(result.is_err());
    let _ = host.observe(&host.initial_state().unwrap()).unwrap();
    assert_system_temp_is_empty(&host);
}

#[test]
fn injected_runner_failure_removes_the_system_temp_file() {
    let mut spec = profile();
    spec.entries.push(injected_script(
        "shell",
        "Injected shell",
        "TOKEN=before\nprintf '%s\\n' \"$TOKEN\"\n",
        "script.sh",
    ));
    let mut host = RealWalkerHost::spawn(spec).unwrap();
    host.fail_next_launch(io::ErrorKind::PermissionDenied, "walker launch denied");
    host.clear_transcript();

    let result = host.dispatch(Effect::Submit {
        purpose: FormPurpose::Run,
        selector: Some("Injected shell".to_owned()),
        values: BTreeMap::from([("value:TOKEN".to_owned(), FieldValue::text("after"))]),
    });

    assert!(result.is_err());
    let _ = host.observe(&host.initial_state().unwrap()).unwrap();
    assert_system_temp_is_empty(&host);
}

#[test]
fn uv_bootstrap_drives_consent_transport_and_invalid_archive_outcomes() {
    let mut spec = profile();
    spec.entries.push(CreateEntry {
        name: "Python uv".to_owned(),
        kind: EntryKind::parse("python").unwrap(),
        mode: StorageMode::Copy,
        source: String::new(),
        workdir: "invoke".to_owned(),
        description: String::new(),
        payload: Some(EntryPayload {
            bytes: b"print('uv')\n".to_vec(),
            stored_name: Some("script.py".to_owned()),
            permissions: SourcePermissions::default(),
        }),
        settings: EntrySettings::default(),
    });
    let submit = || Effect::Submit {
        purpose: FormPurpose::Run,
        selector: Some("Python uv".to_owned()),
        values: BTreeMap::new(),
    };
    let asset = skit_runtime::uv_asset(&skit_runtime::UvTarget::current().unwrap(), None).unwrap();

    let mut declined = RealWalkerHost::spawn(spec.clone()).unwrap();
    declined.adapters.programs.remove("uv");
    declined.clear_transcript();
    assert!(declined.dispatch(submit()).is_err());
    let declined = declined
        .observe(&declined.initial_state().unwrap())
        .unwrap();
    assert_eq!(
        events_with_keys(&declined.transcript, &["uv_consent", "uv_fetch", "launch"]),
        vec![json!({ "uv_consent": {
            "version": skit_runtime::UV_VERSION,
            "destination": "<profile:fixture>/data/bin",
            "result": false,
        }})]
    );

    let mut transport = RealWalkerHost::spawn(spec.clone()).unwrap();
    transport.adapters.programs.remove("uv");
    transport.adapters.uv_consent.set(true);
    transport
        .adapters
        .uv_fetch_script
        .replace(UvFetchScript::Failure("walker network offline".to_owned()));
    transport.clear_transcript();
    assert!(transport.dispatch(submit()).is_err());
    let transport = transport
        .observe(&transport.initial_state().unwrap())
        .unwrap();
    assert_eq!(
        events_with_keys(&transport.transcript, &["uv_consent", "uv_fetch", "launch"]),
        vec![
            json!({ "uv_consent": {
                "version": skit_runtime::UV_VERSION,
                "destination": "<profile:fixture>/data/bin",
                "result": true,
            }}),
            json!({ "uv_fetch": {
                "url": asset.url.clone(),
                "limit": 104_857_600_u64,
                "outcome": { "error": "walker network offline" },
            }}),
        ]
    );

    let mut invalid = RealWalkerHost::spawn(spec.clone()).unwrap();
    invalid.adapters.programs.remove("uv");
    invalid.adapters.uv_consent.set(true);
    invalid
        .adapters
        .uv_fetch_script
        .replace(UvFetchScript::Bytes(Vec::new()));
    invalid.clear_transcript();
    assert!(invalid.dispatch(submit()).is_err());
    let invalid = invalid.observe(&invalid.initial_state().unwrap()).unwrap();
    assert_eq!(
        events_with_keys(&invalid.transcript, &["uv_consent", "uv_fetch", "launch"]),
        vec![
            json!({ "uv_consent": {
                "version": skit_runtime::UV_VERSION,
                "destination": "<profile:fixture>/data/bin",
                "result": true,
            }}),
            json!({ "uv_fetch": {
                "url": asset.url.clone(),
                "limit": 104_857_600_u64,
                "outcome": { "ok": {
                    "length": 0,
                    "sha256": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
                }},
            }}),
        ]
    );

    let fetch_outcome = |bytes: Vec<u8>| {
        let mut host = RealWalkerHost::spawn(spec.clone()).unwrap();
        host.adapters.programs.remove("uv");
        host.adapters.uv_consent.set(true);
        host.adapters
            .uv_fetch_script
            .replace(UvFetchScript::Bytes(bytes));
        host.clear_transcript();
        assert!(host.dispatch(submit()).is_err());
        host.observe(&host.initial_state().unwrap())
            .unwrap()
            .transcript
            .into_iter()
            .find_map(|event| event.get("uv_fetch").cloned())
            .unwrap()["outcome"]
            .clone()
    };
    let first_outcome = fetch_outcome(vec![1, 2, 3, 4]);
    let second_outcome = fetch_outcome(vec![4, 3, 2, 1]);
    assert_eq!(first_outcome["ok"]["length"], 4);
    assert_eq!(second_outcome["ok"]["length"], 4);
    assert_ne!(
        first_outcome["ok"]["sha256"],
        second_outcome["ok"]["sha256"]
    );

    let mut managed = RealWalkerHost::spawn(spec).unwrap();
    managed.adapters.programs.remove("uv");
    managed.adapters.uv_consent.set(true);
    let managed_uv = skit_runtime::managed_uv_path(&managed.roots().data);
    let managed_relative = format!(
        "data/bin/{}",
        managed_uv.file_name().unwrap().to_str().unwrap()
    );
    let projected_uv = format!("<profile:fixture>/{managed_relative}");
    fs::create_dir_all(managed_uv.parent().unwrap()).unwrap();
    fs::write(&managed_uv, b"managed uv fixture\n").unwrap();
    set_unix_mode(&managed_uv, 0o751).unwrap();
    managed.clear_transcript();
    assert!(matches!(
        managed.dispatch(submit()).unwrap(),
        Action::Complete { .. }
    ));
    let managed = managed.observe(&managed.initial_state().unwrap()).unwrap();
    let managed_executable = managed
        .tree
        .iter()
        .find(|row| row.path == managed_relative)
        .unwrap();
    #[cfg(unix)]
    assert_eq!(managed_executable.mode, Some(0o751));
    #[cfg(not(unix))]
    assert_eq!(managed_executable.mode, None);
    assert!(
        !managed
            .transcript
            .iter()
            .any(|event| event.get("uv_consent").is_some() || event.get("uv_fetch").is_some())
    );
    assert_eq!(
        events_with_parsed_launch_displays(&managed.transcript, &["launch"]),
        vec![json!({ "launch": {
            "program": projected_uv,
            "args": [
                "run",
                "--no-project",
                "--script",
                "<profile:fixture>/data/scripts/python-uv/script.py",
            ],
            "environment": {},
            "cwd": "<profile:fixture>/cwd",
            "display": [projected_uv, "run", "--no-project", "--script", "<profile:fixture>/data/scripts/python-uv/script.py"],
            "warnings": [],
            "readable_files": [
                {
                    "path": projected_uv,
                    "content": { "encoding": "utf8", "data": "managed uv fixture\n" },
                },
                {
                    "path": "<profile:fixture>/data/scripts/python-uv/script.py",
                    "content": { "encoding": "utf8", "data": "print('uv')\n" },
                },
            ],
            "outcome": { "ok": { "exit_code": 0, "signal": null }},
        }})]
    );
}

#[test]
fn launch_io_failure_is_raw_in_the_transcript_and_does_not_complete_state() {
    let host = RealWalkerHost::spawn(profile()).unwrap();
    host.clear_transcript();
    host.fail_next_launch(io::ErrorKind::PermissionDenied, "walker launch denied");

    let error = host
        .dispatch(Effect::Submit {
            purpose: FormPurpose::Run,
            selector: Some("Command".to_owned()),
            values: BTreeMap::from([("value".to_owned(), FieldValue::text("walked"))]),
        })
        .unwrap_err();
    assert!(error.to_string().contains("walker launch denied"));
    let mut paths = PathMap::new(
        &host.profile,
        host._sandbox.path(),
        &host.service,
        &host.file_picker_tree.files,
    )
    .unwrap();
    let transcript = host.adapters.transcript(&mut paths);
    assert_eq!(
        transcript.last(),
        Some(&json!({ "launch": {
            "program": "/virtual/bin/sh",
            "args": ["-c", "printf remembered tail"],
            "environment": {},
            "cwd": "<profile:fixture>/cwd",
            "display": "/virtual/bin/sh -c 'printf remembered tail'",
            "warnings": [],
            "readable_files": [],
            "outcome": {
                "error": {
                    "kind": "PermissionDenied",
                    "reason": "walker launch denied",
                }
            },
        }}))
    );
    let state = FormStateService::new(FileFormStateStore::new(&host.roots().state))
        .load(&Slug::parse("command").unwrap());
    assert_eq!(state.last_run.exit, Some(7));
    assert_eq!(
        state.last_run.at.as_deref(),
        Some("2026-08-28T11:00:00+00:00")
    );
}

#[test]
fn rejected_preflight_is_localized_and_never_enters_the_effect_server() {
    let process_cwd = std::env::current_dir().unwrap();
    let ambient_executable = tempfile::Builder::new()
        .prefix("skit-walker-cwd-")
        .suffix("-program")
        .tempfile_in(process_cwd)
        .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;

        fs::set_permissions(ambient_executable.path(), fs::Permissions::from_mode(0o751)).unwrap();
    }
    let bare_program = ambient_executable
        .path()
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .expect("the generated program name is UTF-8")
        .to_owned();
    let mut spec = profile();
    spec.settings.insert("lang".to_owned(), "zh-TW".to_owned());
    let javascript = spec
        .entries
        .iter_mut()
        .find(|entry| entry.name == "JavaScript")
        .expect("the fixture has one JavaScript entry");
    javascript.settings.interpreter = bare_program.clone();
    let host = RealWalkerHost::spawn(spec).unwrap();
    let effect = Effect::Open {
        request: HostRequest::Run,
        selector: Some("JavaScript".to_owned()),
    };
    host.clear_transcript();
    let error = host.preflight(&effect).unwrap_err();
    let expected_transcript = {
        let mut paths = PathMap::new(
            &host.profile,
            host._sandbox.path(),
            &host.service,
            &host.file_picker_tree.files,
        )
        .unwrap();
        host.adapters.transcript(&mut paths)
    };
    assert!(expected_transcript.iter().any(|event| {
        event
            == &json!({
                "probe": "find_program",
                "path": bare_program,
                "result": null,
            })
    }));
    let expected_action = Action::SetStatus(
        Message::new("Error: {}")
            .nested(error.message())
            .localize(Locale::ZhTw),
    );

    host.clear_transcript();
    assert_eq!(host.dispatch(effect).unwrap(), expected_action);
    let mut paths = PathMap::new(
        &host.profile,
        host._sandbox.path(),
        &host.service,
        &host.file_picker_tree.files,
    )
    .unwrap();
    assert_eq!(host.adapters.transcript(&mut paths), expected_transcript);
}
