//! Recorded traces, transition causes, and the schema-three sink.

use std::collections::{BTreeMap, BTreeSet};

use ratatui_core::layout::Size;
use serde_json::{Value, json};
use skit_i18n::Locale;
use skit_tui_walker_support::{
    EventChainIdentity, LivenessResult, ObjectKind, ObjectRef, Presentation, RectSnapshot,
    StyledFrameSnapshot, TimelineBoundary, TimelinePhase, TimelineRow, TransitionCause,
    asciicast::{AsciicastRecorder, FRAME_INTERVAL},
    build_object, bundle,
    engine::{CheckpointFor, CheckpointSink, EngineBoundary, EngineCause, EnginePhase},
    expected_presentation,
    sandbox::{SafeProfileId, SandboxMetadata, SandboxMode},
    timeline_row_digest, validate_asciicast_v3, validate_object, validate_styled_frame,
    validate_timeline, validate_timeline_semantics,
};
use skit_ui::{Action, Effect, HostRequest, LibraryState, Screen};

use super::{
    corpus::{
        CorpusEvent, CorpusOperation, CorpusResolution, FrontendObservation, SmokeEvent,
        SmokeHandling, SmokeOperation, SmokeResolution, corpus_operation_value,
        corpus_resolution_value,
    },
    engine::{CheckpointLeakContext, CorpusHostBoundary, RealHostBoundary},
    frontend::{CorpusFrontend, RealFrontend},
};
use crate::cli::tui_real_host::{
    HostObservation, LeakOracleFacts, SandboxError, StableHostPrimaryError,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum CauseObservation {
    Initial,
    Reducer {
        dispatch_sequence: usize,
        operation_index: usize,
        operation: SmokeOperation,
        resolved: SmokeResolution,
        event: SmokeEvent,
        selector: String,
    },
    Host {
        dispatch_sequence: usize,
        operation_index: usize,
        operation: SmokeOperation,
        round: usize,
        selector: String,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct CanonicalCheckpoint {
    phase: EnginePhase,
    pub(super) boundary: EngineBoundary,
    pub(super) cause: CauseObservation,
    pub(super) canonical_state: Value,
    pub(super) host: HostObservation,
    pub(super) session: Value,
    pub(super) styled_frame: StyledFrameSnapshot,
    pub(super) geometry: Value,
    locale: Locale,
}

#[derive(Default)]
pub(super) struct MemoryTrace {
    pub(super) checkpoints: Vec<CanonicalCheckpoint>,
}

impl CheckpointSink<CheckpointFor<RealFrontend, RealHostBoundary>> for MemoryTrace {
    fn record(
        &mut self,
        checkpoint: CheckpointFor<RealFrontend, RealHostBoundary>,
    ) -> Result<(), String> {
        let cause = match &checkpoint.cause {
            EngineCause::Initial => CauseObservation::Initial,
            EngineCause::Reducer {
                dispatch_sequence,
                operation_index: Some(operation_index),
                operation,
                resolved,
                event,
                action: Action::OpenRun,
                emitted:
                    Effect::Open {
                        request: HostRequest::Run,
                        selector: Some(selector),
                    },
            } => CauseObservation::Reducer {
                dispatch_sequence: *dispatch_sequence,
                operation_index: *operation_index,
                operation: *operation,
                resolved: *resolved,
                event: *event,
                selector: selector.clone(),
            },
            EngineCause::Host {
                dispatch_sequence,
                operation_index: Some(operation_index),
                operation,
                round,
                request:
                    Effect::Open {
                        request: HostRequest::Run,
                        selector: Some(selector),
                    },
                response: Action::Present(Screen::Run(_)),
                emitted: Effect::None,
            } => CauseObservation::Host {
                dispatch_sequence: *dispatch_sequence,
                operation_index: *operation_index,
                operation: *operation,
                round: *round,
                selector: selector.clone(),
            },
            EngineCause::NotApplicable { .. } | EngineCause::Session { .. } => {
                return Err("the real walker smoke operation did not cross the host".to_owned());
            }
            EngineCause::Reducer { .. } | EngineCause::Host { .. } => {
                return Err("the real walker smoke host transition changed shape".to_owned());
            }
        };
        self.checkpoints.push(CanonicalCheckpoint {
            phase: checkpoint.phase,
            boundary: checkpoint.boundary,
            cause,
            canonical_state: checkpoint.host.state.clone(),
            host: checkpoint.host,
            session: checkpoint.frontend.session,
            styled_frame: checkpoint.frontend.styled_frame,
            geometry: checkpoint.frontend.geometry,
            locale: checkpoint.frontend.locale,
        });
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RealTrace {
    pub(crate) review_profile: SafeProfileId,
    pub(crate) locale: String,
    pub(crate) viewport: RectSnapshot,
    pub(crate) operations: Vec<Value>,
    pub(crate) final_liveness_requested: Value,
    pub(crate) rows: Vec<TimelineRow>,
    pub(crate) objects: BTreeMap<(ObjectKind, String), Vec<u8>>,
    pub(crate) cast: Vec<u8>,
}

#[derive(Clone, Debug)]
pub(crate) struct RecordedRealTrace {
    pub(crate) trace: RealTrace,
    pub(crate) sandbox: SandboxMetadata,
    pub(crate) leak_oracle_facts: LeakOracleFacts,
}

impl std::ops::Deref for RecordedRealTrace {
    type Target = RealTrace;

    fn deref(&self) -> &Self::Target {
        &self.trace
    }
}

impl std::ops::DerefMut for RecordedRealTrace {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.trace
    }
}

#[derive(Clone, Debug)]
pub(crate) struct RecordedRealCorpus {
    pub(crate) profiles: Vec<RecordedRealTrace>,
    pub(crate) sandbox: SandboxMetadata,
}

impl RecordedRealCorpus {
    pub(crate) fn new(profiles: Vec<RecordedRealTrace>) -> Result<Self, String> {
        let required = skit_tui_walker_support::required_review_profiles();
        if profiles.len() != required.len() {
            return Err("the recorded corpus does not contain the exact four profiles".to_owned());
        }
        let operations = profiles[0].operations.clone();
        let final_liveness = profiles[0].final_liveness_requested.clone();
        if operations.is_empty() {
            return Err("the recorded corpus has an empty operation vector".to_owned());
        }
        let platform = profiles[0].sandbox.platform();
        let root = profiles[0].sandbox.root().to_owned();
        let mut ids = BTreeSet::new();
        for (recorded, expected) in profiles.iter().zip(required) {
            let id = SafeProfileId::try_from(expected.id).map_err(|error| error.to_string())?;
            if recorded.review_profile != id {
                return Err("the recorded corpus profiles are not in required order".to_owned());
            }
            if recorded.locale != expected.locale || recorded.viewport != expected.viewport {
                return Err("a recorded corpus profile has the wrong locale or viewport".to_owned());
            }
            if recorded.operations != operations
                || recorded.final_liveness_requested != final_liveness
            {
                return Err("recorded corpus profiles do not share one operation vector".to_owned());
            }
            if recorded.rows.last().and_then(|row| row.liveness.clone())
                != Some(LivenessResult::Passed)
            {
                return Err("a recorded corpus profile did not pass final liveness".to_owned());
            }
            if recorded.rows.iter().any(|row| {
                matches!(
                    &row.cause,
                    TransitionCause::Session { handling, .. } if handling == "not_applicable"
                )
            }) {
                return Err("a recorded corpus profile contains a refused operation".to_owned());
            }
            if recorded.rows.iter().any(|row| row.profile != id) {
                return Err("a recorded corpus timeline has the wrong profile".to_owned());
            }
            if recorded.sandbox.mode() != SandboxMode::Stable
                || recorded.sandbox.platform() != platform
                || recorded.sandbox.root() != root
            {
                return Err(
                    "recorded corpus sandboxes do not share one stable namespace".to_owned(),
                );
            }
            let singleton =
                SandboxMetadata::stable(platform, root.clone(), BTreeSet::from([id.clone()]))
                    .map_err(|error| error.to_string())?;
            if recorded.sandbox != singleton {
                return Err("a recorded corpus profile has invalid singleton metadata".to_owned());
            }
            ids.insert(id);
        }
        let sandbox =
            SandboxMetadata::stable(platform, root, ids).map_err(|error| error.to_string())?;
        Ok(Self { profiles, sandbox })
    }
}

#[derive(Clone, Debug)]
pub(crate) struct StableCorpusTracePair {
    pub(crate) main: RecordedRealCorpus,
    pub(crate) replay: RecordedRealCorpus,
}

impl StableCorpusTracePair {
    pub(crate) fn new(
        main: RecordedRealCorpus,
        replay: RecordedRealCorpus,
    ) -> Result<Self, String> {
        if main.sandbox != replay.sandbox || main.profiles.len() != replay.profiles.len() {
            return Err("stable corpus generations use different aggregate metadata".to_owned());
        }
        if let Some((diverging, _)) = main
            .profiles
            .iter()
            .zip(&replay.profiles)
            .find(|(main, replay)| main.trace != replay.trace || main.sandbox != replay.sandbox)
        {
            return Err(format!(
                "stable corpus main and replay traces differ at {}",
                diverging.review_profile
            ));
        }
        Ok(Self { main, replay })
    }
}

#[derive(Clone, Debug)]
pub(crate) struct StableTracePair {
    pub(crate) main: RecordedRealTrace,
    pub(crate) replay: RecordedRealTrace,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StablePairPhase {
    Main,
    Replay,
}

#[derive(Debug)]
pub(crate) enum StablePairPrimaryError {
    Sandbox(Box<SandboxError>),
    Seed(StableHostPrimaryError),
    InitialState(String),
    Frontend(String),
    Trace(String),
    Close(Box<SandboxError>),
}

#[derive(Debug)]
pub(crate) struct StablePairError {
    pub(crate) phase: StablePairPhase,
    pub(crate) primary: StablePairPrimaryError,
    pub(crate) cleanup: Option<Box<SandboxError>>,
}

impl std::fmt::Display for StablePairPrimaryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sandbox(error) => error.fmt(formatter),
            Self::Seed(error) => error.fmt(formatter),
            Self::InitialState(error) => {
                write!(
                    formatter,
                    "could not read the stable walker initial state: {error}"
                )
            }
            Self::Frontend(error) => {
                write!(
                    formatter,
                    "could not create the stable walker frontend: {error}"
                )
            }
            Self::Trace(error) => write!(formatter, "could not record the stable walker: {error}"),
            Self::Close(error) => write!(formatter, "could not close the stable walker: {error}"),
        }
    }
}

impl std::fmt::Display for StablePairError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let phase = match self.phase {
            StablePairPhase::Main => "main",
            StablePairPhase::Replay => "replay",
        };
        write!(formatter, "stable walker {phase} failed: {}", self.primary)?;
        if let Some(cleanup) = &self.cleanup {
            write!(formatter, "; sandbox cleanup also failed: {cleanup}")?;
        }
        Ok(())
    }
}

