//! Crossterm lifecycle and blocking event loop.

use std::cell::Cell;
use std::collections::VecDeque;
use std::io::{self, Write};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ratatui_core::terminal::Terminal;
use ratatui_crossterm::{
    CrosstermBackend,
    crossterm::{
        event::{
            self, DisableBracketedPaste, DisableFocusChange, DisableMouseCapture,
            EnableBracketedPaste, EnableFocusChange, EnableMouseCapture, KeyCode, KeyEvent,
            KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
        },
        execute,
        style::{Colored, force_color_output},
        terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
    },
};
use skit_application::path_completion::PathCompletionProvider;
use skit_domain::Slug;
use skit_i18n::{Locale, Localize, Message, detect_locale};
use skit_ui::{
    Action, AddWorkflowState, Effect, FormView, LibraryState, RunFormView, Screen, StatusKind,
    SubmittedValues,
};
use thiserror::Error;

use crate::editor::{EditCommand, edit_command};
use crate::status_clock::StatusClock;
use crate::system_clipboard::SystemClipboard;
use crate::{Appearance, EventHandling, TuiSession, ViewGeometry, render_with_session};

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
    let enter = || enter_screen_modes(&mut io::stdout());
    claim_terminal_with(enable_raw_mode, enter, restore_terminal)
}

/// Whether skit asks the terminal for bracketed paste.
///
/// Bracketed paste makes a paste one event, so a line break in it can never act as Enter and a Tab
/// in it can never move the focus. crossterm on Windows reads console input records and reports no
/// paste event, and the legacy console refuses the mode, so Windows does not ask for it: there a
/// terminal types a paste as keys, and [`TerminalInput`] turns those keys back into a paste.
const BRACKETED_PASTE: bool = cfg!(not(windows));

/// Enter the screen modes of a session: the alternate screen, mouse capture, focus reporting, and
/// bracketed paste where the platform has it ([`BRACKETED_PASTE`]).
///
/// The terminal claim and the resume after a host effect share this set, so the two paths always
/// enter the same modes.
fn enter_screen_modes(out: &mut impl Write) -> io::Result<()> {
    execute!(
        out,
        EnterAlternateScreen,
        EnableMouseCapture,
        EnableFocusChange
    )?;
    if BRACKETED_PASTE {
        execute!(out, EnableBracketedPaste)?;
    }
    Ok(())
}

/// Leave every screen mode of a session.
///
/// The suspend before a run or an editor and the final restore share this set. Mouse capture, focus
/// reporting, and bracketed paste are not part of the alternate screen. After
/// `LeaveAlternateScreen` alone, the terminal keeps them on, and a child process on the same
/// terminal then receives mouse and focus reports and paste markers as input.
fn leave_screen_modes(out: &mut impl Write) -> io::Result<()> {
    execute!(
        out,
        LeaveAlternateScreen,
        DisableMouseCapture,
        DisableFocusChange
    )?;
    if BRACKETED_PASTE {
        execute!(out, DisableBracketedPaste)?;
    }
    Ok(())
}

/// The longest pause between two keys of one burst.
///
/// A terminal with no bracketed paste types a paste by writing all of its keys at once, so they
/// follow each other with no pause. A person leaves far more time than this between two keys.
const BURST_GAP: Duration = Duration::from_millis(5);

/// The input of one session.
///
/// It gives the loop one crossterm event at a time. On a terminal with no bracketed paste, it
/// reads the keys that arrive together as one burst, and it gives a burst that is a paste to the
/// loop as one paste event, as bracketed paste does ([`fold_paste_burst`]).
#[derive(Debug)]
struct TerminalInput {
    /// Turn a burst of typed keys back into the paste that the terminal typed.
    fold_pastes: bool,
    /// The events of the last burst that the loop has not taken yet.
    queued: VecDeque<event::Event>,
}

impl TerminalInput {
    const fn new(fold_pastes: bool) -> Self {
        Self {
            fold_pastes,
            queued: VecDeque::new(),
        }
    }

    /// Take the next event, or report that the poll window closed with nothing to take.
    fn next(&mut self, wait: TerminalEventWait) -> io::Result<Option<event::Event>> {
        self.next_with(wait, event::read, event::poll)
    }

    /// Take the next event with the two terminal calls given as parameters.
    ///
    /// A test can then drive every burst without a live terminal: `poll` answers whether the next
    /// event arrives within its pause.
    fn next_with<R, P>(
        &mut self,
        wait: TerminalEventWait,
        mut read: R,
        mut poll: P,
    ) -> io::Result<Option<event::Event>>
    where
        R: FnMut() -> io::Result<event::Event>,
        P: FnMut(Duration) -> io::Result<bool>,
    {
        if let Some(event) = self.queued.pop_front() {
            return Ok(Some(event));
        }
        let Some(first) = read_terminal_event_with(wait, &mut read, &mut poll)? else {
            return Ok(None);
        };
        if !self.fold_pastes || !is_typed_text(&first) {
            return Ok(Some(first));
        }
        let mut burst = vec![first];
        let mut after = None;
        while poll(BURST_GAP)? {
            let event = read()?;
            if is_typed_text(&event) || is_key_release(&event) {
                burst.push(event);
            } else {
                after = Some(event);
                break;
            }
        }
        self.queued.extend(fold_paste_burst(burst));
        self.queued.extend(after);
        Ok(self.queued.pop_front())
    }
}

/// Return the status that says why the line break of `event` did not act as Enter.
///
/// With no bracketed paste, [`TerminalInput`] puts each paste event together from typed keys, and
/// the line break of such a paste never acts as Enter. A tool can also type text and Enter that
/// fast, for example a text expander or the auto-type of a password manager. Without this
/// warning, that Enter does nothing, and nothing on the screen tells why. The warning stays until
/// the next key, which then acts as usual. A box that keeps more lines takes the line break as
/// text, so it gets no warning.
fn guessed_paste_warning(
    input: &TerminalInput,
    event: &event::Event,
    session: &TuiSession,
    state: &LibraryState,
    locale: Locale,
) -> Option<Action> {
    let event::Event::Paste(text) = event else {
        return None;
    };
    (input.fold_pastes && text.contains('\n') && session.focused_box_keeps_one_line(state)).then(
        || Action::SetStatus {
            kind: StatusKind::Warning,
            message: Message::new(
                "skit took this input as a paste, so its line break did not act as Enter.",
            )
            .localize(locale),
        },
    )
}

