//! Generic deterministic walker orchestration.
//!
//! Product crates implement the adapters. This crate owns only transition ordering, effect
//! draining, checkpoint delivery, pure parity snapshots, and fresh prefix replay.

/// How the engine routes one reducer effect.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EffectRoute {
    /// The reducer and host cycle is stable.
    Settled,
    /// The frontend has requested termination.
    Quit,
    /// The effect must cross the host boundary.
    Host,
}

/// One late-bound operation resolution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OperationResolution<E, R> {
    /// The operation has no applicable event in the current frontend state.
    NotApplicable {
        /// Complete adapter-owned resolution evidence.
        resolved: R,
    },
    /// Dispatch the ordered event sequence against one live frontend.
    Events {
        /// Complete adapter-owned resolution evidence.
        resolved: R,
        /// Exact events in dispatch order.
        events: Vec<E>,
    },
}

/// The frontend result of one terminal event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DispatchOutcome<A, H> {
    /// The event produced one reducer action.
    Action(A),
    /// The event changed or inspected frontend-local state only.
    Session(H),
}

/// The transition boundary represented by one engine checkpoint.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EngineBoundary {
    /// The initial state before an operation.
    Initial,
    /// A frontend-local or inapplicable operation step.
    Session,
    /// A reducer action before host service.
    Reducer,
    /// A host response after reducer application.
    Host,
}

/// The independent walker phase that owns one checkpoint.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EnginePhase {
    /// The main semantic-operation walk.
    Operations,
    /// The final real-input liveness check.
    FinalLiveness,
}

/// The complete typed cause of one engine checkpoint.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EngineCause<O, R, E, A, F, H> {
    /// The initial checkpoint has no incoming operation.
    Initial,
    /// A late-bound operation did not apply.
    NotApplicable {
        /// Zero-based dispatched-event ordinal.
        dispatch_sequence: usize,
        /// Zero-based successful-operation index.
        operation_index: Option<usize>,
        /// Source semantic operation.
        operation: O,
        /// Complete adapter-owned resolution evidence.
        resolved: R,
    },
    /// One event produced only frontend-local handling.
    Session {
        /// Zero-based dispatched-event ordinal.
        dispatch_sequence: usize,
        /// Zero-based successful-operation index.
        operation_index: Option<usize>,
        /// Source semantic operation.
        operation: O,
        /// Complete adapter-owned resolution evidence.
        resolved: R,
        /// Exact dispatched event.
        event: E,
        /// Frontend-local handling result.
        handling: H,
    },
    /// One event produced a reducer action and effect.
    Reducer {
        /// Zero-based dispatched-event ordinal.
        dispatch_sequence: usize,
        /// Zero-based successful-operation index.
        operation_index: Option<usize>,
        /// Source semantic operation.
        operation: O,
        /// Complete adapter-owned resolution evidence.
        resolved: R,
        /// Exact dispatched event.
        event: E,
        /// Reducer action.
        action: A,
        /// Effect emitted by the reducer.
        emitted: F,
    },
    /// One host request produced a response and next effect.
    Host {
        /// Dispatched-event ordinal that owns this host chain.
        dispatch_sequence: usize,
        /// Zero-based successful-operation index.
        operation_index: Option<usize>,
        /// Source semantic operation.
        operation: O,
        /// Host round within the event chain.
        round: usize,
        /// Effect sent to the host.
        request: F,
        /// Action returned by the host.
        response: A,
        /// Next effect emitted by the reducer.
        emitted: F,
    },
}

/// Recorded cause fields that a host can project at one checkpoint boundary.
#[derive(Debug)]
pub enum CheckpointCauseProjection<'a, A, E> {
    /// The checkpoint has no reducer or host cause fields.
    Observation,
    /// Project one recorded reducer action and its recorded effect.
    Reducer {
        /// Clone of the action that the reducer consumed.
        action: &'a mut A,
        /// Clone of the effect that the reducer emitted.
        emitted: &'a mut E,
    },
    /// Project one recorded host request, response, and next effect.
    Host {
        /// Clone of the effect that the host consumed.
        request: &'a mut E,
        /// Clone of the action that the reducer consumed.
        response: &'a mut A,
        /// Clone of the next effect that the reducer emitted.
        emitted: &'a mut E,
    },
}