impl std::error::Error for StablePairError {}

pub(crate) struct SchemaThreeSink {
    review_profile: SafeProfileId,
    initial_locale: String,
    viewport: RectSnapshot,
    canvas: Size,
    pub(super) rows: Vec<TimelineRow>,
    pub(super) objects: BTreeMap<(ObjectKind, String), Vec<u8>>,
    pub(super) cast: AsciicastRecorder,
}

pub(super) struct ProjectedCheckpoint {
    pub(super) phase: EnginePhase,
    pub(super) boundary: EngineBoundary,
    pub(super) operation_index: Option<u32>,
    pub(super) event_chain: Option<EventChainIdentity>,
    pub(super) cause: TransitionCause,
    pub(super) frontend: FrontendObservation,
    pub(super) host: HostObservation,
    pub(super) leak_context: Option<CheckpointLeakContext>,
}

impl SchemaThreeSink {
    pub(super) fn new(
        review_profile: SafeProfileId,
        locale: Locale,
        viewport: Size,
        canvas: Size,
    ) -> Result<Self, String> {
        Ok(Self {
            review_profile,
            initial_locale: locale.tag().to_owned(),
            viewport: RectSnapshot {
                x: 0,
                y: 0,
                width: viewport.width,
                height: viewport.height,
            },
            canvas,
            rows: Vec::new(),
            objects: BTreeMap::new(),
            cast: AsciicastRecorder::new(canvas.width, canvas.height)
                .map_err(|error| error.to_string())?,
        })
    }

