//! Randomized walk over the real production TUI host.
//!
//! One case draws a vector of late-bound operations from the model crate. Each profile spawns one
//! fresh random-mode host, binds every operation against the live frame, and runs it through the
//! walker engine. A failure keeps the cast and one replay description.

use std::{
    any::Any,
    cell::RefCell,
    fs,
    panic::{AssertUnwindSafe, catch_unwind},
    path::Path,
};

use proptest::{
    collection,
    test_runner::{
        Config, FailurePersistence, FileFailurePersistence, Reason, RngAlgorithm, TestCaseError,
        TestError, TestRng, TestRunner,
    },
};
use ratatui_core::layout::Size;
use ratatui_crossterm::crossterm::event::KeyEvent;
use serde_json::{Value, json};
use skit_i18n::Locale;
use skit_tui_walker_model::{
    artifacts::{
        LivenessSampling, ProfileMode, ReproSource, SuccessRecording, liveness_every,
        record_success, repro_source, walk_cases, walk_profiles, walk_steps, write_failure_bundle,
        write_success_bundle,
    },
    model::{
        LiveInventory, MouseKind, RandomOperation, ResolveRefusal, ResolvedInput,
        operation_strategy, random_walk_profiles, resolve,
    },
    parity::binding_event,
};
use skit_tui_walker_support::engine::{NoopCheckpointSink, replay_prefix_with_sink};
use skit_ui::{LibraryState, UiCommand};

use super::tui_real_host::RealWalkerHost;
use super::tui_real_walker::{
    CorpusEngine, CorpusKeyEvent, CorpusMouseKind, CorpusOperation, CorpusWalkerFactory,
    SchemaThreeSink, close_corpus_engine, corpus_canvas_size, corpus_operation_value,
    random_walk_factory, real_initial_state,
};

/// Directory that keeps every walk artifact.
///
/// A test binary runs from its package directory, so both paths start at the manifest.
const ARTIFACT_DIRECTORY: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../target/ui-walker-artifacts"
);

/// File that keeps the seed of one failing complete walk.
const REGRESSION_FILE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../target/ui-walker-artifacts/regressions.txt"
);

/// Number of random cases in one walk without an environment value.
const DEFAULT_CASES: u32 = 4;

/// Number of operations in one case without an environment value.
const DEFAULT_STEPS: usize = 24;

/// Measured shrink budget for one failing case.
///
/// One measured case of 100 operations costs 68 to 80 seconds against one real host. Each shrink
/// step spawns one more host, so this budget keeps the artifact write of a failing nightly walk
/// inside the 65-minute job timeout beside the walk itself.
const MAX_SHRINK_ITERS: u32 = 8;

/// The one profile that the bounded default replays each trace against.
const BOUNDED_PROFILES: &[(Locale, Size)] = &[(Locale::En, Size::new(80, 24))];

/// Shape version of one replay description.
const REPRO_VERSION: u64 = 1;

/// Every locale that one replay description can name.
const WALK_LOCALES: &[Locale] = &[Locale::En, Locale::ZhCn, Locale::ZhTw, Locale::Pseudo];

/// One locale and viewport that a walk replays one trace against.
type WalkProfile = (Locale, Size);

/// Every environment value that scales one walk.
#[derive(Clone, Debug, Eq, PartialEq)]
struct WalkSettings {
    cases: u32,
    steps: usize,
    mode: ProfileMode,
    success: SuccessRecording,
    liveness: LivenessSampling,
    repro: ReproSource,
}

/// Read every environment value through the model crate.
fn walk_settings() -> WalkSettings {
    WalkSettings {
        cases: walk_cases(DEFAULT_CASES),
        steps: walk_steps(DEFAULT_STEPS),
        mode: walk_profiles(),
        success: record_success(),
        liveness: liveness_every(),
        repro: repro_source(),
    }
}

/// One case that reached its final liveness.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct CaseSuccess {
    cast: Vec<u8>,
    operations: Vec<Value>,
    skipped: Vec<Value>,
}

/// One case that broke a rule.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct CaseFailure {
    error: String,
    cast: Vec<u8>,
    operations: Vec<Value>,
    skipped: Vec<Value>,
}

/// The last successful case of one walk.
#[derive(Clone, Debug, Eq, PartialEq)]
struct RecordedSuccess {
    profile: WalkProfile,
    operations: Vec<RandomOperation>,
    case: CaseSuccess,
}

/// One bound operation, or the typed reason the frame offered none.
#[derive(Clone, Debug, Eq, PartialEq)]
enum CorpusStep {
    Operation(CorpusOperation),
    Skipped(ResolveRefusal),
}

/// Live state of one case that survives a panic inside the walk.
struct CaseRun {
    sink: SchemaThreeSink,
    engine: Option<CorpusEngine>,
    resolved: Vec<CorpusOperation>,
    values: Vec<Value>,
    skipped: Vec<Value>,
}

/// Name the reason one drawn operation binds to nothing.
const fn refusal_label(refusal: ResolveRefusal) -> &'static str {
    match refusal {
        ResolveRefusal::Unavailable => "unavailable",
        ResolveRefusal::Clipped => "clipped",
        ResolveRefusal::QuitFiltered => "quit_filtered",
    }
}

/// Map one pointer kind of the model onto the corpus vocabulary.
const fn corpus_mouse_kind(kind: MouseKind) -> CorpusMouseKind {
    match kind {
        MouseKind::LeftDown => CorpusMouseKind::PrimaryDown,
        MouseKind::RightDown => CorpusMouseKind::SecondaryDown,
        MouseKind::MiddleDown => CorpusMouseKind::MiddleDown,
        MouseKind::LeftUp => CorpusMouseKind::PrimaryUp,
        MouseKind::LeftDrag => CorpusMouseKind::PrimaryDrag,
        MouseKind::Move => CorpusMouseKind::Move,
        MouseKind::ScrollUp => CorpusMouseKind::ScrollUp,
        MouseKind::ScrollDown => CorpusMouseKind::ScrollDown,
    }
}

/// Map one key event onto the corpus vocabulary.
fn corpus_key(key: KeyEvent) -> Result<CorpusKeyEvent, String> {
    CorpusKeyEvent::from_terminal(key)
        .ok_or_else(|| format!("the corpus vocabulary has no key for {key:?}"))
}

/// Map one advertised command chord.
///
/// The corpus command operation carries the command, and the corpus resolver reaches it through
/// the first advertised chord. Every other chord of the same command is one raw key event, so the
/// corpus operation keeps one shape.
fn corpus_command_step(
    command: UiCommand,
    key: KeyEvent,
    live: &LiveInventory,
) -> Result<CorpusOperation, String> {
    let primary = live
        .commands
        .iter()
        .find(|entry| entry.command == command)
        .and_then(|entry| entry.bindings.first().copied())
        .ok_or_else(|| format!("the live inventory lost command {command:?}"))?;
    if binding_event(primary) == key {
        return Ok(CorpusOperation::CommandKeyboard(command));
    }
    Ok(CorpusOperation::RawKey(corpus_key(key)?))
}