/// Frontend and host observations returned by one atomic host capture.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CheckpointCapture<FO, HO> {
    /// Detached frontend observation after host-owned projection.
    pub frontend: FO,
    /// Complete deterministic host observation from the same capture.
    pub host: HO,
}

/// One complete generic engine checkpoint.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EngineCheckpoint<O, R, E, A, F, H, FO, HO> {
    /// Walker phase that owns this checkpoint.
    pub phase: EnginePhase,
    /// Transition boundary.
    pub boundary: EngineBoundary,
    /// Complete transition cause.
    pub cause: EngineCause<O, R, E, A, F, H>,
    /// Frontend-owned observation for invariants, inventory, frames, and geometry.
    pub frontend: FO,
    /// Host-owned deterministic observation.
    pub host: HO,
}

/// Adapter over one concrete reducer and frontend session.
pub trait FrontendAdapter {
    /// Semantic operation generated by a strategy.
    type Operation: Clone;
    /// Adapter-owned resolution evidence.
    type Resolution: Clone;
    /// Exact input event.
    type Event: Clone;
    /// Reducer action.
    type Action: Clone;
    /// Reducer effect.
    type Effect: Clone;
    /// Frontend-local event result.
    type Handling: Clone;
    /// Complete checkpoint observation.
    type Observation;
    /// Pure reducer/session snapshot used by parity probes without a host.
    type ParitySnapshot;

    /// Resolve one operation against the current live frontend.
    fn resolve(
        &self,
        operation: &Self::Operation,
    ) -> Result<OperationResolution<Self::Event, Self::Resolution>, String>;
    /// Dispatch one exact event.
    fn dispatch(
        &mut self,
        event: Self::Event,
    ) -> Result<DispatchOutcome<Self::Action, Self::Handling>, String>;
    /// Apply one reducer action.
    fn reduce(&mut self, action: Self::Action) -> Result<Self::Effect, String>;
    /// Classify one reducer effect.
    fn effect_route(&self, effect: &Self::Effect) -> EffectRoute;
    /// Capture the complete frontend observation and run its invariants.
    fn observe(&mut self) -> Result<Self::Observation, String>;
    /// Snapshot only pure reducer and frontend-session state for parity work.
    fn parity_snapshot(&self) -> Result<Self::ParitySnapshot, String>;
}

/// Host boundary paired with one frontend adapter.
pub trait HostAdapter<F: FrontendAdapter> {
    /// Complete deterministic host observation.
    type Observation;
    /// Serve one host effect.
    fn serve(&mut self, effect: F::Effect) -> Result<F::Action, String>;
    /// Project recorded clones and capture one complete checkpoint atomically.
    fn capture_checkpoint(
        &mut self,
        frontend: F::Observation,
        cause: CheckpointCauseProjection<'_, F::Action, F::Effect>,
    ) -> Result<CheckpointCapture<F::Observation, Self::Observation>, String>;
}

/// Sink for checkpoint artifacts, casts, and trace summaries.
pub trait CheckpointSink<C> {
    /// Record one complete checkpoint.
    ///
    /// On error, the sink must not retain an externally visible checkpoint. The engine does not
    /// roll back arbitrary state inside a sink.
    fn record(&mut self, checkpoint: C) -> Result<(), String>;
}

/// A checkpoint sink used for counterfactual replay.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoopCheckpointSink;

impl<C> CheckpointSink<C> for NoopCheckpointSink {
    fn record(&mut self, _checkpoint: C) -> Result<(), String> {
        Ok(())
    }
}

/// Concrete checkpoint type for one frontend and host pair.
pub type CheckpointFor<F, H> = EngineCheckpoint<
    <F as FrontendAdapter>::Operation,
    <F as FrontendAdapter>::Resolution,
    <F as FrontendAdapter>::Event,
    <F as FrontendAdapter>::Action,
    <F as FrontendAdapter>::Effect,
    <F as FrontendAdapter>::Handling,
    <F as FrontendAdapter>::Observation,
    <H as HostAdapter<F>>::Observation,
>;