    /// Return the cast that this sink recorded so far.
    pub(crate) fn cast_bytes(&self) -> &[u8] {
        self.cast.as_bytes()
    }

    /// Record the frame that a failing frontend last drew.
    ///
    /// A checkpoint that breaks one rule never reaches this sink. The failing screen still stays
    /// in the backend, so the artifact keeps one diagnostic frame beside the named rule.
    pub(crate) fn record_frontend_frame(
        &mut self,
        frontend: &CorpusFrontend,
    ) -> Result<(), String> {
        self.cast
            .record_frame(FRAME_INTERVAL, frontend.inner.terminal.backend())
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    pub(super) fn put_value(
        &mut self,
        kind: ObjectKind,
        value: Value,
    ) -> Result<ObjectRef, String> {
        let mut staged = BTreeMap::new();
        let reference = Self::stage_value(&self.objects, &mut staged, kind, value)?;
        self.objects.extend(staged);
        Ok(reference)
    }

    fn stage_value(
        objects: &BTreeMap<(ObjectKind, String), Vec<u8>>,
        staged: &mut BTreeMap<(ObjectKind, String), Vec<u8>>,
        kind: ObjectKind,
        value: Value,
    ) -> Result<ObjectRef, String> {
        let (reference, bytes) = build_object(kind, &value).map_err(|error| error.to_string())?;
        let decoded = validate_object(&reference, &bytes).map_err(|error| error.to_string())?;
        debug_assert_eq!(decoded, value);
        let key = (reference.kind, reference.sha256.clone());
        if let Some(existing) = staged.get(&key).or_else(|| objects.get(&key)) {
            if existing != &bytes {
                return Err("one content digest mapped to different object bytes".to_owned());
            }
        } else {
            staged.insert(key, bytes);
        }
        Ok(reference)
    }

    pub(super) fn object_value(&self, reference: &ObjectRef) -> Result<Value, String> {
        let bytes = self
            .objects
            .get(&(reference.kind, reference.sha256.clone()))
            .ok_or_else(|| "a timeline object is absent from memory".to_owned())?;
        validate_object(reference, bytes).map_err(|error| error.to_string())
    }

    pub(super) fn validate_reducer_transitions(&self) -> Result<(), String> {
        let initial = self
            .rows
            .first()
            .ok_or_else(|| "the schema three trace has no checkpoint".to_owned())?;
        let initial_value = self.object_value(&initial.reducer)?;
        parse_library_state(&initial_value)?;

        for pair in self.rows.windows(2) {
            let previous = &pair[0];
            let current = &pair[1];
            let previous_value = self.object_value(&previous.reducer)?;
            let current_value = self.object_value(&current.reducer)?;
            parse_library_state(&current_value)?;
            let result = match &current.cause {
                TransitionCause::Initial => {
                    Err("only the first reducer object can be initial".to_owned())
                }
                TransitionCause::Session { .. } => {
                    if current_value != previous_value {
                        Err("a session checkpoint changed reducer state".to_owned())
                    } else {
                        Ok(())
                    }
                }
                TransitionCause::Reducer {
                    action, emitted, ..
                } => validate_reducer_action(&previous_value, &current_value, action, emitted),
                TransitionCause::Host {
                    response, emitted, ..
                } => validate_reducer_action(&previous_value, &current_value, response, emitted),
            };
            result.map_err(|error| {
                format!(
                    "timeline row {} operation {:?}: {error}",
                    current.sequence, current.operation_index
                )
            })?;
        }
        Ok(())
    }

    pub(super) fn finish(
        mut self,
        operations: Vec<Value>,
        final_liveness_requested: Value,
    ) -> Result<RealTrace, String> {
        self.validate_reducer_transitions()?;
        let final_row = self
            .rows
            .last()
            .ok_or_else(|| "the schema three trace has no checkpoint".to_owned())?;
        let final_state = self.object_value(&final_row.reducer)?;
        if final_state.pointer("/workflow/active") != Some(&json!("library"))
            || final_state
                .get("modal")
                .is_some_and(|modal| !modal.is_null())
        {
            return Err("final liveness did not return the real reducer to the library".to_owned());
        }
        self.rows
            .last_mut()
            .expect("the final row was checked above")
            .liveness = Some(LivenessResult::Passed);

        let operation_count = checked_u32(operations.len(), "operation count")?;
        validate_timeline(&self.rows, operation_count).map_err(|error| error.to_string())?;
        validate_timeline_semantics(
            &self.rows,
            &operations,
            &final_liveness_requested,
            &self.initial_locale,
        )
        .map_err(|error| error.to_string())?;
        validate_effect_chain_termination(&self.rows)?;

        let canvas = bundle::cast_canvas(&self.rows).map_err(|error| error.to_string())?;
        if canvas != (self.canvas.width, self.canvas.height) {
            return Err("the timeline canvas does not match the live asciicast".to_owned());
        }
        let mut rebuilt = AsciicastRecorder::new(self.canvas.width, self.canvas.height)
            .map_err(|error| error.to_string())?;
        for row in &self.rows {
            for reference in [
                &row.reducer,
                &row.host,
                &row.session,
                &row.styled_frame,
                &row.geometry,
            ] {
                self.object_value(reference)?;
            }
            let frame_value = self.object_value(&row.styled_frame)?;
            let frame: StyledFrameSnapshot =
                serde_json::from_value(frame_value).map_err(|error| error.to_string())?;
            validate_styled_frame(&frame).map_err(|error| error.to_string())?;
            if frame.area != row.viewport {
                return Err("a styled frame does not match its timeline viewport".to_owned());
            }
            record_presentation(&mut rebuilt, row.presentation, &frame)?;
        }
        validate_asciicast_v3(self.cast.as_bytes()).map_err(|error| error.to_string())?;
        validate_asciicast_v3(rebuilt.as_bytes()).map_err(|error| error.to_string())?;
        if self.cast.as_bytes() != rebuilt.as_bytes() {
            return Err("stored styled frames did not rebuild the live asciicast".to_owned());
        }

        Ok(RealTrace {
            review_profile: self.review_profile,
            locale: self.initial_locale,
            viewport: self.viewport,
            operations,
            final_liveness_requested,
            rows: self.rows,
            objects: self.objects,
            cast: self.cast.as_bytes().to_vec(),
        })
    }

    pub(super) fn finish_smoke(self) -> Result<RealTrace, String> {
        self.finish(
            vec![smoke_operation_value(SmokeOperation::OpenRun)],
            smoke_operation_value(SmokeOperation::FinalLiveness),
        )
    }

    pub(super) fn record_checkpoint(
        &mut self,
        checkpoint: ProjectedCheckpoint,
    ) -> Result<(), String> {
        let ProjectedCheckpoint {
            phase,
            boundary,
            operation_index,
            event_chain,
            cause,
            frontend,
            host,
            leak_context,
        } = checkpoint;
        let host_value = json_value(&host);
        if let Some(context) = leak_context {
            let cause_value = json_value(&cause);
            crate::cli::tui_walker_bundle::validate_checkpoint_leaks(
                &context.sandbox,
                &context.facts,
                &[
                    ("cause", &cause_value),
                    ("reducer", &host.state),
                    ("host", &host_value),
                    ("session", &frontend.session),
                    ("geometry", &frontend.geometry),
                ],
                &frontend.styled_frame,
            )
            .map_err(|error| {
                format!("checkpoint phase {phase:?} operation index {operation_index:?}: {error}")
            })?;
        }
        let mut staged = BTreeMap::new();
        let reducer = Self::stage_value(
            &self.objects,
            &mut staged,
            ObjectKind::Reducer,
            host.state.clone(),
        )?;
        let host = Self::stage_value(&self.objects, &mut staged, ObjectKind::Host, host_value)?;
        let session = Self::stage_value(
            &self.objects,
            &mut staged,
            ObjectKind::Session,
            frontend.session,
        )?;
        let frame = frontend.styled_frame;
        let styled_frame = Self::stage_value(
            &self.objects,
            &mut staged,
            ObjectKind::StyledFrame,
            json_value(&frame),
        )?;
        let geometry = Self::stage_value(
            &self.objects,
            &mut staged,
            ObjectKind::Geometry,
            frontend.geometry,
        )?;
        let sequence = u32::try_from(self.rows.len())
            .map_err(|_| "the schema three trace has too many checkpoints".to_owned())?;
        let previous_row_sha256 = self
            .rows
            .last()
            .map(timeline_row_digest)
            .transpose()
            .map_err(|error| error.to_string())?;
        let mut row = TimelineRow {
            schema: 3,
            profile: self.review_profile.clone(),
            sequence,
            phase: timeline_phase(phase),
            event_chain,
            operation_index,
            boundary: timeline_boundary(boundary),
            cause,
            presentation: Presentation::Presented,
            reducer,
            host,
            session,
            styled_frame,
            geometry,
            locale: frontend.locale.tag().to_owned(),
            viewport: frame.area,
            liveness: None,
            previous_row_sha256,
        };
        row.presentation = expected_presentation(&row);
        record_presentation(&mut self.cast, row.presentation, &frame)?;
        self.objects.extend(staged);
        self.rows.push(row);
        Ok(())
    }
}

pub(crate) fn validate_reducer_action(
    previous: &Value,
    current: &Value,
    action: &Value,
    emitted: &Value,
) -> Result<(), String> {
    let mut state = parse_library_state(previous)?;
    let action: Action = serde_json::from_value(action.clone())
        .map_err(|error| format!("a reducer cause is not an Action: {error}"))?;
    let actual_effect = state.update(action);
    let actual_state = json_value(&state);
    // PathBuf equality keeps native separator semantics. The round trip still rejects
    // fields that deserialization would discard.
    let effect_matches = serde_json::from_value::<Effect>(emitted.clone())
        .is_ok_and(|expected| actual_effect == expected && json_value(&expected) == *emitted);
    if !effect_matches {
        return Err("a reducer cause emitted a different production Effect".to_owned());
    }
    if &actual_state != current {
        return Err("a reducer object does not match its production Action".to_owned());
    }
    Ok(())
}

pub(crate) fn parse_library_state(value: &Value) -> Result<LibraryState, String> {
    let state: LibraryState = serde_json::from_value(value.clone())
        .map_err(|error| format!("a reducer object is not a LibraryState: {error}"))?;
    if json_value(&state) != *value {
        return Err("a reducer object is not a canonical LibraryState".to_owned());
    }
    Ok(state)
}

impl CheckpointSink<CheckpointFor<RealFrontend, RealHostBoundary>> for SchemaThreeSink {
    fn record(
        &mut self,
        checkpoint: CheckpointFor<RealFrontend, RealHostBoundary>,
    ) -> Result<(), String> {
        let phase = timeline_phase(checkpoint.phase);
        let (operation_index, event_chain, cause) = transition_cause(phase, &checkpoint.cause)?;
        self.record_checkpoint(ProjectedCheckpoint {
            phase: checkpoint.phase,
            boundary: checkpoint.boundary,
            operation_index,
            event_chain,
            cause,
            frontend: checkpoint.frontend,
            host: checkpoint.host,
            leak_context: None,
        })
    }
}

impl CheckpointSink<CheckpointFor<CorpusFrontend, CorpusHostBoundary>> for SchemaThreeSink {
    fn record(
        &mut self,
        checkpoint: CheckpointFor<CorpusFrontend, CorpusHostBoundary>,
    ) -> Result<(), String> {
        let phase = timeline_phase(checkpoint.phase);
        let (operation_index, event_chain, cause) =
            corpus_transition_cause(phase, &checkpoint.cause)?;
        self.record_checkpoint(ProjectedCheckpoint {
            phase: checkpoint.phase,
            boundary: checkpoint.boundary,
            operation_index,
            event_chain,
            cause,
            frontend: checkpoint.frontend,
            host: checkpoint.host.observation,
            leak_context: checkpoint.host.leak_context,
        })
    }
}

pub(super) fn record_presentation(
    recorder: &mut AsciicastRecorder,
    presentation: Presentation,
    frame: &StyledFrameSnapshot,
) -> Result<(), String> {
    match presentation {
        Presentation::Presented => recorder
            .record_snapshot(FRAME_INTERVAL, frame)
            .map(|_| ())
            .map_err(|error| error.to_string()),
        Presentation::NotPresented => {
            recorder.skip(FRAME_INTERVAL);
            Ok(())
        }
    }
}

const fn timeline_phase(phase: EnginePhase) -> TimelinePhase {
    match phase {
        EnginePhase::Operations => TimelinePhase::Operations,
        EnginePhase::FinalLiveness => TimelinePhase::FinalLiveness,
    }
}

pub(super) const fn timeline_boundary(boundary: EngineBoundary) -> TimelineBoundary {
    match boundary {
        EngineBoundary::Initial => TimelineBoundary::Initial,
        EngineBoundary::Session => TimelineBoundary::Session,
        EngineBoundary::Reducer => TimelineBoundary::UserAction,
        EngineBoundary::Host => TimelineBoundary::HostAction,
    }
}

pub(super) fn smoke_operation_value(operation: SmokeOperation) -> Value {
    match operation {
        SmokeOperation::OpenRun => json!({"operation": "open_run"}),
        SmokeOperation::OpenPreferences => json!({"operation": "open_preferences"}),
        SmokeOperation::OpenLanguagePicker => json!({"operation": "open_language_picker"}),
        SmokeOperation::NextLanguage => json!({"operation": "next_language"}),
        SmokeOperation::ChooseLanguage => json!({"operation": "choose_language"}),
        SmokeOperation::SavePreferences => json!({"operation": "save_preferences"}),
        SmokeOperation::FinalLiveness => json!({"synthetic": "final_liveness"}),
    }
}

pub(super) fn smoke_event_value(event: SmokeEvent) -> Value {
    json!({
        "type": "key",
        "code": json_value(&event.key),
        "modifiers": json_value(&event.modifiers),
    })
}

pub(super) fn smoke_resolution_value(
    phase: TimelinePhase,
    operation: SmokeOperation,
    resolution: SmokeResolution,
) -> Value {
    let event = smoke_event_value(resolution.event);
    match phase {
        TimelinePhase::Operations => json!({
            "input": {"kind": "event", "event": event},
            "semantic_target": match operation {
                SmokeOperation::OpenRun => json!({"command": "run"}),
                SmokeOperation::OpenPreferences => json!({"command": "preferences"}),
                SmokeOperation::OpenLanguagePicker => {
                    json!({"control": "language", "action": "open"})
                }
                SmokeOperation::NextLanguage => {
                    json!({"control": "language", "action": "next"})
                }
                SmokeOperation::ChooseLanguage => {
                    json!({"control": "language", "action": "choose"})
                }
                SmokeOperation::SavePreferences => json!({"command": "save_preferences"}),
                SmokeOperation::FinalLiveness => json!({"synthetic": "final_liveness"}),
            },
        }),
        TimelinePhase::FinalLiveness => json!({"event": event}),
    }
}

pub(super) fn json_value(value: &impl serde::Serialize) -> Value {
    serde_json::to_value(value).expect("walker schema values must serialize")
}

pub(super) fn checked_u32(value: usize, label: &str) -> Result<u32, String> {
    u32::try_from(value).map_err(|_| format!("{label} exceeds u32"))
}

pub(super) fn transition_cause(
    phase: TimelinePhase,
    cause: &EngineCause<SmokeOperation, SmokeResolution, SmokeEvent, Action, Effect, SmokeHandling>,
) -> Result<(Option<u32>, Option<EventChainIdentity>, TransitionCause), String> {
    match cause {
        EngineCause::Initial => Ok((None, None, TransitionCause::Initial)),
        EngineCause::NotApplicable {
            dispatch_sequence: _,
            operation_index: _,
            operation: _,
            resolved: _,
        } => Err("the real walker smoke has no not-applicable operation projection".to_owned()),
        EngineCause::Session {
            dispatch_sequence,
            operation_index,
            operation,
            resolved,
            event,
            handling,
        } => {
            let operation_index = operation_index
                .map(|index| checked_u32(index, "operation index"))
                .transpose()?;
            Ok((
                operation_index,
                Some(EventChainIdentity {
                    phase,
                    sequence: checked_u32(*dispatch_sequence, "dispatch sequence")?,
                }),
                TransitionCause::Session {
                    requested: smoke_operation_value(*operation),
                    resolved: smoke_resolution_value(phase, *operation, *resolved),
                    event: smoke_event_value(*event),
                    handling: json!(match handling {
                        SmokeHandling::Consumed => "consumed",
                        SmokeHandling::Ignored => "ignored",
                    }),
                },
            ))
        }
        EngineCause::Reducer {
            dispatch_sequence,
            operation_index,
            operation,
            resolved,
            event,
            action,
            emitted,
        } => {
            let operation_index = operation_index
                .map(|index| checked_u32(index, "operation index"))
                .transpose()?;
            Ok((
                operation_index,
                Some(EventChainIdentity {
                    phase,
                    sequence: checked_u32(*dispatch_sequence, "dispatch sequence")?,
                }),
                TransitionCause::Reducer {
                    requested: smoke_operation_value(*operation),
                    resolved: smoke_resolution_value(phase, *operation, *resolved),
                    event: smoke_event_value(*event),
                    action: json_value(action),
                    emitted: json_value(emitted),
                },
            ))
        }
        EngineCause::Host {
            dispatch_sequence,
            operation_index,
            round,
            request,
            response,
            emitted,
            ..
        } => {
            let operation_index = operation_index
                .map(|index| checked_u32(index, "operation index"))
                .transpose()?;
            Ok((
                operation_index,
                Some(EventChainIdentity {
                    phase,
                    sequence: checked_u32(*dispatch_sequence, "dispatch sequence")?,
                }),
                TransitionCause::Host {
                    operation_index,
                    round: u16::try_from(*round)
                        .map_err(|_| "host round exceeds u16".to_owned())?,
                    request: json_value(request),
                    response: json_value(response),
                    emitted: json_value(emitted),
                },
            ))
        }
    }
}

fn corpus_transition_cause(
    phase: TimelinePhase,
    cause: &EngineCause<
        CorpusOperation,
        CorpusResolution,
        CorpusEvent,
        Action,
        Effect,
        SmokeHandling,
    >,
) -> Result<(Option<u32>, Option<EventChainIdentity>, TransitionCause), String> {
    match cause {
        EngineCause::Initial => Ok((None, None, TransitionCause::Initial)),
        EngineCause::NotApplicable {
            dispatch_sequence,
            operation_index,
            operation,
            resolved,
        } => {
            let operation_index = operation_index
                .map(|index| checked_u32(index, "operation index"))
                .transpose()?;
            Ok((
                operation_index,
                Some(EventChainIdentity {
                    phase,
                    sequence: checked_u32(*dispatch_sequence, "dispatch sequence")?,
                }),
                TransitionCause::Session {
                    requested: corpus_operation_value(operation)?,
                    resolved: corpus_resolution_value(phase, resolved)?,
                    event: json!({"synthetic": "not_applicable"}),
                    handling: json!("not_applicable"),
                },
            ))
        }
        EngineCause::Session {
            dispatch_sequence,
            operation_index,
            operation,
            resolved,
            event,
            handling,
        } => {
            let operation_index = operation_index
                .map(|index| checked_u32(index, "operation index"))
                .transpose()?;
            Ok((
                operation_index,
                Some(EventChainIdentity {
                    phase,
                    sequence: checked_u32(*dispatch_sequence, "dispatch sequence")?,
                }),
                TransitionCause::Session {
                    requested: corpus_operation_value(operation)?,
                    resolved: corpus_resolution_value(phase, resolved)?,
                    event: event.value(),
                    handling: json!(match handling {
                        SmokeHandling::Consumed => "consumed",
                        SmokeHandling::Ignored => "ignored",
                    }),
                },
            ))
        }
        EngineCause::Reducer {
            dispatch_sequence,
            operation_index,
            operation,
            resolved,
            event,
            action,
            emitted,
        } => {
            let operation_index = operation_index
                .map(|index| checked_u32(index, "operation index"))
                .transpose()?;
            Ok((
                operation_index,
                Some(EventChainIdentity {
                    phase,
                    sequence: checked_u32(*dispatch_sequence, "dispatch sequence")?,
                }),
                TransitionCause::Reducer {
                    requested: corpus_operation_value(operation)?,
                    resolved: corpus_resolution_value(phase, resolved)?,
                    event: event.value(),
                    action: json_value(action),
                    emitted: json_value(emitted),
                },
            ))
        }
        EngineCause::Host {
            dispatch_sequence,
            operation_index,
            round,
            request,
            response,
            emitted,
            ..
        } => {
            let operation_index = operation_index
                .map(|index| checked_u32(index, "operation index"))
                .transpose()?;
            Ok((
                operation_index,
                Some(EventChainIdentity {
                    phase,
                    sequence: checked_u32(*dispatch_sequence, "dispatch sequence")?,
                }),
                TransitionCause::Host {
                    operation_index,
                    round: u16::try_from(*round)
                        .map_err(|_| "host round exceeds u16".to_owned())?,
                    request: json_value(request),
                    response: json_value(response),
                    emitted: json_value(emitted),
                },
            ))
        }
    }
}

pub(crate) fn validate_effect_chain_termination(rows: &[TimelineRow]) -> Result<(), String> {
    let mut pending = false;
    let mut previous_emitted = None;
    for row in rows {
        match &row.cause {
            TransitionCause::Initial | TransitionCause::Session { .. } => {
                if pending {
                    return Err("a host effect was left pending before another event".to_owned());
                }
                previous_emitted = None;
            }
            TransitionCause::Reducer { emitted, .. } => {
                pending = emitted != "none";
                previous_emitted = Some(emitted);
            }
            TransitionCause::Host {
                request, emitted, ..
            } => {
                if !pending {
                    return Err("a host transition has no pending reducer effect".to_owned());
                }
                if previous_emitted != Some(request) {
                    return Err("a host request does not equal its predecessor effect".to_owned());
                }
                pending = emitted != "none";
                previous_emitted = Some(emitted);
            }
        }
    }
    if pending {
        return Err("the timeline ends with a pending host effect".to_owned());
    }
    Ok(())
}