/// Turn the keys of one burst into the paste event that bracketed paste reports.
///
/// A burst is a paste when it holds a typed character and also a line break or a Tab. A person
/// cannot type both in one burst, and a held key repeats only itself. The paste then goes to the
/// focused box with the paste rules of that box: a single-line box keeps the first line, and a
/// paste never runs the launch menu or moves the focus. Every other burst stays as its keys, so a
/// typed key and a held Enter or Tab work as before.
fn fold_paste_burst(burst: Vec<event::Event>) -> Vec<event::Event> {
    let mut text = String::new();
    let mut has_character = false;
    let mut has_separator = false;
    for event in &burst {
        if let event::Event::Key(key) = event
            && key.kind != KeyEventKind::Release
            && let Some(character) = pasted_character(key)
        {
            if matches!(character, '\n' | '\t') {
                has_separator = true;
            } else {
                has_character = true;
            }
            text.push(character);
        }
    }
    if has_character && has_separator {
        vec![event::Event::Paste(text)]
    } else {
        burst
    }
}

/// The character that one typed key adds to a paste: a character, a line break, or a Tab.
///
/// A pseudo-console types a line feed as Ctrl+Enter, so Enter with Ctrl is a line break too.
fn pasted_character(key: &KeyEvent) -> Option<char> {
    match key.code {
        KeyCode::Enter if !key.modifiers.contains(KeyModifiers::ALT) => Some('\n'),
        KeyCode::Tab if key.modifiers.is_empty() => Some('\t'),
        _ => match edit_command(key, false) {
            Some(EditCommand::Insert(character)) => Some(character),
            _ => None,
        },
    }
}

/// Report whether `event` is a key press that types text, a line break, or a Tab.
fn is_typed_text(event: &event::Event) -> bool {
    matches!(event, event::Event::Key(key)
        if key.kind != KeyEventKind::Release && pasted_character(key).is_some())
}

/// Report whether `event` is a key release. crossterm on Windows reports one for every key.
fn is_key_release(event: &event::Event) -> bool {
    matches!(event, event::Event::Key(key) if key.kind == KeyEventKind::Release)
}

/// The longest time between the two presses of a double click.
const DOUBLE_CLICK: Duration = Duration::from_millis(500);

/// The time of the last primary press, so that a slow second press starts a new double click.
///
/// The session counts presses with no clock, so a walker gets the same answer every time. The
/// live loop adds the time limit here.
#[derive(Debug, Default)]
struct PressClock {
    last: Option<Instant>,
}

impl PressClock {
    /// Report whether `event` is a primary press that comes too late to be a second press.
    fn starts_new_sequence(&mut self, event: &event::Event, now: Instant) -> bool {
        let event::Event::Mouse(mouse) = event else {
            return false;
        };
        if mouse.kind != MouseEventKind::Down(MouseButton::Left) {
            return false;
        }
        let late = self
            .last
            .is_none_or(|last| now.saturating_duration_since(last) > DOUBLE_CLICK);
        self.last = Some(now);
        late
    }
}

/// Send the last copy to the system clipboard, when there is a new one.
///
/// The copy goes to the terminal as OSC 52, and in a local session also to the clipboard program
/// of the platform ([`SystemClipboard`]), which never makes the loop wait.
fn export_clipboard(
    session: &mut TuiSession,
    out: &mut impl Write,
    system: &SystemClipboard,
) -> io::Result<()> {
    if let Some(text) = session.take_clipboard_export() {
        out.write_all(crate::editor::osc52_copy(&text).as_bytes())?;
        out.flush()?;
        system.copy(&text);
    }
    Ok(())
}

/// Restore both terminal modes after a completed session or a partial claim.
///
/// Raw mode and the screen modes are independent terminal resources. Restoration always attempts
/// both and keeps the first restoration error. The screen modes go off before raw mode, the
/// reverse of the claim: a cooked terminal echoes a mouse or focus report that arrives while its
/// mode is still on. A failed claim uses this function for best-effort rollback and keeps the
/// original claim error.
fn restore_terminal() -> io::Result<()> {
    restorative_terminal_transition(|| leave_screen_modes(&mut io::stdout()), disable_raw_mode)
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
    // A hover, and a press that starts or ends, change only the look of a chip. The session
    // ignores an event that changes only the look, so a changed look asks for a frame here.
    let look = session.pointer_look(state);
    session.track_pointer(&event);
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
            redraw: resized || session.pointer_look(state) != look,
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
pub fn run<F, E>(
    state: LibraryState,
    host: F,
    locale: Locale,
    appearance: Appearance,
) -> Result<(), TuiError>
where
    F: FnMut(Effect) -> Result<Action, E>,
    E: Localize,
{
    run_preflighted(state, |_| Ok::<(), E>(()), host, locale, appearance)
}

/// Run the terminal frontend with asynchronous path completion.
pub fn run_with_path_completion<F, E>(
    state: LibraryState,
    host: F,
    locale: Locale,
    appearance: Appearance,
    provider: Arc<dyn PathCompletionProvider>,
) -> Result<(), TuiError>
where
    F: FnMut(Effect) -> Result<Action, E>,
    E: Localize,
{
    run_preflighted_with_path_completion(
        state,
        |_| Ok::<(), E>(()),
        host,
        locale,
        appearance,
        provider,
    )
}

/// Run the terminal frontend with a check that occurs before the host serves an effect.
///
/// The check must be local and read-only. A refusal stays on the active screen as a localized
/// status. The host does not receive the refused effect.
pub fn run_preflighted<F, P, E>(
    state: LibraryState,
    preflight: P,
    host: F,
    locale: Locale,
    appearance: Appearance,
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
        SessionOptions::new(appearance, None),
    )?;
    Ok(())
}