type CauseFor<F> = EngineCause<
    <F as FrontendAdapter>::Operation,
    <F as FrontendAdapter>::Resolution,
    <F as FrontendAdapter>::Event,
    <F as FrontendAdapter>::Action,
    <F as FrontendAdapter>::Effect,
    <F as FrontendAdapter>::Handling,
>;

/// Generic deterministic main-walk engine.
#[derive(Debug)]
pub struct WalkerEngine<F: FrontendAdapter, H: HostAdapter<F>> {
    frontend: F,
    host: H,
    effect_limit: usize,
    started: bool,
    quit: bool,
    poisoned: bool,
    final_liveness_started: bool,
    next_operation_dispatch_sequence: usize,
    next_liveness_dispatch_sequence: usize,
    successful_operations: Vec<F::Operation>,
}

impl<F: FrontendAdapter, H: HostAdapter<F>> WalkerEngine<F, H> {
    /// Construct an unstarted engine with a bounded host-effect chain.
    pub fn new(frontend: F, host: H, effect_limit: usize) -> Result<Self, String> {
        if effect_limit == 0 {
            return Err("the walker host effect limit must be positive".to_owned());
        }
        Ok(Self {
            frontend,
            host,
            effect_limit,
            started: false,
            quit: false,
            poisoned: false,
            final_liveness_started: false,
            next_operation_dispatch_sequence: 0,
            next_liveness_dispatch_sequence: 0,
            successful_operations: Vec::new(),
        })
    }

    /// Record the initial checkpoint exactly once.
    pub fn start<S>(&mut self, sink: &mut S) -> Result<(), String>
    where
        S: CheckpointSink<CheckpointFor<F, H>>,
    {
        self.ensure_healthy()?;
        let result = self.start_inner(sink);
        if result.is_err() {
            self.poisoned = true;
        }
        result
    }

    fn start_inner<S>(&mut self, sink: &mut S) -> Result<(), String>
    where
        S: CheckpointSink<CheckpointFor<F, H>>,
    {
        if self.started {
            return Err("the walker engine was started more than once".to_owned());
        }
        self.started = true;
        self.record(
            EnginePhase::Operations,
            EngineBoundary::Initial,
            EngineCause::Initial,
            sink,
        )
    }

    /// Resolve and run one semantic operation against the live frontend.
    pub fn run_operation<S>(&mut self, operation: F::Operation, sink: &mut S) -> Result<(), String>
    where
        S: CheckpointSink<CheckpointFor<F, H>>,
    {
        self.ensure_healthy()?;
        let result = self.run_operation_inner(operation, sink);
        if result.is_err() {
            self.poisoned = true;
        }
        result
    }

    fn run_operation_inner<S>(
        &mut self,
        operation: F::Operation,
        sink: &mut S,
    ) -> Result<(), String>
    where
        S: CheckpointSink<CheckpointFor<F, H>>,
    {
        if !self.started {
            return Err("the walker engine must start before it runs an operation".to_owned());
        }
        if self.quit {
            return Err("the walker engine cannot run an operation after quit".to_owned());
        }
        if self.final_liveness_started {
            return Err(
                "the walker engine cannot run an operation after final liveness".to_owned(),
            );
        }
        let operation_index = self.successful_operations.len();
        self.run_resolved_operation(
            EnginePhase::Operations,
            Some(operation_index),
            &operation,
            sink,
        )?;
        self.successful_operations.push(operation);
        Ok(())
    }

    /// Run one real final-liveness operation without changing the successful prefix.
    pub fn run_final_liveness<S>(
        &mut self,
        operation: F::Operation,
        sink: &mut S,
    ) -> Result<(), String>
    where
        S: CheckpointSink<CheckpointFor<F, H>>,
    {
        self.ensure_healthy()?;
        let result = self.run_final_liveness_inner(operation, sink);
        if result.is_err() {
            self.poisoned = true;
        }
        result
    }

    fn run_final_liveness_inner<S>(
        &mut self,
        operation: F::Operation,
        sink: &mut S,
    ) -> Result<(), String>
    where
        S: CheckpointSink<CheckpointFor<F, H>>,
    {
        if !self.started {
            return Err("the walker engine must start before final liveness".to_owned());
        }
        if self.quit {
            return Err("the walker engine cannot run final liveness after quit".to_owned());
        }
        if self.final_liveness_started {
            return Err("the walker final liveness phase ran more than once".to_owned());
        }
        self.final_liveness_started = true;
        self.run_resolved_operation(EnginePhase::FinalLiveness, None, &operation, sink)
    }

