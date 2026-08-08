#![forbid(unsafe_code)]

use std::io;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use skit_core::{EntrySummary, Store};

/// The result of one input event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Keep the workbench open.
    None,
    /// Close the workbench.
    Quit,
}

/// State for the library workbench.
#[derive(Debug, Clone)]
pub struct App {
    entries: Vec<EntrySummary>,
    selected: usize,
}

impl App {
    /// Create a workbench from a library snapshot.
    #[must_use]
    pub fn new(entries: Vec<EntrySummary>) -> Self {
        Self {
            entries,
            selected: 0,
        }
    }

    /// Return the selected entry, if the library is not empty.
    #[must_use]
    pub fn selected(&self) -> Option<&EntrySummary> {
        self.entries.get(self.selected)
    }

    /// Apply one keyboard event.
    #[must_use]
    pub fn handle_key(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => Action::Quit,
            KeyCode::Down | KeyCode::Char('j') => {
                self.move_down();
                Action::None
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.move_up();
                Action::None
            }
            _ => Action::None,
        }
    }

    fn move_down(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        self.selected = (self.selected + 1).min(self.entries.len() - 1);
    }

    fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }
}

/// Render the current library workbench.
pub fn render(frame: &mut Frame<'_>, app: &mut App) {
    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .split(frame.area());

    let title = Paragraph::new("skit — script launcher and parameter manager")
        .style(Style::default().add_modifier(Modifier::BOLD));
    frame.render_widget(title, areas[0]);

    let items = if app.entries.is_empty() {
        vec![ListItem::new(
            "  No entries yet. Add one with: skit add <path>",
        )]
    } else {
        app.entries
            .iter()
            .enumerate()
            .map(|(index, entry)| {
                let marker = if index == app.selected { ">" } else { " " };
                let description = if entry.description.is_empty() {
                    "—"
                } else {
                    entry.description.as_str()
                };
                ListItem::new(format!(
                    "{marker} {}  [{}]  {description}",
                    entry.name, entry.kind
                ))
            })
            .collect()
    };
    let library = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Library "),
    );
    frame.render_widget(library, areas[1]);

    frame.render_widget(
        Paragraph::new("↑/↓ move   j/k move   q quit   Esc quit"),
        areas[2],
    );
}

/// Run the fullscreen Ratatui library workbench.
///
/// # Errors
///
/// Returns an I/O error if the terminal, input stream, or library cannot be read.
pub fn run(store: &Store) -> io::Result<()> {
    let entries = store.list().map_err(io::Error::other)?;
    let mut app = App::new(entries);
    ratatui::run(|terminal| {
        loop {
            terminal.draw(|frame| render(frame, &mut app))?;
            match event::read()? {
                Event::Key(key)
                    if key.kind == KeyEventKind::Press
                        && app.handle_key(key) == Action::Quit =>
                {
                    break Ok(());
                }
                _ => {}
            }
        }
    })
}