/// Map one bound input onto one corpus operation.
fn corpus_step(input: ResolvedInput, live: &LiveInventory) -> Result<CorpusStep, String> {
    let operation = match input {
        ResolvedInput::CommandKeyboard { command, key } => corpus_command_step(command, key, live)?,
        ResolvedInput::Hit(target) => CorpusOperation::HitTarget(target),
        ResolvedInput::LocalKeyboard { target, key } => CorpusOperation::LocalKeyboard {
            target,
            key: corpus_key(key)?,
        },
        ResolvedInput::LocalHit(target) => CorpusOperation::LocalHit(target),
        ResolvedInput::ScreenFocus(target) => CorpusOperation::ScreenFocus(target),
        ResolvedInput::ScreenHit(target) => CorpusOperation::ScreenHit(target),
        ResolvedInput::RawMouse { column, row, kind } => CorpusOperation::RawMouse {
            column,
            row,
            kind: corpus_mouse_kind(kind),
        },
        ResolvedInput::RawKey(key) => CorpusOperation::RawKey(corpus_key(key)?),
        ResolvedInput::Paste(value) => CorpusOperation::Paste(value),
        ResolvedInput::Focus { gained } => CorpusOperation::Focus { gained },
        ResolvedInput::Resize { width, height } => CorpusOperation::Resize { width, height },
        ResolvedInput::NotApplicable(reason) => return Ok(CorpusStep::Skipped(reason)),
    };
    Ok(CorpusStep::Operation(operation))
}

/// Return the canvas that one drawn vector resizes into.
fn random_canvas(initial: Size, operations: &[RandomOperation]) -> Size {
    let resizes = operations
        .iter()
        .filter_map(|operation| match operation {
            RandomOperation::Resize { width, height } => Some(CorpusOperation::Resize {
                width: *width,
                height: *height,
            }),
            RandomOperation::AdvertisedKey { .. }
            | RandomOperation::PublicHit { .. }
            | RandomOperation::LocalAdvertisedKey { .. }
            | RandomOperation::LocalHit { .. }
            | RandomOperation::MouseCell { .. }
            | RandomOperation::Paste { .. }
            | RandomOperation::RawKey { .. }
            | RandomOperation::Focus { .. } => None,
        })
        .collect::<Vec<_>>();
    corpus_canvas_size(initial, &resizes)
}

/// Name one failed diagnostic capture.
fn diagnostic_failure(capture: Result<(), String>) -> Result<(), String> {
    capture.map_err(|failure| format!("the walk could not record its diagnostic frame: {failure}"))
}

/// Keep the primary failure and name a failed diagnostic capture beside it.
fn after_diagnostic(
    primary: Result<(), String>,
    diagnostic: Result<(), String>,
) -> Result<(), String> {
    match (primary, diagnostic) {
        (primary, Ok(())) => primary,
        (Ok(()), Err(diagnostic)) => Err(diagnostic),
        (Err(primary), Err(diagnostic)) => Err(format!("{primary}; {diagnostic}")),
    }
}

/// Report the primary result of one step and the result of its cleanup.
fn after_close(primary: Result<(), String>, cleanup: Result<(), String>) -> Result<(), String> {
    match (primary, cleanup) {
        (Ok(()), Ok(())) => Ok(()),
        (Ok(()), Err(cleanup)) => Err(format!("the random walk could not close a host: {cleanup}")),
        (Err(primary), Ok(())) => Err(primary),
        (Err(primary), Err(cleanup)) => Err(format!(
            "{primary}; the host cleanup also failed: {cleanup}"
        )),
    }
}

/// Read the text of one panic payload.
fn panic_message(payload: &(dyn Any + Send)) -> String {
    payload.downcast_ref::<String>().map_or_else(
        || {
            payload.downcast_ref::<&str>().map_or_else(
                || "non-text panic".to_owned(),
                |message| (*message).to_owned(),
            )
        },
        Clone::clone,
    )
}

/// Run one final liveness against a fresh replay of the successful prefix.
fn replay_final_liveness(
    factory: &CorpusWalkerFactory,
    prefix: &[CorpusOperation],
) -> Result<(), String> {
    let mut sink = NoopCheckpointSink;
    let mut replay = replay_prefix_with_sink(factory, prefix, &mut sink)?;
    let liveness = replay.run_final_liveness(CorpusOperation::FinalLiveness, &mut sink);
    after_close(liveness, close_corpus_engine(replay))
}

/// Run sampled liveness after one operation.
fn sample_liveness(
    factory: &CorpusWalkerFactory,
    prefix: &[CorpusOperation],
    liveness: LivenessSampling,
) -> Result<(), String> {
    let LivenessSampling::Every(interval) = liveness else {
        return Ok(());
    };
    let due = prefix.len().is_multiple_of(interval.get());
    let sample = due.then(|| replay_final_liveness(factory, prefix));
    sample.unwrap_or(Ok(()))
}

/// Run every drawn operation of one case against one live engine.
///
/// Every operation family filters Quit. A quit that still reaches the engine fails the case
/// through the engine's own refusal, so the walk needs no separate quit branch.
fn walk_case(
    run: &mut CaseRun,
    factory: &CorpusWalkerFactory,
    operations: &[RandomOperation],
    liveness: LivenessSampling,
) -> Result<(), String> {
    let CaseRun {
        sink,
        engine,
        resolved,
        values,
        skipped,
    } = run;
    let engine = engine
        .as_mut()
        .ok_or("the random walk needs one live engine")?;
    engine.start(sink)?;
    for (index, operation) in operations.iter().enumerate() {
        let live = engine.frontend().live_inventory()?;
        match corpus_step(resolve(operation, &live), &live)? {
            CorpusStep::Skipped(reason) => {
                skipped.push(json!({"index": index, "reason": refusal_label(reason)}));
            }
            CorpusStep::Operation(corpus) => {
                values.push(corpus_operation_value(&corpus)?);
                engine.run_operation(corpus.clone(), sink)?;
                resolved.push(corpus);
                sample_liveness(factory, resolved, liveness)?;
            }
        }
    }
    engine.run_final_liveness(CorpusOperation::FinalLiveness, sink)
}

/// Run one case against one fresh host and keep its cast.
fn evaluate_case(
    factory: &CorpusWalkerFactory,
    canvas: Size,
    operations: &[RandomOperation],
    liveness: LivenessSampling,
) -> Result<CaseSuccess, CaseFailure> {
    evaluate_case_with_initial_state(factory, canvas, operations, liveness, real_initial_state)
}

/// Run one case from one caller-supplied initial state.
///
/// The recorder exists before the host, so a panic inside the construction still keeps a valid
/// cast header. A failure records the screen that the engine last drew.
fn evaluate_case_with_initial_state(
    factory: &CorpusWalkerFactory,
    canvas: Size,
    operations: &[RandomOperation],
    liveness: LivenessSampling,
    initial_state: impl FnOnce(&RealWalkerHost) -> Result<LibraryState, String>,
) -> Result<CaseSuccess, CaseFailure> {
    let sink = match factory.walk_sink(canvas) {
        Ok(sink) => sink,
        Err(error) => {
            return Err(CaseFailure {
                error,
                ..CaseFailure::default()
            });
        }
    };
    let run = RefCell::new(CaseRun {
        sink,
        engine: None,
        resolved: Vec::new(),
        values: Vec::new(),
        skipped: Vec::new(),
    });
    let outcome = catch_unwind(AssertUnwindSafe(|| -> Result<(), String> {
        let engine = factory.engine_with_initial_state(initial_state)?;
        run.borrow_mut().engine = Some(engine);
        walk_case(&mut run.borrow_mut(), factory, operations, liveness)
    }));
    let CaseRun {
        mut sink,
        engine,
        values,
        skipped,
        ..
    } = run.into_inner();
    let primary = match outcome {
        Ok(result) => result,
        Err(payload) => Err(format!(
            "the random walk panicked: {}",
            panic_message(payload.as_ref())
        )),
    };
    let mut primary = primary;
    let mut cleanup = Ok(());
    if let Some(engine) = engine {
        let diagnostic = primary
            .as_ref()
            .err()
            .map(|_| sink.record_frontend_frame(engine.frontend()))
            .unwrap_or(Ok(()));
        primary = after_diagnostic(primary, diagnostic_failure(diagnostic));
        cleanup = close_corpus_engine(engine);
    }
    let cast = sink.cast_bytes().to_vec();
    match after_close(primary, cleanup) {
        Ok(()) => Ok(CaseSuccess {
            cast,
            operations: values,
            skipped,
        }),
        Err(error) => Err(CaseFailure {
            error,
            cast,
            operations: values,
            skipped,
        }),
    }
}