    fn run_resolved_operation<S>(
        &mut self,
        phase: EnginePhase,
        operation_index: Option<usize>,
        operation: &F::Operation,
        sink: &mut S,
    ) -> Result<(), String>
    where
        S: CheckpointSink<CheckpointFor<F, H>>,
    {
        match self.frontend.resolve(operation)? {
            OperationResolution::NotApplicable { resolved } => {
                let dispatch_sequence = self.begin_dispatch(phase);
                self.record(
                    phase,
                    EngineBoundary::Session,
                    EngineCause::NotApplicable {
                        dispatch_sequence,
                        operation_index,
                        operation: operation.clone(),
                        resolved,
                    },
                    sink,
                )?;
            }
            OperationResolution::Events { resolved, events } => {
                if events.is_empty() {
                    return Err(
                        "an applicable walker operation resolved to an empty event list".to_owned(),
                    );
                }
                for event in events {
                    let dispatch_sequence = self.begin_dispatch(phase);
                    let outcome = self.frontend.dispatch(event.clone())?;
                    match outcome {
                        DispatchOutcome::Session(handling) => {
                            self.record(
                                phase,
                                EngineBoundary::Session,
                                EngineCause::Session {
                                    dispatch_sequence,
                                    operation_index,
                                    operation: operation.clone(),
                                    resolved: resolved.clone(),
                                    event,
                                    handling,
                                },
                                sink,
                            )?;
                        }
                        DispatchOutcome::Action(action) => {
                            let recorded_action = action.clone();
                            let effect = self.frontend.reduce(action)?;
                            self.record(
                                phase,
                                EngineBoundary::Reducer,
                                EngineCause::Reducer {
                                    dispatch_sequence,
                                    operation_index,
                                    operation: operation.clone(),
                                    resolved: resolved.clone(),
                                    event,
                                    action: recorded_action,
                                    emitted: effect.clone(),
                                },
                                sink,
                            )?;
                            self.drain_host_effects(
                                phase,
                                dispatch_sequence,
                                operation_index,
                                operation,
                                effect,
                                sink,
                            )?;
                        }
                    }
                    if self.quit {
                        break;
                    }
                }
            }
        }
        Ok(())
    }

    fn drain_host_effects<S>(
        &mut self,
        phase: EnginePhase,
        dispatch_sequence: usize,
        operation_index: Option<usize>,
        operation: &F::Operation,
        mut effect: F::Effect,
        sink: &mut S,
    ) -> Result<(), String>
    where
        S: CheckpointSink<CheckpointFor<F, H>>,
    {
        for round in 0..self.effect_limit {
            match self.frontend.effect_route(&effect) {
                EffectRoute::Settled => return Ok(()),
                EffectRoute::Quit => {
                    self.quit = true;
                    return Ok(());
                }
                EffectRoute::Host => {
                    let request = effect.clone();
                    let response = self.host.serve(effect)?;
                    let recorded_response = response.clone();
                    effect = self.frontend.reduce(response)?;
                    self.record(
                        phase,
                        EngineBoundary::Host,
                        EngineCause::Host {
                            dispatch_sequence,
                            operation_index,
                            operation: operation.clone(),
                            round,
                            request,
                            response: recorded_response,
                            emitted: effect.clone(),
                        },
                        sink,
                    )?;
                }
            }
        }
        match self.frontend.effect_route(&effect) {
            EffectRoute::Settled => Ok(()),
            EffectRoute::Quit => {
                self.quit = true;
                Ok(())
            }
            EffectRoute::Host => Err(format!(
                "the walker host effect limit of {} was exceeded",
                self.effect_limit
            )),
        }
    }

    fn begin_dispatch(&mut self, phase: EnginePhase) -> usize {
        let next = match phase {
            EnginePhase::Operations => &mut self.next_operation_dispatch_sequence,
            EnginePhase::FinalLiveness => &mut self.next_liveness_dispatch_sequence,
        };
        let sequence = *next;
        *next = next.saturating_add(1);
        sequence
    }

