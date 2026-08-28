//! Crossterm lifecycle and blocking event loop.

use std::io;
use std::sync::Arc;
use std::time::Duration;

use ratatui_core::terminal::Terminal;
use ratatui_crossterm::{
    CrosstermBackend,
    crossterm::{
        event::{self, DisableMouseCapture, EnableMouseCapture},
        execute,
        terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
    },
};
use skit_application::path_completion::PathCompletionProvider;
use skit_domain::Slug;
use skit_i18n::{Locale, Localize, Message, detect_locale};
use skit_ui::{
    Action, AddWorkflowState, Effect, FormView, LibraryState, RunFormView, Screen, SubmittedValues,
};
use thiserror::Error;

use crate::{EventHandling, TuiSession, ViewGeometry, render_with_session};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TerminalEventWait {
    Blocking,
    Poll(Duration),
}

pub(crate) const fn terminal_event_wait(path_completion_pending: bool) -> TerminalEventWait {
    if path_completion_pending {
        TerminalEventWait::Poll(Duration::from_millis(25))
    } else {
        TerminalEventWait::Blocking
    }
}

fn read_terminal_event(wait: TerminalEventWait) -> io::Result<Option<event::Event>> {
    read_terminal_event_with(wait, event::read, event::poll)
}

/// The refusal a session gets when its standard streams are not a terminal, if any.
///
/// The two answers arrive as parameters, so a test can drive every combination without changing
/// the process's own streams. An interactive session needs both: input reads keys from stdin, and
/// every frame draws to stdout.
fn terminal_claim_refusal(stdin_is_terminal: bool, stdout_is_terminal: bool) -> Option<io::Error> {
    if stdin_is_terminal && stdout_is_terminal {
        None
    } else {
        Some(io::Error::new(
            io::ErrorKind::NotConnected,
            "stdin and stdout are not a terminal",
        ))
    }
}

/// Claim the terminal for a session: refuse a non-terminal, then enter raw mode and the
/// alternate screen.
///
/// The explicit check is the one cross-platform guard. Unix raw mode fails on its own for a
/// piped process, but Windows crossterm attaches to the process console even when the standard
/// streams are redirected, and the event loop would then wait on a console no caller can type
/// into. The CLI enforces the same policy at every prompt door (`is_terminal` on both streams).
fn claim_terminal() -> io::Result<()> {
    use std::io::IsTerminal as _;
    if let Some(refusal) =
        terminal_claim_refusal(io::stdin().is_terminal(), io::stdout().is_terminal())
    {
        return Err(refusal);
    }
    let enter = || execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture);
    claim_terminal_with(enable_raw_mode, enter, restore_terminal)
}

/// Restore both terminal modes after a completed session or a partial claim.
///
/// Raw mode and the alternate screen are independent terminal resources. Restoration always
/// attempts both and keeps the first restoration error. A failed claim uses this function for
/// best-effort rollback and keeps the original claim error.
fn restore_terminal() -> io::Result<()> {
    restorative_terminal_transition(disable_raw_mode, || {
        execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture)
    })
}

fn claim_terminal_with<E, T, R>(enable: E, transition: T, rollback: R) -> io::Result<()>
where
    E: FnOnce() -> io::Result<()>,
    T: FnOnce() -> io::Result<()>,
    R: FnOnce() -> io::Result<()>,
{
    enable()?;
    match transition() {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = rollback();
            Err(error)
        }
    }
}

