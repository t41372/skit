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
use skit_i18n::{Locale, Localize, Message};
use skit_ui::{Action, Effect, FormView, LibraryState, Screen};
use thiserror::Error;

use crate::{ViewGeometry, map_event, render_localized};

/// A terminal lifecycle or input failure.
#[derive(Debug, Error)]
pub enum TuiError {
    /// Crossterm or terminal backend I/O failed.
    #[error("terminal I/O failed: {0}")]
    Io(#[from] io::Error),
}

impl Localize for TuiError {
    fn message(&self) -> Message {
        match self {
            Self::Io(error) => Message::new("terminal I/O failed: {}").with(error),
        }
    }
}

/// Run the terminal frontend and send each requested effect to its host adapter.
pub fn run<F, E>(mut state: LibraryState, mut host: F, locale: Locale) -> Result<(), TuiError>
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

    loop {
        let mut geometry = ViewGeometry::default();
        terminal.draw(|frame| geometry = render_localized(frame, &state, locale))?;
        let Some(action) = map_event(event::read()?, &state, &geometry) else {
            continue;
        };
        let effect = state.update(action);
        match effect {
            Effect::None => {}
            Effect::Quit => break,
            effect => {
                terminal.show_cursor()?;
                suspend_terminal()?;
                let result = host(effect);
                if result.as_ref().is_ok_and(|action| *action == Action::Quit) {
                    return Ok(());
                }
                resume_terminal()?;
                terminal.clear()?;
                match result {
                    Ok(action) => {
                        state.update(action);
                    }
                    Err(error) => {
                        state.update(Action::SetStatus(localized_status(&error, locale)));
                    }
                }
            }
        }
    }
    terminal.show_cursor()?;
    Ok(())
}

fn localized_status(error: &impl Localize, locale: Locale) -> String {
    error.message().localize(locale)
}

/// Collect one generic form and restore the terminal before returning its values.
pub fn collect_form(
    form: FormView,
    locale: Locale,
) -> Result<Option<BTreeMap<String, String>>, TuiError> {
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Form(form)));
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
    let _restore = RestoreTerminal;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    loop {
        let mut geometry = ViewGeometry::default();
        terminal.draw(|frame| geometry = render_localized(frame, &state, locale))?;
        let Some(action) = map_event(event::read()?, &state, &geometry) else {
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
}
