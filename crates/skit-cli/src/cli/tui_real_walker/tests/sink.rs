//! Schema-three sink, trace pair, and cause projection contracts.

use super::*;

#[test]
fn schema_three_projection_keeps_full_objects_timeline_and_cast_in_memory() {
    let factory = smoke_factory();

    let main = record_schema_three_main(&factory).unwrap();
    let replay = record_schema_three_replay(&factory).unwrap();

    assert_eq!(main.trace, replay.trace);
    assert_eq!(main.sandbox.mode(), SandboxMode::Random);
    assert_eq!(replay.sandbox.mode(), SandboxMode::Random);
    assert_eq!(
        main.sandbox.profiles().get(&factory.review_profile),
        Some(&main.sandbox.root().to_owned())
    );
    assert_eq!(
        replay.sandbox.profiles().get(&factory.review_profile),
        Some(&replay.sandbox.root().to_owned())
    );
    #[cfg(target_os = "linux")]
    assert_eq!(main.sandbox.platform(), SandboxPlatform::Linux);
    #[cfg(target_os = "macos")]
    assert_eq!(main.sandbox.platform(), SandboxPlatform::Macos);
    #[cfg(target_os = "windows")]
    assert_eq!(main.sandbox.platform(), SandboxPlatform::Windows);
    assert_eq!(main.rows.len(), 4);
    assert_eq!(
        main.rows
            .iter()
            .map(|row| row.presentation)
            .collect::<Vec<_>>(),
        [
            Presentation::Presented,
            Presentation::NotPresented,
            Presentation::Presented,
            Presentation::Presented,
        ]
    );
    assert_eq!(
        main.rows.last().unwrap().liveness,
        Some(LivenessResult::Passed)
    );
    assert_eq!(main.operations.len(), 1);
    assert_eq!(main.review_profile.as_str(), "engine-smoke-60x24");
    assert!(
        main.rows
            .iter()
            .all(|row| row.profile == main.review_profile)
    );
    assert_eq!(
        main.rows
            .iter()
            .map(|row| {
                (
                    row.sequence,
                    row.phase,
                    row.event_chain,
                    row.operation_index,
                    row.boundary,
                )
            })
            .collect::<Vec<_>>(),
        [
            (
                0,
                TimelinePhase::Operations,
                None,
                None,
                TimelineBoundary::Initial,
            ),
            (
                1,
                TimelinePhase::Operations,
                Some(EventChainIdentity {
                    phase: TimelinePhase::Operations,
                    sequence: 0,
                }),
                Some(0),
                TimelineBoundary::UserAction,
            ),
            (
                2,
                TimelinePhase::Operations,
                Some(EventChainIdentity {
                    phase: TimelinePhase::Operations,
                    sequence: 0,
                }),
                Some(0),
                TimelineBoundary::HostAction,
            ),
            (
                3,
                TimelinePhase::FinalLiveness,
                Some(EventChainIdentity {
                    phase: TimelinePhase::FinalLiveness,
                    sequence: 0,
                }),
                None,
                TimelineBoundary::UserAction,
            ),
        ]
    );
    assert_eq!(main.rows[0].cause, TransitionCause::Initial);
    assert_eq!(
        main.rows[1].cause,
        TransitionCause::Reducer {
            requested: json!({"operation": "open_run"}),
            resolved: json!({
                "input": {"kind": "event", "event": {
                    "type": "key", "code": "Enter", "modifiers": "",
                }},
                "semantic_target": {"command": "run"},
            }),
            event: json!({"type": "key", "code": "Enter", "modifiers": ""}),
            action: json!("open_run"),
            emitted: json!({"open": {"request": "run", "selector": "reference"}}),
        }
    );
    let reducer_cause = json_value(&main.rows[1].cause);
    let host_cause = json_value(&main.rows[2].cause);
    assert_eq!(
        reducer_cause.pointer("/emitted"),
        host_cause.pointer("/request")
    );
    assert_eq!(host_cause.pointer("/cause"), Some(&json!("host")));
    assert_eq!(host_cause.pointer("/operation_index"), Some(&json!(0)));
    assert_eq!(host_cause.pointer("/round"), Some(&json!(0)));
    assert_eq!(
        host_cause.pointer("/request"),
        Some(&json!({"open": {"request": "run", "selector": "reference"}}))
    );
    assert_eq!(
        host_cause.pointer("/response/present/run/selector"),
        Some(&json!("reference"))
    );
    assert_eq!(
        host_cause.pointer("/response/present/run/name"),
        Some(&json!("Reference"))
    );
    assert_eq!(host_cause.pointer("/emitted"), Some(&json!("none")));
    let round_trip: Action = serde_json::from_value(host_cause["response"].clone()).unwrap();
    assert!(matches!(
        round_trip,
        Action::Present(Screen::Run(screen)) if screen.selector() == "reference"
    ));
    assert_eq!(
        main.rows[3].cause,
        TransitionCause::Reducer {
            requested: json!({"synthetic": "final_liveness"}),
            resolved: json!({"event": {
                "type": "key", "code": "Esc", "modifiers": "",
            }}),
            event: json!({"type": "key", "code": "Esc", "modifiers": ""}),
            action: json!("back"),
            emitted: json!("none"),
        }
    );
    for row in &main.rows {
        for reference in [
            &row.reducer,
            &row.host,
            &row.session,
            &row.styled_frame,
            &row.geometry,
        ] {
            assert!(
                main.objects
                    .contains_key(&(reference.kind, reference.sha256.clone()))
            );
        }
    }
    let cast = main
        .cast
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_slice::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(cast.len(), 4);
    assert_eq!(
        cast[0],
        json!({"version": 3, "term": {"cols": 60, "rows": 24}})
    );
    assert_eq!(
        cast[1..]
            .iter()
            .map(|event| (event[0].as_f64().unwrap(), event[1].as_str().unwrap()))
            .collect::<Vec<_>>(),
        [(0.1, "o"), (0.2, "o"), (0.1, "o")]
    );
}