/// Take the next event, or report that the poll window closed with nothing to take.
///
/// The two terminal calls arrive as parameters, so a test can drive every answer without a live
/// terminal and without waiting on one. A blocking wait must never poll: it has nothing else to do
/// until a key arrives.
fn read_terminal_event_with<R, P>(
    wait: TerminalEventWait,
    read: R,
    poll: P,
) -> io::Result<Option<event::Event>>
where
    R: FnOnce() -> io::Result<event::Event>,
    P: FnOnce(Duration) -> io::Result<bool>,
{
    match wait {
        TerminalEventWait::Blocking => read().map(Some),
        TerminalEventWait::Poll(duration) => {
            if poll(duration)? {
                read().map(Some)
            } else {
                Ok(None)
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct TerminalDispatch {
    action: Option<Action>,
    redraw: bool,
}

const fn request_redraw(current: bool, changed: bool) -> bool {
    current || changed
}

fn dispatch_event(
    session: &mut TuiSession,
    event: event::Event,
    state: &LibraryState,
    geometry: &ViewGeometry,
) -> TerminalDispatch {
    let resized = matches!(event, event::Event::Resize(_, _));
    match session.handle_event(event, state, geometry) {
        EventHandling::Action(action) => TerminalDispatch {
            action: Some(action),
            redraw: true,
        },
        EventHandling::Consumed => TerminalDispatch {
            action: None,
            redraw: true,
        },
        EventHandling::Ignored => TerminalDispatch {
            action: None,
            redraw: resized,
        },
    }
}

/// A terminal lifecycle or input failure.
#[derive(Debug, Error)]
pub enum TuiError {
    /// Crossterm or terminal backend I/O failed.
    #[error("terminal I/O failed: {0}")]
    Io(#[from] io::Error),
    /// A host/reducer cycle did not reach a stable state.
    #[error("terminal host effects did not settle")]
    EffectCycle,
}

impl Localize for TuiError {
    fn message(&self) -> Message {
        match self {
            Self::Io(error) => Message::new("terminal I/O failed: {}").with(error),
            Self::EffectCycle => Message::new("terminal host effects did not settle"),
        }
    }
}

/// Run the terminal frontend and send each requested effect to its host adapter.
pub fn run<F, E>(state: LibraryState, host: F, locale: Locale) -> Result<(), TuiError>
where
    F: FnMut(Effect) -> Result<Action, E>,
    E: Localize,
{
    run_preflighted(state, |_| Ok::<(), E>(()), host, locale)
}

/// Run the terminal frontend with asynchronous path completion.
pub fn run_with_path_completion<F, E>(
    state: LibraryState,
    host: F,
    locale: Locale,
    provider: Arc<dyn PathCompletionProvider>,
) -> Result<(), TuiError>
where
    F: FnMut(Effect) -> Result<Action, E>,
    E: Localize,
{
    run_preflighted_with_path_completion(state, |_| Ok::<(), E>(()), host, locale, provider)
}

/// Run the terminal frontend with a check that occurs before terminal suspension.
///
/// The check must be local and read-only. A refusal stays on the active screen as a localized
/// status. The host does not receive the refused effect.
pub fn run_preflighted<F, P, E>(
    state: LibraryState,
    preflight: P,
    host: F,
    locale: Locale,
) -> Result<(), TuiError>
where
    F: FnMut(Effect) -> Result<Action, E>,
    P: FnMut(&Effect) -> Result<(), E>,
    E: Localize,
{
    let _: Option<()> =
        run_hosted_state(state, Vec::new(), preflight, host, locale, |_| None, None)?;
    Ok(())
}

/// Run with both a pre-suspend check and asynchronous path completion.
pub fn run_preflighted_with_path_completion<F, P, E>(
    state: LibraryState,
    preflight: P,
    host: F,
    locale: Locale,
    provider: Arc<dyn PathCompletionProvider>,
) -> Result<(), TuiError>
where
    F: FnMut(Effect) -> Result<Action, E>,
    P: FnMut(&Effect) -> Result<(), E>,
    E: Localize,
{
    let _: Option<()> = run_hosted_state(
        state,
        Vec::new(),
        preflight,
        host,
        locale,
        |_| None,
        Some(provider),
    )?;
    Ok(())
}

/// Run the typed add workflow with the same hosted reducer loop as the library workbench.
///
/// The result is the created slug, or `None` when the user cancels or quits. Completion and
/// cancellation use explicit reducer actions. The adapter does not infer an outcome from library
/// selection or status text.
pub fn run_add_workflow<F, E>(
    workflow: AddWorkflowState,
    opening: Vec<Action>,
    host: F,
    locale: Locale,
) -> Result<Option<Slug>, TuiError>
where
    F: FnMut(Effect) -> Result<Action, E>,
    E: Localize,
{
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Add(Box::new(workflow))));
    run_hosted_state(
        state,
        opening,
        |_| Ok::<(), E>(()),
        host,
        locale,
        add_workflow_outcome,
        None,
    )
    .map(|outcome| {
        outcome.and_then(|outcome| match outcome {
            AddWorkflowOutcome::Completed(slug) => Some(slug),
            AddWorkflowOutcome::Cancelled => None,
        })
    })
}

/// Drive one hosted screen until it reports an outcome.
///
/// `opening` is applied before the first draw, through the same reducer and the same host as a key
/// press. A shell that already named its subject — `skit add greet.py` — arrives at the panel the
/// subject belongs to rather than at a source picker asking for what it was just given
/// (`src/skit/cli.py:2116-2126`). Replaying it as actions rather than as a second construction path
/// means the shell door and the `a` door cannot answer the same command differently.
fn run_hosted_state<F, P, E, O>(
    mut state: LibraryState,
    opening: Vec<Action>,
    mut preflight: P,
    mut host: F,
    mut locale: Locale,
    mut observe: impl FnMut(&Action) -> Option<O>,
    path_completion: Option<Arc<dyn PathCompletionProvider>>,
) -> Result<Option<O>, TuiError>
where
    F: FnMut(Effect) -> Result<Action, E>,
    P: FnMut(&Effect) -> Result<(), E>,
    E: Localize,
{
    for action in opening {
        if let Some(outcome) = observe(&action) {
            return Ok(Some(outcome));
        }
        let effect = state.update(action);
        if !matches!(effect, Effect::None) {
            let (quit, outcome) = drain_host_effects_observed(
                &mut state,
                &mut host,
                effect,
                &mut locale,
                &mut observe,
            )?;
            if quit || outcome.is_some() {
                return Ok(outcome);
            }
        }
    }
    claim_terminal()?;
    let _restore = RestoreTerminal::new(restore_terminal);
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;
    let mut session = path_completion.map_or_else(TuiSession::default, |provider| {
        TuiSession::with_path_completion(provider)
    });

    let mut geometry = ViewGeometry::default();
    let mut redraw = true;
    let outcome = loop {
        redraw = request_redraw(redraw, session.refresh_background());
        if redraw {
            terminal.draw(|frame| {
                geometry = render_with_session(frame, &state, locale, &mut session);
            })?;
            redraw = false;
        }
        let Some(event) =
            read_terminal_event(terminal_event_wait(session.has_pending_path_completion()))?
        else {
            continue;
        };
        let dispatched = dispatch_event(&mut session, event, &state, &geometry);
        redraw = request_redraw(redraw, dispatched.redraw);
        if let Some(action) = dispatched.action {
            let step = advance_hosted_action(
                &mut state,
                action,
                &mut preflight,
                &mut host,
                &mut locale,
                &mut observe,
                &mut |transition| match transition {
                    HostedTerminalTransition::Suspend => {
                        terminal.show_cursor()?;
                        sequential_terminal_transition(disable_raw_mode, || {
                            execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture)
                        })
                    }
                    HostedTerminalTransition::Resume => {
                        sequential_terminal_transition(enable_raw_mode, || {
                            execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)
                        })?;
                        terminal.clear()
                    }
                },
            )?;
            if let HostedActionStep::Finish(outcome) = step {
                break outcome;
            }
        }
    };
    terminal.show_cursor()?;
    Ok(outcome)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HostedTerminalTransition {
    Suspend,
    Resume,
}

#[derive(Debug, Eq, PartialEq)]
enum HostedActionStep<O> {
    Continue,
    Finish(Option<O>),
}

/// Apply one dispatched action and report whether the hosted loop continues.
///
/// The transition callback is the terminal boundary. Production leaves and re-enters the
/// alternate screen there. Tests use the same path with a recorded fake lifecycle.
fn advance_hosted_action<F, P, E, O>(
    state: &mut LibraryState,
    action: Action,
    preflight: &mut P,
    host: &mut F,
    locale: &mut Locale,
    observe: &mut impl FnMut(&Action) -> Option<O>,
    transition: &mut impl FnMut(HostedTerminalTransition) -> io::Result<()>,
) -> Result<HostedActionStep<O>, TuiError>
where
    F: FnMut(Effect) -> Result<Action, E>,
    P: FnMut(&Effect) -> Result<(), E>,
    E: Localize,
{
    let mut outcome = observe(&action);
    let effect = state.update(action);
    match effect {
        Effect::None => {
            if outcome.is_some() {
                Ok(HostedActionStep::Finish(outcome))
            } else {
                Ok(HostedActionStep::Continue)
            }
        }
        Effect::Quit => Ok(HostedActionStep::Finish(outcome)),
        effect => {
            if !accept_host_effect(state, &effect, preflight, *locale) {
                return Ok(HostedActionStep::Continue);
            }
            transition(HostedTerminalTransition::Suspend)?;
            let (quit, host_outcome) =
                drain_host_effects_observed(state, host, effect, locale, observe)?;
            if outcome.is_none() {
                outcome = host_outcome;
            }
            if quit {
                return Ok(HostedActionStep::Finish(outcome));
            }
            if outcome.is_some() {
                return Ok(HostedActionStep::Finish(outcome));
            }
            transition(HostedTerminalTransition::Resume)?;
            Ok(HostedActionStep::Continue)
        }
    }
}

fn accept_host_effect<P, E>(
    state: &mut LibraryState,
    effect: &Effect,
    preflight: &mut P,
    locale: Locale,
) -> bool
where
    P: FnMut(&Effect) -> Result<(), E>,
    E: Localize,
{
    match preflight(effect) {
        Ok(()) => true,
        Err(error) => {
            let status = Message::new("Error: {}")
                .nested(error.message())
                .localize(locale);
            state.update(Action::SetStatus(status));
            false
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum AddWorkflowOutcome {
    Completed(Slug),
    Cancelled,
}

fn add_workflow_outcome(action: &Action) -> Option<AddWorkflowOutcome> {
    match action {
        Action::AddCompleted { slug, .. } => Some(AddWorkflowOutcome::Completed(slug.clone())),
        Action::AddCancelled | Action::Quit => Some(AddWorkflowOutcome::Cancelled),
        _ => None,
    }
}

fn localized_status(error: &impl Localize, locale: Locale) -> String {
    error.message().localize(locale)
}

fn action_locale(action: &Action) -> Option<Locale> {
    let Action::PreferencesSaved { locale, .. } = action else {
        return None;
    };
    Some(detect_locale(Some(locale)))
}

const HOST_EFFECT_LIMIT: usize = 64;

#[cfg(test)]
fn drain_host_effects<F, E>(
    state: &mut LibraryState,
    host: &mut F,
    effect: Effect,
    locale: &mut Locale,
) -> Result<bool, TuiError>
where
    F: FnMut(Effect) -> Result<Action, E>,
    E: Localize,
{
    let (quit, _): (bool, Option<()>) =
        drain_host_effects_observed(state, host, effect, locale, &mut |_| None)?;
    Ok(quit)
}

fn drain_host_effects_observed<F, E, O>(
    state: &mut LibraryState,
    host: &mut F,
    mut effect: Effect,
    locale: &mut Locale,
    observe: &mut impl FnMut(&Action) -> Option<O>,
) -> Result<(bool, Option<O>), TuiError>
where
    F: FnMut(Effect) -> Result<Action, E>,
    E: Localize,
{
    let mut outcome = None;
    for _ in 0..HOST_EFFECT_LIMIT {
        match effect {
            Effect::None => return Ok((false, outcome)),
            Effect::Quit => return Ok((true, outcome)),
            current => match host(current) {
                Ok(action) => {
                    if outcome.is_none() {
                        outcome = observe(&action);
                    }
                    if let Some(next_locale) = action_locale(&action) {
                        *locale = next_locale;
                    }
                    effect = state.update(action);
                }
                Err(error) => {
                    state.update(Action::SetStatus(localized_status(&error, *locale)));
                    return Ok((false, outcome));
                }
            },
        }
    }
    Err(TuiError::EffectCycle)
}

/// Collect one generic form and restore the terminal before returning its values.
pub fn collect_form<F, E>(
    form: FormView,
    host: F,
    locale: Locale,
) -> Result<Option<SubmittedValues>, TuiError>
where
    F: FnMut(Effect) -> Result<Action, E>,
    E: Localize,
{
    collect_screen(Screen::Form(form), host, locale, None)
}

/// Collect one typed launch form and restore the terminal before returning its values.
///
/// The host serves the same effects the full workbench serves. A form chip that the footer
/// advertises — `Ctrl+S Save as preset` above all — must do its work here too, exactly as version
/// 0.4 saves a preset from the inline run window (`src/skit/tui_form.py:929-959`).
pub fn collect_run_form<F, E>(
    form: RunFormView,
    host: F,
    locale: Locale,
) -> Result<Option<SubmittedValues>, TuiError>
where
    F: FnMut(Effect) -> Result<Action, E>,
    E: Localize,
{
    collect_screen(Screen::Run(Box::new(form)), host, locale, None)
}

/// Collect one typed launch form with asynchronous path completion.
pub fn collect_run_form_with_path_completion<F, E>(
    form: RunFormView,
    host: F,
    locale: Locale,
    provider: Arc<dyn PathCompletionProvider>,
) -> Result<Option<SubmittedValues>, TuiError>
where
    F: FnMut(Effect) -> Result<Action, E>,
    E: Localize,
{
    collect_screen(Screen::Run(Box::new(form)), host, locale, Some(provider))
}

fn collect_screen<F, E>(
    screen: Screen,
    mut host: F,
    mut locale: Locale,
    path_completion: Option<Arc<dyn PathCompletionProvider>>,
) -> Result<Option<SubmittedValues>, TuiError>
where
    F: FnMut(Effect) -> Result<Action, E>,
    E: Localize,
{
    let mut state = LibraryState::default();
    state.update(Action::Present(screen));
    claim_terminal()?;
    let _restore = RestoreTerminal::new(restore_terminal);
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;
    let mut session = path_completion.map_or_else(TuiSession::default, |provider| {
        TuiSession::with_path_completion(provider)
    });

    let mut geometry = ViewGeometry::default();
    let mut redraw = true;
    let values = loop {
        redraw = request_redraw(redraw, session.refresh_background());
        if redraw {
            terminal.draw(|frame| {
                geometry = render_with_session(frame, &state, locale, &mut session);
            })?;
            redraw = false;
        }
        let Some(event) =
            read_terminal_event(terminal_event_wait(session.has_pending_path_completion()))?
        else {
            continue;
        };
        let dispatched = dispatch_event(&mut session, event, &state, &geometry);
        redraw = request_redraw(redraw, dispatched.redraw);
        if let Some(action) = dispatched.action {
            match collect_action_step(&mut state, action) {
                CollectActionStep::Cancel => break None,
                CollectActionStep::Effect(effect) => {
                    if let Some(values) =
                        drain_collect_effects(&mut state, &mut host, effect, &mut locale)?
                    {
                        break values;
                    }
                }
            }
        }
    };
    terminal.show_cursor()?;
    Ok(values)
}

#[derive(Debug, PartialEq)]
enum CollectActionStep {
    Cancel,
    Effect(Effect),
}

fn collect_action_step(state: &mut LibraryState, action: Action) -> CollectActionStep {
    if matches!(action, Action::Quit) {
        return CollectActionStep::Cancel;
    }
    // Escape inside a modal closes only that modal and keeps the form
    // (`src/skit/tui_form.py:376-377` `action_cancel` dismisses the preset modal). The form itself
    // is the last screen here, so Escape outside a modal cancels the collection.
    if matches!(action, Action::Back) && state.modal().is_none() {
        return CollectActionStep::Cancel;
    }
    CollectActionStep::Effect(state.update(action))
}

/// Serve one effect chain for a collected form.
///
/// `Ok(None)` continues the event loop. `Ok(Some(values))` finishes the collection, and
/// `Ok(Some(None))` inside it means the user quit.
type CollectOutcome = Option<Option<SubmittedValues>>;

fn drain_collect_effects<F, E>(
    state: &mut LibraryState,
    host: &mut F,
    mut effect: Effect,
    locale: &mut Locale,
) -> Result<CollectOutcome, TuiError>
where
    F: FnMut(Effect) -> Result<Action, E>,
    E: Localize,
{
    for _ in 0..HOST_EFFECT_LIMIT {
        match effect {
            Effect::None => return Ok(None),
            Effect::Quit => return Ok(Some(None)),
            Effect::Submit { values, .. } => return Ok(Some(Some(values))),
            current => match host(current) {
                Ok(action) => {
                    if let Some(next_locale) = action_locale(&action) {
                        *locale = next_locale;
                    }
                    effect = state.update(action);
                }
                Err(error) => {
                    state.update(Action::SetStatus(localized_status(&error, *locale)));
                    return Ok(None);
                }
            },
        }
    }
    Err(TuiError::EffectCycle)
}

fn sequential_terminal_transition<F, S>(first: F, second: S) -> io::Result<()>
where
    F: FnOnce() -> io::Result<()>,
    S: FnOnce() -> io::Result<()>,
{
    first()?;
    second()
}

fn restorative_terminal_transition<F, S>(first: F, second: S) -> io::Result<()>
where
    F: FnOnce() -> io::Result<()>,
    S: FnOnce() -> io::Result<()>,
{
    let first = first();
    let second = second();
    first.and(second)
}

struct RestoreTerminal<F>
where
    F: FnOnce() -> io::Result<()>,
{
    restore: Option<F>,
}

impl<F> RestoreTerminal<F>
where
    F: FnOnce() -> io::Result<()>,
{
    fn new(restore: F) -> Self {
        Self {
            restore: Some(restore),
        }
    }
}

impl<F> Drop for RestoreTerminal<F>
where
    F: FnOnce() -> io::Result<()>,
{
    fn drop(&mut self) {
        if let Some(restore) = self.restore.take() {
            let _ = restore();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_run_form() -> RunFormView {
        use std::collections::BTreeMap;

        RunFormView::from_declarations(
            "demo",
            "Demo",
            &[],
            &BTreeMap::new(),
            &[],
            "",
            &BTreeMap::new(),
            "",
        )
    }

    /// Every stream combination answers the same way on every host: only two real terminals
    /// proceed, and each refusal names both streams so the caller's report is exact.
    #[test]
    fn terminal_claim_refuses_every_stream_shape_that_is_not_two_terminals() {
        assert!(terminal_claim_refusal(true, true).is_none());
        for (stdin_is_terminal, stdout_is_terminal) in
            [(false, true), (true, false), (false, false)]
        {
            let refusal = terminal_claim_refusal(stdin_is_terminal, stdout_is_terminal)
                .expect("a non-terminal stream must refuse");
            assert_eq!(refusal.kind(), io::ErrorKind::NotConnected);
            assert_eq!(refusal.to_string(), "stdin and stdout are not a terminal");
        }
    }

    #[test]
    fn terminal_claim_rolls_back_when_the_screen_transition_fails() {
        use std::cell::Cell;

        let step_calls = Cell::new(0_usize);
        let step = || {
            step_calls.set(step_calls.get().saturating_add(1));
            Ok(())
        };
        let error = claim_terminal_with(
            || Ok(()),
            || Err(io::Error::other("screen transition failed")),
            step,
        )
        .unwrap_err();

        assert_eq!(error.to_string(), "screen transition failed");
        assert_eq!(step_calls.get(), 1, "raw mode was not rolled back");

        step_calls.set(0);
        let error = claim_terminal_with(|| Err(io::Error::other("raw mode failed")), step, step)
            .unwrap_err();
        assert_eq!(error.to_string(), "raw mode failed");
        assert_eq!(
            step_calls.get(),
            0,
            "a failed raw-mode claim ran a later terminal step"
        );
    }

    #[test]
    fn redraw_requests_are_monotonic_until_the_loop_draws() {
        assert!(!request_redraw(false, false));
        assert!(request_redraw(false, true));
        assert!(request_redraw(true, false));
        assert!(request_redraw(true, true));
    }

    #[derive(Debug)]
    struct HostError;

    impl Localize for HostError {
        fn message(&self) -> Message {
            Message::new("entry not found: {}").with("demo")
        }
    }

    fn harmless_host(_effect: Effect) -> Result<Action, HostError> {
        Ok(Action::ClearStatus)
    }

    #[test]
    fn terminal_takes_an_event_for_every_wait_the_loop_can_choose() {
        use std::cell::Cell;

        const WINDOW: Duration = Duration::from_millis(25);

        // One pair of stubs serves every case, and each keeps a count. A case that must not reach a
        // call proves it by the count standing still, so nothing here is written to never run.
        let reads = Cell::new(0_u32);
        let polls = Cell::new(0_u32);
        let asked_for = Cell::new(Duration::ZERO);
        let window_has_event = Cell::new(true);
        let read_fails = Cell::new(false);
        let poll_fails = Cell::new(false);

        let read = || {
            reads.set(reads.get() + 1);
            if read_fails.get() {
                Err(io::Error::other("read failed"))
            } else {
                Ok(event::Event::FocusGained)
            }
        };
        let poll = |waited: Duration| {
            polls.set(polls.get() + 1);
            asked_for.set(waited);
            if poll_fails.get() {
                Err(io::Error::other("poll failed"))
            } else {
                Ok(window_has_event.get())
            }
        };

        // A blocking wait takes the next event, and never polls: it has nothing else to do.
        assert_eq!(
            read_terminal_event_with(TerminalEventWait::Blocking, read, poll).unwrap(),
            Some(event::Event::FocusGained)
        );
        assert_eq!(reads.get(), 1);
        assert_eq!(polls.get(), 0, "a blocking wait must not poll");

        // A polling wait asks for the loop's window, and takes the event it reports.
        assert_eq!(
            read_terminal_event_with(TerminalEventWait::Poll(WINDOW), read, poll).unwrap(),
            Some(event::Event::FocusGained)
        );
        assert_eq!(asked_for.get(), WINDOW);
        assert_eq!((reads.get(), polls.get()), (2, 1));

        // An empty window reports nothing to take, so nothing is taken.
        window_has_event.set(false);
        assert_eq!(
            read_terminal_event_with(TerminalEventWait::Poll(WINDOW), read, poll).unwrap(),
            None
        );
        assert_eq!(reads.get(), 2, "an empty window must not take an event");
        assert_eq!(polls.get(), 2);

        // A window that fails stops there, and the failure reaches the caller.
        poll_fails.set(true);
        assert!(read_terminal_event_with(TerminalEventWait::Poll(WINDOW), read, poll).is_err());
        assert_eq!(reads.get(), 2, "a failed window must not take an event");
        assert_eq!(polls.get(), 3);

        // A failure while taking the event reaches the caller too.
        poll_fails.set(false);
        read_fails.set(true);
        assert!(read_terminal_event_with(TerminalEventWait::Blocking, read, poll).is_err());
        assert_eq!(reads.get(), 3);
        assert_eq!(polls.get(), 3, "a blocking wait must not poll");
    }

    #[test]
    fn terminal_waits_for_input_without_pending_path_work() {
        assert_eq!(terminal_event_wait(false), TerminalEventWait::Blocking);
        assert_eq!(
            terminal_event_wait(true),
            TerminalEventWait::Poll(Duration::from_millis(25))
        );
    }

    #[test]
    fn host_errors_use_the_terminal_locale() {
        assert_eq!(
            localized_status(&HostError, Locale::ZhCn),
            "找不到条目：demo"
        );
        assert_eq!(
            localized_status(&HostError, Locale::ZhTw),
            "找不到項目：demo"
        );
    }

    #[test]
    fn rejected_preflight_becomes_status_before_the_terminal_can_suspend() {
        let mut state = LibraryState::default();
        let effect = Effect::Open {
            request: skit_ui::HostRequest::Run,
            selector: Some("demo".to_owned()),
        };
        let mut preflight = |_effect: &Effect| Err(HostError);

        assert!(!accept_host_effect(
            &mut state,
            &effect,
            &mut preflight,
            Locale::ZhCn,
        ));
        assert_eq!(state.status(), Some("错误：找不到条目：demo"));
    }

    #[test]
    fn a_saved_language_changes_the_running_terminal_locale() {
        assert_eq!(
            action_locale(&Action::PreferencesSaved {
                locale: "zh-TW".to_owned(),
                message: "Preferences saved".to_owned(),
            }),
            Some(Locale::ZhTw)
        );
        assert_eq!(action_locale(&Action::ClearStatus), None);
    }

    #[test]
    fn host_actions_drain_follow_up_effects_before_the_terminal_resumes() {
        let mut state = LibraryState::default();
        let mut calls = 0;
        let mut host = |_effect| -> Result<Action, HostError> {
            calls += 1;
            Ok(if calls == 1 {
                Action::Reload
            } else {
                Action::ClearStatus
            })
        };
        let mut locale = Locale::En;

        assert!(!drain_host_effects(&mut state, &mut host, Effect::Reload, &mut locale).unwrap());
        assert_eq!(calls, 2);
    }

    #[test]
    fn an_unrelated_host_error_keeps_its_form_owner() {
        let mut state = LibraryState::default();
        state.update(Action::Present(Screen::Form(FormView {
            purpose: skit_ui::FormPurpose::Settings,
            title: "Edit entry".to_owned(),
            title_arguments: Vec::new(),
            translate_title: false,
            selector: Some("demo".to_owned()),
            fields: vec![skit_ui::FormField::text("name", "Name", "demo")],
            focused: 0,
            submit_label: "Save".to_owned(),
        })));
        let effect = state.update(Action::Submit);
        let mut host = |_effect| -> Result<Action, HostError> { Err(HostError) };
        let mut locale = Locale::En;

        assert!(!drain_host_effects(&mut state, &mut host, effect, &mut locale).unwrap());
        assert!(matches!(state.screen(), Screen::Form(_)));
        assert_eq!(state.status(), Some("entry not found: demo"));
    }

    #[test]
    fn a_broken_host_cycle_stops_at_a_fixed_boundary() {
        let mut state = LibraryState::default();
        let mut calls = 0;
        let mut host = |_effect| -> Result<Action, HostError> {
            calls += 1;
            Ok(Action::Reload)
        };
        let mut locale = Locale::En;

        assert!(matches!(
            drain_host_effects(&mut state, &mut host, Effect::Reload, &mut locale),
            Err(TuiError::EffectCycle)
        ));
        assert_eq!(calls, HOST_EFFECT_LIMIT);
    }

    #[test]
    fn hosted_add_observes_typed_completion_after_the_host_effect_settles() {
        let slug = Slug::parse("new-tool").unwrap();
        let completed = slug.clone();
        let mut state = LibraryState::default();
        let mut host = move |_effect| -> Result<Action, HostError> {
            Ok(Action::AddCompleted {
                surface: skit_application::library_detail::LibrarySurface::default(),
                rerunnable: Vec::new(),
                slug: completed.clone(),
                message: "Added".to_owned(),
            })
        };
        let mut locale = Locale::En;
        let (quit, outcome) = drain_host_effects_observed(
            &mut state,
            &mut host,
            Effect::Reload,
            &mut locale,
            &mut add_workflow_outcome,
        )
        .unwrap();

        assert!(!quit);
        assert_eq!(outcome, Some(AddWorkflowOutcome::Completed(slug)));
        assert_eq!(state.screen(), &Screen::Library);
    }

    #[test]
    fn hosted_add_cancel_is_an_explicit_outcome() {
        assert_eq!(
            add_workflow_outcome(&Action::AddCancelled),
            Some(AddWorkflowOutcome::Cancelled)
        );
        assert_eq!(add_workflow_outcome(&Action::ClearStatus), None);
    }

    #[test]
    fn public_add_opening_actions_finish_before_terminal_setup() {
        assert_eq!(harmless_host(Effect::Quit).unwrap(), Action::ClearStatus);
        let cancelled = run_add_workflow(
            AddWorkflowState::new(Vec::new()),
            vec![Action::AddCancelled],
            harmless_host,
            Locale::En,
        )
        .unwrap();
        assert_eq!(cancelled, None);

        let slug = Slug::parse("opened").unwrap();
        let completed = run_add_workflow(
            AddWorkflowState::new(Vec::new()),
            vec![Action::AddCompleted {
                surface: skit_application::library_detail::LibrarySurface::default(),
                rerunnable: Vec::new(),
                slug: slug.clone(),
                message: "Added".to_owned(),
            }],
            harmless_host,
            Locale::En,
        )
        .unwrap();
        assert_eq!(completed, Some(slug));

        let slug = Slug::parse("hosted").unwrap();
        let hosted = run_add_workflow(
            AddWorkflowState::new(Vec::new()),
            vec![Action::Reload],
            |effect| -> Result<Action, HostError> {
                assert_eq!(effect, Effect::Reload);
                Ok(Action::AddCompleted {
                    surface: skit_application::library_detail::LibrarySurface::default(),
                    rerunnable: Vec::new(),
                    slug: slug.clone(),
                    message: "Added".to_owned(),
                })
            },
            Locale::En,
        )
        .unwrap();
        assert_eq!(hosted, Some(slug));
    }

    #[test]
    fn hosted_action_step_distinguishes_local_continue_and_typed_finish() {
        use std::cell::RefCell;

        let mut state = LibraryState::default();
        state.update(Action::Present(Screen::Add(Box::new(
            AddWorkflowState::new(Vec::new()),
        ))));
        let mut locale = Locale::En;
        let mut host = harmless_host;
        let mut preflight = |_effect: &Effect| Ok::<(), HostError>(());
        let mut observe = add_workflow_outcome;
        let transitions = RefCell::new(Vec::new());
        let mut record_transition = |transition| {
            transitions.borrow_mut().push(transition);
            Ok(())
        };

        assert_eq!(
            advance_hosted_action(
                &mut state,
                Action::ClearStatus,
                &mut preflight,
                &mut host,
                &mut locale,
                &mut observe,
                &mut record_transition,
            )
            .unwrap(),
            HostedActionStep::Continue
        );
        assert!(transitions.borrow().is_empty());
        assert_eq!(
            advance_hosted_action(
                &mut state,
                Action::AddCancelled,
                &mut preflight,
                &mut host,
                &mut locale,
                &mut observe,
                &mut record_transition,
            )
            .unwrap(),
            HostedActionStep::Finish(Some(AddWorkflowOutcome::Cancelled))
        );
        assert!(transitions.borrow().is_empty());

        // The same recorder is live when an actual host effect crosses the terminal boundary.
        // This distinguishes the two local actions above from an accidentally suppressed
        // transition callback.
        let mut continuing_state = LibraryState::default();
        let mut continuing_host = harmless_host;
        let mut no_outcome = |_action: &Action| None::<()>;
        assert_eq!(
            advance_hosted_action(
                &mut continuing_state,
                Action::Reload,
                &mut preflight,
                &mut continuing_host,
                &mut locale,
                &mut no_outcome,
                &mut record_transition,
            )
            .unwrap(),
            HostedActionStep::Continue
        );
        assert_eq!(
            &*transitions.borrow(),
            &[
                HostedTerminalTransition::Suspend,
                HostedTerminalTransition::Resume,
            ]
        );
    }

    #[test]
    fn hosted_action_step_finishes_for_quit_or_outcome_independently() {
        let mut locale = Locale::En;
        let mut preflight = |_effect: &Effect| Ok::<(), HostError>(());

        let mut quit_state = LibraryState::default();
        let mut quit_host = |_effect| -> Result<Action, HostError> { Ok(Action::Quit) };
        let mut no_outcome = |_action: &Action| None::<()>;
        let mut transitions = Vec::new();
        assert_eq!(
            advance_hosted_action(
                &mut quit_state,
                Action::Reload,
                &mut preflight,
                &mut quit_host,
                &mut locale,
                &mut no_outcome,
                &mut |transition| {
                    transitions.push(transition);
                    Ok(())
                },
            )
            .unwrap(),
            HostedActionStep::Finish(None)
        );
        assert_eq!(transitions, [HostedTerminalTransition::Suspend]);

        let slug = Slug::parse("settled").unwrap();
        let completed = slug.clone();
        let mut outcome_state = LibraryState::default();
        outcome_state.update(Action::Present(Screen::Add(Box::new(
            AddWorkflowState::new(Vec::new()),
        ))));
        let mut outcome_host = move |_effect| -> Result<Action, HostError> {
            Ok(Action::AddCompleted {
                surface: skit_application::library_detail::LibrarySurface::default(),
                rerunnable: Vec::new(),
                slug: completed.clone(),
                message: "Added".to_owned(),
            })
        };
        let mut observe = add_workflow_outcome;
        transitions.clear();
        assert_eq!(
            advance_hosted_action(
                &mut outcome_state,
                Action::Reload,
                &mut preflight,
                &mut outcome_host,
                &mut locale,
                &mut observe,
                &mut |transition| {
                    transitions.push(transition);
                    Ok(())
                },
            )
            .unwrap(),
            HostedActionStep::Finish(Some(AddWorkflowOutcome::Completed(slug)))
        );
        assert_eq!(transitions, [HostedTerminalTransition::Suspend]);

        let mut continuing_state = LibraryState::default();
        let mut continuing_host = harmless_host;
        let mut no_outcome = |_action: &Action| None::<()>;
        transitions.clear();
        assert_eq!(
            advance_hosted_action(
                &mut continuing_state,
                Action::Reload,
                &mut preflight,
                &mut continuing_host,
                &mut locale,
                &mut no_outcome,
                &mut |transition| {
                    transitions.push(transition);
                    Ok(())
                },
            )
            .unwrap(),
            HostedActionStep::Continue
        );
        assert_eq!(
            transitions,
            [
                HostedTerminalTransition::Suspend,
                HostedTerminalTransition::Resume,
            ]
        );
    }

    #[test]
    fn collected_form_actions_cancel_only_the_form_owner() {
        let mut state = LibraryState::default();
        state.update(Action::Present(Screen::Run(Box::new(empty_run_form()))));
        assert_eq!(
            collect_action_step(&mut state, Action::Quit),
            CollectActionStep::Cancel
        );

        let mut state = LibraryState::default();
        state.update(Action::Present(Screen::Run(Box::new(empty_run_form()))));
        assert_eq!(
            collect_action_step(&mut state, Action::Back),
            CollectActionStep::Cancel
        );

        let mut state = LibraryState::default();
        state.update(Action::Present(Screen::Run(Box::new(empty_run_form()))));
        state.update(Action::OpenHelp);
        assert!(state.modal().is_some());
        assert_eq!(
            collect_action_step(&mut state, Action::Back),
            CollectActionStep::Effect(Effect::None)
        );
        assert!(state.modal().is_none());
    }

    #[test]
    fn public_run_collection_still_claims_a_real_terminal() {
        use std::process::{Command, Stdio};

        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "terminal::tests::public_run_collection_rejects_piped_child",
                "--ignored",
                "--nocapture",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .unwrap();
        assert!(output.status.success());
    }

    #[test]
    #[ignore = "runs only as the piped child of the public wrapper contract"]
    fn public_run_collection_rejects_piped_child() {
        let error = collect_run_form(empty_run_form(), harmless_host, Locale::En)
            .expect_err("a collected form must reject non-terminal test streams");
        assert!(matches!(
            &error,
            TuiError::Io(error)
                if error.kind() == io::ErrorKind::NotConnected
                    && error.to_string() == "stdin and stdout are not a terminal"
        ));
    }

    #[test]
    fn terminal_transition_primitives_keep_order_errors_and_drop_restoration() {
        use std::cell::RefCell;
        use std::rc::Rc;

        let calls = Rc::new(RefCell::new(Vec::new()));
        let first_calls = Rc::clone(&calls);
        let second_calls = Rc::clone(&calls);
        let mut second = move || {
            second_calls.borrow_mut().push("second");
            Ok(())
        };
        sequential_terminal_transition(
            move || {
                first_calls.borrow_mut().push("first");
                Ok(())
            },
            &mut second,
        )
        .unwrap();
        assert_eq!(&*calls.borrow(), &["first", "second"]);

        calls.borrow_mut().clear();
        let first_calls = Rc::clone(&calls);
        let error = sequential_terminal_transition(
            move || {
                first_calls.borrow_mut().push("first-error");
                Err(io::Error::other("first failed"))
            },
            &mut second,
        )
        .unwrap_err();
        assert_eq!(error.to_string(), "first failed");
        assert_eq!(&*calls.borrow(), &["first-error"]);

        calls.borrow_mut().clear();
        let first_calls = Rc::clone(&calls);
        let second_calls = Rc::clone(&calls);
        let error = restorative_terminal_transition(
            move || {
                first_calls.borrow_mut().push("restore-raw-error");
                Err(io::Error::other("raw restore failed"))
            },
            move || {
                second_calls.borrow_mut().push("restore-screen");
                Ok(())
            },
        )
        .unwrap_err();
        assert_eq!(error.to_string(), "raw restore failed");
        assert_eq!(&*calls.borrow(), &["restore-raw-error", "restore-screen"]);

        calls.borrow_mut().clear();
        let restore_calls = Rc::clone(&calls);
        {
            let _restore = RestoreTerminal::new(move || {
                restore_calls.borrow_mut().push("drop-restore");
                Ok(())
            });
        }
        assert_eq!(&*calls.borrow(), &["drop-restore"]);
    }

    #[test]
    fn host_and_collection_drains_cover_quit_submit_locale_error_and_cycles() {
        use std::collections::BTreeMap;

        assert_eq!(
            TuiError::EffectCycle.message().localize(Locale::En),
            "terminal host effects did not settle"
        );
        let mut state = LibraryState::default();
        let mut locale = Locale::En;
        let mut host = |_effect| -> Result<Action, HostError> { Ok(Action::ClearStatus) };
        assert!(drain_host_effects(&mut state, &mut host, Effect::Quit, &mut locale).unwrap());
        let mut locale_host = |_effect| -> Result<Action, HostError> {
            Ok(Action::PreferencesSaved {
                locale: "zh-CN".to_owned(),
                message: "saved".to_owned(),
            })
        };
        assert!(
            !drain_host_effects(&mut state, &mut locale_host, Effect::Reload, &mut locale,)
                .unwrap()
        );
        assert_eq!(locale, Locale::ZhCn);

        let values = BTreeMap::new();
        let mut unused = harmless_host;
        assert_eq!(
            drain_collect_effects(&mut state, &mut unused, Effect::None, &mut locale).unwrap(),
            None
        );
        assert_eq!(
            drain_collect_effects(&mut state, &mut unused, Effect::Quit, &mut locale).unwrap(),
            Some(None)
        );
        assert_eq!(
            drain_collect_effects(
                &mut state,
                &mut unused,
                Effect::Submit {
                    purpose: skit_ui::FormPurpose::Add,
                    selector: None,
                    values: values.clone(),
                },
                &mut locale,
            )
            .unwrap(),
            Some(Some(values))
        );

        let mut locale_host = |_effect| -> Result<Action, HostError> {
            Ok(Action::PreferencesSaved {
                locale: "zh-TW".to_owned(),
                message: "saved".to_owned(),
            })
        };
        let _ = drain_collect_effects(&mut state, &mut locale_host, Effect::Reload, &mut locale);
        assert_eq!(locale, Locale::ZhTw);

        let mut failed = |_effect| -> Result<Action, HostError> { Err(HostError) };
        assert_eq!(
            drain_collect_effects(&mut state, &mut failed, Effect::Reload, &mut locale).unwrap(),
            None
        );

        let mut cycling = |_effect| -> Result<Action, HostError> { Ok(Action::Reload) };
        assert!(matches!(
            drain_collect_effects(&mut state, &mut cycling, Effect::Reload, &mut locale),
            Err(TuiError::EffectCycle)
        ));
    }

    #[test]
    fn deterministic_dispatch_uses_a_real_rendered_session_without_terminal_lifecycle() {
        use ratatui_core::{backend::TestBackend, terminal::Terminal};
        use ratatui_crossterm::crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

        let state = LibraryState::default();
        let mut session = TuiSession::default();
        let mut terminal = Terminal::new(TestBackend::new(60, 14)).unwrap();
        let mut geometry = ViewGeometry::default();
        terminal
            .draw(|frame| {
                geometry = render_with_session(frame, &state, Locale::En, &mut session);
            })
            .unwrap();
        let ignored = dispatch_event(&mut session, Event::FocusGained, &state, &geometry);
        assert_eq!(ignored.action, None);
        assert!(!ignored.redraw);
        let ctrl_c = Event::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        let consumed = dispatch_event(&mut session, ctrl_c.clone(), &state, &geometry);
        assert_eq!(consumed.action, None);
        assert!(consumed.redraw);
        let action = dispatch_event(&mut session, ctrl_c, &state, &geometry);
        assert_eq!(action.action, Some(Action::Quit));
        assert!(action.redraw);
    }

    #[test]
    fn dispatch_keeps_ignored_consumed_action_and_resize_redraw_decisions_distinct() {
        use ratatui_core::{backend::TestBackend, terminal::Terminal};
        use ratatui_crossterm::crossterm::event::{
            Event, KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind,
        };

        let state = LibraryState::default();
        let mut session = TuiSession::default();
        let mut terminal = Terminal::new(TestBackend::new(60, 14)).unwrap();
        let mut geometry = ViewGeometry::default();
        terminal
            .draw(|frame| {
                geometry = render_with_session(frame, &state, Locale::En, &mut session);
            })
            .unwrap();

        let ignored = dispatch_event(
            &mut session,
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Moved,
                column: 0,
                row: 0,
                modifiers: KeyModifiers::NONE,
            }),
            &state,
            &geometry,
        );
        let consumed = dispatch_event(
            &mut session,
            Event::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            &state,
            &geometry,
        );
        assert!(!ignored.redraw, "an ignored pointer move must not redraw");
        assert!(consumed.redraw, "a consumed event must request a redraw");

        let resized = dispatch_event(&mut session, Event::Resize(59, 13), &state, &geometry);
        assert!(resized.redraw, "a terminal resize must request a redraw");

        let action = dispatch_event(
            &mut session,
            Event::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            &state,
            &geometry,
        );
        assert!(action.redraw, "an action must request a redraw");
        assert_eq!(action.action, Some(Action::Quit));
    }
}