/// Run one case of one profile against a real host.
fn real_case(
    settings: &WalkSettings,
    profile: &WalkProfile,
    operations: &[RandomOperation],
) -> Result<CaseSuccess, CaseFailure> {
    let (locale, size) = *profile;
    let factory = random_walk_factory(locale, size);
    evaluate_case(
        &factory,
        random_canvas(size, operations),
        operations,
        settings.liveness,
    )
}

/// Return every profile that one mode replays each trace against.
const fn walk_profile_matrix(mode: ProfileMode) -> &'static [WalkProfile] {
    match mode {
        ProfileMode::Bounded => BOUNDED_PROFILES,
        ProfileMode::Complete => random_walk_profiles(),
    }
}

/// Return where one mode keeps the seed of a failing walk.
fn walk_persistence(mode: ProfileMode) -> Option<Box<dyn FailurePersistence>> {
    match mode {
        ProfileMode::Bounded => Some(Box::new(FileFailurePersistence::Off)),
        ProfileMode::Complete => Some(Box::new(FileFailurePersistence::Direct(REGRESSION_FILE))),
    }
}

/// Name where a reader finds the seed of a failing walk.
const fn walk_seed_hint(mode: ProfileMode) -> &'static str {
    match mode {
        ProfileMode::Bounded => "the fixed ChaCha seed of the bounded profile reproduces it",
        ProfileMode::Complete => "the seed is in target/ui-walker-artifacts/regressions.txt",
    }
}

/// Build the proptest configuration of one walk.
fn walk_config(settings: &WalkSettings) -> Config {
    Config {
        cases: settings.cases,
        max_shrink_iters: MAX_SHRINK_ITERS,
        failure_persistence: walk_persistence(settings.mode),
        source_file: Some(file!()),
        test_name: Some("real_random_walk"),
        rng_algorithm: RngAlgorithm::ChaCha,
        ..Config::default()
    }
}

/// Build the runner of one walk.
///
/// The bounded default keeps one fixed seed, so every pull request runs the same trace. The
/// complete matrix takes a fresh seed and keeps it in the regression file.
fn walk_runner(mode: ProfileMode, config: Config) -> TestRunner {
    match mode {
        ProfileMode::Bounded => {
            TestRunner::new_with_rng(config, TestRng::deterministic_rng(RngAlgorithm::ChaCha))
        }
        ProfileMode::Complete => TestRunner::new(config),
    }
}

/// Build the replay description of one case.
fn repro_value(
    profile: &WalkProfile,
    operations: &[RandomOperation],
    case: &CaseRecord<'_>,
    error: Option<&str>,
) -> Result<Value, String> {
    let (locale, size) = profile;
    let drawn = serde_json::to_value(operations).map_err(|failure| failure.to_string())?;
    Ok(json!({
        "version": REPRO_VERSION,
        "locale": locale.tag(),
        "initial_size": {"cols": size.width, "rows": size.height},
        "error": error,
        "operations": drawn,
        "resolved": case.resolved,
        "skipped": case.skipped,
    }))
}

/// What one case resolved and what it skipped.
#[derive(Clone, Copy, Debug)]
struct CaseRecord<'a> {
    resolved: &'a [Value],
    skipped: &'a [Value],
}

/// Read the profile of one replay description.
fn repro_profile(value: &Value) -> Result<WalkProfile, String> {
    let tag = value["locale"]
        .as_str()
        .ok_or("the replay description needs a locale tag")?;
    let cols = value["initial_size"]["cols"]
        .as_u64()
        .and_then(|value| u16::try_from(value).ok())
        .ok_or("the replay description needs a column count")?;
    let rows = value["initial_size"]["rows"]
        .as_u64()
        .and_then(|value| u16::try_from(value).ok())
        .ok_or("the replay description needs a row count")?;
    let locale = WALK_LOCALES
        .iter()
        .copied()
        .find(|locale| locale.tag() == tag)
        .ok_or_else(|| format!("the replay description names an unknown locale: {tag}"))?;
    Ok((locale, Size::new(cols, rows)))
}

/// Compare the rule that the recorded walk broke against the rule that the replay broke.
///
/// A recorded bug that still breaks is a failure. A recorded bug that stopped breaking is the
/// result the reader wants, so the replay names it and passes.
fn compare_replay_error(recorded: Option<&str>, replayed: Option<&str>) -> Result<(), String> {
    match (recorded, replayed) {
        (None, None) => Ok(()),
        (Some(recorded), None) => {
            eprintln!("the recorded rule no longer breaks: {recorded}");
            Ok(())
        }
        (None, Some(replayed)) => Err(format!(
            "the replay broke one rule that the recorded walk kept: {replayed}"
        )),
        (Some(recorded), Some(replayed)) if recorded == replayed => {
            Err(format!("the recorded rule still breaks: {replayed}"))
        }
        (Some(recorded), Some(replayed)) => Err(format!(
            "the replay broke a different rule: recorded={recorded}; replayed={replayed}"
        )),
    }
}

/// Replay one saved description through the same adapter.
///
/// The replay draws no operation. It binds the saved vector against a fresh host of the saved
/// profile. A failing walk stops at the operation that broke the rule, so the saved vector is a
/// prefix of a replay that runs further. Compare all binding decisions through that boundary.
/// A successful recording requires exact resolutions and skips. Then compare the broken rule.
fn replay_repro(
    path: &Path,
    evaluate: &impl Fn(&WalkProfile, &[RandomOperation]) -> Result<CaseSuccess, CaseFailure>,
) -> Result<(), String> {
    let bytes =
        fs::read(path).map_err(|failure| format!("cannot read {}: {failure}", path.display()))?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|failure| failure.to_string())?;
    let version = value["version"]
        .as_u64()
        .ok_or("the replay description needs a version")?;
    (version == REPRO_VERSION).then_some(()).ok_or_else(|| {
        format!("the replay description has version {version}, and this walk reads {REPRO_VERSION}")
    })?;
    let profile = repro_profile(&value)?;
    let operations: Vec<RandomOperation> = serde_json::from_value(value["operations"].clone())
        .map_err(|failure| format!("the replay description has no operation vector: {failure}"))?;
    let recorded = value["resolved"]
        .as_array()
        .ok_or("the replay description needs a resolved operation vector")?;
    let recorded_skips = value["skipped"]
        .as_array()
        .ok_or("the replay description needs a skipped operation vector")?;
    let recorded_error = value["error"].as_str();
    let outcome = evaluate(&profile, &operations);
    let (replayed, replayed_skips, error) = match &outcome {
        Ok(case) => (&case.operations, &case.skipped, None),
        Err(failure) => (
            &failure.operations,
            &failure.skipped,
            Some(failure.error.as_str()),
        ),
    };
    let same_operations = if recorded_error.is_some() {
        replayed.starts_with(recorded)
    } else {
        replayed == recorded
    };
    same_operations.then_some(()).ok_or_else(|| {
        format!(
            "the replay resolved a different operation vector: the description holds {} operations and the replay holds {}",
            recorded.len(),
            replayed.len(),
        )
    })?;
    // Each recorded resolution or skip consumed one drawn operation. A repaired failure may
    // continue past that boundary, but every earlier skip must still match.
    let boundary = recorded.len() + recorded_skips.len();
    let comparable_skips = if recorded_error.is_some() {
        let count = replayed_skips
            .iter()
            .take_while(|skip| {
                skip["index"]
                    .as_u64()
                    .is_some_and(|index| index < boundary as u64)
            })
            .count();
        &replayed_skips[..count]
    } else {
        replayed_skips.as_slice()
    };
    (comparable_skips == recorded_skips.as_slice())
        .then_some(())
        .ok_or("the replay made different skipped operation decisions")?;
    compare_replay_error(recorded_error, error)
}

