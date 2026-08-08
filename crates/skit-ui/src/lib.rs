//! Frontend-neutral state and reducer shared by terminal and future GUI adapters.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;

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
    /// Printable keys edit the active form field.
    Form,
}

/// Identify data that a frontend must request from its host adapter.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HostRequest {
    /// Build a launch form for the selected entry.
    Run,
    /// Build a form for a new library entry.
    Add,
    /// Build an entry settings form.
    Settings,
    /// Build the application preferences form.
    Preferences,
    /// Build a health report.
    Health,
    /// Build the prompt runner manager.
    Runners,
    /// Build the preset manager for the selected entry.
    Presets,
    /// Build a rename form for the selected entry.
    Rename,
}

/// Identify the operation that owns a generic form.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FormPurpose {
    /// Launch one entry.
    Run,
    /// Add one entry.
    Add,
    /// Change one entry.
    Settings,
    /// Change application preferences.
    Preferences,
    /// Add or change a prompt runner.
    Runners,
    /// Add or change a parameter preset.
    Presets,
    /// Rename one entry.
    Rename,
}

/// One editable frontend-neutral form field.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FormField {
    /// Stable field key used by host adapters.
    pub key: String,
    /// User-visible field label.
    pub label: String,
    /// Current text value.
    pub value: String,
    /// Hide the value during presentation.
    pub secret: bool,
    /// Permit embedded line breaks.
    pub multiline: bool,
}

impl FormField {
    /// Create one plain text field.
    #[must_use]
    pub fn text(
        key: impl Into<String>,
        label: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            value: value.into(),
            secret: false,
            multiline: false,
        }
    }

    /// Create one masked text field.
    #[must_use]
    pub fn secret(
        key: impl Into<String>,
        label: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        Self {
            secret: true,
            ..Self::text(key, label, value)
        }
    }

    /// Create one multiline text field.
    #[must_use]
    pub fn multiline(
        key: impl Into<String>,
        label: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        Self {
            multiline: true,
            ..Self::text(key, label, value)
        }
    }
}

/// One complete form that any frontend can render.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FormView {
    /// Operation performed when the form is submitted.
    pub purpose: FormPurpose,
    /// User-visible form title.
    pub title: String,
    /// Entry selector when the form owns an existing entry.
    pub selector: Option<String>,
    /// Fields in navigation order.
    pub fields: Vec<FormField>,
    /// Active field index.
    pub focused: usize,
    /// User-visible label for the submit action.
    pub submit_label: String,
}

/// One row in a read-only report.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReportItem {
    /// Short machine-stable status such as `ok` or `error`.
    pub status: String,
    /// User-visible check name.
    pub label: String,
    /// User-visible result detail.
    pub detail: String,
}

/// A read-only report that a frontend can present.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReportView {
    /// User-visible report title.
    pub title: String,
    /// Report rows.
    pub items: Vec<ReportItem>,
}

/// Current frontend screen.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Screen {
    /// Searchable library browser.
    #[default]
    Library,
    /// Generic editable form.
    Form(FormView),
    /// Generic read-only report.
    Report(ReportView),
    /// Destructive confirmation for one selected entry.
    ConfirmRemove {
        /// Stable entry selector.
        selector: String,
        /// User-visible entry name.
        name: String,
    },
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
    /// Request a launch form for the selected entry.
    OpenRun,
    /// Request the add-entry form.
    OpenAdd,
    /// Request settings for the selected entry.
    OpenSettings,
    /// Request application preferences.
    OpenPreferences,
    /// Request the health report.
    OpenHealth,
    /// Request the prompt runner manager.
    OpenRunners,
    /// Request presets for the selected entry.
    OpenPresets,
    /// Request a rename form for the selected entry.
    OpenRename,
    /// Ask the host to open the selected entry in an editor.
    Edit,
    /// Show a remove confirmation for the selected entry.
    AskRemove,
    /// Move focus to the next form field.
    FocusNext,
    /// Move focus to the preceding form field.
    FocusPrevious,
    /// Focus one form field by its index.
    FocusField(usize),
    /// Submit the active form or confirmation.
    Submit,
    /// Return to the library.
    Back,
    /// Present a screen built by the host adapter.
    Present(Screen),
    /// Finish a host operation and optionally replace the library scan.
    Complete {
        /// New library state after a mutation.
        scan: Option<LibraryScan>,
        /// User-visible completion message.
        message: String,
    },
    /// Set a host-generated status line.
    SetStatus(String),
    /// Clear the status line.
    ClearStatus,
    /// Ask the host adapter to exit.
    Quit,
}

