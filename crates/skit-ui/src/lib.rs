//! Frontend-neutral state and reducer shared by terminal and future GUI adapters.

#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use skit_application::{Diagnostic, LibraryScan};
use skit_domain::{EntrySummary, Slug};

/// Which key grammar is currently active.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InputMode {
    /// Navigation and command shortcuts.
    #[default]
    Browse,
    /// Printable keys edit the library filter.
    Search,
}

/// A user intent independent of terminal, webview, or native-window event types.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// Select the preceding visible entry.
    Previous,
    /// Select the next visible entry.
    Next,
    /// Move up by a viewport-sized approximation.
    PagePrevious,
    /// Move down by a viewport-sized approximation.
    PageNext,
    /// Select the first visible entry.
    Home,
    /// Select the final visible entry.
    End,
    /// Select a row by its index in the filtered projection.
    SelectVisible(usize),
    /// Enter search-editing mode.
    BeginSearch,
    /// Leave search-editing mode while keeping the filter.
    FinishSearch,
    /// Append one character to the filter.
    Input(char),
    /// Delete one character from the filter.
    Backspace,
    /// Clear the filter.
    ClearSearch,
    /// Replace the library projection after a refresh.
    Replace(LibraryScan),
    /// Ask the host adapter to refresh application data.
    Reload,
    /// Set a host-generated status line.
    SetStatus(String),
    /// Clear the status line.
    ClearStatus,
    /// Ask the host adapter to exit.
    Quit,
}

/// Side effect requested by the pure reducer.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Effect {
    /// No host-side work.
    #[default]
    None,
    /// Reload application data through the repository port.
    Reload,
    /// Close the frontend.
    Quit,
}

/// Serializable state rendered by Ratatui today and available to a future Tauri shell.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct LibraryState {
    entries: Vec<EntrySummary>,
    diagnostics: Vec<Diagnostic>,
    query: String,
    input_mode: InputMode,
    selected: Option<usize>,
    visible: Vec<usize>,
    status: Option<String>,
}

impl LibraryState {
    /// Build state from one application-layer scan.
    #[must_use]
    pub fn from_scan(scan: LibraryScan) -> Self {
        let mut state = Self {
            entries: scan.entries,
            diagnostics: scan.diagnostics,
            ..Self::default()
        };
        state.recompute_visible(None);
        state
    }

    /// Apply one action and return only the host effect it requests.
    pub fn update(&mut self, action: Action) -> Effect {
        match action {
            Action::Previous => self.move_selection(-1),
            Action::Next => self.move_selection(1),
            Action::PagePrevious => self.move_selection(-10),
            Action::PageNext => self.move_selection(10),
            Action::Home => self.select_boundary(false),
            Action::End => self.select_boundary(true),
            Action::SelectVisible(index) => {
                if index < self.visible.len() {
                    self.selected = Some(index);
                }
            }
            Action::BeginSearch => self.input_mode = InputMode::Search,
            Action::FinishSearch => self.input_mode = InputMode::Browse,
            Action::Input(character) => {
                if self.input_mode == InputMode::Search {
                    let selected = self.selected_slug().cloned();
                    self.query.push(character);
                    self.recompute_visible(selected.as_ref());
                }
            }
            Action::Backspace => {
                if self.input_mode == InputMode::Search {
                    let selected = self.selected_slug().cloned();
                    self.query.pop();
                    self.recompute_visible(selected.as_ref());
                }
            }
            Action::ClearSearch => {
                let selected = self.selected_slug().cloned();
                self.query.clear();
                self.recompute_visible(selected.as_ref());
            }
            Action::Replace(scan) => {
                let selected = self.selected_slug().cloned();
                self.entries = scan.entries;
                self.diagnostics = scan.diagnostics;
                self.recompute_visible(selected.as_ref());
            }
            Action::Reload => return Effect::Reload,
            Action::SetStatus(message) => self.status = Some(message),
            Action::ClearStatus => self.status = None,
            Action::Quit => return Effect::Quit,
        }
        Effect::None
    }

    /// Return the current input grammar.
    #[must_use]
    pub const fn input_mode(&self) -> InputMode {
        self.input_mode
    }

    /// Return the active filter text.
    #[must_use]
    pub fn query(&self) -> &str {
        &self.query
    }

    /// Return all scan diagnostics.
    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// Return the current status line.
    #[must_use]
    pub fn status(&self) -> Option<&str> {
        self.status.as_deref()
    }

    /// Iterate over entries surviving the current filter.
    pub fn visible_entries(&self) -> impl ExactSizeIterator<Item = &EntrySummary> {
        self.visible.iter().map(|index| &self.entries[*index])
    }

    /// Return the selected visible-row index.
    #[must_use]
    pub const fn selected_visible_index(&self) -> Option<usize> {
        self.selected
    }

    /// Return the selected entry.
    #[must_use]
    pub fn selected(&self) -> Option<&EntrySummary> {
        self.selected
            .and_then(|visible_index| self.visible.get(visible_index))
            .and_then(|entry_index| self.entries.get(*entry_index))
    }

    fn selected_slug(&self) -> Option<&Slug> {
        self.selected().map(|entry| &entry.slug)
    }

    fn move_selection(&mut self, delta: isize) {
        if self.visible.is_empty() {
            self.selected = None;
            return;
        }
        let current = self.selected.unwrap_or_default();
        let last = self.visible.len() - 1;
        let next = if delta.is_negative() {
            current.saturating_sub(delta.unsigned_abs())
        } else {
            current.saturating_add(delta as usize).min(last)
        };
        self.selected = Some(next);
    }

    fn select_boundary(&mut self, end: bool) {
        self.selected = if self.visible.is_empty() {
            None
        } else if end {
            Some(self.visible.len() - 1)
        } else {
            Some(0)
        };
    }

    fn recompute_visible(&mut self, preferred: Option<&Slug>) {
        let needle = self.query.to_lowercase();
        self.visible = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| matches(entry, &needle).then_some(index))
            .collect();
        self.selected = if self.visible.is_empty() {
            None
        } else {
            preferred
                .and_then(|slug| {
                    self.visible
                        .iter()
                        .position(|index| self.entries[*index].slug == *slug)
                })
                .or(Some(0))
        };
    }
}

fn matches(entry: &EntrySummary, needle: &str) -> bool {
    needle.is_empty()
        || entry.name.to_lowercase().contains(needle)
        || entry.slug.as_str().to_lowercase().contains(needle)
        || entry.kind.as_str().to_lowercase().contains(needle)
        || entry.description.to_lowercase().contains(needle)
}
