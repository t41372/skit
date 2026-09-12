//! Shared fixtures and contracts of the real walker.

use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use ratatui_core::layout::{Rect, Size};
use ratatui_crossterm::crossterm::event::{Event, KeyCode, KeyModifiers};
use serde_json::{Value, json};
use skit_application::AgentScope;
use skit_i18n::Locale;
use skit_tui::{
    AddControlId, AddTextField, HitTarget, LocalActionOutcome, LocalActionTarget,
    LocalAdvertisedAction, RunFieldCommand, ScreenTarget, ScreenTargetError, ScreenTargetInventory,
    ViewGeometry,
};
use skit_tui_walker_support::{
    EventChainIdentity, LivenessResult, ObjectKind, Presentation, RectSnapshot,
    StyledFrameSnapshot, TimelineBoundary, TimelinePhase, TimelineRow, TransitionCause,
    asciicast::{AsciicastRecorder, FRAME_INTERVAL},
    engine::{
        CheckpointCapture, CheckpointCauseProjection, CheckpointFor, CheckpointSink,
        DispatchOutcome, EffectRoute, EngineBoundary, EngineCause, EngineCheckpoint, EnginePhase,
        FrontendAdapter, HostAdapter, OperationResolution, ReplayFactory, WalkerEngine,
        replay_prefix_with_sink,
    },
    sandbox::{SandboxMetadata, SandboxMode, SandboxPlatform},
    timeline_row_digest, validate_object,
};
#[cfg(any(target_os = "linux", target_os = "windows"))]
use skit_tui_walker_support::{
    bundle, canonical_json_bytes,
    sandbox::{SANDBOX_MARKER_FILE, STABLE_SANDBOX_NAMESPACE, SafeProfileId, profile_sandbox_path},
    sha256_hex,
};
use skit_ui::{
    Action, Effect, HealthAction, HostRequest, LibraryState, PreferencesAction,
    PreferencesControlId, RunnerEditorAction, RunnerManagerAction, Screen, UiCommand, UiKey,
};

use super::{
    corpus::{
        CorpusEvent, CorpusInputKind, CorpusKey, CorpusKeyEvent, CorpusKeyKind, CorpusModifiers,
        CorpusMouseKind, CorpusNotApplicable, CorpusOperation, CorpusResolution,
        CorpusSemanticTarget, EFFECT_LIMIT, FrontendObservation, FrontendParity, LocaleSmokeTarget,
        SmokeEvent, SmokeHandling, SmokeOperation, SmokeResolution, corpus_add_control_value,
        corpus_add_text_field_value, corpus_agent_scope_label, corpus_canvas_size,
        corpus_hit_target_value, corpus_local_outcome_value, corpus_local_target_value,
        corpus_operation_value, corpus_preferences_control_value, corpus_relative_component_values,
        corpus_resolution_value, corpus_screen_target_value, corpus_semantic_target_value,
        resolve_corpus_operation, resolve_corpus_operation_with_local_actions,
        resolve_screen_operation_with_inventory, screen_target_error_message,
        validate_screen_target,
    },
    engine::{
        CheckpointLeakContext, CorpusEngine, CorpusHostBoundary, RealHostBoundary,
        RealWalkerFactory, close_corpus_engine,
    },
    frontend::{CorpusFrontend, RealFrontend},
    recording::{
        combine_corpus_result_after_close, new_schema_three_sink, noop_leak_oracle_facts,
        noop_real_host_boundary, noop_schema_sink, read_boundary_sandbox_metadata,
        record_real_locale_smoke, record_schema_three_main, record_schema_three_random_with_hooks,
        record_schema_three_replay, required_corpus_factories, smoke_factory, smoke_seed,
    },
    trace::{
        CauseObservation, MemoryTrace, ProjectedCheckpoint, RealTrace, SchemaThreeSink,
        checked_u32, json_value, record_presentation, smoke_event_value, smoke_resolution_value,
        timeline_boundary, transition_cause, validate_effect_chain_termination,
        validate_reducer_action,
    },
};
#[cfg(any(target_os = "linux", target_os = "windows"))]
use super::{
    recording::{
        canonical_corpus_operations, noop_stable_host, prepare_corpus_host_with,
        read_sandbox_metadata, record_corpus_profile, record_real_smoke_main,
        record_real_smoke_stable_pair_in, record_schema_three_stable,
        record_schema_three_stable_with_hooks,
    },
    trace::{
        RecordedRealCorpus, RecordedRealTrace, StableCorpusTracePair, StablePairPhase,
        StablePairPrimaryError,
    },
};
#[cfg(any(target_os = "linux", target_os = "windows"))]
use crate::cli::tui_real_host::{
    ArtifactLeakOracleFact, SandboxError, StableSandboxNamespace, WalkerExternalSeed,
};
use crate::cli::tui_real_host::{HostObservation, LeakOracleFacts, RealWalkerHost};

