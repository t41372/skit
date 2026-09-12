//! Recording drivers for smoke, random, stable, and review corpus walks.

use std::{collections::BTreeMap, path::PathBuf};

use ratatui_core::layout::Size;
use serde_json::Value;
use skit_application::{AgentScope, CreateEntry, EntryPayload, SourcePermissions};
use skit_domain::{EntryKind, EntrySettings, StorageMode, parameters::ParamDecl};
use skit_i18n::{Locale, detect_locale};
use skit_store::PromptRunner;
use skit_tui::{AddControlId, AddTextField, HitTarget, LocalActionTarget, ScreenTarget};
use skit_tui_walker_support::{
    engine::{ReplayFactory, WalkerEngine},
    sandbox::{SafeProfileId, SandboxMetadata},
};
use skit_ui::{
    HealthAction, LibraryState, PreferencesControlId, RunnerEditorAction, RunnerManagerAction,
    UiCommand,
};

use super::{
    corpus::{
        CorpusKey, CorpusKeyEvent, CorpusModifiers, CorpusOperation, EFFECT_LIMIT,
        LocaleSmokeTarget, SmokeOperation, corpus_canvas_size, corpus_operation_value,
    },
    engine::{CorpusHostBoundary, RealHostBoundary, RealWalkerFactory},
    frontend::{CorpusFrontend, RealFrontend},
    trace::{
        RealTrace, RecordedRealCorpus, RecordedRealTrace, SchemaThreeSink, StablePairError,
        StablePairPhase, StablePairPrimaryError, StableTracePair, smoke_operation_value,
    },
};
use crate::cli::tui_real_host::{
    LeakOracleFacts, RealWalkerHost, SandboxError, StableHostError, StableSandboxNamespace,
    WalkerDirectoryRoot, WalkerDirectorySeed, WalkerExternalReferenceSeed, WalkerExternalSeed,
    WalkerFilePickerTree, WalkerFormSeed, WalkerLastRunSeed, WalkerSeedSpec,
};

pub(super) fn new_schema_three_sink(
    factory: &RealWalkerFactory,
) -> Result<SchemaThreeSink, String> {
    SchemaThreeSink::new(
        factory.review_profile.clone(),
        factory.locale,
        factory.size,
        factory.size,
    )
}

fn new_corpus_schema_three_sink(
    factory: &RealWalkerFactory,
    operations: &[CorpusOperation],
) -> Result<SchemaThreeSink, String> {
    SchemaThreeSink::new(
        factory.review_profile.clone(),
        factory.locale,
        factory.size,
        corpus_canvas_size(factory.size, operations),
    )
}

pub(super) fn combine_corpus_result_after_close<T>(
    result: Result<T, String>,
    close: Result<(), impl std::fmt::Display>,
) -> Result<T, String> {
    match (result, close) {
        (Ok(value), Ok(())) => Ok(value),
        (Ok(_), Err(error)) => Err(format!("could not close the real corpus host: {error}")),
        (Err(primary), Ok(())) => Err(primary),
        (Err(primary), Err(cleanup)) => Err(format!(
            "{primary}; real corpus host cleanup also failed: {cleanup}"
        )),
    }
}

pub(super) fn prepare_corpus_host_with(
    host: RealWalkerHost,
    profile: &SafeProfileId,
    context: &impl Fn(String) -> String,
    read_metadata: impl FnOnce(&RealWalkerHost, &SafeProfileId) -> Result<SandboxMetadata, String>,
    read_state: impl FnOnce(&RealWalkerHost) -> Result<LibraryState, String>,
) -> Result<
    (
        RealWalkerHost,
        SandboxMetadata,
        LibraryState,
        WalkerFilePickerTree,
    ),
    String,
> {
    let sandbox = match read_metadata(&host, profile) {
        Ok(sandbox) => sandbox,
        Err(error) => {
            return combine_corpus_result_after_close(Err(context(error)), host.close());
        }
    };
    let state = match read_state(&host) {
        Ok(state) => state,
        Err(error) => {
            return combine_corpus_result_after_close(Err(context(error)), host.close());
        }
    };
    let file_picker_tree = host.file_picker_tree();
    Ok((host, sandbox, state, file_picker_tree))
}

