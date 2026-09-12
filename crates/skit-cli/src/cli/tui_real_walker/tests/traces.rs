//! Recorded corpus, host preparation, frontend, and host engine contracts.

use super::*;

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn synthetic_recorded_corpus_profiles() -> Vec<RecordedRealTrace> {
    let template = record_real_smoke_main().unwrap();
    let operations = canonical_corpus_operations()
        .iter()
        .map(corpus_operation_value)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let final_liveness = corpus_operation_value(&CorpusOperation::FinalLiveness).unwrap();
    let root = synthetic_stable_root(false).to_owned();
    skit_tui_walker_support::required_review_profiles()
        .iter()
        .map(|required| {
            let id = SafeProfileId::try_from(required.id).unwrap();
            let mut recorded = template.clone();
            recorded.trace.review_profile = id.clone();
            recorded.trace.locale = required.locale.to_owned();
            recorded.trace.viewport = required.viewport;
            recorded.trace.operations = operations.clone();
            recorded.trace.final_liveness_requested = final_liveness.clone();
            for row in &mut recorded.trace.rows {
                row.profile = id.clone();
                row.locale = required.locale.to_owned();
            }
            recorded.sandbox = SandboxMetadata::stable(
                template.sandbox.platform(),
                root.clone(),
                BTreeSet::from([id]),
            )
            .unwrap();
            recorded.leak_oracle_facts = LeakOracleFacts::default();
            recorded
        })
        .collect()
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn recorded_corpus_requires_exact_shared_profiles_and_values() {
    let profiles = synthetic_recorded_corpus_profiles();
    let corpus = RecordedRealCorpus::new(profiles.clone()).unwrap();
    assert_eq!(corpus.profiles.len(), 4);
    assert_eq!(corpus.sandbox.profiles().len(), 4);
    let mut independent_facts = profiles.clone();
    independent_facts[0].leak_oracle_facts.artifacts.insert(
        "projected-only".to_owned(),
        ArtifactLeakOracleFact::default(),
    );
    let pair = StableCorpusTracePair::new(
        corpus.clone(),
        RecordedRealCorpus::new(independent_facts).unwrap(),
    )
    .unwrap();
    assert_eq!(pair.main.sandbox, pair.replay.sandbox);
    assert_ne!(
        pair.main.profiles[0].leak_oracle_facts,
        pair.replay.profiles[0].leak_oracle_facts
    );

    let mut missing = profiles.clone();
    missing.pop();
    assert_eq!(
        RecordedRealCorpus::new(missing).unwrap_err(),
        "the recorded corpus does not contain the exact four profiles"
    );

    let mut extra = profiles.clone();
    extra.push(profiles[0].clone());
    assert_eq!(
        RecordedRealCorpus::new(extra).unwrap_err(),
        "the recorded corpus does not contain the exact four profiles"
    );

    let mut wrong_order = profiles.clone();
    wrong_order[0].trace.review_profile = profiles[1].review_profile.clone();
    assert_eq!(
        RecordedRealCorpus::new(wrong_order).unwrap_err(),
        "the recorded corpus profiles are not in required order"
    );

    let mut wrong_locale = profiles.clone();
    wrong_locale[3].trace.locale = "en".to_owned();
    assert_eq!(
        RecordedRealCorpus::new(wrong_locale).unwrap_err(),
        "a recorded corpus profile has the wrong locale or viewport"
    );

    let mut wrong_viewport = profiles.clone();
    wrong_viewport[3].trace.viewport.width = 1;
    assert_eq!(
        RecordedRealCorpus::new(wrong_viewport).unwrap_err(),
        "a recorded corpus profile has the wrong locale or viewport"
    );

    let mut wrong_operation = profiles.clone();
    wrong_operation[3].trace.operations[0] = json!({"wrong": true});
    assert_eq!(
        RecordedRealCorpus::new(wrong_operation).unwrap_err(),
        "recorded corpus profiles do not share one operation vector"
    );

    let mut wrong_liveness = profiles.clone();
    wrong_liveness[3].trace.final_liveness_requested = json!({"wrong": true});
    assert_eq!(
        RecordedRealCorpus::new(wrong_liveness).unwrap_err(),
        "recorded corpus profiles do not share one operation vector"
    );

    let mut empty_operations = profiles.clone();
    for recorded in &mut empty_operations {
        recorded.trace.operations.clear();
    }
    assert_eq!(
        RecordedRealCorpus::new(empty_operations).unwrap_err(),
        "the recorded corpus has an empty operation vector"
    );

    let mut no_liveness = profiles.clone();
    no_liveness[3].trace.rows.last_mut().unwrap().liveness = None;
    assert_eq!(
        RecordedRealCorpus::new(no_liveness).unwrap_err(),
        "a recorded corpus profile did not pass final liveness"
    );

    let mut refused = profiles.clone();
    refused[3].trace.rows[0].cause = TransitionCause::Session {
        requested: Value::Null,
        resolved: json!({"refusal": "unavailable"}),
        event: Value::Null,
        handling: json!("not_applicable"),
    };
    assert_eq!(
        RecordedRealCorpus::new(refused).unwrap_err(),
        "a recorded corpus profile contains a refused operation"
    );

    let mut wrong_timeline = profiles.clone();
    wrong_timeline[3].trace.rows[0].profile = wrong_timeline[0].review_profile.clone();
    assert_eq!(
        RecordedRealCorpus::new(wrong_timeline).unwrap_err(),
        "a recorded corpus timeline has the wrong profile"
    );

    let platform = profiles[3].sandbox.platform();
    let shared_root = profiles[3].sandbox.root().to_owned();

    let mut split_root = profiles.clone();
    split_root[3].sandbox = SandboxMetadata::stable(
        platform,
        synthetic_stable_root(true),
        BTreeSet::from([split_root[3].review_profile.clone()]),
    )
    .unwrap();
    assert_eq!(
        RecordedRealCorpus::new(split_root).unwrap_err(),
        "recorded corpus sandboxes do not share one stable namespace"
    );

    let mut random = profiles.clone();
    random[3].sandbox = SandboxMetadata::random(
        platform,
        shared_root.clone(),
        random[3].review_profile.clone(),
    )
    .unwrap();
    assert_eq!(
        RecordedRealCorpus::new(random).unwrap_err(),
        "recorded corpus sandboxes do not share one stable namespace"
    );

    let mut wrong_singleton = profiles.clone();
    wrong_singleton[3].sandbox = SandboxMetadata::stable(
        platform,
        shared_root,
        BTreeSet::from([SafeProfileId::try_from("wrong-profile").unwrap()]),
    )
    .unwrap();
    assert_eq!(
        RecordedRealCorpus::new(wrong_singleton).unwrap_err(),
        "a recorded corpus profile has invalid singleton metadata"
    );

    let mut changed_trace = RecordedRealCorpus::new(profiles.clone()).unwrap();
    changed_trace.profiles[3].trace.cast.push(b'X');
    assert_eq!(
        StableCorpusTracePair::new(corpus.clone(), changed_trace).unwrap_err(),
        "stable corpus main and replay traces differ at pseudo-120x12"
    );

    let mut other_root = profiles;
    for recorded in &mut other_root {
        recorded.sandbox = SandboxMetadata::stable(
            platform,
            synthetic_stable_root(true),
            BTreeSet::from([recorded.review_profile.clone()]),
        )
        .unwrap();
    }
    let other_root = RecordedRealCorpus::new(other_root).unwrap();
    assert_eq!(
        StableCorpusTracePair::new(corpus, other_root).unwrap_err(),
        "stable corpus generations use different aggregate metadata"
    );
}

#[test]
fn corpus_result_preserves_primary_and_cleanup_errors() {
    assert_eq!(
        combine_corpus_result_after_close(Ok(7), Ok::<(), &str>(())),
        Ok(7)
    );
    assert_eq!(
        combine_corpus_result_after_close(Ok(7), Err("close")),
        Err("could not close the real corpus host: close".to_owned())
    );
    assert_eq!(
        combine_corpus_result_after_close::<u8>(Err("primary".to_owned()), Ok::<(), &str>(())),
        Err("primary".to_owned())
    );
    assert_eq!(
        combine_corpus_result_after_close::<u8>(Err("primary".to_owned()), Err("close")),
        Err("primary; real corpus host cleanup also failed: close".to_owned())
    );
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn corpus_host_preparation_closes_metadata_and_state_failures() {
    let profile = smoke_factory().review_profile;
    let context = |message| format!("context: {message}");

    let metadata_parent = tempfile::TempDir::new().unwrap();
    let metadata_namespace =
        StableSandboxNamespace::explicit(metadata_parent.path().join(STABLE_SANDBOX_NAMESPACE))
            .unwrap();
    let metadata_host =
        RealWalkerHost::spawn_stable_in(smoke_seed(), profile.clone(), metadata_namespace.clone())
            .unwrap();
    let metadata_error = prepare_corpus_host_with(
        metadata_host,
        &profile,
        &context,
        |_, _| Err("metadata".to_owned()),
        read_initial_state,
    )
    .unwrap_err();
    assert_eq!(metadata_error, "context: metadata");
    record_corpus_profile(
        &smoke_factory(),
        metadata_namespace,
        &[CorpusOperation::Focus { gained: false }],
        StablePairPhase::Main,
    )
    .unwrap();

    let state_parent = tempfile::TempDir::new().unwrap();
    let state_namespace =
        StableSandboxNamespace::explicit(state_parent.path().join(STABLE_SANDBOX_NAMESPACE))
            .unwrap();
    let state_host =
        RealWalkerHost::spawn_stable_in(smoke_seed(), profile.clone(), state_namespace.clone())
            .unwrap();
    let state_error = prepare_corpus_host_with(
        state_host,
        &profile,
        &context,
        |host, profile| {
            host.sandbox_metadata(profile)
                .map_err(|error| error.to_string())
        },
        |_| Err("state".to_owned()),
    )
    .unwrap_err();
    assert_eq!(state_error, "context: state");
    record_corpus_profile(
        &smoke_factory(),
        state_namespace,
        &[CorpusOperation::Focus { gained: false }],
        StablePairPhase::Main,
    )
    .unwrap();
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn corpus_profile_recorder_reports_phase_and_releases_failures() {
    let replay_parent = tempfile::TempDir::new().unwrap();
    let replay_namespace =
        StableSandboxNamespace::explicit(replay_parent.path().join(STABLE_SANDBOX_NAMESPACE))
            .unwrap();
    let replay = record_corpus_profile(
        &smoke_factory(),
        replay_namespace,
        &[CorpusOperation::Focus { gained: false }],
        StablePairPhase::Replay,
    )
    .unwrap();
    assert_eq!(replay.operations.len(), 1);

    let frontend_parent = tempfile::TempDir::new().unwrap();
    let frontend_namespace =
        StableSandboxNamespace::explicit(frontend_parent.path().join(STABLE_SANDBOX_NAMESPACE))
            .unwrap();
    let mut invalid_frontend = smoke_factory();
    invalid_frontend.size = Size::new(0, 24);
    let error = record_corpus_profile(
        &invalid_frontend,
        frontend_namespace.clone(),
        &[],
        StablePairPhase::Main,
    )
    .unwrap_err();
    assert!(error.contains("engine-smoke-60x24"));
    record_corpus_profile(
        &smoke_factory(),
        frontend_namespace,
        &[CorpusOperation::Focus { gained: false }],
        StablePairPhase::Main,
    )
    .unwrap();

    let operation_parent = tempfile::TempDir::new().unwrap();
    let operation_namespace =
        StableSandboxNamespace::explicit(operation_parent.path().join(STABLE_SANDBOX_NAMESPACE))
            .unwrap();
    let invalid_operation = CorpusOperation::ScreenHit(ScreenTarget::FilePickerEntry {
        relative: PathBuf::from("../outside"),
    });
    let error = record_corpus_profile(
        &smoke_factory(),
        operation_namespace.clone(),
        &[invalid_operation],
        StablePairPhase::Main,
    )
    .unwrap_err();
    assert!(error.contains("operation 1"));
    record_corpus_profile(
        &smoke_factory(),
        operation_namespace,
        &[CorpusOperation::Focus { gained: false }],
        StablePairPhase::Main,
    )
    .unwrap();
}

struct FailPreferencesOpenHost {
    inner: RealHostBoundary,
}

impl FailPreferencesOpenHost {
    fn new(inner: RealHostBoundary) -> Self {
        Self { inner }
    }

    fn close(self) -> Result<(), String> {
        self.inner.close().map_err(|error| error.to_string())
    }
}

impl HostAdapter<RealFrontend> for FailPreferencesOpenHost {
    type Observation = HostObservation;

    fn serve(&mut self, _effect: Effect) -> Result<Action, String> {
        Err("injected Preferences host failure".to_owned())
    }

    fn capture_checkpoint(
        &mut self,
        frontend: FrontendObservation,
        cause: CheckpointCauseProjection<'_, Action, Effect>,
    ) -> Result<CheckpointCapture<FrontendObservation, Self::Observation>, String> {
        self.inner.capture_checkpoint(frontend, cause)
    }
}

#[derive(Default)]
struct BoundarySink(Vec<EngineBoundary>);

impl CheckpointSink<CheckpointFor<RealFrontend, FailPreferencesOpenHost>> for BoundarySink {
    fn record(
        &mut self,
        checkpoint: CheckpointFor<RealFrontend, FailPreferencesOpenHost>,
    ) -> Result<(), String> {
        self.0.push(checkpoint.boundary);
        Ok(())
    }
}

#[test]
#[should_panic(expected = "the test checkpoint must be a reducer action")]
fn reducer_emitted_rejects_a_non_reducer_checkpoint() {
    let mut cause = TransitionCause::Initial;
    let _ = reducer_emitted(&mut cause);
}

#[test]
fn checkpoint_leak_names_the_cause_and_keeps_the_sink_unchanged() {
    let factory = smoke_factory();
    let mut sink = prepared_schema_three_sink(&factory);
    let rows = sink.rows.clone();
    let objects = sink.objects.clone();
    let cast = sink.cast_bytes().to_vec();
    let checkpoint = checkpoint_with_cause(EngineCause::Initial);
    let sandbox = SandboxMetadata::stable(
        SandboxPlatform::Linux,
        "/sandbox/skit-ui-walker-v1",
        BTreeSet::from([factory.review_profile]),
    )
    .unwrap();
    let facts = LeakOracleFacts {
        ambient_paths: BTreeSet::from(["/ambient/review-root".to_owned()]),
        ..LeakOracleFacts::default()
    };

    let error = sink
        .record_checkpoint(ProjectedCheckpoint {
            phase: EnginePhase::Operations,
            boundary: EngineBoundary::Host,
            operation_index: Some(7),
            event_chain: None,
            cause: TransitionCause::Host {
                operation_index: Some(7),
                round: 0,
                request: json!({"add": [{"commit": {"entry": {"payload": {
                    "stored_name": "/ambient/review-root/item",
                }}}}]}),
                response: json!("none"),
                emitted: json!("none"),
            },
            frontend: checkpoint.frontend,
            host: checkpoint.host,
            leak_context: Some(CheckpointLeakContext { sandbox, facts }),
        })
        .unwrap_err();

    assert!(error.contains("phase Operations"), "{error}");
    assert!(error.contains("operation index Some(7)"), "{error}");
    assert!(error.contains("cause"), "{error}");
    assert!(
        error.contains("/request/add/0/commit/entry/payload/stored_name"),
        "{error}"
    );
    assert_eq!(sink.rows, rows);
    assert_eq!(sink.objects, objects);
    assert_eq!(sink.cast_bytes(), cast);
}

fn trace_frame_text(trace: &RealTrace, row: &TimelineRow) -> String {
    let bytes = &trace.objects[&(row.styled_frame.kind, row.styled_frame.sha256.clone())];
    let value = validate_object(&row.styled_frame, bytes).unwrap();
    let frame: StyledFrameSnapshot = serde_json::from_value(value).unwrap();
    frame.readable_lines().unwrap().join("\n")
}

#[test]
fn smoke_resolution_projection_is_total_for_synthetic_liveness() {
    let event = SmokeEvent {
        key: KeyCode::Esc,
        modifiers: KeyModifiers::NONE,
    };
    assert_eq!(
        smoke_resolution_value(
            TimelinePhase::Operations,
            SmokeOperation::FinalLiveness,
            SmokeResolution { event },
        ),
        json!({
            "input": {"kind": "event", "event": {
                "type": "key", "code": "Esc", "modifiers": "",
            }},
            "semantic_target": {"synthetic": "final_liveness"},
        })
    );
}

fn open_real_preferences(frontend: &mut RealFrontend, host: &mut RealHostBoundary) {
    let request = frontend.reduce(Action::OpenPreferences).unwrap();
    assert!(matches!(
        &request,
        Effect::Open {
            request: HostRequest::Preferences,
            selector: None,
        }
    ));
    let response = host.serve(request).unwrap();
    assert!(matches!(response, Action::Present(Screen::Preferences(_))));
    assert_eq!(frontend.reduce(response).unwrap(), Effect::None);
}

#[test]
fn reducer_replay_compares_native_paths_without_accepting_schema_or_target_drift() {
    let (mut frontend, mut host) = smoke_factory().create().unwrap();
    open_real_preferences(&mut frontend, &mut host);
    let mut state = frontend.state;
    state.update(Action::Preferences(
        skit_ui::PreferencesAction::PresentAgentSkillTargets(vec![skit_application::AgentTarget {
            name: "codex".to_owned(),
            scope: skit_application::AgentScope::User,
            base: PathBuf::from("/declared/agent"),
        }]),
    ));
    let previous = json_value(&state);
    let action = Action::Preferences(skit_ui::PreferencesAction::ActivateAgentSkillTarget(0));
    let effect = state.update(action.clone());
    let current = json_value(&state);
    let action = json_value(&action);
    let mut recorded = json_value(&effect);
    recorded["preferences"]["install_agent_skill"]["skills_dir"] = json!("/declared/agent//skills");
    validate_reducer_action(&previous, &current, &action, &recorded).unwrap();
    recorded["preferences"]["install_agent_skill"]["extra"] = json!(true);
    assert!(validate_reducer_action(&previous, &current, &action, &recorded).is_err());
    recorded["preferences"]["install_agent_skill"]
        .as_object_mut()
        .unwrap()
        .remove("extra");
    recorded["preferences"]["install_agent_skill"]["skills_dir"] = json!("/elsewhere/skills");
    assert!(validate_reducer_action(&previous, &current, &action, &recorded).is_err());
    assert!(validate_reducer_action(&previous, &current, &action, &json!(42)).is_err());
}

#[test]
fn real_frontend_and_smoke_sink_reject_out_of_contract_inputs() {
    assert!(
        RealFrontend::new(LibraryState::default(), Locale::En, Size::new(0, 24))
            .err()
            .unwrap()
            .contains("positive")
    );
    let mut invalid_factory = smoke_factory();
    invalid_factory.size = Size::new(0, 24);
    assert!(invalid_factory.create().err().unwrap().contains("positive"));
    let host = RealWalkerHost::spawn(smoke_seed()).unwrap();
    assert!(
        smoke_factory()
            .frontend_for_host_with_initial_state(host, |_| {
                Err("injected random initial-state failure".to_owned())
            })
            .err()
            .unwrap()
            .contains("injected random initial-state failure")
    );

    let (mut consumed_frontend, _) = smoke_factory().create().unwrap();
    assert_eq!(
        consumed_frontend
            .dispatch(SmokeEvent {
                key: KeyCode::Char('c'),
                modifiers: KeyModifiers::CONTROL,
            })
            .unwrap(),
        DispatchOutcome::Session(SmokeHandling::Consumed)
    );

    let (mut frontend, _) = smoke_factory().create().unwrap();
    assert_eq!(
        frontend
            .dispatch(SmokeEvent {
                key: KeyCode::F(24),
                modifiers: KeyModifiers::NONE,
            })
            .unwrap(),
        DispatchOutcome::Session(SmokeHandling::Ignored)
    );
    assert_eq!(frontend.effect_route(&Effect::Quit), EffectRoute::Quit);
    assert_eq!(
        frontend.parity_snapshot().unwrap(),
        FrontendParity {
            state: frontend.state.clone(),
            session: frontend.session_value().unwrap(),
            locale: Locale::En,
        }
    );

    let mut trace = MemoryTrace::default();
    assert!(
        trace
            .record(checkpoint_with_cause(EngineCause::NotApplicable {
                dispatch_sequence: 0,
                operation_index: Some(0),
                operation: SmokeOperation::OpenRun,
                resolved: SmokeResolution {
                    event: SmokeEvent {
                        key: KeyCode::Enter,
                        modifiers: KeyModifiers::NONE,
                    },
                },
            }))
            .unwrap_err()
            .contains("did not cross")
    );
    assert!(
        trace
            .record(checkpoint_with_cause(EngineCause::Reducer {
                dispatch_sequence: 0,
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
                action: Action::Back,
                emitted: Effect::None,
            }))
            .unwrap_err()
            .contains("changed shape")
    );
    assert!(
        trace
            .record(checkpoint_with_cause(EngineCause::Host {
                dispatch_sequence: 0,
                operation_index: Some(0),
                operation: SmokeOperation::OpenRun,
                round: 0,
                request: Effect::None,
                response: Action::Back,
                emitted: Effect::None,
            }))
            .unwrap_err()
            .contains("changed shape")
    );
}

#[test]
fn real_frontend_switches_locale_only_when_it_consumes_preferences_saved() {
    let (mut frontend, _) = smoke_factory().create().unwrap();
    assert_eq!(frontend.locale, Locale::En);

    assert_eq!(
        frontend
            .reduce(Action::Preferences(PreferencesAction::SetLanguage(
                "zh-TW".to_owned(),
            )))
            .unwrap(),
        Effect::None
    );
    assert_eq!(frontend.locale, Locale::En);

    assert_eq!(
        frontend
            .reduce(Action::SetStatus("host failure".to_owned()))
            .unwrap(),
        Effect::None
    );
    assert_eq!(frontend.locale, Locale::En);
    assert_eq!(frontend.reduce(Action::ClearStatus).unwrap(), Effect::None);
    assert_eq!(frontend.locale, Locale::En);

    for (tag, expected) in [
        ("en", Locale::En),
        ("zh-CN", Locale::ZhCn),
        ("zh-TW", Locale::ZhTw),
        ("x-pseudo", Locale::Pseudo),
        ("zh", Locale::ZhCn),
        ("not-a-supported-locale", Locale::En),
    ] {
        assert_eq!(
            frontend
                .reduce(Action::PreferencesSaved {
                    locale: tag.to_owned(),
                    message: "saved".to_owned(),
                })
                .unwrap(),
            Effect::None
        );
        assert_eq!(frontend.locale, expected, "{tag}");
    }
}

#[test]
fn real_preferences_validation_failure_keeps_the_frontend_locale() {
    let (mut frontend, mut host) = smoke_factory().create().unwrap();
    open_real_preferences(&mut frontend, &mut host);
    assert_eq!(
        frontend
            .reduce(Action::Preferences(PreferencesAction::SetLanguage(
                "zh-TW".to_owned(),
            )))
            .unwrap(),
        Effect::None
    );
    let missing = host.host.sandbox_root().join("missing-bash");
    assert_eq!(
        frontend
            .reduce(Action::Preferences(PreferencesAction::SetBashPath(
                missing.display().to_string(),
            )))
            .unwrap(),
        Effect::None
    );
    let request = frontend
        .reduce(Action::Preferences(PreferencesAction::Save))
        .unwrap();
    assert!(matches!(request, Effect::Preferences(_)));
    assert_eq!(frontend.locale, Locale::En);

    let response = host.serve(request).unwrap();
    assert!(matches!(
        response,
        Action::Preferences(PreferencesAction::ValidationFailed(_))
    ));
    assert_eq!(frontend.reduce(response).unwrap(), Effect::None);
    let observation = frontend.observe().unwrap();
    assert_eq!(observation.locale, Locale::En);
    assert!(
        observation
            .styled_frame
            .readable_lines()
            .unwrap()
            .join("\n")
            .contains("Preferences")
    );
    drop(frontend);
    host.close().unwrap();
}

#[test]
fn real_host_serve_error_keeps_locale_and_records_no_response_checkpoint() {
    let factory = smoke_factory();
    let (frontend, host) = factory.create().unwrap();
    let host = FailPreferencesOpenHost::new(host);
    let mut engine = WalkerEngine::new(frontend, host, EFFECT_LIMIT).unwrap();
    let mut sink = BoundarySink::default();
    engine.start(&mut sink).unwrap();

    let error = engine
        .run_operation(SmokeOperation::OpenPreferences, &mut sink)
        .unwrap_err();
    assert_eq!(error, "injected Preferences host failure");
    assert_eq!(engine.frontend().locale, Locale::En);
    assert_eq!(sink.0, [EngineBoundary::Initial, EngineBoundary::Reducer]);

    let (frontend, host) = engine.into_parts();
    drop(frontend);
    host.close().unwrap();
}

#[test]
fn real_frontend_parity_includes_the_live_locale() {
    let (frontend, _) = smoke_factory().create().unwrap();
    let changed_locale =
        RealFrontend::new(frontend.state.clone(), Locale::ZhTw, smoke_factory().size).unwrap();

    assert_ne!(
        frontend.parity_snapshot().unwrap(),
        changed_locale.parity_snapshot().unwrap()
    );
}

#[test]
fn real_preferences_operation_switches_the_first_host_frame_and_replays_exactly() {
    for (target, library, preferences, saved) in [
        (
            LocaleSmokeTarget::SimplifiedChinese,
            "工具库",
            "偏好设置",
            "偏好设置已保存",
        ),
        (
            LocaleSmokeTarget::TraditionalChinese,
            "工具庫",
            "偏好設定",
            "偏好設定已儲存",
        ),
    ] {
        let tag = target.tag();
        let main = record_real_locale_smoke(target).unwrap();
        let replay = record_real_locale_smoke(target).unwrap();
        assert_eq!(main.trace, replay.trace, "{tag}");
        assert_eq!(main.locale, "en");
        assert_eq!(main.operations.len(), target.next_count() + 5, "{tag}");

        let saved_index = main
            .rows
            .iter()
            .position(|row| {
                matches!(
                    &row.cause,
                    TransitionCause::Host { response, .. }
                        if response.get("preferences_saved").is_some()
                )
            })
            .unwrap();
        let save_request = &main.rows[saved_index - 1];
        let host_cause = json_value(&main.rows[saved_index].cause);
        assert_eq!(save_request.locale, "en");
        assert_eq!(save_request.boundary, TimelineBoundary::UserAction);
        assert_eq!(
            save_request.cause,
            TransitionCause::Reducer {
                requested: json!({"operation": "save_preferences"}),
                resolved: json!({
                    "input": {"kind": "event", "event": {
                        "type": "key", "code": {"Char": "s"}, "modifiers": "CONTROL",
                    }},
                    "semantic_target": {"command": "save_preferences"},
                }),
                event: json!({
                    "type": "key", "code": {"Char": "s"}, "modifiers": "CONTROL",
                }),
                action: json!({"preferences": "save"}),
                emitted: host_cause["request"].clone(),
            }
        );
        assert_eq!(main.rows[saved_index].locale, tag);
        assert_eq!(
            main.rows[saved_index].boundary,
            TimelineBoundary::HostAction
        );
        assert!(
            main.rows[saved_index..].iter().all(|row| row.locale == tag),
            "{tag} reverted after the successful response"
        );

        let before = trace_frame_text(&main, save_request);
        assert!(before.contains("Preferences"), "{tag}: {before}");
        assert!(!before.contains(preferences), "{tag}: {before}");
        let after = trace_frame_text(&main, &main.rows[saved_index]);
        assert!(after.contains(library), "{tag}: {after}");
        assert!(after.contains(saved), "{tag}: {after}");

        assert_eq!(
            &host_cause["response"],
            &json!({"preferences_saved": {"locale": tag, "message": saved}})
        );
    }
}

#[test]
fn real_frontend_and_host_move_raw_values_through_the_production_chain() {
    let (mut frontend, mut host) = smoke_factory().create().unwrap();
    let action = Action::OpenRun;
    assert_eq!(
        frontend
            .dispatch(SmokeEvent {
                key: KeyCode::Enter,
                modifiers: KeyModifiers::NONE,
            })
            .unwrap(),
        DispatchOutcome::Action(action.clone())
    );

    let emitted = frontend.reduce(action).unwrap();
    let emitted_evidence = json_value(&emitted);
    assert_eq!(
        emitted_evidence,
        json!({"open": {"request": "run", "selector": "reference"}})
    );

    let response: Action = host.serve(emitted).unwrap();
    let response_evidence = json_value(&response);
    assert_eq!(
        response_evidence.pointer("/present/run/selector"),
        Some(&json!("reference"))
    );
    assert_eq!(frontend.reduce(response).unwrap(), Effect::None);
}

#[test]
fn real_host_engine_records_and_replays_one_production_open() {
    let factory = smoke_factory();
    let operation = SmokeOperation::OpenRun;
    let (frontend, host) = factory.create().unwrap();
    let main_root = host.host.roots().data.clone();
    let mut main = WalkerEngine::new(frontend, host, EFFECT_LIMIT).unwrap();
    let mut main_trace = MemoryTrace::default();
    main.start(&mut main_trace).unwrap();
    main.run_operation(operation, &mut main_trace).unwrap();

    let mut replay_trace = MemoryTrace::default();
    let replay = replay_prefix_with_sink(&factory, &[operation], &mut replay_trace).unwrap();
    let replay_root = replay.host().host.roots().data.clone();

    assert_ne!(main_root, replay_root);
    assert_eq!(main.successful_operations(), &[operation]);
    assert_eq!(replay.successful_operations(), &[operation]);
    assert_eq!(main_trace.checkpoints, replay_trace.checkpoints);
    assert_eq!(
        main_trace
            .checkpoints
            .iter()
            .map(|checkpoint| (checkpoint.boundary, checkpoint.cause.clone()))
            .collect::<Vec<_>>(),
        [
            (EngineBoundary::Initial, CauseObservation::Initial),
            (
                EngineBoundary::Reducer,
                CauseObservation::Reducer {
                    dispatch_sequence: 0,
                    operation_index: 0,
                    operation,
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
                    selector: "reference".to_owned(),
                },
            ),
            (
                EngineBoundary::Host,
                CauseObservation::Host {
                    dispatch_sequence: 0,
                    operation_index: 0,
                    operation,
                    round: 0,
                    selector: "reference".to_owned(),
                },
            ),
        ]
    );

    let observations = serde_json::to_string(&main_trace.checkpoints.last().unwrap().host).unwrap();
    assert!(observations.contains("<profile:engine-smoke>/external/original.sh"));
    assert!(!observations.contains(&main_root.display().to_string()));
    assert!(!observations.contains(&replay_root.display().to_string()));

    assert!(main_trace.checkpoints.iter().all(|checkpoint| {
        checkpoint.styled_frame.area
            == RectSnapshot {
                x: 0,
                y: 0,
                width: 60,
                height: 24,
            }
    }));
    let initial = &main_trace.checkpoints[0];
    let settled = &main_trace.checkpoints[2];
    assert_eq!(
        initial.canonical_state.pointer("/workflow/active"),
        Some(&json!("library"))
    );
    assert_eq!(
        settled
            .canonical_state
            .pointer("/workflow/active/run/selector"),
        Some(&json!("reference"))
    );
    assert_ne!(initial.canonical_state, settled.canonical_state);
    assert_eq!(initial.canonical_state, initial.host.state);
    assert_eq!(settled.canonical_state, settled.host.state);
    assert_ne!(initial.styled_frame, settled.styled_frame);
    let initial_text = initial.styled_frame.readable_lines().unwrap().join("\n");
    let settled_text = settled.styled_frame.readable_lines().unwrap().join("\n");
    assert!(initial_text.contains("Library"));
    assert!(initial_text.contains("Reference"));
    assert!(settled_text.contains("Run Reference"));
    assert!(
        initial
            .session
            .pointer("/run/fields/signature")
            .is_some_and(Value::is_null)
    );
    assert!(
        settled
            .session
            .pointer("/run/fields/signature")
            .is_some_and(|signature| !signature.is_null())
    );
    let rows = settled.geometry.get("rows").unwrap();
    assert_eq!(rows.get("x"), Some(&json!(1)));
    assert_eq!(rows.get("y"), Some(&json!(1)));
    assert_eq!(rows.get("width"), Some(&json!(58)));
    assert!(rows.get("height").is_some_and(Value::is_u64));
    assert!(
        settled
            .geometry
            .get("hits")
            .and_then(Value::as_array)
            .is_some_and(|hits| !hits.is_empty())
    );
}