#[test]
fn random_trace_failures_still_use_explicit_close() {
    let factory = smoke_factory();
    let prefix = [SmokeOperation::OpenRun];

    let error = record_schema_three_random_with_hooks(
        &factory,
        &prefix,
        noop_real_host_boundary,
        |_| Err("injected random sink failure".to_owned()),
        read_boundary_sandbox_metadata,
        noop_schema_sink,
        noop_leak_oracle_facts,
    )
    .unwrap_err();
    assert!(error.contains("injected random sink failure"));

    let error = record_schema_three_random_with_hooks(
        &factory,
        &prefix,
        noop_real_host_boundary,
        new_schema_three_sink,
        |_, _| Err("injected random metadata failure".to_owned()),
        noop_schema_sink,
        noop_leak_oracle_facts,
    )
    .unwrap_err();
    assert!(error.contains("injected random metadata failure"));

    let error = record_schema_three_random_with_hooks(
        &factory,
        &prefix,
        noop_real_host_boundary,
        new_schema_three_sink,
        read_boundary_sandbox_metadata,
        |sink| sink.rows.clear(),
        noop_leak_oracle_facts,
    )
    .unwrap_err();
    assert!(error.contains("no checkpoint"));

    let error = record_schema_three_random_with_hooks(
        &factory,
        &prefix,
        |host| {
            std::fs::write(
                host.host.sandbox_root().join("system-temp/residual"),
                b"residual",
            )
            .unwrap();
        },
        new_schema_three_sink,
        read_boundary_sandbox_metadata,
        noop_schema_sink,
        noop_leak_oracle_facts,
    )
    .unwrap_err();
    assert!(error.contains("system temp retained an artifact"));

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;

        let retained_root = std::cell::RefCell::new(None);
        let retained_child = std::cell::RefCell::new(None);
        let error = record_schema_three_random_with_hooks(
            &factory,
            &prefix,
            |host| {
                let root = host.host.sandbox_root().to_path_buf();
                let blocked = root.join("blocked-cleanup");
                std::fs::create_dir(&blocked).unwrap();
                std::fs::set_permissions(&blocked, std::fs::Permissions::from_mode(0o000)).unwrap();
                std::fs::write(root.join("system-temp/residual"), b"residual").unwrap();
                retained_root.replace(Some(root));
                retained_child.replace(Some(blocked));
            },
            new_schema_three_sink,
            read_boundary_sandbox_metadata,
            noop_schema_sink,
            noop_leak_oracle_facts,
        )
        .unwrap_err();
        assert!(error.contains("sandbox cleanup also failed"));
        let root = retained_root.into_inner().unwrap();
        let child = retained_child.into_inner().unwrap();
        std::fs::set_permissions(&child, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn unrendered_random_draft_retains_facts_for_recorded_data() {
    let factory = smoke_factory();
    let raw_path = std::rc::Rc::new(std::cell::RefCell::new(None));
    let raw_path_from_hook = raw_path.clone();
    let recorded = record_schema_three_random_with_hooks(
        &factory,
        &[SmokeOperation::OpenRun],
        move |host| {
            let drafts = crate::cli::create_owned_drafts_dir(&host.host.roots().data).unwrap();
            let draft = drafts.join("skit-new-000000.py");
            std::fs::write(&draft, b"print('facts')\n").unwrap();
            raw_path_from_hook.replace(Some(draft.display().to_string()));
        },
        new_schema_three_sink,
        read_boundary_sandbox_metadata,
        noop_schema_sink,
        noop_leak_oracle_facts,
    )
    .unwrap();
    let raw_path = raw_path.borrow().clone().unwrap();
    assert!(
        recorded
            .leak_oracle_facts
            .renderer_drafts
            .values()
            .any(|draft| draft.raw_path == raw_path)
    );
    assert!(!recorded.leak_oracle_facts.artifacts.is_empty());

    let mut trace_bytes = serde_json::to_vec(&recorded.operations).unwrap();
    trace_bytes.extend(serde_json::to_vec(&recorded.final_liveness_requested).unwrap());
    trace_bytes.extend(serde_json::to_vec(&recorded.rows).unwrap());
    for object in recorded.objects.values() {
        trace_bytes.extend(object);
    }
    trace_bytes.extend(&recorded.cast);
    assert!(
        !trace_bytes
            .windows(raw_path.len())
            .any(|window| window == raw_path.as_bytes())
    );
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn stable_add_draft_state(
    host: &RealWalkerHost,
    modified_seconds: u64,
) -> Result<LibraryState, String> {
    let drafts = crate::cli::create_owned_drafts_dir(&host.roots().data).unwrap();
    let path = drafts.join("skit-new-000000.py");
    std::fs::write(&path, b"print('stable facts')\n").unwrap();
    std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(
            std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(modified_seconds),
        ))
        .unwrap();
    Ok(host.initial_state().unwrap())
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn stable_main_and_replay_keep_equal_traces_but_independent_raw_facts() {
    let parent = tempfile::TempDir::new().unwrap();
    let namespace =
        StableSandboxNamespace::explicit(parent.path().join(STABLE_SANDBOX_NAMESPACE)).unwrap();
    let factory = smoke_factory();
    let (main, operations) = record_schema_three_stable_with_hooks(
        &factory,
        namespace.clone(),
        StablePairPhase::Main,
        &[SmokeOperation::OpenRun],
        read_sandbox_metadata,
        |host| stable_add_draft_state(host, 10),
        noop_stable_host,
        new_schema_three_sink,
        noop_schema_sink,
        noop_leak_oracle_facts,
        noop_stable_host,
    )
    .unwrap();
    let (replay, replayed) = record_schema_three_stable_with_hooks(
        &factory,
        namespace,
        StablePairPhase::Replay,
        &operations,
        read_sandbox_metadata,
        |host| stable_add_draft_state(host, 20),
        noop_stable_host,
        new_schema_three_sink,
        noop_schema_sink,
        noop_leak_oracle_facts,
        noop_stable_host,
    )
    .unwrap();

    assert_eq!(replayed, operations);
    assert_eq!(main.trace, replay.trace);
    assert_ne!(main.leak_oracle_facts, replay.leak_oracle_facts);
    assert!(main.leak_oracle_facts.artifacts.values().any(|artifact| {
        artifact
            .modified_values
            .iter()
            .any(|pair| pair.raw != pair.projected)
    }));
    assert!(replay.leak_oracle_facts.artifacts.values().any(|artifact| {
        artifact
            .modified_values
            .iter()
            .any(|pair| pair.raw != pair.projected)
    }));
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn facts_snapshot_precedes_an_explicit_close_failure() {
    let parent = tempfile::TempDir::new().unwrap();
    let namespace =
        StableSandboxNamespace::explicit(parent.path().join(STABLE_SANDBOX_NAMESPACE)).unwrap();
    let order = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let facts = std::rc::Rc::new(std::cell::RefCell::new(None));
    let order_at_snapshot = order.clone();
    let facts_at_snapshot = facts.clone();
    let order_at_close = order.clone();
    let error = record_schema_three_stable_with_hooks(
        &smoke_factory(),
        namespace,
        StablePairPhase::Main,
        &[SmokeOperation::OpenRun],
        read_sandbox_metadata,
        read_initial_state,
        |host| {
            let drafts = crate::cli::create_owned_drafts_dir(&host.roots().data).unwrap();
            std::fs::write(
                drafts.join("skit-new-000000.py"),
                b"print('unrendered hook fact')\n",
            )
            .unwrap();
        },
        new_schema_three_sink,
        noop_schema_sink,
        move |snapshot| {
            assert!(
                snapshot
                    .renderer_drafts
                    .values()
                    .any(|draft| { draft.raw_kind_picker_basename == "skit-new-000000.py" })
            );
            assert!(!snapshot.artifacts.is_empty());
            order_at_snapshot.borrow_mut().push("facts");
            facts_at_snapshot.replace(Some(snapshot.clone()));
        },
        move |host| {
            order_at_close.borrow_mut().push("close");
            std::fs::write(
                host.sandbox_root().join(SANDBOX_MARKER_FILE),
                b"corrupt marker",
            )
            .unwrap();
        },
    )
    .unwrap_err();

    assert!(matches!(error.primary, StablePairPrimaryError::Close(_)));
    assert_eq!(*order.borrow(), ["facts", "close"]);
    assert!(facts.borrow().is_some());
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn stable_pair_closes_main_before_fresh_replay_in_the_same_namespace() {
    let parent = tempfile::TempDir::new().unwrap();
    let namespace_root = parent.path().join(STABLE_SANDBOX_NAMESPACE);
    let namespace = StableSandboxNamespace::explicit(namespace_root.clone()).unwrap();

    let pair = record_real_smoke_stable_pair_in(namespace).unwrap();

    assert_eq!(pair.main.trace, pair.replay.trace);
    assert_eq!(pair.main.sandbox, pair.replay.sandbox);
    assert_eq!(pair.main.sandbox.mode(), SandboxMode::Stable);
    assert_eq!(
        pair.main.sandbox.root(),
        namespace_root.to_str().expect("the test path is UTF-8")
    );
    let profile_root = namespace_root.join(profile_sandbox_path(&pair.main.review_profile));
    assert_eq!(
        pair.main.sandbox.profiles().get(&pair.main.review_profile),
        Some(&profile_root.to_string_lossy().into_owned())
    );
    assert!(!profile_root.exists());
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn stable_pair_refuses_a_live_main_profile_without_random_fallback() {
    let parent = tempfile::TempDir::new().unwrap();
    let namespace =
        StableSandboxNamespace::explicit(parent.path().join(STABLE_SANDBOX_NAMESPACE)).unwrap();
    let factory = smoke_factory();
    let held = RealWalkerHost::spawn_stable_in(
        factory.seed.clone(),
        factory.review_profile.clone(),
        namespace.clone(),
    )
    .unwrap();

    let error = record_real_smoke_stable_pair_in(namespace).unwrap_err();

    assert_eq!(error.phase, StablePairPhase::Main);
    assert!(error.to_string().contains("is busy"));
    assert!(matches!(
        &error.primary,
        StablePairPrimaryError::Sandbox(error)
            if matches!(error.as_ref(), SandboxError::Busy { .. })
    ));
    assert!(error.cleanup.is_none());
    held.close().unwrap();
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn stable_frontend_failure_explicitly_closes_the_profile() {
    let parent = tempfile::TempDir::new().unwrap();
    let namespace =
        StableSandboxNamespace::explicit(parent.path().join(STABLE_SANDBOX_NAMESPACE)).unwrap();
    let mut invalid = smoke_factory();
    invalid.size = Size::new(0, 24);

    let error = record_schema_three_stable(
        &invalid,
        namespace.clone(),
        StablePairPhase::Main,
        &[SmokeOperation::OpenRun],
    )
    .unwrap_err();
    assert!(error.to_string().contains("could not create"));
    assert!(matches!(
        &error.primary,
        StablePairPrimaryError::Frontend(_)
    ));
    assert!(error.cleanup.is_none());

    record_real_smoke_stable_pair_in(namespace).unwrap();
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn stable_pair_preserves_typed_primary_and_cleanup_failures() {
    let seed_parent = tempfile::TempDir::new().unwrap();
    let seed_namespace =
        StableSandboxNamespace::explicit(seed_parent.path().join(STABLE_SANDBOX_NAMESPACE))
            .unwrap();
    let mut invalid_seed = smoke_factory();
    invalid_seed.seed.external = vec![WalkerExternalSeed::File {
        path: PathBuf::from("../outside"),
        bytes: Vec::new(),
        readonly: false,
        unix_mode: 0o600,
    }];
    let error = record_schema_three_stable(
        &invalid_seed,
        seed_namespace.clone(),
        StablePairPhase::Main,
        &[SmokeOperation::OpenRun],
    )
    .unwrap_err();
    assert!(error.to_string().contains("could not seed"));
    assert!(matches!(&error.primary, StablePairPrimaryError::Seed(_)));
    assert!(error.cleanup.is_none());
    record_real_smoke_stable_pair_in(seed_namespace).unwrap();

    let metadata_parent = tempfile::TempDir::new().unwrap();
    let metadata_namespace =
        StableSandboxNamespace::explicit(metadata_parent.path().join(STABLE_SANDBOX_NAMESPACE))
            .unwrap();
    let error = record_schema_three_stable_with_hooks(
        &smoke_factory(),
        metadata_namespace.clone(),
        StablePairPhase::Main,
        &[SmokeOperation::OpenRun],
        |host, _| {
            Err(SandboxError::InvalidEvidence {
                path: host.sandbox_root().to_path_buf(),
                reason: "injected metadata failure".to_owned(),
            })
        },
        read_initial_state,
        noop_stable_host,
        new_schema_three_sink,
        noop_schema_sink,
        noop_leak_oracle_facts,
        noop_stable_host,
    )
    .unwrap_err();
    assert!(error.to_string().contains("injected metadata failure"));
    assert!(matches!(&error.primary, StablePairPrimaryError::Sandbox(_)));
    assert!(error.cleanup.is_none());
    record_real_smoke_stable_pair_in(metadata_namespace).unwrap();

    let initial_parent = tempfile::TempDir::new().unwrap();
    let initial_namespace =
        StableSandboxNamespace::explicit(initial_parent.path().join(STABLE_SANDBOX_NAMESPACE))
            .unwrap();
    let error = record_schema_three_stable_with_hooks(
        &smoke_factory(),
        initial_namespace.clone(),
        StablePairPhase::Main,
        &[SmokeOperation::OpenRun],
        read_sandbox_metadata,
        |_| Err("injected initial-state failure".to_owned()),
        noop_stable_host,
        new_schema_three_sink,
        noop_schema_sink,
        noop_leak_oracle_facts,
        noop_stable_host,
    )
    .unwrap_err();
    assert!(error.to_string().contains("could not read"));
    assert!(
        matches!(&error.primary, StablePairPrimaryError::InitialState(_)),
        "{error:?}"
    );
    assert!(error.cleanup.is_none());
    record_real_smoke_stable_pair_in(initial_namespace).unwrap();

    let sink_parent = tempfile::TempDir::new().unwrap();
    let sink_namespace =
        StableSandboxNamespace::explicit(sink_parent.path().join(STABLE_SANDBOX_NAMESPACE))
            .unwrap();
    let error = record_schema_three_stable_with_hooks(
        &smoke_factory(),
        sink_namespace.clone(),
        StablePairPhase::Main,
        &[SmokeOperation::OpenRun],
        read_sandbox_metadata,
        read_initial_state,
        noop_stable_host,
        |_| Err("injected sink failure".to_owned()),
        noop_schema_sink,
        noop_leak_oracle_facts,
        noop_stable_host,
    )
    .unwrap_err();
    assert!(error.to_string().contains("injected sink failure"));
    assert!(matches!(&error.primary, StablePairPrimaryError::Trace(_)));
    assert!(error.cleanup.is_none());
    record_real_smoke_stable_pair_in(sink_namespace).unwrap();

    let trace_parent = tempfile::TempDir::new().unwrap();
    let trace_namespace =
        StableSandboxNamespace::explicit(trace_parent.path().join(STABLE_SANDBOX_NAMESPACE))
            .unwrap();
    let error = record_schema_three_stable_with_hooks(
        &smoke_factory(),
        trace_namespace.clone(),
        StablePairPhase::Main,
        &[SmokeOperation::OpenRun],
        read_sandbox_metadata,
        read_initial_state,
        |host| {
            std::fs::write(
                host.sandbox_root().join("system-temp/residual"),
                b"residual",
            )
            .unwrap();
        },
        new_schema_three_sink,
        noop_schema_sink,
        noop_leak_oracle_facts,
        noop_stable_host,
    )
    .unwrap_err();
    assert!(error.to_string().contains("could not record"));
    assert!(matches!(&error.primary, StablePairPrimaryError::Trace(_)));
    assert!(error.cleanup.is_none());
    record_real_smoke_stable_pair_in(trace_namespace).unwrap();

    let aggregate_parent = tempfile::TempDir::new().unwrap();
    let aggregate_namespace =
        StableSandboxNamespace::explicit(aggregate_parent.path().join(STABLE_SANDBOX_NAMESPACE))
            .unwrap();
    let error = record_schema_three_stable_with_hooks(
        &smoke_factory(),
        aggregate_namespace,
        StablePairPhase::Replay,
        &[SmokeOperation::OpenRun],
        read_sandbox_metadata,
        read_initial_state,
        |host| {
            std::fs::write(
                host.sandbox_root().join(SANDBOX_MARKER_FILE),
                b"corrupt marker",
            )
            .unwrap();
        },
        new_schema_three_sink,
        |sink| sink.rows.clear(),
        noop_leak_oracle_facts,
        noop_stable_host,
    )
    .unwrap_err();
    assert!(matches!(&error.primary, StablePairPrimaryError::Trace(_)));
    assert!(error.cleanup.is_some());
    let message = error.to_string();
    assert!(message.contains("stable walker replay failed"));
    assert!(message.contains("sandbox cleanup also failed"));

    let close_parent = tempfile::TempDir::new().unwrap();
    let close_namespace =
        StableSandboxNamespace::explicit(close_parent.path().join(STABLE_SANDBOX_NAMESPACE))
            .unwrap();
    let error = record_schema_three_stable_with_hooks(
        &smoke_factory(),
        close_namespace,
        StablePairPhase::Main,
        &[SmokeOperation::OpenRun],
        read_sandbox_metadata,
        read_initial_state,
        noop_stable_host,
        new_schema_three_sink,
        noop_schema_sink,
        noop_leak_oracle_facts,
        |host| {
            std::fs::write(
                host.sandbox_root().join(SANDBOX_MARKER_FILE),
                b"corrupt marker",
            )
            .unwrap();
        },
    )
    .unwrap_err();
    assert!(error.to_string().contains("could not close"));
    assert!(matches!(&error.primary, StablePairPrimaryError::Close(_)));
    assert!(error.cleanup.is_none());
}

#[test]
fn schema_three_sink_rejects_missing_corrupt_and_inconsistent_artifacts() {
    let factory = smoke_factory();
    assert!(
        new_schema_three_sink(&factory)
            .unwrap()
            .finish_smoke()
            .unwrap_err()
            .contains("no checkpoint")
    );

    let mut collision = new_schema_three_sink(&factory).unwrap();
    let reference = collision
        .put_value(ObjectKind::Reducer, json!({"stable": true}))
        .unwrap();
    let key = (reference.kind, reference.sha256.clone());
    collision.objects.insert(key.clone(), b"corrupt".to_vec());
    assert!(
        collision
            .put_value(ObjectKind::Reducer, json!({"stable": true}))
            .unwrap_err()
            .contains("different object bytes")
    );
    assert_eq!(collision.objects[&key], b"corrupt");
    assert!(
        collision
            .object_value(&reference)
            .unwrap_err()
            .contains("expected")
    );
    collision.objects.remove(&key);
    assert!(
        collision
            .object_value(&reference)
            .unwrap_err()
            .contains("absent")
    );

    let mut malformed_final = prepared_schema_three_sink(&factory);
    let malformed_state = malformed_final
        .put_value(
            ObjectKind::Reducer,
            json!({"workflow": {"active": "library"}, "modal": null}),
        )
        .unwrap();
    malformed_final.rows.last_mut().unwrap().reducer = malformed_state;
    let error = malformed_final.finish_smoke().unwrap_err();
    assert!(error.contains("not a LibraryState"), "{error}");

    let mut noncanonical_final = prepared_schema_three_sink(&factory);
    let final_ref = noncanonical_final.rows.last().unwrap().reducer.clone();
    let mut noncanonical_state = noncanonical_final.object_value(&final_ref).unwrap();
    noncanonical_state
        .as_object_mut()
        .unwrap()
        .remove("details");
    let noncanonical_ref = noncanonical_final
        .put_value(ObjectKind::Reducer, noncanonical_state)
        .unwrap();
    noncanonical_final.rows.last_mut().unwrap().reducer = noncanonical_ref;
    assert!(
        noncanonical_final
            .finish_smoke()
            .unwrap_err()
            .contains("canonical LibraryState")
    );

    let mut wrong_host_state = prepared_schema_three_sink(&factory);
    wrong_host_state.rows[2].reducer = wrong_host_state.rows[0].reducer.clone();
    assert!(
        wrong_host_state
            .finish_smoke()
            .unwrap_err()
            .contains("production Action")
    );

    let mut wrong_effect = prepared_schema_three_sink(&factory);
    *reducer_emitted(&mut wrong_effect.rows[1].cause) = json!("none");
    assert!(
        wrong_effect
            .validate_reducer_transitions()
            .unwrap_err()
            .contains("production Effect")
    );

    let mut repeated_initial = prepared_schema_three_sink(&factory);
    repeated_initial.rows[1].cause = TransitionCause::Initial;
    assert!(
        repeated_initial
            .validate_reducer_transitions()
            .unwrap_err()
            .contains("only the first")
    );

    let mut unchanged_session = prepared_schema_three_sink(&factory);
    unchanged_session.rows[1].cause = TransitionCause::Session {
        requested: json!({"operation": "session"}),
        resolved: json!({
            "input": {"kind": "event", "event": {
                "type": "key", "code": "Enter", "modifiers": "",
            }},
            "semantic_target": null,
        }),
        event: json!({"type": "key", "code": "Enter", "modifiers": ""}),
        handling: json!("consumed"),
    };
    unchanged_session.rows[1].reducer = unchanged_session.rows[0].reducer.clone();
    unchanged_session.validate_reducer_transitions().unwrap();
    unchanged_session.rows[1].reducer = unchanged_session.rows[2].reducer.clone();
    assert!(
        unchanged_session
            .validate_reducer_transitions()
            .unwrap_err()
            .contains("session checkpoint")
    );

    let mut nonterminal_final = prepared_schema_three_sink(&factory);
    let mut only_run = nonterminal_final.rows[2].clone();
    only_run.sequence = 0;
    only_run.previous_row_sha256 = None;
    only_run.event_chain = None;
    only_run.operation_index = None;
    only_run.boundary = TimelineBoundary::Initial;
    only_run.cause = TransitionCause::Initial;
    nonterminal_final.rows = vec![only_run];
    assert!(
        nonterminal_final
            .finish_smoke()
            .unwrap_err()
            .contains("return the real reducer")
    );

    let mut missing = prepared_schema_three_sink(&factory);
    let missing_ref = missing.rows.last().unwrap().geometry.clone();
    missing
        .objects
        .remove(&(missing_ref.kind, missing_ref.sha256));
    assert!(missing.finish_smoke().unwrap_err().contains("absent"));

    let mut wrong_canvas = prepared_schema_three_sink(&factory);
    for row in &mut wrong_canvas.rows {
        row.viewport.width = 59;
    }
    for index in 1..wrong_canvas.rows.len() {
        wrong_canvas.rows[index].previous_row_sha256 =
            Some(timeline_row_digest(&wrong_canvas.rows[index - 1]).unwrap());
    }
    assert!(
        wrong_canvas
            .finish_smoke()
            .unwrap_err()
            .contains("timeline canvas")
    );

    let mut wrong_viewport = prepared_schema_three_sink(&factory);
    wrong_viewport.rows.last_mut().unwrap().viewport.width = 59;
    assert!(
        wrong_viewport
            .finish_smoke()
            .unwrap_err()
            .contains("timeline viewport")
    );

    let mut wrong_cast = prepared_schema_three_sink(&factory);
    let frame_ref = wrong_cast.rows.last().unwrap().styled_frame.clone();
    let mut extra_frame: StyledFrameSnapshot =
        serde_json::from_value(wrong_cast.object_value(&frame_ref).unwrap()).unwrap();
    extra_frame.cells[0].symbol = "X".to_owned();
    wrong_cast
        .cast
        .record_snapshot(FRAME_INTERVAL, &extra_frame)
        .unwrap();
    assert!(
        wrong_cast
            .finish_smoke()
            .unwrap_err()
            .contains("did not rebuild")
    );
}

#[test]
fn schema_three_sink_failure_publishes_no_checkpoint_or_artifact() {
    let factory = smoke_factory();
    let mut sink = new_schema_three_sink(&factory).unwrap();
    let mut checkpoint = checkpoint_with_cause(EngineCause::Initial);
    checkpoint.boundary = EngineBoundary::Initial;
    checkpoint.frontend.styled_frame.cells.clear();
    let rows = sink.rows.clone();
    let objects = sink.objects.clone();
    let cast = sink.cast.as_bytes().to_vec();

    assert!(sink.record(checkpoint).is_err());
    assert_eq!(sink.rows, rows);
    assert_eq!(sink.objects, objects);
    assert_eq!(sink.cast.as_bytes(), cast);
}

#[test]
fn not_presented_distinct_frame_is_absent_and_its_interval_reaches_the_next_frame() {
    let factory = smoke_factory();
    let trace = record_schema_three_main(&factory).unwrap();
    let reference = &trace.rows[0].styled_frame;
    let bytes = &trace.objects[&(reference.kind, reference.sha256.clone())];
    let value = validate_object(reference, bytes).unwrap();
    let mut first: StyledFrameSnapshot = serde_json::from_value(value).unwrap();
    first.cells[0].symbol = "A".to_owned();
    let mut diagnostic = first.clone();
    diagnostic.cells[0].symbol = "¤".to_owned();
    let mut last = first.clone();
    last.cells[0].symbol = "B".to_owned();

    let mut recorder = AsciicastRecorder::new(60, 24).unwrap();
    record_presentation(&mut recorder, Presentation::Presented, &first).unwrap();
    record_presentation(&mut recorder, Presentation::NotPresented, &diagnostic).unwrap();
    record_presentation(&mut recorder, Presentation::Presented, &last).unwrap();

    let lines = recorder
        .as_bytes()
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_slice::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[1][0], json!(0.1));
    assert_eq!(lines[2][0], json!(0.2));
    assert!(!String::from_utf8_lossy(recorder.as_bytes()).contains('¤'));
}

#[test]
fn schema_three_cause_projection_covers_session_refusal_and_bounds() {
    assert_eq!(
        timeline_boundary(EngineBoundary::Session),
        TimelineBoundary::Session
    );
    assert_eq!(
        smoke_event_value(SmokeEvent {
            key: KeyCode::F(24),
            modifiers: KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        }),
        json!({
            "type": "key",
            "code": {"F": 24},
            "modifiers": "SHIFT | CONTROL",
        })
    );

    assert!(
        transition_cause(
            TimelinePhase::Operations,
            &EngineCause::NotApplicable {
                dispatch_sequence: 3,
                operation_index: Some(2),
                operation: SmokeOperation::OpenRun,
                resolved: SmokeResolution {
                    event: SmokeEvent {
                        key: KeyCode::Enter,
                        modifiers: KeyModifiers::NONE,
                    },
                },
            },
        )
        .unwrap_err()
        .contains("no not-applicable")
    );

    for (handling, expected) in [
        (SmokeHandling::Consumed, "consumed"),
        (SmokeHandling::Ignored, "ignored"),
    ] {
        let (_, _, cause) = transition_cause(
            TimelinePhase::Operations,
            &EngineCause::Session {
                dispatch_sequence: 1,
                operation_index: Some(0),
                operation: SmokeOperation::OpenRun,
                resolved: SmokeResolution {
                    event: SmokeEvent {
                        key: KeyCode::Enter,
                        modifiers: KeyModifiers::NONE,
                    },
                },
                event: SmokeEvent {
                    key: KeyCode::Enter,
                    modifiers: KeyModifiers::NONE,
                },
                handling,
            },
        )
        .unwrap();
        assert!(matches!(
            cause,
            TransitionCause::Session { handling, .. } if handling == json!(expected)
        ));
    }

    #[cfg(target_pointer_width = "64")]
    {
        let overflow = usize::try_from(u64::from(u32::MAX) + 1).unwrap();
        assert!(checked_u32(overflow, "dispatch sequence").is_err());
        let host = EngineCause::Host {
            dispatch_sequence: 0,
            operation_index: None,
            operation: SmokeOperation::FinalLiveness,
            round: usize::from(u16::MAX) + 1,
            request: Effect::None,
            response: Action::Back,
            emitted: Effect::None,
        };
        assert!(transition_cause(TimelinePhase::FinalLiveness, &host).is_err());
    }
}

#[test]
fn schema_three_termination_check_rejects_every_dangling_effect_shape() {
    let factory = smoke_factory();
    let trace = record_schema_three_main(&factory).unwrap();
    let reducer = trace.rows[1].clone();
    let initial = trace.rows[0].clone();
    let host = trace.rows[2].clone();

    assert!(
        validate_effect_chain_termination(&[reducer.clone(), initial])
            .unwrap_err()
            .contains("before another event")
    );
    assert!(
        validate_effect_chain_termination(std::slice::from_ref(&host))
            .unwrap_err()
            .contains("no pending")
    );
    let mut wrong_host = host;
    if let TransitionCause::Host { request, .. } = &mut wrong_host.cause {
        *request = json!({"wrong": true});
    }
    assert!(
        validate_effect_chain_termination(&[reducer.clone(), wrong_host])
            .unwrap_err()
            .contains("predecessor")
    );
    assert!(
        validate_effect_chain_termination(&[reducer])
            .unwrap_err()
            .contains("ends with")
    );
}

#[test]
fn random_corpus_capture_keeps_diagnostic_recording_without_full_leak_scans() {
    let (mut frontend, mut host) = corpus_engine().into_parts();
    let capture = host
        .capture_checkpoint(
            frontend.observe().unwrap(),
            CheckpointCauseProjection::Observation,
        )
        .unwrap();

    assert!(capture.host.leak_context.is_none());
    drop(frontend);
    host.close().unwrap();
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn stable_corpus_capture_snapshots_facts_after_registering_a_new_draft() {
    let parent = tempfile::TempDir::new().unwrap();
    let namespace =
        StableSandboxNamespace::explicit(parent.path().join(STABLE_SANDBOX_NAMESPACE)).unwrap();
    let (mut frontend, mut host) = stable_corpus_engine(namespace).into_parts();
    let drafts = crate::cli::create_owned_drafts_dir(&host.inner.host.roots().data).unwrap();
    let raw_path = drafts.join("skit-new-000000.py");
    std::fs::write(&raw_path, b"print('checkpoint facts')\n").unwrap();
    let capture = host
        .capture_checkpoint(
            frontend.observe().unwrap(),
            CheckpointCauseProjection::Observation,
        )
        .unwrap();

    let context = capture.host.leak_context.unwrap();
    assert_eq!(context.sandbox, host.leak_sandbox.clone().unwrap());
    let draft = context
        .facts
        .renderer_drafts
        .values()
        .find(|draft| draft.raw_path == raw_path.display().to_string())
        .unwrap();
    let artifact = &context.facts.artifacts[&draft.projected_path];
    assert!(artifact.raw_path_spellings.contains(&draft.raw_path));
    assert!(!artifact.source_identities.is_empty());
    assert!(!artifact.modified_values.is_empty());
    assert_eq!(
        capture.host.observation.drafts[0]["path"],
        draft.projected_path
    );
    drop(frontend);
    host.close().unwrap();
}