pub(super) fn record_corpus_profile(
    factory: &RealWalkerFactory,
    namespace: StableSandboxNamespace,
    operations: &[CorpusOperation],
    phase: StablePairPhase,
) -> Result<RecordedRealTrace, String> {
    let profile = factory.review_profile.clone();
    let phase_label = match phase {
        StablePairPhase::Main => "main",
        StablePairPhase::Replay => "replay",
    };
    let context = |message: String| {
        format!("stable corpus {phase_label} profile {profile} failed: {message}")
    };
    let host = RealWalkerHost::spawn_stable_in(factory.seed.clone(), profile.clone(), namespace)
        .map_err(|error| context(error.to_string()))?;
    let read_metadata = |host: &RealWalkerHost, profile: &SafeProfileId| {
        host.sandbox_metadata(profile)
            .map_err(|error| error.to_string())
    };
    let read_state =
        |host: &RealWalkerHost| host.initial_state().map_err(|error| error.to_string());
    let prepared = prepare_corpus_host_with(host, &profile, &context, read_metadata, read_state);
    let (host, sandbox, state, file_picker_tree) = prepared?;
    let frontend = match CorpusFrontend::new(state, factory.locale, factory.size, file_picker_tree)
    {
        Ok(frontend) => frontend,
        Err(error) => {
            return combine_corpus_result_after_close(Err(context(error)), host.close());
        }
    };
    let boundary = CorpusHostBoundary {
        inner: RealHostBoundary { host },
        leak_sandbox: Some(sandbox.clone()),
    };
    let mut engine = WalkerEngine::new(frontend, boundary, EFFECT_LIMIT)
        .expect("the corpus effect limit is positive");
    let mut sink = Some(
        new_corpus_schema_three_sink(factory, operations)
            .expect("the validated positive corpus viewport creates an asciicast"),
    );
    let result = (|| {
        let sink_ref = sink
            .as_mut()
            .expect("the corpus sink exists before trace finish");
        engine.start(sink_ref).map_err(&context)?;
        for (index, operation) in operations.iter().enumerate() {
            engine
                .run_operation(operation.clone(), sink_ref)
                .map_err(|error| {
                    context(format!(
                        "operation {} {operation:?}: {error}",
                        index.saturating_add(1)
                    ))
                })?;
        }
        engine
            .run_final_liveness(CorpusOperation::FinalLiveness, sink_ref)
            .map_err(|error| context(format!("final liveness: {error}")))?;
        let successful_operations = engine.successful_operations();
        let operation_values = successful_operations
            .iter()
            .map(corpus_operation_value)
            .collect::<Result<Vec<_>, _>>()
            .map_err(&context)?;
        let final_liveness = corpus_operation_value(&CorpusOperation::FinalLiveness)
            .expect("the fixed final-liveness operation serializes");
        let facts = engine.host().inner.leak_oracle_facts();
        let trace = sink
            .take()
            .expect("the corpus sink is consumed exactly once")
            .finish(operation_values, final_liveness)
            .map_err(&context)?;
        Ok((trace, facts))
    })();
    drop(sink);
    let (frontend, host) = engine.into_parts();
    drop(frontend);
    combine_corpus_result_after_close(result, host.close()).map(|(trace, leak_oracle_facts)| {
        RecordedRealTrace {
            trace,
            sandbox,
            leak_oracle_facts,
        }
    })
}

/// Record every required review profile in order into one stable namespace.
pub(crate) fn record_real_review_corpus_in(
    namespace: StableSandboxNamespace,
    operations: &[CorpusOperation],
    phase: StablePairPhase,
) -> Result<RecordedRealCorpus, String> {
    let mut profiles = Vec::new();
    for factory in required_corpus_factories()? {
        profiles.push(record_corpus_profile(
            &factory,
            namespace.clone(),
            operations,
            phase,
        )?);
    }
    RecordedRealCorpus::new(profiles)
}

fn finish_smoke_trace(
    sink: SchemaThreeSink,
    successful_operations: &[SmokeOperation],
) -> Result<RealTrace, String> {
    sink.finish(
        successful_operations
            .iter()
            .copied()
            .map(smoke_operation_value)
            .collect(),
        smoke_operation_value(SmokeOperation::FinalLiveness),
    )
}

fn run_smoke_prefix(
    engine: &mut WalkerEngine<RealFrontend, RealHostBoundary>,
    sink: &mut SchemaThreeSink,
    prefix: &[SmokeOperation],
) -> Result<(), String> {
    engine.start(sink)?;
    for operation in prefix {
        engine.run_operation(*operation, sink)?;
    }
    engine.run_final_liveness(SmokeOperation::FinalLiveness, sink)
}

pub(super) fn close_after_primary(host: RealWalkerHost, primary: String) -> String {
    primary_with_cleanup(primary, host.close().err())
}

fn primary_with_cleanup(primary: String, cleanup: Option<impl std::fmt::Display>) -> String {
    match cleanup {
        None => primary,
        Some(cleanup) => format!("{primary}; sandbox cleanup also failed: {cleanup}"),
    }
}

fn close_random_engine(engine: WalkerEngine<RealFrontend, RealHostBoundary>) -> Result<(), String> {
    let (frontend, host) = engine.into_parts();
    drop(frontend);
    host.close().map_err(|error| error.to_string())
}

fn random_trace_failure(
    engine: WalkerEngine<RealFrontend, RealHostBoundary>,
    sink: SchemaThreeSink,
    primary: String,
) -> String {
    drop(sink);
    primary_with_cleanup(primary, close_random_engine(engine).err())
}

fn record_schema_three_random(
    factory: &RealWalkerFactory,
    prefix: &[SmokeOperation],
) -> Result<RecordedRealTrace, String> {
    record_schema_three_random_with_hooks(
        factory,
        prefix,
        noop_real_host_boundary,
        new_schema_three_sink,
        read_boundary_sandbox_metadata,
        noop_schema_sink,
        noop_leak_oracle_facts,
    )
}