mod corpus;
mod resolution;
mod sink;
mod traces;

#[derive(Default)]
struct CorpusRefusalSink {
    refusals: Vec<(usize, CorpusNotApplicable)>,
    boundaries: Vec<EngineBoundary>,
    last: Option<(EnginePhase, Option<usize>)>,
}

impl CheckpointSink<CheckpointFor<CorpusFrontend, CorpusHostBoundary>> for CorpusRefusalSink {
    fn record(
        &mut self,
        checkpoint: CheckpointFor<CorpusFrontend, CorpusHostBoundary>,
    ) -> Result<(), String> {
        let operation_index = match &checkpoint.cause {
            EngineCause::Initial => None,
            EngineCause::NotApplicable {
                operation_index,
                resolved,
                ..
            } => {
                let refusal = operation_index
                    .map(|index| {
                        resolved
                            .refusal
                            .ok_or("a not-applicable corpus operation has no refusal".to_owned())
                            .map(|refusal| (index, refusal))
                    })
                    .transpose()?;
                self.refusals.extend(refusal);
                *operation_index
            }
            EngineCause::Session {
                operation_index, ..
            }
            | EngineCause::Reducer {
                operation_index, ..
            }
            | EngineCause::Host {
                operation_index, ..
            } => *operation_index,
        };
        self.boundaries.push(checkpoint.boundary);
        self.last = Some((checkpoint.phase, operation_index));
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn synthetic_stable_root(alternate: bool) -> &'static str {
    if alternate {
        "/tmp/other/skit-ui-walker-v1"
    } else {
        "/tmp/skit-ui-walker-v1"
    }
}

#[cfg(target_os = "windows")]
fn synthetic_stable_root(alternate: bool) -> &'static str {
    if alternate {
        r"C:\tmp\other\skit-ui-walker-v1"
    } else {
        r"C:\tmp\skit-ui-walker-v1"
    }
}

fn reducer_emitted(cause: &mut TransitionCause) -> &mut Value {
    match cause {
        TransitionCause::Reducer { emitted, .. } => emitted,
        TransitionCause::Initial
        | TransitionCause::Session { .. }
        | TransitionCause::Host { .. } => {
            panic!("the test checkpoint must be a reducer action");
        }
    }
}

fn prepared_schema_three_sink(factory: &RealWalkerFactory) -> SchemaThreeSink {
    let (frontend, host) = factory.create().unwrap();
    let mut engine = WalkerEngine::new(frontend, host, EFFECT_LIMIT).unwrap();
    let mut sink = new_schema_three_sink(factory).unwrap();
    engine.start(&mut sink).unwrap();
    engine
        .run_operation(SmokeOperation::OpenRun, &mut sink)
        .unwrap();
    engine
        .run_final_liveness(SmokeOperation::FinalLiveness, &mut sink)
        .unwrap();
    sink
}