/// What one complete set of walk cases produced.
struct WalkCases {
    /// The proptest outcome of the walk.
    outcome: Result<(), TestError<Vec<RandomOperation>>>,
    /// The last case that reached its final liveness.
    success: Option<RecordedSuccess>,
    /// The profile that the walk latched when one case failed.
    latched: Option<WalkProfile>,
}

/// Draw and run every case of one walk.
fn run_walk_cases(
    settings: &WalkSettings,
    evaluate: &impl Fn(&WalkProfile, &[RandomOperation]) -> Result<CaseSuccess, CaseFailure>,
) -> WalkCases {
    let profiles = walk_profile_matrix(settings.mode);
    let strategy = collection::vec(operation_strategy(), settings.steps);
    let mut runner = walk_runner(settings.mode, walk_config(settings));
    let recorded = RefCell::new(None);
    let failing: RefCell<Option<WalkProfile>> = RefCell::new(None);
    let outcome = runner.run(&strategy, |operations| {
        // A shrink step reruns only the profile that failed, so one failing profile does not pay
        // for the complete matrix on every candidate.
        let selected = failing
            .borrow()
            .map_or_else(|| profiles.to_vec(), |profile| vec![profile]);
        for profile in selected {
            match evaluate(&profile, &operations) {
                Ok(case) => {
                    *recorded.borrow_mut() = Some(RecordedSuccess {
                        profile,
                        operations: operations.clone(),
                        case,
                    });
                }
                Err(failure) => {
                    *failing.borrow_mut() = Some(profile);
                    return Err(TestCaseError::fail(format!(
                        "locale={} initial={}x{}: {}",
                        profile.0.tag(),
                        profile.1.width,
                        profile.1.height,
                        failure.error,
                    )));
                }
            }
        }
        Ok(())
    });
    WalkCases {
        outcome,
        success: recorded.into_inner(),
        latched: failing.into_inner(),
    }
}

/// Write the cast of one requested successful walk.
fn finish_success(
    settings: &WalkSettings,
    directory: &Path,
    success: Option<RecordedSuccess>,
) -> Result<(), String> {
    let SuccessRecording::Record = settings.success else {
        return Ok(());
    };
    let recorded = success.ok_or("a requested successful recording must retain the final case")?;
    let repro = repro_value(
        &recorded.profile,
        &recorded.operations,
        &CaseRecord {
            resolved: &recorded.case.operations,
            skipped: &recorded.case.skipped,
        },
        None,
    )?;
    let bundle = write_success_bundle(directory, &repro, &recorded.case.cast)?;
    eprintln!(
        "the requested successful UI walk is in {}",
        bundle.display()
    );
    Ok(())
}

/// The message that one bundle keeps when the minimal trace stopped breaking its rule.
const REPLAY_STOPPED: &str =
    "the minimal trace no longer breaks its rule on the profile that latched it";

/// Replay the minimal failing vector on the latched profile and keep its artifacts.
///
/// The walk latched one profile when it failed, and the panic message names that profile. The
/// replay uses the same one, so the bundle and the message agree and the job spawns one host
/// instead of the complete matrix.
fn finish_failure(
    settings: &WalkSettings,
    directory: &Path,
    reason: &Reason,
    minimal: &[RandomOperation],
    profile: WalkProfile,
    evaluate: &impl Fn(&WalkProfile, &[RandomOperation]) -> Result<CaseSuccess, CaseFailure>,
) -> String {
    let replay = evaluate(&profile, minimal);
    let (error, operations, skipped, cast) = match &replay {
        Ok(case) => (REPLAY_STOPPED, &case.operations, &case.skipped, &case.cast),
        Err(failure) => (
            failure.error.as_str(),
            &failure.operations,
            &failure.skipped,
            &failure.cast,
        ),
    };
    let record = CaseRecord {
        resolved: operations,
        skipped,
    };
    let bundle = repro_value(&profile, minimal, &record, Some(error))
        .and_then(|repro| write_failure_bundle(directory, &repro, cast));
    match bundle {
        Ok(path) => format!(
            "the random walk found a minimal failure: {reason}; error={error}; artifacts={}; {}",
            path.display(),
            walk_seed_hint(settings.mode),
        ),
        Err(artifact) => format!(
            "the random walk failed and artifact capture also failed: {reason}; artifact={artifact}"
        ),
    }
}

/// Report the outcome of one complete walk.
fn finish_walk(
    settings: &WalkSettings,
    directory: &Path,
    outcome: Result<(), TestError<Vec<RandomOperation>>>,
    success: Option<RecordedSuccess>,
    latched: Option<WalkProfile>,
    evaluate: &impl Fn(&WalkProfile, &[RandomOperation]) -> Result<CaseSuccess, CaseFailure>,
) -> Result<(), String> {
    match outcome {
        Ok(()) => finish_success(settings, directory, success),
        Err(TestError::Fail(reason, minimal)) => Err(latched.map_or_else(
            || format!("the random walk failed on no profile: {reason}"),
            |profile| finish_failure(settings, directory, &reason, &minimal, profile, evaluate),
        )),
        Err(TestError::Abort(reason)) => Err(format!(
            "the random walk aborted before it produced a case: {reason}"
        )),
    }
}

/// Run every case of one walk and report its outcome.
fn run_random_walk(
    settings: &WalkSettings,
    directory: &Path,
    evaluate: &impl Fn(&WalkProfile, &[RandomOperation]) -> Result<CaseSuccess, CaseFailure>,
) -> Result<(), String> {
    fs::create_dir_all(directory).map_err(|failure| failure.to_string())?;
    let cases = run_walk_cases(settings, evaluate);
    finish_walk(
        settings,
        directory,
        cases.outcome,
        cases.success,
        cases.latched,
        evaluate,
    )
}

/// Draw a new walk, or replay one saved description.
fn run_walk_or_replay(
    settings: &WalkSettings,
    directory: &Path,
    evaluate: &impl Fn(&WalkProfile, &[RandomOperation]) -> Result<CaseSuccess, CaseFailure>,
) -> Result<(), String> {
    match &settings.repro {
        ReproSource::Fresh => run_random_walk(settings, directory, evaluate),
        ReproSource::Replay(path) => replay_repro(path, evaluate),
    }
}

#[test]
fn real_random_walk() {
    let settings = walk_settings();
    let evaluate = |profile: &WalkProfile, operations: &[RandomOperation]| {
        real_case(&settings, profile, operations)
    };
    run_walk_or_replay(&settings, Path::new(ARTIFACT_DIRECTORY), &evaluate)
        .unwrap_or_else(|error| panic!("{error}"));
}

#[cfg(test)]
mod tests {
    use std::{num::NonZeroUsize, path::PathBuf};

    use ratatui_crossterm::crossterm::event::{KeyCode, KeyModifiers};
    use skit_tui::{HitTarget, LocalActionTarget, ScreenTarget};
    use skit_tui_walker_model::artifacts::REPRO_NAME;
    use skit_tui_walker_model::model::{AdvertisedCommand, KeyKind, RawKey};
    use skit_ui::{HealthAction, PreferencesControlId, UiBinding, UiKey, UiModifiers};

    use super::*;

    /// Forge an initial state whose visible order breaks its rule.
    fn broken_visible_order(host: &RealWalkerHost) -> Result<LibraryState, String> {
        let state = real_initial_state(host)?;
        let mut value = serde_json::to_value(state).map_err(|error| error.to_string())?;
        value["visible"] = json!([0, 0]);
        value["selected"] = json!(0);
        serde_json::from_value(value).map_err(|error| error.to_string())
    }