pub(super) fn record_schema_three_random_with_hooks(
    factory: &RealWalkerFactory,
    prefix: &[SmokeOperation],
    after_frontend: impl FnOnce(&mut RealHostBoundary),
    make_sink: impl FnOnce(&RealWalkerFactory) -> Result<SchemaThreeSink, String>,
    sandbox_metadata: impl FnOnce(&RealHostBoundary, &SafeProfileId) -> Result<SandboxMetadata, String>,
    before_finish: impl FnOnce(&mut SchemaThreeSink),
    after_facts: impl FnOnce(&LeakOracleFacts),
) -> Result<RecordedRealTrace, String> {
    let (frontend, mut host) = factory.create()?;
    after_frontend(&mut host);
    let mut engine = WalkerEngine::new(frontend, host, factory.effect_limit())
        .expect("the real walker effect limit is positive");
    let mut sink = match make_sink(factory) {
        Ok(sink) => sink,
        Err(primary) => {
            return Err(primary_with_cleanup(
                primary,
                close_random_engine(engine).err(),
            ));
        }
    };
    if let Err(primary) = run_smoke_prefix(&mut engine, &mut sink, prefix) {
        return Err(random_trace_failure(engine, sink, primary));
    }
    let successful_operations = engine.successful_operations().to_vec();
    let leak_oracle_facts = engine.host().leak_oracle_facts();
    let sandbox = match sandbox_metadata(engine.host(), &factory.review_profile) {
        Ok(metadata) => metadata,
        Err(primary) => return Err(random_trace_failure(engine, sink, primary)),
    };
    before_finish(&mut sink);
    let trace = match finish_smoke_trace(sink, &successful_operations) {
        Ok(trace) => trace,
        Err(primary) => {
            return Err(primary_with_cleanup(
                primary,
                close_random_engine(engine).err(),
            ));
        }
    };
    after_facts(&leak_oracle_facts);
    close_random_engine(engine)?;
    Ok(RecordedRealTrace {
        trace,
        sandbox,
        leak_oracle_facts,
    })
}

pub(super) fn noop_real_host_boundary(_host: &mut RealHostBoundary) {}

pub(super) fn read_boundary_sandbox_metadata(
    host: &RealHostBoundary,
    review_profile: &SafeProfileId,
) -> Result<SandboxMetadata, String> {
    host.sandbox_metadata(review_profile)
        .map_err(|error| error.to_string())
}

pub(super) fn record_schema_three_main(
    factory: &RealWalkerFactory,
) -> Result<RecordedRealTrace, String> {
    record_schema_three_random(factory, &[SmokeOperation::OpenRun])
}

pub(super) fn record_schema_three_replay(
    factory: &RealWalkerFactory,
) -> Result<RecordedRealTrace, String> {
    record_schema_three_random(factory, &[SmokeOperation::OpenRun])
}

fn stable_pair_error_from_host(phase: StablePairPhase, error: StableHostError) -> StablePairError {
    match error {
        StableHostError::Sandbox(primary) => StablePairError {
            phase,
            primary: StablePairPrimaryError::Sandbox(Box::new(primary)),
            cleanup: None,
        },
        StableHostError::Primary { primary, cleanup } => StablePairError {
            phase,
            primary: StablePairPrimaryError::Seed(primary),
            cleanup: cleanup.map(Box::new),
        },
    }
}

fn stable_host_failure(
    phase: StablePairPhase,
    host: RealWalkerHost,
    primary: StablePairPrimaryError,
) -> StablePairError {
    StablePairError {
        phase,
        primary,
        cleanup: host.close().err().map(Box::new),
    }
}

fn stable_engine_failure(
    phase: StablePairPhase,
    engine: WalkerEngine<RealFrontend, RealHostBoundary>,
    sink: SchemaThreeSink,
    primary: StablePairPrimaryError,
) -> StablePairError {
    drop(sink);
    let (frontend, host) = engine.into_parts();
    drop(frontend);
    StablePairError {
        phase,
        primary,
        cleanup: host.close().err().map(Box::new),
    }
}

pub(super) fn record_schema_three_stable(
    factory: &RealWalkerFactory,
    namespace: StableSandboxNamespace,
    phase: StablePairPhase,
    prefix: &[SmokeOperation],
) -> Result<(RecordedRealTrace, Vec<SmokeOperation>), StablePairError> {
    record_schema_three_stable_with_hooks(
        factory,
        namespace,
        phase,
        prefix,
        read_sandbox_metadata,
        |host| host.initial_state().map_err(|error| error.to_string()),
        noop_stable_host,
        new_schema_three_sink,
        noop_schema_sink,
        noop_leak_oracle_facts,
        noop_stable_host,
    )
}

pub(super) fn noop_stable_host(_host: &mut RealWalkerHost) {}

pub(super) fn noop_schema_sink(_sink: &mut SchemaThreeSink) {}

pub(super) fn noop_leak_oracle_facts(_facts: &LeakOracleFacts) {}