/// Run with both a pre-suspend check and asynchronous path completion.
pub fn run_preflighted_with_path_completion<F, P, E>(
    state: LibraryState,
    preflight: P,
    host: F,
    locale: Locale,
    appearance: Appearance,
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
        SessionOptions::new(appearance, Some(provider)),
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
    appearance: Appearance,
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
        SessionOptions::new(appearance, None),
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
    options: SessionOptions,
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
            // The session has not claimed the terminal yet, so it has nothing to give away.
            let (quit, outcome) = drain_host_effects_observed(
                &mut state,
                &mut host,
                effect,
                &mut locale,
                &mut observe,
                &mut |_| Ok(()),
            )?;
            if quit || outcome.is_some() {
                return Ok(outcome);
            }
        }
    }
    claim_terminal()?;
    // A host effect can end the session while skit has the terminal suspended, for example a run
    // with `after_run = "exit"`. The suspend already restored every mode. `\x1b[?1049l` also
    // restores the cursor that the claim saved, so a second restore moves the cursor back to the
    // row where skit started, and the shell prompt then overwrites the output of the run.
    let suspended = Cell::new(false);
    let _restore = RestoreTerminal::new(|| {
        if suspended.get() {
            Ok(())
        } else {
            restore_terminal()
        }
    });
    let _colors = ColorOutputClaim::new();
    let backend = options
        .appearance
        .backend(CrosstermBackend::new(io::stdout()));
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;
    let mut session = options
        .path_completion
        .map_or_else(TuiSession::default, |provider| {
            TuiSession::with_path_completion(provider)
        });
    session.set_appearance(options.appearance);

    let mut geometry = ViewGeometry::default();
    let mut redraw = true;
    let mut status = StatusClock::default();
    let mut presses = PressClock::default();
    let mut input = TerminalInput::new(!BRACKETED_PASTE);
    let system_clipboard = SystemClipboard::for_this_session();
    let outcome = loop {
        redraw = request_redraw(redraw, session.refresh_background());
        redraw = request_redraw(redraw, tick_status(&mut status, &mut state));
        if redraw {
            terminal.draw(|frame| {
                geometry = render_with_session(frame, &state, locale, &mut session);
            })?;
            redraw = false;
        }
        let wait = status.event_wait(
            terminal_event_wait(
                session.has_pending_path_completion() || session.has_timed_notice(),
            ),
            Instant::now(),
        );
        let Some(event) = input.next(wait)? else {
            continue;
        };
        redraw = request_redraw(redraw, status.acknowledge(&mut state, &event));
        if presses.starts_new_sequence(&event, Instant::now()) {
            session.end_press_sequence();
        }
        let paste_warning = guessed_paste_warning(&input, &event, &session, &state, locale);
        let dispatched = dispatch_event(&mut session, event, &state, &geometry);
        export_clipboard(&mut session, &mut io::stdout(), &system_clipboard)?;
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
                        // The reverse of the resume: the screen modes go off before raw mode.
                        sequential_terminal_transition(
                            || leave_screen_modes(&mut io::stdout()),
                            disable_raw_mode,
                        )?;
                        suspended.set(true);
                        Ok(())
                    }
                    HostedTerminalTransition::Resume => {
                        // A resume that fails part of the way still needs the full restore.
                        suspended.set(false);
                        sequential_terminal_transition(enable_raw_mode, || {
                            enter_screen_modes(&mut io::stdout())
                        })?;
                        terminal.clear()
                    }
                },
            )?;
            if let HostedActionStep::Finish(outcome) = step {
                break outcome;
            }
        }
        if let Some(warning) = paste_warning {
            let _ = state.update(warning);
            redraw = true;
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
/// The transition callback is the terminal boundary. Production leaves and re-enters every screen
/// mode there. Tests use the same path with a recorded fake lifecycle.
///
/// The session leaves its screen only for an effect that needs the terminal
/// ([`Effect::needs_the_terminal`]): a run or an editor. Every other effect is served on the
/// screen, as version 0.4 does. Leaving the screen shows the shell, and the return waits until the
/// terminal reports the cursor position, which takes one round trip over SSH. A preset save, a
/// reload, or a key in a pattern field must not make the screen flash.
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
            // A chained effect can need the terminal after one that does not, so the session
            // leaves its screen just before the first such effect, and at most once.
            let mut suspended = false;
            let (quit, host_outcome) = drain_host_effects_observed(
                state,
                host,
                effect,
                locale,
                observe,
                &mut |effect: &Effect| {
                    if !suspended && effect.needs_the_terminal() {
                        transition(HostedTerminalTransition::Suspend)?;
                        suspended = true;
                    }
                    Ok(())
                },
            )?;
            if outcome.is_none() {
                outcome = host_outcome;
            }
            if quit {
                return Ok(HostedActionStep::Finish(outcome));
            }
            if outcome.is_some() {
                return Ok(HostedActionStep::Finish(outcome));
            }
            if suspended {
                transition(HostedTerminalTransition::Resume)?;
            }
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
            state.update(preflight_refusal_status(&error, locale));
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

/// Return the status action that the hosted terminal shows when a preflight check refuses an
/// effect.
///
/// The terminal shows it on its screen, and the effect never reaches the host. A walker
/// host uses the same action, so a walk answers the refusal as the product answers it.
#[must_use]
pub fn preflight_refusal_status(error: &impl Localize, locale: Locale) -> Action {
    Action::SetStatus {
        kind: StatusKind::Error,
        message: Message::new("Error: {}")
            .nested(error.message())
            .localize(locale),
    }
}

/// Return the status action that the hosted terminal shows when the host fails one effect.
///
/// The terminal keeps the session and stops the effect chain. A walker host uses the same
/// action, so a walk survives every host failure that the product survives.
#[must_use]
pub fn host_failure_status(error: &impl Localize, locale: Locale) -> Action {
    Action::SetStatus {
        kind: StatusKind::Error,
        message: error.message().localize(locale),
    }
}

/// Start the time of a status that the last change showed, and clear one whose time ended.
///
/// Return whether the state changed, so the loop draws again.
fn tick_status(status: &mut StatusClock, state: &mut LibraryState) -> bool {
    let now = Instant::now();
    status.observe(state, now);
    status.expire(state, now)
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
        drain_host_effects_observed(state, host, effect, locale, &mut |_| None, &mut |_| Ok(()))?;
    Ok(quit)
}

/// Serve one effect and every effect that its answer starts, in order.
///
/// `before_host` sees each effect just before the host serves it, so the caller can give the
/// terminal away first.
fn drain_host_effects_observed<F, E, O>(
    state: &mut LibraryState,
    host: &mut F,
    mut effect: Effect,
    locale: &mut Locale,
    observe: &mut impl FnMut(&Action) -> Option<O>,
    before_host: &mut impl FnMut(&Effect) -> io::Result<()>,
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
            current => {
                before_host(&current)?;
                match host(current) {
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
                        state.update(host_failure_status(&error, *locale));
                        return Ok((false, outcome));
                    }
                }
            }
        }
    }
    Err(TuiError::EffectCycle)
}