/// Side effect requested by the pure reducer.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Effect {
    /// No host-side work.
    #[default]
    None,
    /// Reload application data through the repository port.
    Reload,
    /// Close the frontend.
    Quit,
    /// Ask the host to build one screen.
    Open {
        /// Requested operation.
        request: HostRequest,
        /// Selected entry, when the operation needs one.
        selector: Option<String>,
    },
    /// Submit a generic form to the host.
    Submit {
        /// Operation that owns the form.
        purpose: FormPurpose,
        /// Entry selector, when present.
        selector: Option<String>,
        /// Field values indexed by stable key.
        values: BTreeMap<String, String>,
    },
    /// Open the selected entry in the configured editor.
    Edit {
        /// Stable entry selector.
        selector: String,
    },
    /// Remove one entry after explicit confirmation.
    Remove {
        /// Stable entry selector.
        selector: String,
    },
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
    screen: Screen,
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
            Action::Input(character) => match self.input_mode {
                InputMode::Search => {
                    let selected = self.selected_slug().cloned();
                    self.query.push(character);
                    self.recompute_visible(selected.as_ref());
                }
                InputMode::Form => {
                    if let Some(field) = self.focused_field_mut() {
                        field.value.push(character);
                    }
                }
                InputMode::Browse => {}
            },
            Action::Backspace => match self.input_mode {
                InputMode::Search => {
                    let selected = self.selected_slug().cloned();
                    self.query.pop();
                    self.recompute_visible(selected.as_ref());
                }
                InputMode::Form => {
                    if let Some(field) = self.focused_field_mut() {
                        field.value.pop();
                    }
                }
                InputMode::Browse => {}
            },
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
            Action::OpenRun => return self.open_selected(HostRequest::Run),
            Action::OpenAdd => return self.open(HostRequest::Add, None),
            Action::OpenSettings => return self.open_selected(HostRequest::Settings),
            Action::OpenPreferences => return self.open(HostRequest::Preferences, None),
            Action::OpenHealth => return self.open(HostRequest::Health, None),
            Action::OpenRunners => return self.open(HostRequest::Runners, None),
            Action::OpenPresets => return self.open_selected(HostRequest::Presets),
            Action::OpenRename => return self.open_selected(HostRequest::Rename),
            Action::Edit => {
                return self
                    .selected_selector()
                    .map_or(Effect::None, |selector| Effect::Edit { selector });
            }
            Action::AskRemove => {
                if let Some(entry) = self.selected() {
                    self.screen = Screen::ConfirmRemove {
                        selector: entry.slug.as_str().to_owned(),
                        name: entry.name.clone(),
                    };
                    self.input_mode = InputMode::Browse;
                }
            }
            Action::FocusNext => self.move_form_focus(1),
            Action::FocusPrevious => self.move_form_focus(-1),
            Action::FocusField(index) => {
                if let Screen::Form(form) = &mut self.screen
                    && index < form.fields.len()
                {
                    form.focused = index;
                }
            }
            Action::Submit => return self.submit(),
            Action::Back => {
                self.screen = Screen::Library;
                self.input_mode = InputMode::Browse;
            }
            Action::Present(screen) => {
                self.input_mode = if matches!(screen, Screen::Form(_)) {
                    InputMode::Form
                } else {
                    InputMode::Browse
                };
                self.screen = screen;
            }
            Action::Complete { scan, message } => {
                if let Some(scan) = scan {
                    let selected = self.selected_slug().cloned();
                    self.entries = scan.entries;
                    self.diagnostics = scan.diagnostics;
                    self.recompute_visible(selected.as_ref());
                }
                self.status = Some(message);
                self.screen = Screen::Library;
                self.input_mode = InputMode::Browse;
            }
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

    /// Return the active screen.
    #[must_use]
    pub const fn screen(&self) -> &Screen {
        &self.screen
    }

    /// Return the active form, when present.
    #[must_use]
    pub fn form(&self) -> Option<&FormView> {
        match &self.screen {
            Screen::Form(form) => Some(form),
            Screen::Library | Screen::Report(_) | Screen::ConfirmRemove { .. } => None,
        }
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

    fn selected_selector(&self) -> Option<String> {
        self.selected_slug().map(|slug| slug.as_str().to_owned())
    }

    fn open_selected(&self, request: HostRequest) -> Effect {
        self.selected_selector()
            .map_or(Effect::None, |selector| self.open(request, Some(selector)))
    }

    fn open(&self, request: HostRequest, selector: Option<String>) -> Effect {
        Effect::Open { request, selector }
    }

    fn focused_field_mut(&mut self) -> Option<&mut FormField> {
        match &mut self.screen {
            Screen::Form(form) => form.fields.get_mut(form.focused),
            Screen::Library | Screen::Report(_) | Screen::ConfirmRemove { .. } => None,
        }
    }

    fn move_form_focus(&mut self, delta: isize) {
        let Screen::Form(form) = &mut self.screen else {
            return;
        };
        if form.fields.is_empty() {
            form.focused = 0;
            return;
        }
        let last = form.fields.len() - 1;
        form.focused = if delta.is_negative() {
            form.focused.saturating_sub(delta.unsigned_abs())
        } else {
            form.focused.saturating_add(delta as usize).min(last)
        };
    }

    fn submit(&self) -> Effect {
        match &self.screen {
            Screen::Form(form) => Effect::Submit {
                purpose: form.purpose,
                selector: form.selector.clone(),
                values: form
                    .fields
                    .iter()
                    .map(|field| (field.key.clone(), field.value.clone()))
                    .collect(),
            },
            Screen::ConfirmRemove { selector, .. } => Effect::Remove {
                selector: selector.clone(),
            },
            Screen::Library | Screen::Report(_) => Effect::None,
        }
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