fn checkpoint_with_cause(
    mut cause: EngineCause<
        SmokeOperation,
        SmokeResolution,
        SmokeEvent,
        Action,
        Effect,
        SmokeHandling,
    >,
) -> CheckpointFor<RealFrontend, RealHostBoundary> {
    let (mut frontend, mut host) = smoke_factory().create().unwrap();
    let frontend = frontend.observe().unwrap();
    let projection = match &mut cause {
        EngineCause::Initial | EngineCause::NotApplicable { .. } | EngineCause::Session { .. } => {
            CheckpointCauseProjection::Observation
        }
        EngineCause::Reducer {
            action, emitted, ..
        } => CheckpointCauseProjection::Reducer { action, emitted },
        EngineCause::Host {
            request,
            response,
            emitted,
            ..
        } => CheckpointCauseProjection::Host {
            request,
            response,
            emitted,
        },
    };
    let CheckpointCapture { frontend, host } =
        host.capture_checkpoint(frontend, projection).unwrap();
    EngineCheckpoint {
        phase: EnginePhase::Operations,
        boundary: EngineBoundary::Session,
        cause,
        frontend,
        host,
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn read_initial_state(host: &RealWalkerHost) -> Result<LibraryState, String> {
    host.initial_state().map_err(|error| error.to_string())
}

fn corpus_engine() -> WalkerEngine<CorpusFrontend, CorpusHostBoundary> {
    let factory = smoke_factory();
    let host = RealWalkerHost::spawn(factory.seed).unwrap();
    let file_picker_tree = host.file_picker_tree();
    let state = host.initial_state().unwrap();
    let frontend =
        CorpusFrontend::new(state, factory.locale, factory.size, file_picker_tree).unwrap();
    WalkerEngine::new(
        frontend,
        CorpusHostBoundary {
            inner: RealHostBoundary { host },
            leak_sandbox: None,
        },
        EFFECT_LIMIT,
    )
    .unwrap()
}

fn corpus_engine_for_factory(
    factory: RealWalkerFactory,
) -> WalkerEngine<CorpusFrontend, CorpusHostBoundary> {
    let host = RealWalkerHost::spawn(factory.seed).unwrap();
    let file_picker_tree = host.file_picker_tree();
    let state = host.initial_state().unwrap();
    let frontend =
        CorpusFrontend::new(state, factory.locale, factory.size, file_picker_tree).unwrap();
    WalkerEngine::new(
        frontend,
        CorpusHostBoundary {
            inner: RealHostBoundary { host },
            leak_sandbox: None,
        },
        EFFECT_LIMIT,
    )
    .unwrap()
}

fn resolution_parts(
    resolution: OperationResolution<CorpusEvent, CorpusResolution>,
) -> (CorpusResolution, Vec<CorpusEvent>) {
    match resolution {
        OperationResolution::NotApplicable { resolved } => (resolved, Vec::new()),
        OperationResolution::Events { resolved, events } => (resolved, events),
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn stable_corpus_engine_for_factory(
    factory: RealWalkerFactory,
    namespace: StableSandboxNamespace,
) -> WalkerEngine<CorpusFrontend, CorpusHostBoundary> {
    let host =
        RealWalkerHost::spawn_stable_in(factory.seed, factory.review_profile.clone(), namespace)
            .unwrap();
    let sandbox = host.sandbox_metadata(&factory.review_profile).unwrap();
    let file_picker_tree = host.file_picker_tree();
    let state = host.initial_state().unwrap();
    let frontend =
        CorpusFrontend::new(state, factory.locale, factory.size, file_picker_tree).unwrap();
    WalkerEngine::new(
        frontend,
        CorpusHostBoundary {
            inner: RealHostBoundary { host },
            leak_sandbox: Some(sandbox),
        },
        EFFECT_LIMIT,
    )
    .unwrap()
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn stable_corpus_engine(
    namespace: StableSandboxNamespace,
) -> WalkerEngine<CorpusFrontend, CorpusHostBoundary> {
    stable_corpus_engine_for_factory(smoke_factory(), namespace)
}