#[allow(clippy::too_many_arguments)]
pub(super) fn record_schema_three_stable_with_hooks(
    factory: &RealWalkerFactory,
    namespace: StableSandboxNamespace,
    phase: StablePairPhase,
    prefix: &[SmokeOperation],
    sandbox_metadata: impl FnOnce(
        &RealWalkerHost,
        &SafeProfileId,
    ) -> Result<SandboxMetadata, SandboxError>,
    initial_state: impl FnOnce(&RealWalkerHost) -> Result<LibraryState, String>,
    after_frontend: impl FnOnce(&mut RealWalkerHost),
    make_sink: impl FnOnce(&RealWalkerFactory) -> Result<SchemaThreeSink, String>,
    before_finish: impl FnOnce(&mut SchemaThreeSink),
    after_facts: impl FnOnce(&LeakOracleFacts),
    before_close: impl FnOnce(&mut RealWalkerHost),
) -> Result<(RecordedRealTrace, Vec<SmokeOperation>), StablePairError> {
    let mut host = RealWalkerHost::spawn_stable_in(
        factory.seed.clone(),
        factory.review_profile.clone(),
        namespace,
    )
    .map_err(|error| stable_pair_error_from_host(phase, error))?;
    let sandbox = match sandbox_metadata(&host, &factory.review_profile) {
        Ok(metadata) => metadata,
        Err(primary) => {
            return Err(stable_host_failure(
                phase,
                host,
                StablePairPrimaryError::Sandbox(Box::new(primary)),
            ));
        }
    };
    let state = match initial_state(&host) {
        Ok(state) => state,
        Err(primary) => {
            return Err(stable_host_failure(
                phase,
                host,
                StablePairPrimaryError::InitialState(primary),
            ));
        }
    };
    let frontend = match RealFrontend::new(state, factory.locale, factory.size) {
        Ok(frontend) => frontend,
        Err(primary) => {
            return Err(stable_host_failure(
                phase,
                host,
                StablePairPrimaryError::Frontend(primary),
            ));
        }
    };
    after_frontend(&mut host);
    let boundary = RealHostBoundary { host };
    let mut engine = WalkerEngine::new(frontend, boundary, factory.effect_limit())
        .expect("the real walker effect limit is positive");
    let mut sink = match make_sink(factory) {
        Ok(sink) => sink,
        Err(primary) => {
            let (frontend, host) = engine.into_parts();
            drop(frontend);
            return Err(StablePairError {
                phase,
                primary: StablePairPrimaryError::Trace(primary),
                cleanup: host.close().err().map(Box::new),
            });
        }
    };
    if let Err(primary) = run_smoke_prefix(&mut engine, &mut sink, prefix) {
        return Err(stable_engine_failure(
            phase,
            engine,
            sink,
            StablePairPrimaryError::Trace(primary),
        ));
    }
    let successful_operations = engine.successful_operations().to_vec();
    let leak_oracle_facts = engine.host().leak_oracle_facts();
    before_finish(&mut sink);
    let trace = match finish_smoke_trace(sink, &successful_operations) {
        Ok(trace) => trace,
        Err(primary) => {
            let (frontend, host) = engine.into_parts();
            drop(frontend);
            return Err(StablePairError {
                phase,
                primary: StablePairPrimaryError::Trace(primary),
                cleanup: host.close().err().map(Box::new),
            });
        }
    };
    after_facts(&leak_oracle_facts);
    let (frontend, mut host) = engine.into_parts();
    drop(frontend);
    before_close(&mut host.host);
    host.close().map_err(|primary| StablePairError {
        phase,
        primary: StablePairPrimaryError::Close(Box::new(primary)),
        cleanup: None,
    })?;
    Ok((
        RecordedRealTrace {
            trace,
            sandbox,
            leak_oracle_facts,
        },
        successful_operations,
    ))
}

pub(super) fn read_sandbox_metadata(
    host: &RealWalkerHost,
    review_profile: &SafeProfileId,
) -> Result<SandboxMetadata, SandboxError> {
    host.sandbox_metadata(review_profile)
}

pub(super) fn smoke_factory() -> RealWalkerFactory {
    RealWalkerFactory {
        seed: smoke_seed(),
        review_profile: SafeProfileId::try_from("engine-smoke-60x24")
            .expect("the smoke review profile is valid"),
        locale: Locale::En,
        size: Size::new(60, 24),
    }
}

pub(super) fn canonical_corpus_seed(locale: Locale) -> WalkerSeedSpec {
    let mut pattern = ParamDecl::new("pattern");
    pattern.multiple = true;
    let source = b"#!/bin/sh\nprintf '%s\\n' \"$1\"\n".to_vec();
    WalkerSeedSpec {
        profile: "canonical-corpus".to_owned(),
        entries: vec![CreateEntry {
            name: "A Corpus Tool".to_owned(),
            kind: EntryKind::parse("shell").expect("shell is a supported kind"),
            mode: StorageMode::Copy,
            source: String::new(),
            workdir: "store".to_owned(),
            description: "Real-host corpus entry.".to_owned(),
            payload: Some(EntryPayload {
                bytes: source,
                stored_name: Some("corpus.sh".to_owned()),
                permissions: SourcePermissions {
                    readonly: false,
                    unix_mode: Some(0o640),
                },
            }),
            settings: EntrySettings {
                params: vec![pattern.name.clone()],
                parameters: vec![pattern],
                ..EntrySettings::default()
            },
        }],
        settings: BTreeMap::from([
            ("after_run".to_owned(), "stay".to_owned()),
            ("lang".to_owned(), locale.tag().to_owned()),
        ]),
        runners: vec![PromptRunner {
            name: "seed-agent".to_owned(),
            argv: vec![
                "corpus-agent".to_owned(),
                "--message".to_owned(),
                "{{prompt}}".to_owned(),
            ],
        }],
        forms: vec![WalkerFormSeed {
            selector: "A Corpus Tool".to_owned(),
            values: BTreeMap::new(),
            extra_args: vec!["seed-tail".to_owned()],
            extra_args_raw: false,
            preset: Some("seed".to_owned()),
            last_run: Some(WalkerLastRunSeed {
                exit: 0,
                at: "2026-09-02T12:00:00+00:00".to_owned(),
                values: Some(BTreeMap::new()),
            }),
        }],
        prompt_runner: "seed-agent".to_owned(),
        external: vec![WalkerExternalSeed::File {
            path: PathBuf::from("picked.sh"),
            bytes: b"#!/bin/sh\nprintf picked\n".to_vec(),
            readonly: false,
            unix_mode: 0o640,
        }],
        external_references: Vec::new(),
        directories: vec![
            WalkerDirectorySeed {
                root: WalkerDirectoryRoot::Home,
                path: PathBuf::from(".codex/skills"),
            },
            WalkerDirectorySeed {
                root: WalkerDirectoryRoot::Cwd,
                path: PathBuf::from(".claude/skills"),
            },
        ],
        editor_writes: vec![
            b"#!/bin/sh\nprintf edited\n".to_vec(),
            b"#!/bin/sh\nprintf kept\n".to_vec(),
            b"Write {{topic}} as a short note.\n".to_vec(),
            b"Write {{topic}} as a revised note.\n".to_vec(),
        ],
    }
}

