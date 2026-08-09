//! Crossterm lifecycle and blocking event loop.

use std::{collections::BTreeMap, io};

use ratatui_core::terminal::Terminal;
use ratatui_crossterm::{
    CrosstermBackend,
    crossterm::{
        event::{self, DisableMouseCapture, EnableMouseCapture},
        execute,
        terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
    },
};
use skit_domain::Slug;
use skit_i18n::{Locale, Localize, Message, detect_locale};
use skit_ui::{Action, AddWorkflowState, Effect, FormView, LibraryState, RunFormView, Screen};
use thiserror::Error;

use crate::{EventHandling, TuiSession, ViewGeometry, render_with_session};

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
    let _: Option<()> = run_hosted_state(state, host, locale, |_| None)?;
    Ok(())
}

/// Run the typed add workflow with the same hosted reducer loop as the library workbench.
///
/// The result is the created slug, or `None` when the user cancels or quits. Completion and
/// cancellation use explicit reducer actions. The adapter does not infer an outcome from library
/// selection or status text.
pub fn run_add_workflow<F, E>(
    workflow: AddWorkflowState,
    host: F,
    locale: Locale,
) -> Result<Option<Slug>, TuiError>
where
    F: FnMut(Effect) -> Result<Action, E>,
    E: Localize,
{
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Add(Box::new(workflow))));
    run_hosted_state(state, host, locale, add_workflow_outcome).map(|outcome| {
        outcome.and_then(|outcome| match outcome {
            AddWorkflowOutcome::Completed(slug) => Some(slug),
            AddWorkflowOutcome::Cancelled => None,
        })
    })
}

fn run_hosted_state<F, E, O>(
    mut state: LibraryState,
    mut host: F,
    mut locale: Locale,
    mut observe: impl FnMut(&Action) -> Option<O>,
) -> Result<Option<O>, TuiError>
where
    F: FnMut(Effect) -> Result<Action, E>,
    E: Localize,
{
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
    let _restore = RestoreTerminal;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;
    let mut session = TuiSession::default();

    loop {
        let mut geometry = ViewGeometry::default();
        terminal.draw(|frame| {
            geometry = render_with_session(frame, &state, locale, &mut session);
        })?;
        let EventHandling::Action(action) = session.handle_event(event::read()?, &state, &geometry)
        else {
            continue;
        };
        let mut outcome = observe(&action);
        let effect = state.update(action);
        match effect {
            Effect::None if outcome.is_some() => {
                terminal.show_cursor()?;
                return Ok(outcome);
            }
            Effect::None => {}
            Effect::Quit => {
                terminal.show_cursor()?;
                return Ok(outcome);
            }
            effect => {
                terminal.show_cursor()?;
                suspend_terminal()?;
                let (quit, host_outcome) = drain_host_effects_observed(
                    &mut state,
                    &mut host,
                    effect,
                    &mut locale,
                    &mut observe,
                )?;
                if outcome.is_none() {
                    outcome = host_outcome;
                }
                if quit || outcome.is_some() {
                    return Ok(outcome);
                }
                resume_terminal()?;
                terminal.clear()?;
            }
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
pub fn collect_form(
    form: FormView,
    locale: Locale,
) -> Result<Option<BTreeMap<String, String>>, TuiError> {
    collect_screen(Screen::Form(form), locale)
}

/// Collect one typed launch form and restore the terminal before returning its values.
pub fn collect_run_form(
    form: RunFormView,
    locale: Locale,
) -> Result<Option<BTreeMap<String, String>>, TuiError> {
    collect_screen(Screen::Run(Box::new(form)), locale)
}

fn collect_screen(
    screen: Screen,
    locale: Locale,
) -> Result<Option<BTreeMap<String, String>>, TuiError> {
    let mut state = LibraryState::default();
    state.update(Action::Present(screen));
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
    let _restore = RestoreTerminal;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;
    let mut session = TuiSession::default();

    loop {
        let mut geometry = ViewGeometry::default();
        terminal.draw(|frame| {
            geometry = render_with_session(frame, &state, locale, &mut session);
        })?;
        let EventHandling::Action(action) = session.handle_event(event::read()?, &state, &geometry)
        else {
            continue;
        };
        if action == Action::Back || action == Action::Quit {
            return Ok(None);
        }
        if let Effect::Submit { values, .. } = state.update(action) {
            return Ok(Some(values));
        }
    }
}

#[derive(Debug)]
struct RestoreTerminal;

fn suspend_terminal() -> io::Result<()> {
    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture)
}

fn resume_terminal() -> io::Result<()> {
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)
}

impl Drop for RestoreTerminal {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct HostError;

    impl Localize for HostError {
        fn message(&self) -> Message {
            Message::new("entry not found: {}").with("demo")
        }
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
                scan: skit_application::LibraryScan::default(),
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
}