    fn ensure_healthy(&self) -> Result<(), String> {
        if self.poisoned {
            return Err("the walker engine is poisoned after an earlier failure".to_owned());
        }
        Ok(())
    }

    fn record<S>(
        &mut self,
        phase: EnginePhase,
        boundary: EngineBoundary,
        mut cause: CauseFor<F>,
        sink: &mut S,
    ) -> Result<(), String>
    where
        S: CheckpointSink<CheckpointFor<F, H>>,
    {
        let frontend = self.frontend.observe()?;
        let projection = match &mut cause {
            EngineCause::Initial
            | EngineCause::NotApplicable { .. }
            | EngineCause::Session { .. } => CheckpointCauseProjection::Observation,
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
            self.host.capture_checkpoint(frontend, projection)?;
        sink.record(EngineCheckpoint {
            phase,
            boundary,
            cause,
            frontend,
            host,
        })
    }

    /// Snapshot pure reducer and frontend-session state for parity probes.
    pub fn parity_snapshot(&mut self) -> Result<F::ParitySnapshot, String> {
        self.ensure_healthy()?;
        let result = self.frontend.parity_snapshot();
        if result.is_err() {
            self.poisoned = true;
        }
        result
    }

    /// Consume the engine and return its live frontend and host.
    pub fn into_parts(self) -> (F, H) {
        (self.frontend, self.host)
    }

    /// Return the host for read-only assertions.
    pub const fn host(&self) -> &H {
        &self.host
    }

    /// Return the frontend for read-only assertions.
    pub const fn frontend(&self) -> &F {
        &self.frontend
    }

    /// Return whether a reducer effect requested termination.
    pub const fn is_quit(&self) -> bool {
        self.quit
    }

    /// Return whether a prior failure made the trace terminal.
    pub const fn is_poisoned(&self) -> bool {
        self.poisoned
    }

    /// Return the successful semantic-operation prefix.
    pub fn successful_operations(&self) -> &[F::Operation] {
        &self.successful_operations
    }
}

/// Factory that creates a fresh frontend and non-cloned host for replay.
pub trait ReplayFactory {
    /// Fresh frontend adapter.
    type Frontend: FrontendAdapter;
    /// Fresh host adapter. It does not need to implement `Clone`.
    type Host: HostAdapter<Self::Frontend>;

    /// Create a fresh seeded frontend and host pair.
    fn create(&self) -> Result<(Self::Frontend, Self::Host), String>;

    /// Maximum host rounds for one replayed reducer effect.
    fn effect_limit(&self) -> usize {
        64
    }
}

/// Replay a successful operation prefix on a fresh host without producing main-trace artifacts.
pub fn replay_prefix<R>(
    factory: &R,
    operations: &[<R::Frontend as FrontendAdapter>::Operation],
) -> Result<WalkerEngine<R::Frontend, R::Host>, String>
where
    R: ReplayFactory,
{
    let mut sink = NoopCheckpointSink;
    replay_prefix_with_sink(factory, operations, &mut sink)
}

/// Replay a successful operation prefix on a fresh host and record every replay checkpoint.
pub fn replay_prefix_with_sink<R, S>(
    factory: &R,
    operations: &[<R::Frontend as FrontendAdapter>::Operation],
    sink: &mut S,
) -> Result<WalkerEngine<R::Frontend, R::Host>, String>
where
    R: ReplayFactory,
    S: CheckpointSink<CheckpointFor<R::Frontend, R::Host>>,
{
    let (frontend, host) = factory.create()?;
    let mut replay = WalkerEngine::new(frontend, host, factory.effect_limit())?;
    replay.start(sink)?;
    for operation in operations {
        replay.run_operation(operation.clone(), sink)?;
    }
    Ok(replay)
}

/// Run one host-dependent check against a fresh prefix replay.
pub fn with_fresh_replay<R, T>(
    factory: &R,
    operations: &[<R::Frontend as FrontendAdapter>::Operation],
    check: impl FnOnce(&mut WalkerEngine<R::Frontend, R::Host>) -> Result<T, String>,
) -> Result<T, String>
where
    R: ReplayFactory,
{
    let mut replay = replay_prefix(factory, operations)?;
    check(&mut replay)
}