pub(super) fn required_corpus_factories() -> Result<Vec<RealWalkerFactory>, String> {
    skit_tui_walker_support::required_review_profiles()
        .iter()
        .map(|required| {
            let locale = detect_locale(Some(required.locale));
            Ok(RealWalkerFactory {
                seed: canonical_corpus_seed(locale),
                review_profile: SafeProfileId::try_from(required.id)
                    .map_err(|error| error.to_string())?,
                locale,
                size: Size::new(required.viewport.width, required.viewport.height),
            })
        })
        .collect()
}

pub(crate) fn canonical_corpus_operations() -> Vec<CorpusOperation> {
    let plain = |code| CorpusKeyEvent::press(code, CorpusModifiers::NONE);
    let control = |character| {
        CorpusKeyEvent::press(
            CorpusKey::Character(character),
            CorpusModifiers {
                control: true,
                ..CorpusModifiers::NONE
            },
        )
    };
    let add_focus = |target| CorpusOperation::ScreenFocus(ScreenTarget::Add(target));
    let add_hit = |target| CorpusOperation::ScreenHit(ScreenTarget::Add(target));
    let add_local = |target, key| CorpusOperation::LocalKeyboard {
        target: LocalActionTarget::Add(target),
        key,
    };
    let preferences_focus =
        |target| CorpusOperation::ScreenFocus(ScreenTarget::Preferences(target));
    let preferences_hit = |target| CorpusOperation::ScreenHit(ScreenTarget::Preferences(target));
    let runner_hit = |name: &str| {
        CorpusOperation::ScreenHit(ScreenTarget::Runner {
            name: name.to_owned(),
        })
    };
    let mut operations = vec![
        CorpusOperation::Focus { gained: false },
        CorpusOperation::CommandKeyboard(UiCommand::Run),
        CorpusOperation::HitTarget(HitTarget::FocusField(1)),
        CorpusOperation::Paste("*".to_owned()),
        CorpusOperation::CommandKeyboard(UiCommand::SavePreset),
        CorpusOperation::Paste("corpus".to_owned()),
        CorpusOperation::CommandKeyboard(UiCommand::Submit),
        CorpusOperation::CommandKeyboard(UiCommand::Submit),
        CorpusOperation::CommandKeyboard(UiCommand::Rerun),
        CorpusOperation::CommandKeyboard(UiCommand::Presets),
        CorpusOperation::CommandKeyboard(UiCommand::CloseSettings),
        CorpusOperation::CommandKeyboard(UiCommand::Settings),
        CorpusOperation::CommandKeyboard(UiCommand::SaveSettings),
        CorpusOperation::CommandKeyboard(UiCommand::Edit),
        CorpusOperation::CommandKeyboard(UiCommand::Rename),
        CorpusOperation::Paste("-renamed".to_owned()),
        CorpusOperation::CommandKeyboard(UiCommand::Submit),
        CorpusOperation::CommandKeyboard(UiCommand::Health),
        CorpusOperation::LocalKeyboard {
            target: LocalActionTarget::Health(HealthAction::Rebuild),
            key: control('r'),
        },
        CorpusOperation::LocalHit(LocalActionTarget::Health(HealthAction::Back)),
        CorpusOperation::CommandKeyboard(UiCommand::Runners),
        CorpusOperation::LocalKeyboard {
            target: LocalActionTarget::Runners(RunnerManagerAction::Back),
            key: plain(CorpusKey::Escape),
        },
        CorpusOperation::CommandKeyboard(UiCommand::Preferences),
        CorpusOperation::CommandKeyboard(UiCommand::SavePreferences),
        CorpusOperation::CommandKeyboard(UiCommand::Preferences),
        preferences_focus(PreferencesControlId::InstallAgentSkill),
        preferences_hit(PreferencesControlId::InstallAgentSkill),
        CorpusOperation::ScreenHit(ScreenTarget::AgentSkill {
            name: "codex".to_owned(),
            scope: AgentScope::User,
        }),
        preferences_focus(PreferencesControlId::ManageAgents),
        preferences_hit(PreferencesControlId::ManageAgents),
        runner_hit("claude"),
        CorpusOperation::LocalKeyboard {
            target: LocalActionTarget::Runners(RunnerManagerAction::RemoveSelected),
            key: plain(CorpusKey::Character('d')),
        },
        CorpusOperation::LocalKeyboard {
            target: LocalActionTarget::Runners(RunnerManagerAction::ConfirmRemove),
            key: plain(CorpusKey::Character('y')),
        },
        runner_hit("codex"),
        CorpusOperation::LocalKeyboard {
            target: LocalActionTarget::Runners(RunnerManagerAction::RemoveSelected),
            key: plain(CorpusKey::Character('d')),
        },
        CorpusOperation::LocalKeyboard {
            target: LocalActionTarget::Runners(RunnerManagerAction::ConfirmRemove),
            key: plain(CorpusKey::Character('y')),
        },
        CorpusOperation::LocalKeyboard {
            target: LocalActionTarget::Runners(RunnerManagerAction::Back),
            key: plain(CorpusKey::Escape),
        },
        CorpusOperation::CommandKeyboard(UiCommand::ClosePreferences),
        CorpusOperation::CommandKeyboard(UiCommand::Add),
        add_hit(AddControlId::BrowseSource),
        CorpusOperation::ScreenHit(ScreenTarget::FilePickerEntry {
            relative: PathBuf::from("picked.sh"),
        }),
        add_focus(AddControlId::Continue),
        add_local(AddControlId::Continue, plain(CorpusKey::Enter)),
        add_local(AddControlId::Cancel, plain(CorpusKey::Escape)),
        add_local(AddControlId::Cancel, plain(CorpusKey::Escape)),
        CorpusOperation::CommandKeyboard(UiCommand::Add),
        add_focus(AddControlId::NewScript),
        add_local(AddControlId::NewScript, plain(CorpusKey::Enter)),
        add_local(AddControlId::Cancel, plain(CorpusKey::Escape)),
        add_local(AddControlId::Cancel, plain(CorpusKey::Escape)),
        CorpusOperation::CommandKeyboard(UiCommand::Add),
        add_focus(AddControlId::Draft(0)),
        add_local(AddControlId::Draft(0), plain(CorpusKey::Enter)),
        add_focus(AddControlId::DeleteDraft),
        add_local(AddControlId::DeleteDraft, plain(CorpusKey::Enter)),
        add_local(AddControlId::DeleteDraft, plain(CorpusKey::Enter)),
        add_local(AddControlId::Cancel, plain(CorpusKey::Escape)),
        CorpusOperation::CommandKeyboard(UiCommand::Add),
        add_focus(AddControlId::NewPrompt),
        add_local(AddControlId::NewPrompt, plain(CorpusKey::Enter)),
        add_focus(AddControlId::Interpolate),
        add_hit(AddControlId::Interpolate),
        add_focus(AddControlId::EditSource),
        add_local(AddControlId::EditSource, control('e')),
        add_focus(AddControlId::Text(AddTextField::ReviewName)),
        CorpusOperation::RawKey(control('u')),
        CorpusOperation::Paste("Corpus Prompt".to_owned()),
        add_focus(AddControlId::NewRunner),
        add_hit(AddControlId::NewRunner),
        CorpusOperation::Paste("new-agent".to_owned()),
        CorpusOperation::LocalKeyboard {
            target: LocalActionTarget::RunnerEditor(RunnerEditorAction::FocusNext),
            key: plain(CorpusKey::Tab),
        },
        CorpusOperation::Paste("agent {{prompt}}".to_owned()),
        CorpusOperation::LocalKeyboard {
            target: LocalActionTarget::RunnerEditor(RunnerEditorAction::Submit),
            key: plain(CorpusKey::Enter),
        },
        add_focus(AddControlId::Save),
        add_local(AddControlId::Save, control('s')),
        CorpusOperation::CommandKeyboard(UiCommand::Remove),
        CorpusOperation::CommandKeyboard(UiCommand::Submit),
        CorpusOperation::CommandKeyboard(UiCommand::Reload),
        CorpusOperation::CommandKeyboard(UiCommand::Search),
        CorpusOperation::Paste("Corpus".to_owned()),
        CorpusOperation::CommandKeyboard(UiCommand::ClearSearch),
        CorpusOperation::CommandKeyboard(UiCommand::LeaveSearch),
        CorpusOperation::CommandKeyboard(UiCommand::ToggleDetail),
        CorpusOperation::CommandKeyboard(UiCommand::ToggleDetail),
        CorpusOperation::CommandKeyboard(UiCommand::Help),
        CorpusOperation::CommandKeyboard(UiCommand::CloseModal),
        CorpusOperation::CommandKeyboard(UiCommand::Next),
        CorpusOperation::CommandKeyboard(UiCommand::Previous),
        CorpusOperation::CommandKeyboard(UiCommand::Home),
        CorpusOperation::CommandKeyboard(UiCommand::End),
        CorpusOperation::CommandKeyboard(UiCommand::Run),
        CorpusOperation::CommandKeyboard(UiCommand::Back),
        CorpusOperation::HitTarget(HitTarget::Command(UiCommand::Run)),
        CorpusOperation::CommandKeyboard(UiCommand::Back),
        CorpusOperation::CommandKeyboard(UiCommand::ToggleDetail),
        CorpusOperation::CommandKeyboard(UiCommand::Help),
        CorpusOperation::CommandKeyboard(UiCommand::CloseModal),
        CorpusOperation::Focus { gained: false },
    ];
    assert_eq!(operations.len(), 98);
    operations.push(CorpusOperation::Resize {
        width: 24,
        height: 6,
    });
    operations.push(CorpusOperation::Focus { gained: true });
    operations
}

