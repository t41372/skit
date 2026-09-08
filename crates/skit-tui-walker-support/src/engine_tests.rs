use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use crate::engine::{
    CheckpointCapture, CheckpointCauseProjection, CheckpointSink, DispatchOutcome, EffectRoute,
    EngineBoundary, EngineCause, EnginePhase, FrontendAdapter, HostAdapter, OperationResolution,
    ReplayFactory, WalkerEngine, replay_prefix, replay_prefix_with_sink, with_fresh_replay,
};

#[derive(Clone, Debug, Eq, PartialEq)]
enum Operation {
    Noop,
    Empty,
    ResolveError,
    Local,
    DispatchError,
    ReduceError,
    Quit,
    Hosted,
    HostQuit,
    MultiHost,
    HostError,
    HostReduceError,
    Cycle,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Event {
    Press,
    Local,
    DispatchError,
    ReduceError,
    Quit,
    Hosted,
    HostQuit,
    MultiHost,
    HostError,
    HostReduceError,
    Cycle,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Action {
    Local,
    ReduceError,
    Quit,
    Request(&'static str),
    Complete,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Effect {
    None,
    Quit,
    Host(&'static str),
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FrontendObservation {
    reducer: usize,
    session: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ParitySnapshot {
    reducer: usize,
    session: usize,
}

struct DropProbe {
    label: &'static str,
    log: Rc<RefCell<Vec<&'static str>>>,
}

impl Drop for DropProbe {
    fn drop(&mut self) {
        self.log.borrow_mut().push(self.label);
    }
}

struct Frontend {
    reducer: usize,
    session: usize,
    observation_calls: usize,
    fail_observation_at: Option<usize>,
    parity_error: bool,
    drop_probe: Option<DropProbe>,
}

impl FrontendAdapter for Frontend {
    type Operation = Operation;
    type Resolution = &'static str;
    type Event = Event;
    type Action = Action;
    type Effect = Effect;
    type Handling = &'static str;
    type Observation = FrontendObservation;
    type ParitySnapshot = ParitySnapshot;

    fn resolve(
        &self,
        operation: &Self::Operation,
    ) -> Result<OperationResolution<Self::Event, Self::Resolution>, String> {
        Ok(match operation {
            Operation::Noop => OperationResolution::NotApplicable {
                resolved: "not_applicable",
            },
            Operation::Empty => OperationResolution::Events {
                resolved: "empty",
                events: Vec::new(),
            },
            Operation::ResolveError => return Err("resolve failed".to_owned()),
            Operation::Local => OperationResolution::Events {
                resolved: "local",
                events: vec![Event::Local],
            },
            Operation::DispatchError => OperationResolution::Events {
                resolved: "dispatch_error",
                events: vec![Event::DispatchError],
            },
            Operation::ReduceError => OperationResolution::Events {
                resolved: "reduce_error",
                events: vec![Event::ReduceError],
            },
            Operation::Quit => OperationResolution::Events {
                resolved: "quit",
                events: vec![Event::Quit],
            },
            Operation::Hosted => OperationResolution::Events {
                resolved: "hosted",
                events: vec![Event::Press, Event::Hosted],
            },
            Operation::HostQuit => OperationResolution::Events {
                resolved: "host_quit",
                events: vec![Event::HostQuit],
            },
            Operation::MultiHost => OperationResolution::Events {
                resolved: "multi_host",
                events: vec![Event::MultiHost],
            },
            Operation::HostError => OperationResolution::Events {
                resolved: "host_error",
                events: vec![Event::HostError],
            },
            Operation::HostReduceError => OperationResolution::Events {
                resolved: "host_reduce_error",
                events: vec![Event::HostReduceError],
            },
            Operation::Cycle => OperationResolution::Events {
                resolved: "cycle",
                events: vec![Event::Cycle],
            },
        })
    }

    fn dispatch(
        &mut self,
        event: Self::Event,
    ) -> Result<DispatchOutcome<Self::Action, Self::Handling>, String> {
        Ok(match event {
            Event::Press => {
                self.session += 1;
                DispatchOutcome::Session("consumed")
            }
            Event::Local => DispatchOutcome::Action(Action::Local),
            Event::DispatchError => return Err("dispatch failed".to_owned()),
            Event::ReduceError => DispatchOutcome::Action(Action::ReduceError),
            Event::Quit => DispatchOutcome::Action(Action::Quit),
            Event::Hosted => DispatchOutcome::Action(Action::Request("complete")),
            Event::HostQuit => DispatchOutcome::Action(Action::Request("quit")),
            Event::MultiHost => DispatchOutcome::Action(Action::Request("multi-a")),
            Event::HostError => DispatchOutcome::Action(Action::Request("error")),
            Event::HostReduceError => DispatchOutcome::Action(Action::Request("reduce-error")),
            Event::Cycle => DispatchOutcome::Action(Action::Request("cycle")),
        })
    }

    fn reduce(&mut self, action: Self::Action) -> Result<Self::Effect, String> {
        self.reducer += 1;
        Ok(match action {
            Action::Local | Action::Complete => Effect::None,
            Action::ReduceError => return Err("reduce failed".to_owned()),
            Action::Quit => Effect::Quit,
            Action::Request(request) => Effect::Host(request),
        })
    }

    fn effect_route(&self, effect: &Self::Effect) -> EffectRoute {
        match effect {
            Effect::None => EffectRoute::Settled,
            Effect::Quit => EffectRoute::Quit,
            Effect::Host(_) => EffectRoute::Host,
        }
    }

    fn observe(&mut self) -> Result<Self::Observation, String> {
        let call = self.observation_calls;
        self.observation_calls += 1;
        if self.fail_observation_at == Some(call) {
            return Err("observation failed".to_owned());
        }
        Ok(FrontendObservation {
            reducer: self.reducer,
            session: self.session,
        })
    }

    fn parity_snapshot(&self) -> Result<Self::ParitySnapshot, String> {
        if self.parity_error {
            return Err("parity snapshot failed".to_owned());
        }
        Ok(ParitySnapshot {
            reducer: self.reducer,
            session: self.session,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct HostObservation {
    generation: usize,
    served: usize,
    frontend_reducer: usize,
    frontend_session: usize,
}

struct NonCloneHost {
    generation: usize,
    served: usize,
    observation_calls: Cell<usize>,
    fail_observation_at: Option<usize>,
    mutate_checkpoint_projection: bool,
    capture_causes: Vec<CaptureCause>,
    drop_probe: Option<DropProbe>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CaptureCause {
    Observation,
    Reducer,
    Host,
}

impl HostAdapter<Frontend> for NonCloneHost {
    type Observation = HostObservation;

    fn serve(&mut self, effect: Effect) -> Result<Action, String> {
        self.served += 1;
        match effect {
            Effect::Host("complete") => Ok(Action::Complete),
            Effect::Host("quit") => Ok(Action::Quit),
            Effect::Host("multi-a") => Ok(Action::Request("multi-b")),
            Effect::Host("multi-b") => Ok(Action::Complete),
            Effect::Host("error") => Err("host failed".to_owned()),
            Effect::Host("reduce-error") => Ok(Action::ReduceError),
            Effect::Host("cycle") => Ok(Action::Request("cycle")),
            Effect::Host(other) => Err(format!("unexpected host request: {other}")),
            Effect::None | Effect::Quit => Err("local effect reached the host".to_owned()),
        }
    }

    fn capture_checkpoint(
        &mut self,
        frontend: FrontendObservation,
        cause: CheckpointCauseProjection<'_, Action, Effect>,
    ) -> Result<CheckpointCapture<FrontendObservation, Self::Observation>, String> {
        let call = self.observation_calls.get();
        self.observation_calls.set(call + 1);
        if self.fail_observation_at == Some(call) {
            return Err("host observation failed".to_owned());
        }
        match cause {
            CheckpointCauseProjection::Observation => {
                self.capture_causes.push(CaptureCause::Observation);
            }
            CheckpointCauseProjection::Reducer { action, emitted } => {
                self.capture_causes.push(CaptureCause::Reducer);
                if self.mutate_checkpoint_projection {
                    *action = Action::Local;
                    *emitted = Effect::Host("projected-reducer-emitted");
                }
            }
            CheckpointCauseProjection::Host {
                request,
                response,
                emitted,
            } => {
                self.capture_causes.push(CaptureCause::Host);
                if self.mutate_checkpoint_projection {
                    *request = Effect::Host("projected-host-request");
                    *response = Action::Local;
                    *emitted = Effect::Host("projected-host-emitted");
                }
            }
        }
        Ok(CheckpointCapture {
            host: HostObservation {
                generation: self.generation,
                served: self.served,
                frontend_reducer: frontend.reducer,
                frontend_session: frontend.session,
            },
            frontend,
        })
    }
}

struct Factory {
    generations: Rc<Cell<usize>>,
    fail_create: bool,
    fail_observation_at: Option<usize>,
    fail_host_observation_at: Option<usize>,
    mutate_checkpoint_projection: bool,
    parity_error: bool,
    effect_limit: usize,
    drop_log: Option<Rc<RefCell<Vec<&'static str>>>>,
}

impl Default for Factory {
    fn default() -> Self {
        Self {
            generations: Rc::default(),
            fail_create: false,
            fail_observation_at: None,
            fail_host_observation_at: None,
            mutate_checkpoint_projection: false,
            parity_error: false,
            effect_limit: 8,
            drop_log: None,
        }
    }
}

impl ReplayFactory for Factory {
    type Frontend = Frontend;
    type Host = NonCloneHost;

    fn create(&self) -> Result<(Self::Frontend, Self::Host), String> {
        if self.fail_create {
            return Err("factory failed".to_owned());
        }
        let generation = self.generations.get();
        self.generations.set(generation + 1);
        Ok((
            Frontend {
                reducer: 0,
                session: 0,
                observation_calls: 0,
                fail_observation_at: self.fail_observation_at,
                parity_error: self.parity_error,
                drop_probe: self.drop_log.as_ref().map(|log| DropProbe {
                    label: "frontend",
                    log: Rc::clone(log),
                }),
            },
            NonCloneHost {
                generation,
                served: 0,
                observation_calls: Cell::new(0),
                fail_observation_at: self.fail_host_observation_at,
                mutate_checkpoint_projection: self.mutate_checkpoint_projection,
                capture_causes: Vec::new(),
                drop_probe: self.drop_log.as_ref().map(|log| DropProbe {
                    label: "host",
                    log: Rc::clone(log),
                }),
            },
        ))
    }

    fn effect_limit(&self) -> usize {
        self.effect_limit
    }
}

struct DefaultLimitFactory;

impl ReplayFactory for DefaultLimitFactory {
    type Frontend = Frontend;
    type Host = NonCloneHost;

    fn create(&self) -> Result<(Self::Frontend, Self::Host), String> {
        Factory::default().create()
    }
}

struct Recorder<C> {
    checkpoints: Vec<C>,
    fail_at: Option<usize>,
}

impl<C> Default for Recorder<C> {
    fn default() -> Self {
        Self {
            checkpoints: Vec::new(),
            fail_at: None,
        }
    }
}

impl<C> CheckpointSink<C> for Recorder<C> {
    fn record(&mut self, checkpoint: C) -> Result<(), String> {
        if self.fail_at == Some(self.checkpoints.len()) {
            return Err("sink failed".to_owned());
        }
        self.checkpoints.push(checkpoint);
        Ok(())
    }
}

fn engine(factory: &Factory, effect_limit: usize) -> WalkerEngine<Frontend, NonCloneHost> {
    let (frontend, host) = factory.create().unwrap();
    WalkerEngine::new(frontend, host, effect_limit).unwrap()
}

fn assert_terminal_poison(
    engine: &mut WalkerEngine<Frontend, NonCloneHost>,
    trace: &mut Recorder<crate::engine::CheckpointFor<Frontend, NonCloneHost>>,
) {
    assert!(engine.is_poisoned());
    assert!(
        engine
            .run_operation(Operation::Local, trace)
            .unwrap_err()
            .contains("poisoned")
    );
    assert!(engine.parity_snapshot().unwrap_err().contains("poisoned"));
}

#[test]
fn into_parts_returns_fresh_adapter_owners_without_adapter_calls() {
    let drop_log = Rc::new(RefCell::new(Vec::new()));
    let factory = Factory {
        drop_log: Some(Rc::clone(&drop_log)),
        ..Factory::default()
    };
    let engine = engine(&factory, 8);

    let (frontend, host) = engine.into_parts();

    assert_eq!(frontend.reducer, 0);
    assert_eq!(frontend.session, 0);
    assert_eq!(frontend.observation_calls, 0);
    assert_eq!(host.generation, 0);
    assert_eq!(host.served, 0);
    assert_eq!(host.observation_calls.get(), 0);
    let frontend_probe = frontend.drop_probe.as_ref().unwrap();
    assert_eq!(frontend_probe.label, "frontend");
    assert!(Rc::ptr_eq(&frontend_probe.log, &drop_log));
    let host_probe = host.drop_probe.as_ref().unwrap();
    assert_eq!(host_probe.label, "host");
    assert!(Rc::ptr_eq(&host_probe.log, &drop_log));
    assert!(drop_log.borrow().is_empty());

    drop(host);
    assert_eq!(&*drop_log.borrow(), &["host"]);
    drop(frontend);
    assert_eq!(&*drop_log.borrow(), &["host", "frontend"]);
}

#[test]
fn into_parts_preserves_a_healthy_complete_engine_and_its_prefix_state() {
    let factory = Factory::default();
    let mut engine = engine(&factory, 8);
    let mut trace = Recorder::default();
    engine.start(&mut trace).unwrap();
    engine.run_operation(Operation::Hosted, &mut trace).unwrap();
    engine
        .run_final_liveness(Operation::Local, &mut trace)
        .unwrap();
    let successful_prefix = engine.successful_operations().to_vec();
    let frontend_observation_calls = engine.frontend().observation_calls;
    let host_observation_calls = engine.host().observation_calls.get();
    let checkpoint_count = trace.checkpoints.len();

    let (frontend, host) = engine.into_parts();

    assert_eq!(successful_prefix, [Operation::Hosted]);
    assert_eq!(frontend.reducer, 3);
    assert_eq!(frontend.session, 1);
    assert_eq!(frontend.observation_calls, frontend_observation_calls);
    assert_eq!(host.generation, 0);
    assert_eq!(host.served, 1);
    assert_eq!(host.observation_calls.get(), host_observation_calls);
    assert_eq!(trace.checkpoints.len(), checkpoint_count);
}

#[test]
fn into_parts_preserves_poisoned_adapter_state_after_the_primary_error() {
    let factory = Factory::default();
    let mut engine = engine(&factory, 8);
    let mut trace = Recorder::default();
    engine.start(&mut trace).unwrap();
    let error = engine
        .run_operation(Operation::HostError, &mut trace)
        .unwrap_err();
    assert_eq!(error, "host failed");
    assert!(engine.is_poisoned());
    assert!(engine.successful_operations().is_empty());
    let frontend_observation_calls = engine.frontend().observation_calls;
    let host_observation_calls = engine.host().observation_calls.get();
    let checkpoint_count = trace.checkpoints.len();

    let (frontend, host) = engine.into_parts();

    assert_eq!(frontend.reducer, 1);
    assert_eq!(frontend.session, 0);
    assert_eq!(frontend.observation_calls, frontend_observation_calls);
    assert_eq!(host.generation, 0);
    assert_eq!(host.served, 1);
    assert_eq!(host.observation_calls.get(), host_observation_calls);
    assert_eq!(trace.checkpoints.len(), checkpoint_count);
}

#[test]
fn constructor_and_start_state_reject_invalid_reuse() {
    let factory = Factory::default();
    let (frontend, host) = factory.create().unwrap();
    let error = WalkerEngine::new(frontend, host, 0).err().unwrap();
    assert!(error.contains("positive"));

    let mut before_start = engine(&factory, 8);
    let mut trace = Recorder::default();
    assert!(
        before_start
            .run_operation(Operation::Local, &mut trace)
            .unwrap_err()
            .contains("must start")
    );
    assert!(
        before_start
            .start(&mut trace)
            .unwrap_err()
            .contains("poisoned")
    );

    let mut double_start = engine(&factory, 8);
    let mut trace = Recorder::default();
    double_start.start(&mut trace).unwrap();
    assert!(
        double_start
            .start(&mut trace)
            .unwrap_err()
            .contains("more than once")
    );
    assert_terminal_poison(&mut double_start, &mut trace);
}

#[test]
fn every_operation_pipeline_error_poisons_the_mutated_engine() {
    for (operation, limit, expected) in [
        (Operation::Empty, 8, "empty event list"),
        (Operation::ResolveError, 8, "resolve failed"),
        (Operation::DispatchError, 8, "dispatch failed"),
        (Operation::ReduceError, 8, "reduce failed"),
        (Operation::HostError, 8, "host failed"),
        (Operation::HostReduceError, 8, "reduce failed"),
        (Operation::Cycle, 2, "effect limit"),
    ] {
        let factory = Factory::default();
        let mut engine = engine(&factory, limit);
        let mut trace = Recorder::default();
        engine.start(&mut trace).unwrap();

        let error = engine.run_operation(operation, &mut trace).unwrap_err();
        assert!(error.contains(expected), "{error}");
        assert!(engine.successful_operations().is_empty());
        assert_terminal_poison(&mut engine, &mut trace);
    }
}

#[test]
fn checkpoint_sink_failures_poison_initial_session_reducer_and_host_boundaries() {
    for (fail_at, operation) in [
        (0, None),
        (1, Some(Operation::Hosted)),
        (2, Some(Operation::Hosted)),
        (3, Some(Operation::Hosted)),
    ] {
        let factory = Factory::default();
        let mut engine = engine(&factory, 8);
        let mut trace = Recorder {
            checkpoints: Vec::new(),
            fail_at: Some(fail_at),
        };
        let error = if let Some(operation) = operation {
            engine.start(&mut trace).unwrap();
            engine.run_operation(operation, &mut trace).unwrap_err()
        } else {
            engine.start(&mut trace).unwrap_err()
        };
        assert!(error.contains("sink failed"));
        assert_eq!(engine.host().observation_calls.get(), fail_at + 1);
        assert_eq!(trace.checkpoints.len(), fail_at);
        assert_eq!(
            trace
                .checkpoints
                .iter()
                .map(|checkpoint| checkpoint.boundary)
                .collect::<Vec<_>>(),
            [
                EngineBoundary::Initial,
                EngineBoundary::Session,
                EngineBoundary::Reducer,
                EngineBoundary::Host,
            ][..fail_at]
        );
        assert_terminal_poison(&mut engine, &mut trace);
    }
}

#[test]
fn frontend_host_observation_and_parity_errors_poison_the_engine() {
    for (factory, operation) in [
        (
            Factory {
                fail_observation_at: Some(0),
                ..Factory::default()
            },
            None,
        ),
        (
            Factory {
                fail_host_observation_at: Some(0),
                ..Factory::default()
            },
            None,
        ),
        (
            Factory {
                fail_observation_at: Some(1),
                ..Factory::default()
            },
            Some(Operation::Local),
        ),
        (
            Factory {
                fail_host_observation_at: Some(3),
                ..Factory::default()
            },
            Some(Operation::Hosted),
        ),
    ] {
        let mut engine = engine(&factory, 8);
        let mut trace = Recorder::default();
        let error = if let Some(operation) = operation {
            engine.start(&mut trace).unwrap();
            engine.run_operation(operation, &mut trace).unwrap_err()
        } else {
            engine.start(&mut trace).unwrap_err()
        };
        assert!(error.contains("observation failed"), "{error}");
        assert_terminal_poison(&mut engine, &mut trace);
    }

    let factory = Factory {
        parity_error: true,
        ..Factory::default()
    };
    let mut engine = engine(&factory, 8);
    let mut trace = Recorder::default();
    engine.start(&mut trace).unwrap();
    assert!(
        engine
            .parity_snapshot()
            .unwrap_err()
            .contains("parity snapshot failed")
    );
    assert_terminal_poison(&mut engine, &mut trace);
}

#[test]
fn checkpoint_capture_failure_publishes_nothing_at_each_engine_boundary() {
    for (fail_at, operation, retained) in [
        (0, None, 0),
        (1, Some(Operation::Noop), 1),
        (1, Some(Operation::Local), 1),
        (3, Some(Operation::Hosted), 3),
    ] {
        let factory = Factory {
            fail_host_observation_at: Some(fail_at),
            ..Factory::default()
        };
        let mut engine = engine(&factory, 8);
        let mut trace = Recorder::default();

        let error = if let Some(operation) = operation {
            engine.start(&mut trace).unwrap();
            engine.run_operation(operation, &mut trace).unwrap_err()
        } else {
            engine.start(&mut trace).unwrap_err()
        };

        assert_eq!(error, "host observation failed");
        assert_eq!(trace.checkpoints.len(), retained);
        assert_terminal_poison(&mut engine, &mut trace);
    }
}

#[test]
fn checkpoint_capture_failure_after_a_frontend_session_event_publishes_nothing() {
    let factory = Factory {
        fail_host_observation_at: Some(1),
        ..Factory::default()
    };
    let mut engine = engine(&factory, 8);
    let mut trace = Recorder::default();
    engine.start(&mut trace).unwrap();

    let error = engine
        .run_operation(Operation::Hosted, &mut trace)
        .unwrap_err();

    assert_eq!(error, "host observation failed");
    assert_eq!(engine.frontend().session, 1);
    assert_eq!(engine.frontend().reducer, 0);
    assert_eq!(engine.host().served, 0);
    assert_eq!(engine.host().observation_calls.get(), 2);
    assert_eq!(trace.checkpoints.len(), 1);
    assert_eq!(trace.checkpoints[0].boundary, EngineBoundary::Initial);
    assert_terminal_poison(&mut engine, &mut trace);
}

#[test]
fn checkpoint_projection_mutates_only_trace_clones_in_all_effect_positions() {
    let factory = Factory {
        mutate_checkpoint_projection: true,
        ..Factory::default()
    };
    let mut engine = engine(&factory, 8);
    let mut trace = Recorder::default();
    engine.start(&mut trace).unwrap();

    engine
        .run_operation(Operation::MultiHost, &mut trace)
        .unwrap();

    assert_eq!(engine.host().served, 2);
    assert_eq!(engine.frontend().reducer, 3);
    assert!(!engine.is_poisoned());
    assert!(matches!(
        &trace.checkpoints[1].cause,
        EngineCause::Reducer {
            action: Action::Local,
            emitted: Effect::Host("projected-reducer-emitted"),
            ..
        }
    ));
    for checkpoint in &trace.checkpoints[2..] {
        assert!(matches!(
            &checkpoint.cause,
            EngineCause::Host {
                request: Effect::Host("projected-host-request"),
                response: Action::Local,
                emitted: Effect::Host("projected-host-emitted"),
                ..
            }
        ));
    }
}

#[test]
fn noop_checkpoint_capture_preserves_values_and_replay_uses_every_cause_kind() {
    let factory = Factory::default();
    let mut trace = Recorder::default();
    let replay = replay_prefix_with_sink(
        &factory,
        &[Operation::Noop, Operation::Local, Operation::Hosted],
        &mut trace,
    )
    .unwrap();

    assert_eq!(
        replay.host().capture_causes,
        [
            CaptureCause::Observation,
            CaptureCause::Observation,
            CaptureCause::Reducer,
            CaptureCause::Observation,
            CaptureCause::Reducer,
            CaptureCause::Host,
        ]
    );
    assert_eq!(replay.host().capture_causes.len(), trace.checkpoints.len());
    assert!(matches!(
        trace.checkpoints[2].cause,
        EngineCause::Reducer {
            action: Action::Local,
            emitted: Effect::None,
            ..
        }
    ));
    assert_eq!(
        trace.checkpoints.last().unwrap().frontend,
        FrontendObservation {
            reducer: 3,
            session: 1,
        }
    );
}

#[test]
fn local_and_exact_limit_host_quit_are_successful_terminal_operations() {
    let factory = Factory::default();
    for (operation, limit, served) in [
        (Operation::Quit, 8, 0),
        (Operation::HostQuit, 2, 1),
        (Operation::HostQuit, 1, 1),
    ] {
        let mut engine = engine(&factory, limit);
        let mut trace = Recorder::default();
        engine.start(&mut trace).unwrap();
        engine.run_operation(operation, &mut trace).unwrap();
        assert!(engine.is_quit());
        assert!(!engine.is_poisoned());
        assert_eq!(engine.host().served, served);

        let error = engine
            .run_operation(Operation::Local, &mut trace)
            .unwrap_err();
        assert!(error.contains("after quit"));
        assert_terminal_poison(&mut engine, &mut trace);
    }
}

#[test]
fn effect_chain_can_settle_exactly_at_the_limit() {
    let factory = Factory::default();
    let mut engine = engine(&factory, 2);
    let mut trace = Recorder::default();
    engine.start(&mut trace).unwrap();

    engine
        .run_operation(Operation::MultiHost, &mut trace)
        .unwrap();

    assert_eq!(engine.host().served, 2);
    assert!(!engine.is_quit());
    assert!(!engine.is_poisoned());
}

#[test]
fn main_trace_keeps_session_reducer_and_host_boundaries_in_order() {
    let factory = Factory::default();
    let (frontend, host) = factory.create().unwrap();
    let mut engine = WalkerEngine::new(frontend, host, 8).unwrap();
    let mut trace = Recorder::default();

    engine.start(&mut trace).unwrap();
    engine.run_operation(Operation::Noop, &mut trace).unwrap();
    engine.run_operation(Operation::Local, &mut trace).unwrap();
    engine.run_operation(Operation::Hosted, &mut trace).unwrap();

    assert_eq!(
        trace
            .checkpoints
            .iter()
            .map(|checkpoint| checkpoint.boundary)
            .collect::<Vec<_>>(),
        [
            EngineBoundary::Initial,
            EngineBoundary::Session,
            EngineBoundary::Reducer,
            EngineBoundary::Session,
            EngineBoundary::Reducer,
            EngineBoundary::Host,
        ]
    );
    assert!(matches!(
        trace.checkpoints[1].cause,
        EngineCause::NotApplicable { .. }
    ));
    assert_eq!(
        trace
            .checkpoints
            .iter()
            .filter_map(|checkpoint| match &checkpoint.cause {
                EngineCause::Initial => None,
                EngineCause::NotApplicable {
                    dispatch_sequence, ..
                }
                | EngineCause::Session {
                    dispatch_sequence, ..
                }
                | EngineCause::Reducer {
                    dispatch_sequence, ..
                }
                | EngineCause::Host {
                    dispatch_sequence, ..
                } => Some(*dispatch_sequence),
            })
            .collect::<Vec<_>>(),
        [0, 1, 2, 3, 3]
    );
    assert_eq!(trace.checkpoints.last().unwrap().frontend.reducer, 3);
    assert_eq!(trace.checkpoints.last().unwrap().host.served, 1);
    assert_eq!(trace.checkpoints.last().unwrap().host.frontend_reducer, 3);
    assert_eq!(trace.checkpoints.last().unwrap().host.frontend_session, 1);
    assert_eq!(engine.frontend().reducer, 3);
    assert_eq!(
        engine.successful_operations(),
        &[Operation::Noop, Operation::Local, Operation::Hosted]
    );
}

#[test]
fn parity_snapshot_contains_no_host_and_can_diverge_without_mutating_main_state() {
    let factory = Factory::default();
    let (frontend, host) = factory.create().unwrap();
    let mut engine = WalkerEngine::new(frontend, host, 8).unwrap();
    let mut trace = Recorder::default();
    engine.start(&mut trace).unwrap();
    engine.run_operation(Operation::Local, &mut trace).unwrap();

    let mut parity = engine.parity_snapshot().unwrap();
    parity.reducer += 10;
    parity.session += 20;

    assert_eq!(
        parity,
        ParitySnapshot {
            reducer: 11,
            session: 20,
        }
    );
    assert_eq!(
        engine.parity_snapshot().unwrap(),
        ParitySnapshot {
            reducer: 1,
            session: 0
        }
    );
    assert_eq!(engine.host().served, 0);
}

#[test]
fn host_liveness_uses_a_fresh_prefix_replay_without_cloning_the_main_host() {
    let factory = Factory::default();
    let mut main = engine(&factory, 8);
    let mut trace = Recorder::default();
    main.start(&mut trace).unwrap();
    for operation in [
        Operation::Noop,
        Operation::Local,
        Operation::Hosted,
        Operation::MultiHost,
    ] {
        main.run_operation(operation, &mut trace).unwrap();
    }

    let main_generation = main.host().generation;
    let main_served = main.host().served;
    let main_parity = main.parity_snapshot().unwrap();
    let mut replay_trace = Recorder::default();
    let mut replay =
        replay_prefix_with_sink(&factory, main.successful_operations(), &mut replay_trace).unwrap();

    assert_ne!(replay.host().generation, main_generation);
    assert_eq!(replay.host().served, 3);
    assert_eq!(replay.parity_snapshot().unwrap(), main_parity);
    assert_eq!(
        replay_trace
            .checkpoints
            .iter()
            .filter_map(|checkpoint| match &checkpoint.cause {
                EngineCause::Initial => None,
                EngineCause::NotApplicable {
                    operation_index, ..
                }
                | EngineCause::Session {
                    operation_index, ..
                }
                | EngineCause::Reducer {
                    operation_index, ..
                }
                | EngineCause::Host {
                    operation_index, ..
                } => Some(*operation_index),
            })
            .collect::<Vec<_>>(),
        [
            Some(0),
            Some(1),
            Some(2),
            Some(2),
            Some(2),
            Some(3),
            Some(3),
            Some(3),
        ]
    );
    assert_eq!(
        replay_trace
            .checkpoints
            .iter()
            .filter_map(|checkpoint| match checkpoint.cause {
                EngineCause::Host {
                    dispatch_sequence,
                    round,
                    ..
                } => Some((dispatch_sequence, round)),
                EngineCause::Initial
                | EngineCause::NotApplicable { .. }
                | EngineCause::Session { .. }
                | EngineCause::Reducer { .. } => None,
            })
            .collect::<Vec<_>>(),
        [(3, 0), (4, 0), (4, 1)]
    );
    assert_eq!(
        replay_trace.checkpoints.last().unwrap().frontend,
        FrontendObservation {
            reducer: 6,
            session: 1,
        }
    );
    assert_eq!(replay_trace.checkpoints.last().unwrap().host.served, 3);
    assert_eq!(
        replay_trace
            .checkpoints
            .last()
            .unwrap()
            .host
            .frontend_reducer,
        6
    );
    assert_eq!(
        replay_trace
            .checkpoints
            .last()
            .unwrap()
            .host
            .frontend_session,
        1
    );
    assert_eq!(replay.successful_operations(), main.successful_operations());
    assert_eq!(main.host().served, main_served);
    assert_eq!(factory.generations.get(), 2);
}

#[test]
fn fresh_replay_propagates_factory_adapter_sink_and_check_errors() {
    let default_limit = replay_prefix(&DefaultLimitFactory, &[]).unwrap();
    assert!(default_limit.successful_operations().is_empty());

    let factory = Factory {
        fail_create: true,
        ..Factory::default()
    };
    assert!(
        replay_prefix(&factory, &[])
            .err()
            .unwrap()
            .contains("factory failed")
    );

    let factory = Factory::default();
    assert!(
        replay_prefix(&factory, &[Operation::ResolveError])
            .err()
            .unwrap()
            .contains("resolve failed")
    );

    let mut sink = Recorder {
        checkpoints: Vec::new(),
        fail_at: Some(1),
    };
    assert!(
        replay_prefix_with_sink(&factory, &[Operation::Noop], &mut sink)
            .err()
            .unwrap()
            .contains("sink failed")
    );

    let mut initial_sink = Recorder {
        checkpoints: Vec::new(),
        fail_at: Some(0),
    };
    assert!(
        replay_prefix_with_sink(&factory, &[], &mut initial_sink)
            .err()
            .unwrap()
            .contains("sink failed")
    );

    assert!(
        with_fresh_replay(&factory, &[Operation::Local], |_replay| {
            Err::<(), _>("check failed".to_owned())
        })
        .unwrap_err()
        .contains("check failed")
    );

    let failing_factory = Factory {
        fail_create: true,
        ..Factory::default()
    };
    assert!(
        with_fresh_replay(&failing_factory, &[], |_replay| Ok(()))
            .unwrap_err()
            .contains("factory failed")
    );

    let zero_limit = Factory {
        effect_limit: 0,
        ..Factory::default()
    };
    assert!(
        replay_prefix(&zero_limit, &[])
            .err()
            .unwrap()
            .contains("positive")
    );
}

#[test]
fn host_effect_cycles_stop_at_the_engine_limit() {
    let factory = Factory::default();
    let (frontend, host) = factory.create().unwrap();
    let mut engine = WalkerEngine::new(frontend, host, 2).unwrap();
    let mut trace = Recorder::default();
    engine.start(&mut trace).unwrap();

    let error = engine
        .run_operation(Operation::Cycle, &mut trace)
        .unwrap_err();

    assert!(error.contains("effect limit"), "{error}");
    assert_eq!(engine.host().served, 2);
}

#[test]
fn final_liveness_uses_a_separate_phase_without_changing_the_successful_prefix() {
    let factory = Factory::default();
    let mut engine = engine(&factory, 8);
    let mut trace = Recorder::default();
    engine.start(&mut trace).unwrap();
    engine.run_operation(Operation::Hosted, &mut trace).unwrap();
    let prefix = engine.successful_operations().to_vec();

    engine
        .run_final_liveness(Operation::MultiHost, &mut trace)
        .unwrap();

    assert_eq!(engine.successful_operations(), prefix);
    let liveness = trace
        .checkpoints
        .iter()
        .filter(|checkpoint| checkpoint.phase == EnginePhase::FinalLiveness)
        .collect::<Vec<_>>();
    assert_eq!(liveness.len(), 3);
    assert_eq!(
        liveness
            .iter()
            .map(|checkpoint| match &checkpoint.cause {
                EngineCause::Reducer {
                    dispatch_sequence,
                    operation_index,
                    ..
                }
                | EngineCause::Host {
                    dispatch_sequence,
                    operation_index,
                    ..
                } => (*dispatch_sequence, *operation_index),
                other => panic!("unexpected final-liveness cause: {other:?}"),
            })
            .collect::<Vec<_>>(),
        [(0, None), (0, None), (0, None)]
    );
    assert_eq!(engine.host().served, 3);

    assert!(
        engine
            .run_operation(Operation::Local, &mut trace)
            .unwrap_err()
            .contains("after final liveness")
    );
    assert!(engine.is_poisoned());
}

#[test]
fn final_liveness_requires_a_started_healthy_engine_and_runs_once() {
    let factory = Factory::default();
    let mut not_started = engine(&factory, 8);
    let mut trace = Recorder::default();
    assert!(
        not_started
            .run_final_liveness(Operation::Local, &mut trace)
            .unwrap_err()
            .contains("must start")
    );
    assert!(not_started.is_poisoned());

    let mut walker = engine(&factory, 8);
    let mut trace = Recorder::default();
    walker.start(&mut trace).unwrap();
    walker
        .run_final_liveness(Operation::Local, &mut trace)
        .unwrap();
    assert!(
        walker
            .run_final_liveness(Operation::Local, &mut trace)
            .unwrap_err()
            .contains("more than once")
    );
    assert!(walker.is_poisoned());

    let mut quit = engine(&factory, 8);
    let mut trace = Recorder::default();
    quit.start(&mut trace).unwrap();
    quit.run_operation(Operation::Quit, &mut trace).unwrap();
    assert!(
        quit.run_final_liveness(Operation::Local, &mut trace)
            .unwrap_err()
            .contains("after quit")
    );
    assert!(quit.is_poisoned());
}

#[test]
fn each_checkpoint_observes_the_frontend_and_host_exactly_once() {
    let factory = Factory::default();
    let mut engine = engine(&factory, 8);
    let mut trace = Recorder::default();

    assert_eq!(engine.frontend().observation_calls, 0);
    assert_eq!(engine.host().observation_calls.get(), 0);
    engine.start(&mut trace).unwrap();
    assert_eq!(trace.checkpoints.len(), 1);
    assert_eq!(engine.frontend().observation_calls, 1);
    assert_eq!(engine.host().observation_calls.get(), 1);

    engine.run_operation(Operation::Noop, &mut trace).unwrap();
    assert_eq!(trace.checkpoints.len(), 2);
    assert_eq!(engine.frontend().observation_calls, 2);
    assert_eq!(engine.host().observation_calls.get(), 2);
}
