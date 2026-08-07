//! Crossterm lifecycle and blocking event loop.

use std::{fmt::Display, io};

use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    crossterm::{
        event::{self, DisableMouseCapture, EnableMouseCapture},
        execute,
        terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
    },
};
use skit_application::LibraryScan;
use skit_ui::{Action, Effect, LibraryState};
use thiserror::Error;

use crate::{ViewGeometry, map_event, render};

/// A terminal lifecycle or input failure.
#[derive(Debug, Error)]
pub enum TuiError {
    /// Crossterm or terminal backend I/O failed.
    #[error("terminal I/O failed: {0}")]
    Io(#[from] io::Error),
}

/// Run the terminal frontend, using the callback only when the user explicitly refreshes.
pub fn run<F, E>(mut state: LibraryState, mut reload: F) -> Result<(), TuiError>
where
    F: FnMut() -> Result<LibraryScan, E>,
    E: Display,
{
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
    let _restore = RestoreTerminal;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    loop {
        let mut geometry = ViewGeometry::default();
        terminal.draw(|frame| geometry = render(frame, &state))?;
        let Some(action) = map_event(event::read()?, &state, &geometry) else {
            continue;
        };
        match state.update(action) {
            Effect::None => {}
            Effect::Quit => break,
            Effect::Reload => match reload() {
                Ok(scan) => {
                    state.update(Action::Replace(scan));
                    state.update(Action::ClearStatus);
                }
                Err(error) => {
                    state.update(Action::SetStatus(error.to_string()));
                }
            },
        }
    }
    terminal.show_cursor()?;
    Ok(())
}

#[derive(Debug)]
struct RestoreTerminal;

impl Drop for RestoreTerminal {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
    }
}