/// One corpus operation that the resolver refuses in every profile.
#[cfg(any(target_os = "linux", target_os = "windows"))]
pub(crate) fn invalid_corpus_operation() -> CorpusOperation {
    CorpusOperation::ScreenHit(ScreenTarget::FilePickerEntry {
        relative: PathBuf::from("../outside"),
    })
}

/// The short viewport-independent operation vector for review corpus contracts.
///
/// Every required profile completes it with no refused operation, and it records one host row.
#[cfg(any(target_os = "linux", target_os = "windows"))]
pub(crate) fn review_corpus_contract_operations() -> Vec<CorpusOperation> {
    vec![
        CorpusOperation::CommandKeyboard(UiCommand::Run),
        CorpusOperation::RawKey(CorpusKeyEvent::press(
            CorpusKey::Escape,
            CorpusModifiers::NONE,
        )),
        CorpusOperation::Focus { gained: false },
    ]
}

/// The short viewport-independent operation vector for the draft allocator contracts.
///
/// It authors one draft, opens the draft again, and confirms one deletion. The deletion
/// quarantines the draft file, so the recording reaches the draft quarantine allocator.
#[cfg(any(target_os = "linux", target_os = "windows"))]
pub(crate) fn draft_quarantine_corpus_operations() -> Vec<CorpusOperation> {
    let plain = |code| CorpusKeyEvent::press(code, CorpusModifiers::NONE);
    let add_focus = |target| CorpusOperation::ScreenFocus(ScreenTarget::Add(target));
    let add_local = |target, key| CorpusOperation::LocalKeyboard {
        target: LocalActionTarget::Add(target),
        key,
    };
    vec![
        CorpusOperation::CommandKeyboard(UiCommand::Add),
        add_focus(AddControlId::NewScript),
        add_local(AddControlId::NewScript, plain(CorpusKey::Enter)),
        add_local(AddControlId::Cancel, plain(CorpusKey::Escape)),
        add_local(AddControlId::Cancel, plain(CorpusKey::Escape)),
        CorpusOperation::CommandKeyboard(UiCommand::Add),
        add_focus(AddControlId::Draft(0)),
        add_local(AddControlId::Draft(0), plain(CorpusKey::Enter)),
        add_focus(AddControlId::DeleteDraft),
        add_local(AddControlId::DeleteDraft, plain(CorpusKey::Enter)),
        add_local(AddControlId::DeleteDraft, plain(CorpusKey::Enter)),
        add_local(AddControlId::Cancel, plain(CorpusKey::Escape)),
    ]
}