    /// Refuse the initial state of one real host.
    fn refused_initial_state(_host: &RealWalkerHost) -> Result<LibraryState, String> {
        Err("the walker host refused its initial state".to_owned())
    }

    /// Panic while the walk reads the initial state of one real host.
    fn panicking_initial_state(_host: &RealWalkerHost) -> Result<LibraryState, String> {
        panic!("the walker host panicked before its initial state");
    }

    fn english_factory() -> CorpusWalkerFactory {
        random_walk_factory(Locale::En, Size::new(80, 24))
    }

    fn stub_success(
        _profile: &WalkProfile,
        _operations: &[RandomOperation],
    ) -> Result<CaseSuccess, CaseFailure> {
        Ok(CaseSuccess {
            cast: b"stub cast".to_vec(),
            operations: vec![json!({"operation": "focus", "gained": true})],
            skipped: vec![json!({"index": 0, "reason": "unavailable"})],
        })
    }

    fn stub_failure(
        _profile: &WalkProfile,
        _operations: &[RandomOperation],
    ) -> Result<CaseSuccess, CaseFailure> {
        Err(CaseFailure {
            error: "the stub case broke one rule".to_owned(),
            cast: b"stub cast".to_vec(),
            operations: vec![json!({"operation": "focus", "gained": true})],
            ..CaseFailure::default()
        })
    }

    /// One walk that breaks its rule at the second of three operations.
    ///
    /// The value of the failing operation reaches the record, and the operations after it never
    /// run, so the recorded vector is shorter than the drawn one.
    fn truncated_failure(
        _profile: &WalkProfile,
        operations: &[RandomOperation],
    ) -> Result<CaseSuccess, CaseFailure> {
        Err(CaseFailure {
            error: "the second operation broke one rule".to_owned(),
            cast: b"failing cast".to_vec(),
            operations: focus_values(operations.len().min(2)),
            skipped: Vec::new(),
        })
    }

    /// The same walk after the rule is repaired: every operation runs.
    fn repaired_walk(
        _profile: &WalkProfile,
        operations: &[RandomOperation],
    ) -> Result<CaseSuccess, CaseFailure> {
        Ok(CaseSuccess {
            cast: b"passing cast".to_vec(),
            operations: focus_values(operations.len()),
            skipped: Vec::new(),
        })
    }

    fn focus_values(count: usize) -> Vec<Value> {
        (0..count)
            .map(|index| json!({"operation": "focus", "gained": index % 2 == 0}))
            .collect()
    }