/// Collect one generic form and restore the terminal before returning its values.
pub fn collect_form<F, E>(
    form: FormView,
    host: F,
    locale: Locale,
    appearance: Appearance,
) -> Result<Option<SubmittedValues>, TuiError>
where
    F: FnMut(Effect) -> Result<Action, E>,
    E: Localize,
{
    collect_screen(
        Screen::Form(form),
        host,
        locale,
        SessionOptions::new(appearance, None),
    )
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
    appearance: Appearance,
) -> Result<Option<SubmittedValues>, TuiError>
where
    F: FnMut(Effect) -> Result<Action, E>,
    E: Localize,
{
    collect_screen(
        Screen::Run(Box::new(form)),
        host,
        locale,
        SessionOptions::new(appearance, None),
    )
}

/// Collect one typed launch form with asynchronous path completion.
pub fn collect_run_form_with_path_completion<F, E>(
    form: RunFormView,
    host: F,
    locale: Locale,
    appearance: Appearance,
    provider: Arc<dyn PathCompletionProvider>,
) -> Result<Option<SubmittedValues>, TuiError>
where
    F: FnMut(Effect) -> Result<Action, E>,
    E: Localize,
{
    collect_screen(
        Screen::Run(Box::new(form)),
        host,
        locale,
        SessionOptions::new(appearance, Some(provider)),
    )
}