/// The short operation vector that cuts the derived Add review name.
///
/// It authors one prompt draft, reaches the review name, and cuts that name with `Control+U`.
/// The cut name stays in the input cut buffer after the paste replaces the value.
#[cfg(any(target_os = "linux", target_os = "windows"))]
pub(crate) fn review_name_cut_corpus_operations() -> Vec<CorpusOperation> {
    let plain = |code| CorpusKeyEvent::press(code, CorpusModifiers::NONE);
    let control = |character| {
        CorpusKeyEvent::press(
            CorpusKey::Character(character),
            CorpusModifiers {
                control: true,
                ..CorpusModifiers::NONE
            },
        )
    };
    let add_focus = |target| CorpusOperation::ScreenFocus(ScreenTarget::Add(target));
    let add_hit = |target| CorpusOperation::ScreenHit(ScreenTarget::Add(target));
    let add_local = |target, key| CorpusOperation::LocalKeyboard {
        target: LocalActionTarget::Add(target),
        key,
    };
    vec![
        CorpusOperation::CommandKeyboard(UiCommand::Add),
        add_focus(AddControlId::NewPrompt),
        add_local(AddControlId::NewPrompt, plain(CorpusKey::Enter)),
        add_focus(AddControlId::Interpolate),
        add_hit(AddControlId::Interpolate),
        add_focus(AddControlId::EditSource),
        add_local(AddControlId::EditSource, control('e')),
        add_focus(AddControlId::Text(AddTextField::ReviewName)),
        CorpusOperation::RawKey(control('u')),
        CorpusOperation::Paste("Corpus Prompt".to_owned()),
        add_local(AddControlId::Cancel, plain(CorpusKey::Escape)),
        add_local(AddControlId::Cancel, plain(CorpusKey::Escape)),
    ]
}