    fn one_bundle(directory: &Path, prefix: &str) -> PathBuf {
        let mut bundles = fs::read_dir(directory)
            .unwrap()
            .map(Result::unwrap)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(prefix))
            })
            .collect::<Vec<_>>();
        assert_eq!(bundles.len(), 1, "one {prefix} bundle");
        bundles.pop().expect("one bundle")
    }

    fn bounded_settings() -> WalkSettings {
        WalkSettings {
            cases: 2,
            steps: 2,
            mode: ProfileMode::Bounded,
            success: SuccessRecording::Skip,
            liveness: LivenessSampling::Never,
            repro: ReproSource::Fresh,
        }
    }

    #[test]
    fn the_walk_defaults_run_four_bounded_cases_of_twenty_four_operations() {
        // The reader of every value is the model crate. This contract pins the defaults that this
        // walk passes to it, so an exported SKIT_WALKER_* value cannot change the result.
        assert_eq!(DEFAULT_CASES, 4);
        assert_eq!(DEFAULT_STEPS, 24);
        assert_eq!(MAX_SHRINK_ITERS, 8);
        assert_eq!(BOUNDED_PROFILES, [(Locale::En, Size::new(80, 24))]);
        assert_eq!(WALK_LOCALES.len(), 4);
        assert_eq!(REPRO_VERSION, 1);

        let settings = WalkSettings {
            cases: DEFAULT_CASES,
            steps: DEFAULT_STEPS,
            mode: ProfileMode::Bounded,
            success: SuccessRecording::Skip,
            liveness: LivenessSampling::Never,
            repro: ReproSource::Fresh,
        };
        let config = walk_config(&settings);
        assert_eq!(config.cases, DEFAULT_CASES);
        assert_eq!(config.max_shrink_iters, MAX_SHRINK_ITERS);
    }

    const fn ui_binding(key: UiKey) -> UiBinding {
        UiBinding {
            key,
            modifiers: UiModifiers::NONE,
            hint: "hint",
            compact_hint: "hint",
        }
    }

    fn health_inventory() -> LiveInventory {
        LiveInventory {
            commands: vec![AdvertisedCommand {
                command: UiCommand::Health,
                bindings: vec![ui_binding(UiKey::Character('h')), ui_binding(UiKey::Escape)],
            }],
            ..LiveInventory::default()
        }
    }

    #[test]
    fn every_resolved_input_maps_to_one_corpus_step() {
        let live = health_inventory();
        let escape = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        let health = LocalActionTarget::Health(HealthAction::Back);
        let screen = ScreenTarget::Preferences(PreferencesControlId::Language);
        let cases = [
            (
                ResolvedInput::CommandKeyboard {
                    command: UiCommand::Health,
                    key: KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE),
                },
                CorpusStep::Operation(CorpusOperation::CommandKeyboard(UiCommand::Health)),
            ),
            (
                ResolvedInput::CommandKeyboard {
                    command: UiCommand::Health,
                    key: escape,
                },
                CorpusStep::Operation(CorpusOperation::RawKey(
                    CorpusKeyEvent::from_terminal(escape).unwrap(),
                )),
            ),
            (
                ResolvedInput::Hit(HitTarget::FocusField(1)),
                CorpusStep::Operation(CorpusOperation::HitTarget(HitTarget::FocusField(1))),
            ),
            (
                ResolvedInput::LocalKeyboard {
                    target: health.clone(),
                    key: escape,
                },
                CorpusStep::Operation(CorpusOperation::LocalKeyboard {
                    target: health.clone(),
                    key: CorpusKeyEvent::from_terminal(escape).unwrap(),
                }),
            ),
            (
                ResolvedInput::LocalHit(health.clone()),
                CorpusStep::Operation(CorpusOperation::LocalHit(health)),
            ),
            (
                ResolvedInput::ScreenFocus(screen.clone()),
                CorpusStep::Operation(CorpusOperation::ScreenFocus(screen.clone())),
            ),
            (
                ResolvedInput::ScreenHit(screen.clone()),
                CorpusStep::Operation(CorpusOperation::ScreenHit(screen)),
            ),
            (
                ResolvedInput::RawMouse {
                    column: 4,
                    row: 5,
                    kind: MouseKind::ScrollDown,
                },
                CorpusStep::Operation(CorpusOperation::RawMouse {
                    column: 4,
                    row: 5,
                    kind: CorpusMouseKind::ScrollDown,
                }),
            ),
            (
                ResolvedInput::RawKey(escape),
                CorpusStep::Operation(CorpusOperation::RawKey(
                    CorpusKeyEvent::from_terminal(escape).unwrap(),
                )),
            ),
            (
                ResolvedInput::Paste("界".to_owned()),
                CorpusStep::Operation(CorpusOperation::Paste("界".to_owned())),
            ),
            (
                ResolvedInput::Focus { gained: true },
                CorpusStep::Operation(CorpusOperation::Focus { gained: true }),
            ),
            (
                ResolvedInput::Resize {
                    width: 40,
                    height: 12,
                },
                CorpusStep::Operation(CorpusOperation::Resize {
                    width: 40,
                    height: 12,
                }),
            ),
            (
                ResolvedInput::NotApplicable(ResolveRefusal::QuitFiltered),
                CorpusStep::Skipped(ResolveRefusal::QuitFiltered),
            ),
        ];

        for (input, expected) in cases {
            assert_eq!(corpus_step(input, &live).unwrap(), expected);
        }

        assert_eq!(
            MouseKind::ALL
                .iter()
                .copied()
                .map(corpus_mouse_kind)
                .count(),
            CorpusMouseKind::ALL.len()
        );
        assert_eq!(
            [
                ResolveRefusal::Unavailable,
                ResolveRefusal::Clipped,
                ResolveRefusal::QuitFiltered,
            ]
            .map(refusal_label),
            ["unavailable", "clipped", "quit_filtered"]
        );
        let insert = KeyEvent::new(KeyCode::Insert, KeyModifiers::NONE);
        assert!(
            corpus_step(ResolvedInput::RawKey(insert), &live)
                .unwrap_err()
                .contains("no key for")
        );
        assert!(
            corpus_step(
                ResolvedInput::CommandKeyboard {
                    command: UiCommand::Quit,
                    key: escape,
                },
                &live,
            )
            .unwrap_err()
            .contains("lost command")
        );
    }

    #[test]
    fn a_broken_initial_state_fails_the_walk_and_keeps_a_diagnostic_frame() {
        let factory = english_factory();
        let failure = evaluate_case_with_initial_state(
            &factory,
            Size::new(80, 24),
            &[],
            LivenessSampling::Never,
            broken_visible_order,
        )
        .unwrap_err();

        assert!(
            failure.error.contains("LIBRARY_VISIBLE_ORDER"),
            "{}",
            failure.error
        );
        let mut lines = failure.cast.split(|byte| *byte == b'\n');
        let header: Value = serde_json::from_slice(lines.next().unwrap()).unwrap();
        assert_eq!(header["version"], 3);
        assert_eq!(header["term"]["cols"], 80);
        assert_eq!(header["term"]["rows"], 24);
        let frame: Value = serde_json::from_slice(
            lines
                .next()
                .expect("the broken state still keeps one diagnostic frame"),
        )
        .unwrap();
        assert_eq!(frame[1], "o");
        assert!(!frame[2].as_str().unwrap().is_empty());
    }

    #[test]
    fn a_refused_initial_state_keeps_a_valid_cast_header() {
        let factory = english_factory();
        let failure = evaluate_case_with_initial_state(
            &factory,
            Size::new(80, 24),
            &[],
            LivenessSampling::Never,
            refused_initial_state,
        )
        .unwrap_err();

        assert_eq!(failure.error, "the walker host refused its initial state");
        let header: Value =
            serde_json::from_slice(failure.cast.split(|byte| *byte == b'\n').next().unwrap())
                .unwrap();
        assert_eq!(header["version"], 3);
    }

    #[test]
    fn a_panicking_construction_keeps_a_valid_cast_header() {
        let factory = english_factory();
        let failure = evaluate_case_with_initial_state(
            &factory,
            Size::new(80, 24),
            &[],
            LivenessSampling::Never,
            panicking_initial_state,
        )
        .unwrap_err();

        assert!(
            failure.error.contains(
                "the random walk panicked: the walker host panicked before its initial state"
            ),
            "{}",
            failure.error
        );
        let header: Value =
            serde_json::from_slice(failure.cast.split(|byte| *byte == b'\n').next().unwrap())
                .unwrap();
        assert_eq!(header["version"], 3);
    }

    #[test]
    fn a_zero_canvas_refuses_the_case_before_it_spawns_a_host() {
        let factory = english_factory();
        let failure = evaluate_case_with_initial_state(
            &factory,
            Size::new(0, 0),
            &[],
            LivenessSampling::Never,
            refused_initial_state,
        )
        .unwrap_err();

        assert!(failure.cast.is_empty());
        assert!(failure.operations.is_empty());
    }

    #[test]
    fn sampled_liveness_replays_the_prefix_on_a_fresh_host() {
        let factory = english_factory();
        let operations = [
            RandomOperation::Focus { gained: false },
            RandomOperation::Focus { gained: true },
        ];
        for interval in [1_usize, 2] {
            let success = evaluate_case(
                &factory,
                Size::new(80, 24),
                &operations,
                LivenessSampling::Every(NonZeroUsize::new(interval).unwrap()),
            )
            .unwrap();
            assert_eq!(success.operations.len(), 2);
        }
    }

    #[test]
    fn a_case_binds_and_runs_every_family_against_the_real_host() {
        let factory = english_factory();
        let operations = [
            RandomOperation::AdvertisedKey {
                command: 0,
                binding: 0,
            },
            RandomOperation::PublicHit { ordinal: 0 },
            RandomOperation::LocalAdvertisedKey {
                action: 0,
                binding: 0,
            },
            RandomOperation::LocalHit { action: 0 },
            RandomOperation::MouseCell {
                x_fraction: 128,
                y_fraction: 128,
                kind: MouseKind::LeftDown,
            },
            RandomOperation::Resize {
                width: 46,
                height: 12,
            },
            RandomOperation::Paste {
                value: "界".to_owned(),
            },
            RandomOperation::RawKey {
                key: RawKey::Tab,
                kind: KeyKind::Press,
            },
            RandomOperation::Focus { gained: true },
        ];
        let success = evaluate_case(
            &factory,
            random_canvas(Size::new(80, 24), &operations),
            &operations,
            LivenessSampling::Never,
        )
        .unwrap();

        assert!(!success.cast.is_empty());
        assert!(!success.operations.is_empty());
    }

    #[test]
    fn the_walk_writes_one_failure_bundle_and_names_its_seed() {
        let directory = tempfile::tempdir().unwrap();
        let settings = bounded_settings();
        let error = run_random_walk(&settings, directory.path(), &stub_failure).unwrap_err();

        assert!(
            error.contains("the random walk found a minimal failure"),
            "{error}"
        );
        assert!(error.contains("the fixed ChaCha seed"), "{error}");
        let bundle = fs::read_dir(directory.path())
            .unwrap()
            .map(Result::unwrap)
            .find(|entry| entry.file_name().to_string_lossy().starts_with("failure-"))
            .expect("the failing walk writes one failure bundle");
        let repro: Value =
            serde_json::from_slice(&fs::read(bundle.path().join("repro.json")).unwrap()).unwrap();
        assert_eq!(repro["locale"], "en");
        assert_eq!(repro["initial_size"]["cols"], 80);
        assert_eq!(repro["error"], "the stub case broke one rule");
        assert_eq!(
            fs::read(bundle.path().join("failure.cast")).unwrap(),
            b"stub cast"
        );
    }

    #[test]
    fn the_walk_writes_one_requested_success_bundle() {
        let directory = tempfile::tempdir().unwrap();
        let settings = WalkSettings {
            success: SuccessRecording::Record,
            ..bounded_settings()
        };
        run_random_walk(&settings, directory.path(), &stub_success).unwrap();

        let bundle = fs::read_dir(directory.path())
            .unwrap()
            .map(Result::unwrap)
            .find(|entry| entry.file_name().to_string_lossy().starts_with("success-"))
            .expect("the requested recording writes one success bundle");
        let repro: Value =
            serde_json::from_slice(&fs::read(bundle.path().join("repro.json")).unwrap()).unwrap();
        assert_eq!(repro["error"], Value::Null);
        assert_eq!(repro["skipped"][0]["reason"], "unavailable");
        assert_eq!(
            fs::read(bundle.path().join("success.cast")).unwrap(),
            b"stub cast"
        );
    }

    #[test]
    fn a_skipped_walk_keeps_no_bundle_and_a_requested_one_needs_a_case() {
        let directory = tempfile::tempdir().unwrap();
        let settings = bounded_settings();
        finish_success(&settings, directory.path(), None).unwrap();
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);

        let requested = WalkSettings {
            success: SuccessRecording::Record,
            ..settings
        };
        assert_eq!(
            finish_success(&requested, directory.path(), None).unwrap_err(),
            "a requested successful recording must retain the final case"
        );
    }

    #[test]
    fn the_walk_reports_an_abort_and_a_trace_that_stopped_failing() {
        let settings = bounded_settings();
        let directory = tempfile::tempdir().unwrap();
        let abort = finish_walk(
            &settings,
            directory.path(),
            Err(TestError::Abort(Reason::from("no case".to_owned()))),
            None,
            None,
            &stub_success,
        )
        .unwrap_err();
        assert!(
            abort.contains("aborted before it produced a case"),
            "{abort}"
        );

        let unlatched = finish_walk(
            &settings,
            directory.path(),
            Err(TestError::Fail(
                Reason::from("stub".to_owned()),
                vec![RandomOperation::Focus { gained: true }],
            )),
            None,
            None,
            &stub_success,
        )
        .unwrap_err();
        assert!(unlatched.contains("failed on no profile"), "{unlatched}");

        let stopped = finish_walk(
            &settings,
            directory.path(),
            Err(TestError::Fail(
                Reason::from("stub".to_owned()),
                vec![RandomOperation::Focus { gained: true }],
            )),
            None,
            Some((Locale::En, Size::new(80, 24))),
            &stub_success,
        )
        .unwrap_err();
        assert!(stopped.contains(REPLAY_STOPPED), "{stopped}");
        let bundle = one_bundle(directory.path(), "failure-");
        let repro: Value =
            serde_json::from_slice(&fs::read(bundle.join("repro.json")).unwrap()).unwrap();
        assert_eq!(repro["error"], REPLAY_STOPPED);
    }

    #[test]
    fn the_failure_bundle_names_the_profile_that_the_walk_latched() {
        let directory = tempfile::tempdir().unwrap();
        let settings = WalkSettings {
            mode: ProfileMode::Complete,
            ..bounded_settings()
        };
        let latched = (Locale::ZhTw, Size::new(40, 40));
        assert_ne!(walk_profile_matrix(ProfileMode::Complete)[0], latched);

        let error = finish_walk(
            &settings,
            directory.path(),
            Err(TestError::Fail(
                Reason::from("stub".to_owned()),
                vec![RandomOperation::Focus { gained: true }],
            )),
            None,
            Some(latched),
            &stub_failure,
        )
        .unwrap_err();

        assert!(error.contains("found a minimal failure"), "{error}");
        let bundle = one_bundle(directory.path(), "failure-");
        let repro: Value =
            serde_json::from_slice(&fs::read(bundle.join("repro.json")).unwrap()).unwrap();
        assert_eq!(repro["locale"], "zh-TW");
        assert_eq!(repro["initial_size"]["cols"], 40);
        assert_eq!(repro["initial_size"]["rows"], 40);
    }

    #[test]
    fn a_failing_artifact_write_keeps_the_property_failure() {
        let parent = tempfile::tempdir().unwrap();
        let blocked = parent.path().join("blocked");
        fs::write(&blocked, b"not a directory").unwrap();
        let settings = bounded_settings();
        let error = finish_walk(
            &settings,
            &blocked,
            Err(TestError::Fail(
                Reason::from("stub".to_owned()),
                vec![RandomOperation::Focus { gained: true }],
            )),
            None,
            Some((Locale::En, Size::new(80, 24))),
            &stub_failure,
        )
        .unwrap_err();

        assert!(error.contains("artifact capture also failed"), "{error}");
    }

    #[test]
    fn the_complete_mode_replays_every_profile_and_keeps_its_seed_file() {
        assert_eq!(walk_profile_matrix(ProfileMode::Complete).len(), 15);
        assert_eq!(walk_profile_matrix(ProfileMode::Bounded), BOUNDED_PROFILES);
        assert!(walk_persistence(ProfileMode::Complete).is_some());
        assert!(walk_persistence(ProfileMode::Bounded).is_some());
        assert!(walk_seed_hint(ProfileMode::Complete).contains("regressions.txt"));
        let settings = WalkSettings {
            mode: ProfileMode::Complete,
            ..bounded_settings()
        };
        let config = walk_config(&settings);
        assert_eq!(config.cases, 2);
        assert_eq!(config.max_shrink_iters, MAX_SHRINK_ITERS);
        let runner = walk_runner(ProfileMode::Complete, walk_config(&settings));
        assert_eq!(runner.config().cases, 2);
    }

    #[test]
    fn successful_cases_use_the_configured_operation_count() {
        let settings = WalkSettings {
            cases: 8,
            steps: 100,
            ..bounded_settings()
        };
        let lengths = RefCell::new(Vec::new());
        let result = run_walk_cases(&settings, &|_, operations| {
            lengths.borrow_mut().push(operations.len());
            Ok(CaseSuccess::default())
        });
        assert!(result.outcome.is_ok());
        assert_eq!(*lengths.borrow(), vec![100; 8]);
    }

    #[test]
    fn successful_replay_refuses_extra_operations_and_changed_skips() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("repro.json");
        let profile = (Locale::En, Size::new(80, 24));
        let operations = vec![RandomOperation::Focus { gained: true }; 2];
        let case = stub_success(&profile, &operations).unwrap();
        let repro = repro_value(
            &profile,
            &operations,
            &CaseRecord {
                resolved: &case.operations,
                skipped: &case.skipped,
            },
            None,
        )
        .unwrap();
        fs::write(&path, serde_json::to_vec(&repro).unwrap()).unwrap();
        for change_operations in [true, false] {
            let error = replay_repro(&path, &|_, _| {
                let mut changed = case.clone();
                if change_operations {
                    changed
                        .operations
                        .push(json!({"operation": "focus", "gained": false}));
                } else {
                    changed.skipped[0]["reason"] = json!("different");
                }
                Ok(changed)
            })
            .expect_err("a successful replay must retain all binding decisions");
            assert!(error.contains("different"), "{error}");
        }
    }

    #[test]
    fn a_saved_description_replays_through_the_same_adapter() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("repro.json");
        let operations = vec![RandomOperation::Focus { gained: true }; 2];
        let repro = repro_value(
            &(Locale::ZhTw, Size::new(24, 6)),
            &operations,
            &CaseRecord {
                resolved: &[json!({"operation": "focus", "gained": true})],
                skipped: &[json!({"index": 0, "reason": "unavailable"})],
            },
            None,
        )
        .unwrap();
        fs::write(&path, serde_json::to_vec(&repro).unwrap()).unwrap();

        let settings = WalkSettings {
            repro: ReproSource::Replay(path.clone()),
            ..bounded_settings()
        };
        run_walk_or_replay(&settings, directory.path(), &stub_success).unwrap();

        let drifted =
            replay_repro(&path, &|_profile, _operations| Ok(CaseSuccess::default())).unwrap_err();
        assert!(
            drifted.contains("resolved a different operation vector"),
            "{drifted}"
        );
        let failed = replay_repro(&path, &|profile, operations| {
            let mut failure = stub_failure(profile, operations).unwrap_err();
            failure.skipped = stub_success(profile, operations).unwrap().skipped;
            Err(failure)
        })
        .unwrap_err();
        assert!(
            failed.contains("broke one rule that the recorded walk kept"),
            "{failed}"
        );
        let absent =
            replay_repro(&directory.path().join("absent.json"), &stub_success).unwrap_err();
        assert!(absent.contains("cannot read"), "{absent}");
    }

    #[test]
    fn a_truncated_failure_bundle_replays_while_the_rule_breaks_and_after_it_is_repaired() {
        let directory = tempfile::tempdir().unwrap();
        let settings = bounded_settings();
        let minimal = vec![
            RandomOperation::Focus { gained: true },
            RandomOperation::Focus { gained: false },
            RandomOperation::Focus { gained: true },
        ];
        let panic_message = finish_walk(
            &settings,
            directory.path(),
            Err(TestError::Fail(Reason::from("stub".to_owned()), minimal)),
            None,
            Some((Locale::En, Size::new(80, 24))),
            &truncated_failure,
        )
        .unwrap_err();
        assert!(
            panic_message.contains("the second operation broke one rule"),
            "{panic_message}"
        );

        let repro_path = one_bundle(directory.path(), "failure-").join(REPRO_NAME);
        let recorded: Value = serde_json::from_slice(&fs::read(&repro_path).unwrap()).unwrap();
        assert_eq!(recorded["resolved"].as_array().unwrap().len(), 2);
        assert_eq!(recorded["operations"].as_array().unwrap().len(), 3);

        let live = replay_repro(&repro_path, &truncated_failure).unwrap_err();
        assert!(
            live.contains("the recorded rule still breaks: the second operation broke one rule"),
            "{live}"
        );

        replay_repro(&repro_path, &repaired_walk).unwrap();
    }

    #[test]
    fn failure_replay_compares_skips_through_the_recorded_boundary() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("repro.json");
        let profile = (Locale::En, Size::new(80, 24));
        let operations = vec![RandomOperation::Focus { gained: true }; 4];
        let resolutions = focus_values(1);
        let skips = vec![json!({"index": 0, "reason": "unavailable"})];
        let repro = repro_value(
            &profile,
            &operations,
            &CaseRecord {
                resolved: &resolutions,
                skipped: &skips,
            },
            Some("the second operation broke one rule"),
        )
        .unwrap();
        fs::write(&path, serde_json::to_vec(&repro).unwrap()).unwrap();
        for next_skip in [1, 2] {
            let replayed = CaseSuccess {
                operations: resolutions.clone(),
                skipped: vec![
                    skips[0].clone(),
                    json!({"index": next_skip, "reason": "unavailable"}),
                ],
                ..CaseSuccess::default()
            };
            let result = replay_repro(&path, &|_, _| Ok(replayed.clone()));
            assert_eq!(result.is_ok(), next_skip == 2, "{result:?}");
        }
        let mut missing_skips = repro;
        missing_skips.as_object_mut().unwrap().remove("skipped");
        fs::write(&path, serde_json::to_vec(&missing_skips).unwrap()).unwrap();
        assert_eq!(
            replay_repro(&path, &stub_success).unwrap_err(),
            "the replay description needs a skipped operation vector"
        );
    }

    #[test]
    fn one_replay_names_whether_the_recorded_rule_still_breaks() {
        assert_eq!(compare_replay_error(None, None), Ok(()));
        assert_eq!(compare_replay_error(Some("broken"), None), Ok(()));
        assert!(
            compare_replay_error(None, Some("fresh"))
                .unwrap_err()
                .contains("broke one rule that the recorded walk kept")
        );
        assert!(
            compare_replay_error(Some("broken"), Some("broken"))
                .unwrap_err()
                .contains("the recorded rule still breaks")
        );
        assert!(
            compare_replay_error(Some("broken"), Some("other"))
                .unwrap_err()
                .contains("broke a different rule")
        );
    }

    #[test]
    fn a_failed_diagnostic_capture_keeps_its_own_name() {
        assert_eq!(diagnostic_failure(Ok(())), Ok(()));
        assert_eq!(
            diagnostic_failure(Err("closed backend".to_owned())).unwrap_err(),
            "the walk could not record its diagnostic frame: closed backend"
        );
        assert_eq!(after_diagnostic(Ok(()), Ok(())), Ok(()));
        assert_eq!(
            after_diagnostic(Err("broken".to_owned()), Ok(())).unwrap_err(),
            "broken"
        );
        assert_eq!(
            after_diagnostic(Ok(()), Err("no frame".to_owned())).unwrap_err(),
            "no frame"
        );
        assert_eq!(
            after_diagnostic(Err("broken".to_owned()), Err("no frame".to_owned())).unwrap_err(),
            "broken; no frame"
        );
    }

    #[test]
    fn a_replay_description_needs_a_complete_profile_and_vector() {
        assert!(
            repro_profile(&json!({}))
                .unwrap_err()
                .contains("locale tag")
        );
        assert!(
            repro_profile(&json!({"locale": "en"}))
                .unwrap_err()
                .contains("column count")
        );
        assert!(
            repro_profile(&json!({"locale": "en", "initial_size": {"cols": 80}}))
                .unwrap_err()
                .contains("row count")
        );
        assert_eq!(
            repro_profile(&json!({"locale": "en", "initial_size": {"cols": 80, "rows": 24}}))
                .unwrap(),
            (Locale::En, Size::new(80, 24))
        );
        assert!(
            repro_profile(&json!({"locale": "fr", "initial_size": {"cols": 80, "rows": 24}}))
                .unwrap_err()
                .contains("unknown locale: fr")
        );

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("repro.json");
        fs::write(&path, b"{").unwrap();
        assert!(
            replay_repro(&path, &stub_success)
                .unwrap_err()
                .contains("EOF")
        );
        fs::write(&path, serde_json::to_vec(&json!({"locale": "en"})).unwrap()).unwrap();
        assert!(
            replay_repro(&path, &stub_success)
                .unwrap_err()
                .contains("needs a version")
        );
        fs::write(&path, serde_json::to_vec(&json!({"version": 2})).unwrap()).unwrap();
        assert!(
            replay_repro(&path, &stub_success)
                .unwrap_err()
                .contains("has version 2")
        );
        fs::write(
            &path,
            serde_json::to_vec(&json!({
                "version": 1,
                "locale": "en",
                "initial_size": {"cols": 80, "rows": 24},
                "operations": "not a vector",
            }))
            .unwrap(),
        )
        .unwrap();
        assert!(
            replay_repro(&path, &stub_success)
                .unwrap_err()
                .contains("no operation vector")
        );
        fs::write(
            &path,
            serde_json::to_vec(&json!({
                "version": 1,
                "locale": "en",
                "initial_size": {"cols": 80, "rows": 24},
                "operations": [],
            }))
            .unwrap(),
        )
        .unwrap();
        assert!(
            replay_repro(&path, &stub_success)
                .unwrap_err()
                .contains("needs a resolved operation vector")
        );
    }

    #[test]
    fn one_cleanup_failure_never_hides_the_primary_failure() {
        assert_eq!(after_close(Ok(()), Ok(())), Ok(()));
        assert_eq!(
            after_close(Ok(()), Err("closed".to_owned())).unwrap_err(),
            "the random walk could not close a host: closed"
        );
        assert_eq!(
            after_close(Err("broken".to_owned()), Ok(())).unwrap_err(),
            "broken"
        );
        assert_eq!(
            after_close(Err("broken".to_owned()), Err("closed".to_owned())).unwrap_err(),
            "broken; the host cleanup also failed: closed"
        );
    }

    #[test]
    fn every_panic_payload_reports_one_message() {
        let text: Box<dyn Any + Send> = Box::new("literal panic");
        let owned: Box<dyn Any + Send> = Box::new("owned panic".to_owned());
        let other: Box<dyn Any + Send> = Box::new(7_u8);

        assert_eq!(panic_message(text.as_ref()), "literal panic");
        assert_eq!(panic_message(owned.as_ref()), "owned panic");
        assert_eq!(panic_message(other.as_ref()), "non-text panic");
    }
}