fn collect_screen<F, E>(
    screen: Screen,
    mut host: F,
    mut locale: Locale,
    options: SessionOptions,
) -> Result<Option<SubmittedValues>, TuiError>
where
    F: FnMut(Effect) -> Result<Action, E>,
    E: Localize,
{
    let mut state = LibraryState::default();
    state.update(Action::Present(screen));
    claim_terminal()?;
    let _restore = RestoreTerminal::new(restore_terminal);
    let _colors = ColorOutputClaim::new();
    let backend = options
        .appearance
        .backend(CrosstermBackend::new(io::stdout()));
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;
    let mut session = options
        .path_completion
        .map_or_else(TuiSession::default, |provider| {
            TuiSession::with_path_completion(provider)
        });
    session.set_appearance(options.appearance);

    let mut geometry = ViewGeometry::default();
    let mut redraw = true;
    let mut status = StatusClock::default();
    let mut presses = PressClock::default();
    let mut input = TerminalInput::new(!BRACKETED_PASTE);
    let system_clipboard = SystemClipboard::for_this_session();
    let values = loop {
        redraw = request_redraw(redraw, session.refresh_background());
        redraw = request_redraw(redraw, tick_status(&mut status, &mut state));
        if redraw {
            terminal.draw(|frame| {
                geometry = render_with_session(frame, &state, locale, &mut session);
            })?;
            redraw = false;
        }
        let wait = status.event_wait(
            terminal_event_wait(
                session.has_pending_path_completion() || session.has_timed_notice(),
            ),
            Instant::now(),
        );
        let Some(event) = input.next(wait)? else {
            continue;
        };
        redraw = request_redraw(redraw, status.acknowledge(&mut state, &event));
        if presses.starts_new_sequence(&event, Instant::now()) {
            session.end_press_sequence();
        }
        let paste_warning = guessed_paste_warning(&input, &event, &session, &state, locale);
        let dispatched = dispatch_event(&mut session, event, &state, &geometry);
        export_clipboard(&mut session, &mut io::stdout(), &system_clipboard)?;
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
        if let Some(warning) = paste_warning {
            let _ = state.update(warning);
            redraw = true;
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
                    state.update(host_failure_status(&error, *locale));
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

/// Session settings that the terminal loop needs and the reducer does not.
struct SessionOptions {
    appearance: Appearance,
    path_completion: Option<Arc<dyn PathCompletionProvider>>,
}

impl SessionOptions {
    fn new(
        appearance: Appearance,
        path_completion: Option<Arc<dyn PathCompletionProvider>>,
    ) -> Self {
        Self {
            appearance,
            path_completion,
        }
    }
}

/// Let the session, not crossterm, decide whether a frame carries color.
///
/// When `NO_COLOR` holds a value, crossterm 0.29 writes `ESC[;m` in place of each color change.
/// That sequence resets every attribute, so bold, dim, and reverse vanished from the frame. The
/// session removes color itself (see [`Appearance`]), so crossterm writes colors unchanged until
/// the session ends and the previous setting returns.
struct ColorOutputClaim {
    was_disabled: bool,
}

impl ColorOutputClaim {
    fn new() -> Self {
        let was_disabled = Colored::ansi_color_disabled_memoized();
        force_color_output(true);
        Self { was_disabled }
    }
}

impl Drop for ColorOutputClaim {
    fn drop(&mut self) {
        force_color_output(!self.was_disabled);
    }
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

    /// The shared command set writes all three modes in both directions.
    ///
    /// Windows crossterm can send some of these commands to the console API instead of writing
    /// ANSI, so this byte contract is a Unix contract.
    #[cfg(unix)]
    #[test]
    fn screen_modes_toggle_the_alternate_screen_mouse_capture_and_focus_reporting() {
        // The commands write ASCII only, so the lossy conversion is exact.
        let mut entered = Vec::new();
        enter_screen_modes(&mut entered).unwrap();
        let entered = String::from_utf8_lossy(&entered);
        for expected in ["\x1b[?1049h", "\x1b[?1003h", "\x1b[?1004h"] {
            assert!(
                entered.contains(expected),
                "the entered modes miss {expected:?}: {entered:?}"
            );
        }
        // Windows has no bracketed paste, so the mode is there only where the platform has it.
        assert_eq!(entered.contains("\x1b[?2004h"), BRACKETED_PASTE);

        let mut left = Vec::new();
        leave_screen_modes(&mut left).unwrap();
        let left = String::from_utf8_lossy(&left);
        for expected in ["\x1b[?1049l", "\x1b[?1003l", "\x1b[?1004l"] {
            assert!(
                left.contains(expected),
                "the left modes miss {expected:?}: {left:?}"
            );
        }
        assert_eq!(left.contains("\x1b[?2004l"), BRACKETED_PASTE);
    }

    /// A terminal for [`TerminalInput`] that gives a script of events.
    ///
    /// Each event arrives after its pause. `poll` answers yes when the next event arrives within
    /// the pause that it gets, and `read` takes the next event. A read past the end of the script
    /// is an error, so a test that reads too far fails.
    struct ScriptedInput {
        events: std::cell::RefCell<VecDeque<(Duration, event::Event)>>,
        polls: Cell<u32>,
        poll_fails: Cell<bool>,
    }

    impl ScriptedInput {
        fn new(events: impl IntoIterator<Item = (Duration, event::Event)>) -> Self {
            Self {
                events: std::cell::RefCell::new(events.into_iter().collect()),
                polls: Cell::new(0),
                poll_fails: Cell::new(false),
            }
        }

        fn read(&self) -> io::Result<event::Event> {
            self.events
                .borrow_mut()
                .pop_front()
                .map(|(_, event)| event)
                .ok_or_else(|| io::Error::other("the script has no event left"))
        }

        fn poll(&self, pause: Duration) -> io::Result<bool> {
            self.polls.set(self.polls.get() + 1);
            if self.poll_fails.get() {
                return Err(io::Error::other("poll failed"));
            }
            Ok(self
                .events
                .borrow()
                .front()
                .is_some_and(|(after, _)| *after <= pause))
        }

        fn next_in(
            &self,
            input: &mut TerminalInput,
            wait: TerminalEventWait,
        ) -> io::Result<Option<event::Event>> {
            input.next_with(wait, || self.read(), |pause| self.poll(pause))
        }

        /// Take the next event with the blocking wait of a session that has no background work.
        fn next(&self, input: &mut TerminalInput) -> event::Event {
            self.next_in(input, TerminalEventWait::Blocking)
                .unwrap()
                .expect("a blocking wait always gives an event")
        }

        fn is_done(&self) -> bool {
            self.events.borrow().is_empty()
        }
    }

    fn key_press(code: KeyCode, modifiers: KeyModifiers) -> event::Event {
        event::Event::Key(KeyEvent::new(code, modifiers))
    }

    fn key_release(code: KeyCode, modifiers: KeyModifiers) -> event::Event {
        event::Event::Key(KeyEvent::new_with_kind(
            code,
            modifiers,
            KeyEventKind::Release,
        ))
    }

    /// The events that crossterm reads from a Windows console when a terminal types `text` at
    /// once: a press and a release for each key, with no pause between them.
    fn typed_at_once(text: &str) -> Vec<(Duration, event::Event)> {
        text.chars()
            .flat_map(|character| {
                let (code, modifiers) = match character {
                    '\r' => (KeyCode::Enter, KeyModifiers::NONE),
                    '\t' => (KeyCode::Tab, KeyModifiers::NONE),
                    character if character.is_uppercase() => {
                        (KeyCode::Char(character), KeyModifiers::SHIFT)
                    }
                    character => (KeyCode::Char(character), KeyModifiers::NONE),
                };
                [
                    (Duration::ZERO, key_press(code, modifiers)),
                    (Duration::ZERO, key_release(code, modifiers)),
                ]
            })
            .collect()
    }

    // Ways this can fail: the line break of a paste reaches the screen as Enter, or its Tab as the
    // focus key; the paste loses a character, keeps a key release, or keeps an event for later.
    #[test]
    fn keys_that_arrive_together_with_a_line_break_are_one_paste() {
        let script = ScriptedInput::new(typed_at_once("Bob\rCarol\tX"));
        let mut input = TerminalInput::new(true);
        assert_eq!(
            script.next(&mut input),
            event::Event::Paste("Bob\nCarol\tX".to_owned())
        );
        assert!(script.is_done() && input.queued.is_empty());

        // A line break at the end of a paste is part of the paste too.
        let trailing = ScriptedInput::new(typed_at_once("Bob\r"));
        let mut input = TerminalInput::new(true);
        assert_eq!(
            trailing.next(&mut input),
            event::Event::Paste("Bob\n".to_owned())
        );
    }

    // Ways this can fail: the folded paste still runs the launch menu, moves the focus to the next
    // box, or keeps its second line in a single-line box.
    #[test]
    fn a_paste_typed_as_keys_fills_the_launch_menu_box_and_never_runs() {
        let form = RunFormView::from_declarations(
            "demo",
            "Demo",
            &[
                skit_domain::parameters::ParamDecl::new("name"),
                skit_domain::parameters::ParamDecl::new("other"),
            ],
            &std::collections::BTreeMap::new(),
            &[],
            "",
            &std::collections::BTreeMap::new(),
            "",
        );
        let mut state = LibraryState::default();
        state.update(Action::Present(Screen::Run(Box::new(form))));
        let mut session = TuiSession::default();
        let geometry = ViewGeometry::default();
        let script = ScriptedInput::new(typed_at_once("Bob\rCarol"));
        let mut input = TerminalInput::new(true);
        while !script.is_done() || !input.queued.is_empty() {
            let event = script.next(&mut input);
            if let Some(action) = dispatch_event(&mut session, event, &state, &geometry).action {
                assert!(
                    !matches!(action, Action::Submit),
                    "the paste ran the launch menu"
                );
                state.update(action);
            }
        }
        let form = state.run_form().unwrap();
        assert_eq!(form.fields()[0].control.value(), "Bob");
        assert_eq!(form.fields()[1].control.value(), "");
        assert_eq!(form.focused(), 0, "the paste moved the focus");
    }

    // Ways this can fail: a key that a person types alone waits for a paste that never comes, or
    // joins the release that comes after it; a held Enter or Tab, which repeats only itself,
    // becomes a paste; typed text with no line break becomes a paste.
    #[test]
    fn keys_that_are_not_a_paste_stay_keys() {
        let enter = KeyCode::Enter;
        let alone = ScriptedInput::new([
            (Duration::ZERO, key_press(enter, KeyModifiers::NONE)),
            (
                Duration::from_millis(80),
                key_release(enter, KeyModifiers::NONE),
            ),
        ]);
        let mut input = TerminalInput::new(true);
        assert_eq!(alone.next(&mut input), key_press(enter, KeyModifiers::NONE));
        assert_eq!(
            alone.next(&mut input),
            key_release(enter, KeyModifiers::NONE)
        );

        // The release of a key can wait in the input with the key, and the key stays a key.
        let with_release = ScriptedInput::new(typed_at_once("\r"));
        assert_eq!(
            with_release.next(&mut input),
            key_press(enter, KeyModifiers::NONE)
        );
        assert_eq!(
            with_release.next(&mut input),
            key_release(enter, KeyModifiers::NONE)
        );

        for held in ["\t\t\t", "\r\r", "ab"] {
            let script = ScriptedInput::new(typed_at_once(held));
            let expected = typed_at_once(held);
            for (_, event) in expected {
                assert_eq!(script.next(&mut input), event, "{held:?}");
            }
        }
    }

    // Ways this can fail: a line feed, which a pseudo-console types as Ctrl+Enter, or a character
    // that Windows reports with AltGr as Ctrl+Alt is lost from the paste or ends it.
    #[test]
    fn a_paste_keeps_a_line_feed_and_an_altgr_character() {
        let script = ScriptedInput::new([
            (
                Duration::ZERO,
                key_press(
                    KeyCode::Char('@'),
                    KeyModifiers::CONTROL | KeyModifiers::ALT,
                ),
            ),
            (
                Duration::ZERO,
                key_press(KeyCode::Enter, KeyModifiers::CONTROL),
            ),
            (
                Duration::ZERO,
                key_press(KeyCode::Char('b'), KeyModifiers::NONE),
            ),
        ]);
        let mut input = TerminalInput::new(true);
        assert_eq!(
            script.next(&mut input),
            event::Event::Paste("@\nb".to_owned())
        );
    }

    // Ways this can fail: an event that no paste types, such as a mouse move or an arrow key, joins
    // the paste or is lost; a key that comes after the gap joins the burst.
    #[test]
    fn the_burst_ends_at_an_event_that_is_not_typed_text_or_at_the_gap() {
        let mouse = event::Event::Mouse(event::MouseEvent {
            kind: MouseEventKind::Moved,
            column: 1,
            row: 1,
            modifiers: KeyModifiers::NONE,
        });
        let none = KeyModifiers::NONE;
        let script = ScriptedInput::new([
            (Duration::ZERO, key_press(KeyCode::Char('a'), none)),
            (Duration::ZERO, key_press(KeyCode::Enter, none)),
            // A key that comes exactly one gap later is still part of the burst.
            (BURST_GAP, key_press(KeyCode::Char('b'), none)),
            (Duration::ZERO, mouse.clone()),
            (Duration::ZERO, key_press(KeyCode::Char('c'), none)),
            // An Enter that comes after the gap is a key of its own.
            (
                BURST_GAP + Duration::from_millis(1),
                key_press(KeyCode::Enter, none),
            ),
        ]);
        let mut input = TerminalInput::new(true);
        assert_eq!(
            script.next(&mut input),
            event::Event::Paste("a\nb".to_owned())
        );
        assert_eq!(script.next(&mut input), mouse);
        assert_eq!(script.next(&mut input), key_press(KeyCode::Char('c'), none));
        assert_eq!(script.next(&mut input), key_press(KeyCode::Enter, none));
    }

    // Ways this can fail: a terminal with bracketed paste waits after each key, or folds keys that
    // a person types; a poll window with no event reports one; a failed poll or read inside a
    // burst is lost.
    #[test]
    fn terminal_input_waits_only_for_a_burst_and_reports_every_failure() {
        let script = ScriptedInput::new(typed_at_once("a\rb"));
        let mut input = TerminalInput::new(false);
        for (_, event) in typed_at_once("a\rb") {
            assert_eq!(script.next(&mut input), event);
        }
        assert_eq!(script.polls.get(), 0, "a blocking wait must not poll");

        let empty = ScriptedInput::new([]);
        let mut input = TerminalInput::new(true);
        assert!(
            empty
                .next_in(&mut input, TerminalEventWait::Poll(BURST_GAP))
                .unwrap()
                .is_none()
        );

        let failing = ScriptedInput::new(typed_at_once("a\rb"));
        failing.poll_fails.set(true);
        assert!(
            failing
                .next_in(&mut input, TerminalEventWait::Blocking)
                .is_err()
        );

        let cut_short = ScriptedInput::new([
            (
                Duration::ZERO,
                key_press(KeyCode::Char('a'), KeyModifiers::NONE),
            ),
            (
                Duration::ZERO,
                key_press(KeyCode::Char('b'), KeyModifiers::NONE),
            ),
        ]);
        // The poll says that an event is there, but the read finds none.
        let mut polls = 0;
        assert!(
            input
                .next_with(
                    TerminalEventWait::Blocking,
                    || {
                        let event = cut_short.read()?;
                        cut_short.events.borrow_mut().clear();
                        Ok(event)
                    },
                    |_| {
                        polls += 1;
                        Ok(true)
                    },
                )
                .is_err()
        );
        assert_eq!(polls, 1);
    }

    // Ways this can fail: a fast second press starts a new sequence, so no double click is
    // possible; a slow one continues the old sequence; a key or a release counts as a press.
    #[test]
    fn a_press_after_the_double_click_time_starts_a_new_sequence() {
        let press = event::Event::Mouse(event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 1,
            row: 1,
            modifiers: event::KeyModifiers::NONE,
        });
        let release = event::Event::Mouse(event::MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
            column: 1,
            row: 1,
            modifiers: event::KeyModifiers::NONE,
        });
        let start = Instant::now();
        let mut clock = PressClock::default();
        assert!(clock.starts_new_sequence(&press, start));
        assert!(!clock.starts_new_sequence(&release, start + DOUBLE_CLICK * 4));
        assert!(!clock.starts_new_sequence(&event::Event::FocusGained, start));
        assert!(!clock.starts_new_sequence(&press, start + DOUBLE_CLICK));
        assert!(
            clock.starts_new_sequence(&press, start + DOUBLE_CLICK * 2 + Duration::from_millis(1))
        );
    }

    // Ways this can fail: a copy never reaches the terminal, or one copy reaches it twice.
    #[test]
    fn a_copy_reaches_the_terminal_once_as_osc_52() {
        let system = SystemClipboard::new(Vec::new(), Duration::ZERO);
        let mut session = TuiSession::default();
        let mut out = Vec::new();
        export_clipboard(&mut session, &mut out, &system).unwrap();
        assert!(out.is_empty());
        session.copy_for_test("hi");
        export_clipboard(&mut session, &mut out, &system).unwrap();
        assert_eq!(out, b"\x1b]52;c;aGk=\x07");
        export_clipboard(&mut session, &mut out, &system).unwrap();
        assert_eq!(out, b"\x1b]52;c;aGk=\x07");
    }

    // Ways this can fail: a copy does not reach the clipboard program of a local session, or one
    // copy reaches it twice.
    #[cfg(unix)]
    #[test]
    fn a_copy_reaches_the_clipboard_program_once() {
        let directory = tempfile::tempdir().unwrap();
        let saved = directory.path().join("copies.txt");
        let program = crate::system_clipboard::CopyProgram::new(
            "sh",
            [
                std::ffi::OsString::from("-c"),
                std::ffi::OsString::from("cat >> \"$1\""),
                std::ffi::OsString::from("sh"),
                saved.clone().into_os_string(),
            ],
        );
        let system = SystemClipboard::new(vec![program], Duration::from_secs(10));
        let mut session = TuiSession::default();
        let mut out = Vec::new();
        session.copy_for_test("hi");
        export_clipboard(&mut session, &mut out, &system).unwrap();
        export_clipboard(&mut session, &mut out, &system).unwrap();
        drop(system);
        assert_eq!(std::fs::read_to_string(&saved).unwrap(), "hi");
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

    /// A Library with one selected Python entry, so `Action::Edit` asks the host for the editor.
    fn library_with_one_script() -> LibraryState {
        use skit_application::LibraryScan;
        use skit_domain::{EntryKind, EntrySummary, StorageMode};

        LibraryState::from_scan(LibraryScan {
            entries: vec![EntrySummary {
                slug: Slug::parse("alpha").unwrap(),
                name: "Alpha".to_owned(),
                kind: EntryKind::parse("python").unwrap(),
                mode: StorageMode::Copy,
                description: String::new(),
                target: None,
            }],
            diagnostics: Vec::new(),
        })
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
            host_failure_status(&HostError, Locale::ZhCn),
            Action::SetStatus {
                kind: StatusKind::Error,
                message: "找不到条目：demo".to_owned(),
            }
        );
        assert_eq!(
            host_failure_status(&HostError, Locale::ZhTw),
            Action::SetStatus {
                kind: StatusKind::Error,
                message: "找不到項目：demo".to_owned(),
            }
        );
        assert_eq!(
            preflight_refusal_status(&HostError, Locale::ZhTw),
            Action::SetStatus {
                kind: StatusKind::Error,
                message: "錯誤：找不到項目：demo".to_owned(),
            }
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
        assert_eq!(state.status_kind(), StatusKind::Error);
    }

    #[test]
    fn a_saved_language_changes_the_running_terminal_locale() {
        assert_eq!(
            action_locale(&Action::PreferencesSaved {
                locale: "zh-TW".to_owned(),
                message: "Preferences saved".to_owned(),
                theme: None,
                accent: None,
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
        assert_eq!(state.status_kind(), StatusKind::Error);
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
                kind: StatusKind::Success,
            })
        };
        let mut locale = Locale::En;
        let (quit, outcome) = drain_host_effects_observed(
            &mut state,
            &mut host,
            Effect::Reload,
            &mut locale,
            &mut add_workflow_outcome,
            &mut |_| Ok(()),
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
            Appearance::default(),
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
                kind: StatusKind::Success,
            }],
            harmless_host,
            Locale::En,
            Appearance::default(),
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
                    kind: StatusKind::Success,
                })
            },
            Locale::En,
            Appearance::default(),
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

        // A host effect that needs no terminal is served on the screen.
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
        assert!(transitions.borrow().is_empty());

        // The same recorder is live when an effect gives the terminal to another program. This
        // distinguishes the actions above from an accidentally suppressed transition callback.
        let mut editing_state = library_with_one_script();
        let mut editing_host = harmless_host;
        assert_eq!(
            advance_hosted_action(
                &mut editing_state,
                Action::Edit,
                &mut preflight,
                &mut editing_host,
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

    // Ways this can fail: an effect that needs no terminal leads to one that does, and the
    // session serves the second one on its screen, so the editor draws over the interface; or
    // the session leaves its screen twice in one chain.
    #[test]
    fn a_chain_leaves_the_screen_just_before_its_first_effect_that_needs_the_terminal() {
        let mut state = library_with_one_script();
        let mut locale = Locale::En;
        let mut preflight = |_effect: &Effect| Ok::<(), HostError>(());
        let mut no_outcome = |_action: &Action| None::<()>;
        let events = std::cell::RefCell::new(Vec::new());
        // The reload answers with a request for the editor, and the editor ends the chain.
        let mut host = |effect: Effect| -> Result<Action, HostError> {
            events.borrow_mut().push(format!("host {effect:?}"));
            Ok(match effect {
                Effect::Reload => Action::Edit,
                _ => Action::ClearStatus,
            })
        };
        assert_eq!(
            advance_hosted_action(
                &mut state,
                Action::Reload,
                &mut preflight,
                &mut host,
                &mut locale,
                &mut no_outcome,
                &mut |transition| {
                    events.borrow_mut().push(format!("{transition:?}"));
                    Ok(())
                },
            )
            .unwrap(),
            HostedActionStep::Continue
        );
        assert_eq!(
            *events.borrow(),
            [
                "host Reload".to_owned(),
                "Suspend".to_owned(),
                "host Edit { selector: \"alpha\" }".to_owned(),
                "Resume".to_owned(),
            ]
        );
    }

    #[test]
    fn hosted_action_step_finishes_for_quit_or_outcome_independently() {
        let mut locale = Locale::En;
        let mut preflight = |_effect: &Effect| Ok::<(), HostError>(());

        // A run that ends the session quits while the run has the terminal.
        let mut quit_state = library_with_one_script();
        let mut quit_host = |_effect| -> Result<Action, HostError> { Ok(Action::Quit) };
        let mut no_outcome = |_action: &Action| None::<()>;
        let mut transitions = Vec::new();
        assert_eq!(
            advance_hosted_action(
                &mut quit_state,
                Action::Edit,
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
                kind: StatusKind::Success,
            })
        };
        let mut observe = add_workflow_outcome;
        transitions.clear();
        // The add commit needs no terminal, so the session finishes on its screen.
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
        assert!(transitions.is_empty());

        let mut continuing_state = library_with_one_script();
        let mut continuing_host = harmless_host;
        let mut no_outcome = |_action: &Action| None::<()>;
        transitions.clear();
        assert_eq!(
            advance_hosted_action(
                &mut continuing_state,
                Action::Edit,
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
        let error = collect_run_form(
            empty_run_form(),
            harmless_host,
            Locale::En,
            Appearance::default(),
        )
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
                theme: None,
                accent: None,
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
                theme: None,
                accent: None,
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

    // Ways this can fail: a drag or another button ends the press of a chip and changes only its
    // look, but the event asks for no frame, so the screen keeps the pressed look; the release
    // after the drag activates the chip; or a release that changes no look asks for a frame.
    #[test]
    fn a_press_that_ends_without_an_action_asks_for_a_frame() {
        use ratatui_core::{backend::TestBackend, terminal::Terminal};
        use skit_application::LibraryScan;
        use skit_domain::{EntryKind, EntrySummary, StorageMode};

        fn draw(
            terminal: &mut Terminal<TestBackend>,
            session: &mut TuiSession,
            state: &LibraryState,
        ) -> ViewGeometry {
            let mut geometry = ViewGeometry::default();
            terminal
                .draw(|frame| {
                    geometry = render_with_session(frame, state, Locale::En, session);
                })
                .unwrap();
            geometry
        }

        let state = LibraryState::from_scan(LibraryScan {
            entries: vec![EntrySummary {
                slug: Slug::parse("alpha").unwrap(),
                name: "Alpha".to_owned(),
                kind: EntryKind::parse("command").unwrap(),
                mode: StorageMode::Copy,
                description: String::new(),
                target: None,
            }],
            diagnostics: Vec::new(),
        });
        let mut session = TuiSession::default();
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let geometry = draw(&mut terminal, &mut session, &state);
        let buffer = terminal.backend().buffer();
        let (column, row) = (0..buffer.area.height)
            .rev()
            .find_map(|row| {
                let line = (0..buffer.area.width)
                    .map(|column| buffer[(column, row)].symbol())
                    .collect::<String>();
                line.find("? Help")
                    .map(|byte| (u16::try_from(line[..byte].chars().count()).unwrap(), row))
            })
            .expect("the footer shows the Help chip");
        let mouse = |kind, column| {
            event::Event::Mouse(event::MouseEvent {
                kind,
                column,
                row,
                modifiers: event::KeyModifiers::NONE,
            })
        };
        let label = column + 2;

        let pressed = dispatch_event(
            &mut session,
            mouse(MouseEventKind::Down(MouseButton::Left), label),
            &state,
            &geometry,
        );
        assert!(pressed.redraw, "a press asks for no frame");
        let geometry = draw(&mut terminal, &mut session, &state);
        let dragged = dispatch_event(
            &mut session,
            mouse(MouseEventKind::Drag(MouseButton::Left), label + 1),
            &state,
            &geometry,
        );
        assert_eq!(dragged.action, None);
        assert!(
            dragged.redraw,
            "a drag inside the chip ends the press, but asks for no frame"
        );
        let geometry = draw(&mut terminal, &mut session, &state);
        let released = dispatch_event(
            &mut session,
            mouse(MouseEventKind::Up(MouseButton::Left), label + 1),
            &state,
            &geometry,
        );
        assert_eq!(
            released.action, None,
            "the release after the drag activated the chip"
        );
        assert!(
            !released.redraw,
            "a release that changes no look asks for a frame"
        );

        let _ = dispatch_event(
            &mut session,
            mouse(MouseEventKind::Down(MouseButton::Left), label),
            &state,
            &geometry,
        );
        let geometry = draw(&mut terminal, &mut session, &state);
        let other = dispatch_event(
            &mut session,
            mouse(MouseEventKind::Down(MouseButton::Right), label),
            &state,
            &geometry,
        );
        assert_eq!(other.action, None);
        assert!(
            other.redraw,
            "another button ends the press, but asks for no frame"
        );
    }
}