/// The short operation vector that paints an absolute sandbox path on a real screen.
///
/// It installs one Agent Skill, opens the source file picker, and picks one seeded file. The
/// Agent Skill status, the picker directory header, and the Add source field each paint a path
/// that a declared sandbox root holds.
#[cfg(any(target_os = "linux", target_os = "windows"))]
pub(crate) fn rendered_sandbox_root_corpus_operations() -> Vec<CorpusOperation> {
    let plain = |code| CorpusKeyEvent::press(code, CorpusModifiers::NONE);
    let add_focus = |target| CorpusOperation::ScreenFocus(ScreenTarget::Add(target));
    let add_hit = |target| CorpusOperation::ScreenHit(ScreenTarget::Add(target));
    let add_local = |target, key| CorpusOperation::LocalKeyboard {
        target: LocalActionTarget::Add(target),
        key,
    };
    let preferences_focus =
        |target| CorpusOperation::ScreenFocus(ScreenTarget::Preferences(target));
    let preferences_hit = |target| CorpusOperation::ScreenHit(ScreenTarget::Preferences(target));
    vec![
        CorpusOperation::CommandKeyboard(UiCommand::Preferences),
        preferences_focus(PreferencesControlId::InstallAgentSkill),
        preferences_hit(PreferencesControlId::InstallAgentSkill),
        CorpusOperation::ScreenHit(ScreenTarget::AgentSkill {
            name: "codex".to_owned(),
            scope: AgentScope::User,
        }),
        CorpusOperation::CommandKeyboard(UiCommand::ClosePreferences),
        CorpusOperation::CommandKeyboard(UiCommand::Add),
        add_hit(AddControlId::BrowseSource),
        CorpusOperation::ScreenHit(ScreenTarget::FilePickerEntry {
            relative: PathBuf::from("picked.sh"),
        }),
        add_focus(AddControlId::Continue),
        add_local(AddControlId::Continue, plain(CorpusKey::Enter)),
        add_local(AddControlId::Cancel, plain(CorpusKey::Escape)),
        add_local(AddControlId::Cancel, plain(CorpusKey::Escape)),
    ]
}

pub(crate) fn canonical_corpus_artifact_values() -> Result<(Vec<Value>, Value), String> {
    let operations = canonical_corpus_operations()
        .iter()
        .map(corpus_operation_value)
        .collect::<Result<Vec<_>, _>>()?;
    let final_liveness = corpus_operation_value(&CorpusOperation::FinalLiveness)?;
    Ok((operations, final_liveness))
}

pub(crate) fn record_real_smoke_main() -> Result<RecordedRealTrace, String> {
    record_schema_three_main(&smoke_factory())
}

pub(crate) fn record_real_smoke_replay() -> Result<RecordedRealTrace, String> {
    record_schema_three_replay(&smoke_factory())
}

pub(crate) fn record_real_locale_smoke(
    target: LocaleSmokeTarget,
) -> Result<RecordedRealTrace, String> {
    let mut operations = vec![
        SmokeOperation::OpenPreferences,
        SmokeOperation::OpenLanguagePicker,
    ];
    for _ in 0..target.next_count() {
        operations.push(SmokeOperation::NextLanguage);
    }
    operations.extend([
        SmokeOperation::ChooseLanguage,
        SmokeOperation::SavePreferences,
        SmokeOperation::OpenRun,
    ]);
    // Reference details paint the random directory after the locale changes. Keep this
    // locale fixture on a copied entry; stable pairs cover rendered reference paths.
    let mut factory = smoke_factory();
    let entry = &mut factory.seed.external_references[0].request;
    entry.mode = StorageMode::Copy;
    entry.workdir = "store".to_owned();
    record_schema_three_random(&factory, &operations)
}

pub(crate) fn record_real_smoke_stable_pair_in(
    namespace: StableSandboxNamespace,
) -> Result<StableTracePair, StablePairError> {
    let factory = smoke_factory();
    let (main, successful_operations) = record_schema_three_stable(
        &factory,
        namespace.clone(),
        StablePairPhase::Main,
        &[SmokeOperation::OpenRun],
    )?;
    let (replay, replayed_operations) = record_schema_three_stable(
        &factory,
        namespace,
        StablePairPhase::Replay,
        &successful_operations,
    )?;
    debug_assert_eq!(replayed_operations, successful_operations);
    Ok(StableTracePair { main, replay })
}

pub(super) fn smoke_seed() -> WalkerSeedSpec {
    let bytes = b"printf smoke\n".to_vec();
    WalkerSeedSpec {
        profile: "engine-smoke".to_owned(),
        external: vec![WalkerExternalSeed::File {
            path: PathBuf::from("original.sh"),
            bytes: bytes.clone(),
            readonly: false,
            unix_mode: 0o640,
        }],
        external_references: vec![WalkerExternalReferenceSeed {
            request: CreateEntry {
                name: "Reference".to_owned(),
                kind: EntryKind::parse("shell").expect("shell is a supported kind"),
                mode: StorageMode::Reference,
                source: String::new(),
                workdir: "origin".to_owned(),
                description: "real walker reference".to_owned(),
                payload: Some(EntryPayload {
                    bytes,
                    stored_name: Some("original.sh".to_owned()),
                    permissions: SourcePermissions {
                        readonly: false,
                        unix_mode: Some(0o640),
                    },
                }),
                settings: EntrySettings::default(),
            },
            source: PathBuf::from("original.sh"),
        }],
        ..WalkerSeedSpec::default()
    }
}
