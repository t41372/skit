//! Frontend-neutral health and prompt-runner management state.

use serde::{Deserialize, Serialize};
pub use skit_application::health::{
    HealthIssue, HealthIssueKind, HealthRebuildOutcome, HealthSnapshot, MirrorHealth, UvHealth,
};
use skit_application::runner_management::{
    EditableArgvDialect, RunnerArgvError, RunnerCommandError, join_editable_argv,
    split_editable_argv, validate_runner_argv,
};

/// Frontend-neutral Health workflow.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HealthView {
    snapshot: HealthSnapshot,
    selected_issue: Option<usize>,
    rebuilt: Option<HealthRebuildOutcome>,
}

impl HealthView {
    /// Open a Health report from one host-collected snapshot.
    #[must_use]
    pub fn new(snapshot: HealthSnapshot) -> Self {
        let selected_issue = (!snapshot.issues.is_empty()).then_some(0);
        Self {
            snapshot,
            selected_issue,
            rebuilt: None,
        }
    }

    /// Current host-collected facts.
    #[must_use]
    pub const fn snapshot(&self) -> &HealthSnapshot {
        &self.snapshot
    }

    /// Selected issue index, if an issue exists.
    #[must_use]
    pub const fn selected_issue(&self) -> Option<usize> {
        self.selected_issue
    }

    /// Last explicit rebuild outcome.
    #[must_use]
    pub const fn rebuilt(&self) -> Option<&HealthRebuildOutcome> {
        self.rebuilt.as_ref()
    }

    /// Apply one semantic Health action.
    pub fn reduce(&mut self, action: HealthAction) -> HealthEffect {
        match action {
            HealthAction::Previous => self.move_selection(-1),
            HealthAction::Next => self.move_selection(1),
            HealthAction::PagePrevious(amount) => {
                self.move_selection(-isize::try_from(amount).unwrap_or(isize::MAX));
            }
            HealthAction::PageNext(amount) => {
                self.move_selection(isize::try_from(amount).unwrap_or(isize::MAX));
            }
            HealthAction::Home => {
                self.selected_issue = (!self.snapshot.issues.is_empty()).then_some(0)
            }
            HealthAction::End => {
                self.selected_issue = self.snapshot.issues.len().checked_sub(1);
            }
            HealthAction::SelectIssue(index) if index < self.snapshot.issues.len() => {
                self.selected_issue = Some(index);
            }
            HealthAction::SelectIssue(_) => {}
            HealthAction::Jump => {
                return self
                    .selected_slug()
                    .map_or(HealthEffect::None, HealthEffect::JumpToEntry);
            }
            HealthAction::ActivateIssue(index) => {
                if index < self.snapshot.issues.len() {
                    self.selected_issue = Some(index);
                    return HealthEffect::JumpToEntry(self.snapshot.issues[index].slug.clone());
                }
            }
            HealthAction::Rebuild => return HealthEffect::Rebuild,
            HealthAction::Rebuilt { snapshot, outcome } => {
                self.snapshot = *snapshot;
                self.selected_issue = (!self.snapshot.issues.is_empty()).then_some(0);
                self.rebuilt = Some(outcome);
            }
            HealthAction::Back => return HealthEffect::Close,
        }
        HealthEffect::None
    }

    fn selected_slug(&self) -> Option<String> {
        self.selected_issue
            .and_then(|index| self.snapshot.issues.get(index))
            .map(|issue| issue.slug.clone())
    }

    fn move_selection(&mut self, delta: isize) {
        let Some(selected) = self.selected_issue else {
            return;
        };
        let final_index = self.snapshot.issues.len().saturating_sub(1);
        self.selected_issue = Some(selected.saturating_add_signed(delta).min(final_index));
    }
}

/// A semantic Health action.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthAction {
    /// Select the preceding issue.
    Previous,
    /// Select the next issue.
    Next,
    /// Move toward the start by a viewport-sized amount.
    PagePrevious(usize),
    /// Move toward the end by a viewport-sized amount.
    PageNext(usize),
    /// Select the first issue.
    Home,
    /// Select the last issue.
    End,
    /// Select one visible issue.
    SelectIssue(usize),
    /// Jump to the selected entry.
    Jump,
    /// Select and jump from one mouse click.
    ActivateIssue(usize),
    /// Rebuild the registry and collect a fresh report.
    Rebuild,
    /// Apply the host's rebuilt report.
    Rebuilt {
        /// Complete report after rebuilding.
        snapshot: Box<HealthSnapshot>,
        /// Rebuild result retained under the report.
        outcome: HealthRebuildOutcome,
    },
    /// Return to the library without changing its selection.
    Back,
}

/// Host work requested by Health.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthEffect {
    /// No host work.
    #[default]
    None,
    /// Select this entry in the library.
    JumpToEntry(String),
    /// Rebuild the registry and recollect the report.
    Rebuild,
    /// Close Health.
    Close,
}

/// Opaque identity of one raw row or malformed enclosing container.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RunnerRowIdentity {
    /// Zero-based raw row index. A malformed container has no index.
    pub index: Option<usize>,
    /// Complete raw semantic snapshot token supplied by the store adapter.
    pub snapshot_token: String,
}

/// One complete runner-management row.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RunnerRow {
    /// Raw row or container identity.
    pub identity: RunnerRowIdentity,
    /// Normalized stable runner name when present.
    pub name: Option<String>,
    /// Parsed argv when every element is text.
    pub argv: Option<Vec<String>>,
    /// Stable malformed-row reason code.
    pub reason: Option<String>,
    /// Stable raw-shape display label.
    pub descriptor: String,
    /// Complete raw identities for this stable name at inspection time.
    pub key_identities: Vec<RunnerRowIdentity>,
    /// Prompt entries pinned to the active valid key.
    pub pinned_count: usize,
}

impl RunnerRow {
    /// Return whether the row has enough structure for exact repair.
    #[must_use]
    pub const fn is_editable(&self) -> bool {
        self.identity.index.is_some() && self.argv.is_some()
    }

    /// Return whether this row is an active valid stable-key definition.
    #[must_use]
    pub const fn is_valid(&self) -> bool {
        self.reason.is_none()
    }

    /// Stable user-facing row label.
    #[must_use]
    pub fn label(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.descriptor)
    }
}

/// Target protected by one save operation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunnerSaveTarget {
    /// Append a new stable runner key.
    New,
    /// Replace and coalesce all raw rows for one stable name.
    Named {
        /// Immutable name that prompt entries pin.
        name: String,
        /// Complete raw key snapshot used for compare-and-swap.
        expected: Vec<RunnerRowIdentity>,
    },
    /// Repair one recognizable raw row by exact identity.
    RawRow { expected: RunnerRowIdentity },
}

/// Validated runner save request.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RunnerSaveRequest {
    /// Stable runner name.
    pub name: String,
    /// Direct process argv.
    pub argv: Vec<String>,
    /// Atomic mutation target.
    pub target: RunnerSaveTarget,
}

/// Compare-and-swap runner removal request.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunnerRemoveRequest {
    /// Remove every raw row for a stable key.
    Named {
        /// Stable key.
        name: String,
        /// Complete raw key snapshot from before confirmation.
        expected: Vec<RunnerRowIdentity>,
        /// Prompt pins shown when the user confirmed removal.
        expected_pinned_count: usize,
    },
    /// Remove one malformed row or container.
    RawRow { expected: RunnerRowIdentity },
}

/// Editable runner field.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunnerEditorField {
    /// Stable name.
    #[default]
    Name,
    /// One-line argv representation.
    Command,
}

/// Stable editor purpose for titles and host routing.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunnerEditorMode {
    /// Create a new named runner.
    New,
    /// Edit an existing stable runner key.
    Edit,
    /// Repair one malformed anonymous row.
    Repair,
}

/// Inline runner-editor validation error.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunnerEditorError {
    /// The name is empty for a new or anonymous row.
    NameRequired,
    /// The command has unbalanced quotes.
    UnbalancedQuotes,
    /// No program or an empty argument was supplied.
    EmptyCommand,
    /// `{{prompt}}` does not occur exactly once.
    PromptSlotCount,
    /// `{{prompt}}` occurs in the program token.
    PromptInProgram,
    /// Another double-brace hole occurs.
    UnsupportedHole,
}

impl From<RunnerCommandError> for RunnerEditorError {
    fn from(value: RunnerCommandError) -> Self {
        match value {
            RunnerCommandError::UnbalancedQuotes => Self::UnbalancedQuotes,
            RunnerCommandError::EmptyCommand => Self::EmptyCommand,
            RunnerCommandError::PromptSlotCount => Self::PromptSlotCount,
            RunnerCommandError::PromptInProgram => Self::PromptInProgram,
            RunnerCommandError::UnsupportedHole => Self::UnsupportedHole,
        }
    }
}

impl From<RunnerArgvError> for RunnerEditorError {
    fn from(value: RunnerArgvError) -> Self {
        match value {
            RunnerArgvError::EmptyCommand => Self::EmptyCommand,
            RunnerArgvError::PromptSlotCount => Self::PromptSlotCount,
            RunnerArgvError::PromptInProgram => Self::PromptInProgram,
            RunnerArgvError::UnsupportedHole => Self::UnsupportedHole,
        }
    }
}

/// Shared typed runner editor used by every runner-picking surface.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RunnerEditorView {
    name: String,
    command: String,
    target: RunnerSaveTarget,
    focused: RunnerEditorField,
    error: Option<RunnerEditorError>,
    host_error: Option<String>,
}

impl Default for RunnerEditorView {
    fn default() -> Self {
        Self::new()
    }
}

impl RunnerEditorView {
    /// Open an empty new-runner editor.
    #[must_use]
    pub fn new() -> Self {
        Self {
            name: String::new(),
            command: String::new(),
            target: RunnerSaveTarget::New,
            focused: RunnerEditorField::Name,
            error: None,
            host_error: None,
        }
    }

    /// Edit one named row while keeping its pin key immutable.
    #[must_use]
    pub fn edit(row: &RunnerRow) -> Self {
        let name = row.name.clone().unwrap_or_default();
        Self {
            command: row.argv.as_ref().map_or_else(String::new, |argv| {
                join_editable_argv(argv, EditableArgvDialect::host())
            }),
            target: RunnerSaveTarget::Named {
                name: name.clone(),
                expected: row.key_identities.clone(),
            },
            name,
            focused: RunnerEditorField::Command,
            error: None,
            host_error: None,
        }
    }

    /// Repair one anonymous raw row in place.
    #[must_use]
    pub fn repair(row: &RunnerRow) -> Self {
        Self {
            name: String::new(),
            command: row.argv.as_ref().map_or_else(String::new, |argv| {
                join_editable_argv(argv, EditableArgvDialect::host())
            }),
            target: RunnerSaveTarget::RawRow {
                expected: row.identity.clone(),
            },
            focused: RunnerEditorField::Name,
            error: None,
            host_error: None,
        }
    }

    /// Name field value.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Editable argv representation.
    #[must_use]
    pub fn command(&self) -> &str {
        &self.command
    }

    /// Current field focus.
    #[must_use]
    pub const fn focused(&self) -> RunnerEditorField {
        self.focused
    }

    /// Local validation error.
    #[must_use]
    pub const fn error(&self) -> Option<&RunnerEditorError> {
        self.error.as_ref()
    }

    /// Host mutation failure that keeps the user's input visible.
    #[must_use]
    pub fn host_error(&self) -> Option<&str> {
        self.host_error.as_deref()
    }

    /// Return whether the stable name is locked by existing prompt pins.
    #[must_use]
    pub const fn name_is_locked(&self) -> bool {
        matches!(self.target, RunnerSaveTarget::Named { .. })
    }

    /// Stable editor purpose.
    #[must_use]
    pub const fn mode(&self) -> RunnerEditorMode {
        match self.target {
            RunnerSaveTarget::New => RunnerEditorMode::New,
            RunnerSaveTarget::Named { .. } => RunnerEditorMode::Edit,
            RunnerSaveTarget::RawRow { .. } => RunnerEditorMode::Repair,
        }
    }

    /// Apply one editor action.
    pub fn reduce(&mut self, action: RunnerEditorAction) -> RunnerEditorEffect {
        match action {
            RunnerEditorAction::SetName(value) => {
                if !self.name_is_locked() {
                    self.name = value;
                }
                self.clear_errors();
            }
            RunnerEditorAction::SetCommand(value) => {
                self.command = value;
                self.clear_errors();
            }
            RunnerEditorAction::Focus(field) => {
                if field == RunnerEditorField::Command || !self.name_is_locked() {
                    self.focused = field;
                }
            }
            RunnerEditorAction::FocusNext | RunnerEditorAction::FocusPrevious => {
                self.focused = if self.name_is_locked() {
                    RunnerEditorField::Command
                } else {
                    match self.focused {
                        RunnerEditorField::Name => RunnerEditorField::Command,
                        RunnerEditorField::Command => RunnerEditorField::Name,
                    }
                };
            }
            RunnerEditorAction::Submit => return self.submit(),
            RunnerEditorAction::Cancel => return RunnerEditorEffect::Cancel,
            RunnerEditorAction::MutationFailed(message) => self.set_host_error(message),
        }
        RunnerEditorEffect::None
    }

    fn clear_errors(&mut self) {
        self.error = None;
        self.host_error = None;
    }

    fn submit(&mut self) -> RunnerEditorEffect {
        let name = match &self.target {
            RunnerSaveTarget::Named { name, .. } => name.clone(),
            RunnerSaveTarget::New | RunnerSaveTarget::RawRow { .. } => {
                let name = self.name.trim().to_owned();
                if name.is_empty() {
                    self.error = Some(RunnerEditorError::NameRequired);
                    self.focused = RunnerEditorField::Name;
                    return RunnerEditorEffect::None;
                }
                name
            }
        };
        let argv = match split_editable_argv(self.command.trim(), EditableArgvDialect::host()) {
            Ok(argv) => argv,
            Err(error) => {
                self.error = Some(error.into());
                self.focused = RunnerEditorField::Command;
                return RunnerEditorEffect::None;
            }
        };
        if let Err(error) = validate_runner_argv(&argv) {
            self.error = Some(error.into());
            self.focused = RunnerEditorField::Command;
            return RunnerEditorEffect::None;
        }
        RunnerEditorEffect::Save(RunnerSaveRequest {
            name,
            argv,
            target: self.target.clone(),
        })
    }

    fn set_host_error(&mut self, error: String) {
        self.error = None;
        self.host_error = Some(error);
    }
}

/// Semantic runner-editor action.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunnerEditorAction {
    /// Replace the complete name input.
    SetName(String),
    /// Replace the complete command input.
    SetCommand(String),
    /// Focus one input.
    Focus(RunnerEditorField),
    /// Focus the next input.
    FocusNext,
    /// Focus the preceding input.
    FocusPrevious,
    /// Validate and save.
    Submit,
    /// Close without saving.
    Cancel,
    /// Keep all typed input after a host mutation refusal.
    MutationFailed(String),
}

/// Result of one standalone runner-editor action.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunnerEditorEffect {
    /// No host work.
    #[default]
    None,
    /// Persist one validated runner.
    Save(RunnerSaveRequest),
    /// Close the editor.
    Cancel,
}

/// Visible runner removal confirmation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RunnerRemovalView {
    /// Row label.
    pub name: String,
    /// Prompt pins that become unresolved for a valid active key.
    pub pinned_count: usize,
    /// Whether this repairs a malformed row rather than removing an active key.
    pub invalid_row: bool,
    /// Whether this repairs a malformed enclosing container.
    pub container: bool,
    request: RunnerRemoveRequest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum RunnerManagerOverlay {
    Actions(usize),
    Editor(Box<RunnerEditorView>),
    Removal(RunnerRemovalView),
}

/// Complete prompt-runner management workflow.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RunnerManagerView {
    rows: Vec<RunnerRow>,
    selected: Option<usize>,
    overlay: Option<RunnerManagerOverlay>,
    status: Option<String>,
}

impl RunnerManagerView {
    /// Open the complete registry, including malformed rows and containers.
    #[must_use]
    pub fn new(rows: Vec<RunnerRow>) -> Self {
        let selected = (!rows.is_empty()).then_some(0);
        Self {
            rows,
            selected,
            overlay: None,
            status: None,
        }
    }

    /// Complete raw row projection.
    #[must_use]
    pub fn rows(&self) -> &[RunnerRow] {
        &self.rows
    }

    /// Selected row index.
    #[must_use]
    pub const fn selected(&self) -> Option<usize> {
        self.selected
    }

    /// Active shared editor.
    #[must_use]
    pub fn editor(&self) -> Option<&RunnerEditorView> {
        match &self.overlay {
            Some(RunnerManagerOverlay::Editor(editor)) => Some(editor),
            Some(RunnerManagerOverlay::Actions(_) | RunnerManagerOverlay::Removal(_)) | None => {
                None
            }
        }
    }

    /// Active row-action target.
    #[must_use]
    pub const fn action_row(&self) -> Option<usize> {
        match self.overlay {
            Some(RunnerManagerOverlay::Actions(index)) => Some(index),
            Some(RunnerManagerOverlay::Editor(_) | RunnerManagerOverlay::Removal(_)) | None => None,
        }
    }

    /// Active removal confirmation.
    #[must_use]
    pub const fn removal(&self) -> Option<&RunnerRemovalView> {
        match &self.overlay {
            Some(RunnerManagerOverlay::Removal(removal)) => Some(removal),
            Some(RunnerManagerOverlay::Actions(_) | RunnerManagerOverlay::Editor(_)) | None => None,
        }
    }

    /// Last host feedback.
    #[must_use]
    pub fn status(&self) -> Option<&str> {
        self.status.as_deref()
    }

    /// Apply one runner-management action.
    pub fn reduce(&mut self, action: RunnerManagerAction) -> RunnerManagerEffect {
        match action {
            RunnerManagerAction::Previous => self.move_selection(-1),
            RunnerManagerAction::Next => self.move_selection(1),
            RunnerManagerAction::PagePrevious(amount) => {
                self.move_selection(-isize::try_from(amount).unwrap_or(isize::MAX));
            }
            RunnerManagerAction::PageNext(amount) => {
                self.move_selection(isize::try_from(amount).unwrap_or(isize::MAX));
            }
            RunnerManagerAction::Home => self.selected = (!self.rows.is_empty()).then_some(0),
            RunnerManagerAction::End => self.selected = self.rows.len().checked_sub(1),
            RunnerManagerAction::Select(index) if index < self.rows.len() => {
                self.selected = Some(index);
            }
            RunnerManagerAction::Select(_) => {}
            RunnerManagerAction::ActivateSelected => {
                if let Some(index) = self.selected {
                    self.overlay = Some(RunnerManagerOverlay::Actions(index));
                }
            }
            RunnerManagerAction::ActivateRow(index) if index < self.rows.len() => {
                self.selected = Some(index);
                self.overlay = Some(RunnerManagerOverlay::Actions(index));
            }
            RunnerManagerAction::ActivateRow(_) => {}
            RunnerManagerAction::New => {
                self.overlay = Some(RunnerManagerOverlay::Editor(Box::default()));
            }
            RunnerManagerAction::EditSelected => self.open_editor(),
            RunnerManagerAction::RemoveSelected => self.open_removal(),
            RunnerManagerAction::CloseActions => self.overlay = None,
            RunnerManagerAction::Editor(action) => {
                let Some(RunnerManagerOverlay::Editor(editor)) = &mut self.overlay else {
                    return RunnerManagerEffect::None;
                };
                match editor.reduce(action) {
                    RunnerEditorEffect::None => {}
                    RunnerEditorEffect::Save(request) => {
                        return RunnerManagerEffect::Save(request);
                    }
                    RunnerEditorEffect::Cancel => self.overlay = None,
                }
            }
            RunnerManagerAction::CancelEditor => self.overlay = None,
            RunnerManagerAction::ConfirmRemove => {
                if let Some(RunnerManagerOverlay::Removal(removal)) = &self.overlay {
                    return RunnerManagerEffect::Remove(removal.request.clone());
                }
            }
            RunnerManagerAction::CancelRemove => self.overlay = None,
            RunnerManagerAction::MutationSucceeded {
                rows,
                selected_name,
                message,
            } => {
                self.rows = rows;
                self.selected = selected_name
                    .as_deref()
                    .and_then(|name| {
                        self.rows
                            .iter()
                            .position(|row| row.name.as_deref() == Some(name))
                    })
                    .or_else(|| (!self.rows.is_empty()).then_some(0));
                self.overlay = None;
                self.status = Some(message);
            }
            RunnerManagerAction::MutationFailed(message) => {
                if let Some(RunnerManagerOverlay::Editor(editor)) = &mut self.overlay {
                    editor.set_host_error(message);
                } else {
                    self.overlay = None;
                    self.status = Some(message);
                }
            }
            RunnerManagerAction::Back => {
                if self.overlay.is_some() {
                    self.overlay = None;
                } else {
                    return RunnerManagerEffect::Close;
                }
            }
        }
        RunnerManagerEffect::None
    }

    fn move_selection(&mut self, delta: isize) {
        let Some(selected) = self.selected else {
            return;
        };
        self.selected = Some(
            selected
                .saturating_add_signed(delta)
                .min(self.rows.len().saturating_sub(1)),
        );
    }

    fn open_editor(&mut self) {
        if let Some(row) = self
            .action_row()
            .or(self.selected)
            .and_then(|index| self.rows.get(index))
            .filter(|row| row.is_editable())
        {
            let editor = if row.name.is_some() {
                RunnerEditorView::edit(row)
            } else {
                RunnerEditorView::repair(row)
            };
            self.overlay = Some(RunnerManagerOverlay::Editor(Box::new(editor)));
        }
    }

    fn open_removal(&mut self) {
        if let Some(row) = self
            .action_row()
            .or(self.selected)
            .and_then(|index| self.rows.get(index))
        {
            let request = if row.is_valid() {
                RunnerRemoveRequest::Named {
                    name: row.name.clone().unwrap_or_default(),
                    expected: row.key_identities.clone(),
                    expected_pinned_count: row.pinned_count,
                }
            } else {
                RunnerRemoveRequest::RawRow {
                    expected: row.identity.clone(),
                }
            };
            self.overlay = Some(RunnerManagerOverlay::Removal(RunnerRemovalView {
                name: row.label().to_owned(),
                pinned_count: if row.is_valid() { row.pinned_count } else { 0 },
                invalid_row: !row.is_valid(),
                container: row.identity.index.is_none(),
                request,
            }));
        }
    }
}

/// Semantic action for the complete runner manager.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunnerManagerAction {
    /// Select the preceding row.
    Previous,
    /// Select the next row.
    Next,
    /// Move toward the start by a viewport-sized amount.
    PagePrevious(usize),
    /// Move toward the end by a viewport-sized amount.
    PageNext(usize),
    /// Select the first row.
    Home,
    /// Select the last row.
    End,
    /// Select one row.
    Select(usize),
    /// Open actions for the selected row.
    ActivateSelected,
    /// Select a mouse row and open its actions.
    ActivateRow(usize),
    /// Open the reusable new-runner editor.
    New,
    /// Edit the active action row.
    EditSelected,
    /// Confirm removal of the active action row.
    RemoveSelected,
    /// Close the action overlay.
    CloseActions,
    /// Delegate to the shared editor.
    Editor(RunnerEditorAction),
    /// Close an editor without saving.
    CancelEditor,
    /// Confirm the pending removal.
    ConfirmRemove,
    /// Keep the pending runner.
    CancelRemove,
    /// Apply a refreshed registry after one successful mutation.
    MutationSucceeded {
        /// Complete raw registry.
        rows: Vec<RunnerRow>,
        /// Stable name to keep selected after a save.
        selected_name: Option<String>,
        /// Localized host completion message.
        message: String,
    },
    /// Keep typed editor input or list position after a host refusal.
    MutationFailed(String),
    /// Close an overlay first, then the manager.
    Back,
}

/// Host work requested by runner management.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunnerManagerEffect {
    /// No host work.
    #[default]
    None,
    /// Save a validated runner.
    Save(RunnerSaveRequest),
    /// Remove one stable key or raw malformed row.
    Remove(RunnerRemoveRequest),
    /// Return to the owning workflow.
    Close,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn issue(slug: &str, kind: HealthIssueKind) -> HealthIssue {
        HealthIssue {
            slug: slug.to_owned(),
            name: slug.to_owned(),
            kind,
        }
    }

    fn health_snapshot() -> HealthSnapshot {
        HealthSnapshot {
            uv: UvHealth::Found("/usr/bin/uv".to_owned()),
            entry_count: 4,
            issues: vec![
                issue("gone", HealthIssueKind::MissingTarget),
                issue("drift", HealthIssueKind::DriftedForm),
            ],
            invalid_runner_rows: vec!["bad".to_owned()],
            mirror: MirrorHealth::Off,
            library_path: "/data/scripts".to_owned(),
            library_size: "2 KiB".to_owned(),
            diagnostics: Vec::new(),
        }
    }

    fn identity(index: Option<usize>, token: &str) -> RunnerRowIdentity {
        RunnerRowIdentity {
            index,
            snapshot_token: token.to_owned(),
        }
    }

    fn valid_row(name: &str, index: usize, token: &str, pinned_count: usize) -> RunnerRow {
        RunnerRow {
            identity: identity(Some(index), token),
            name: Some(name.to_owned()),
            argv: Some(vec![name.to_owned(), "{{prompt}}".to_owned()]),
            reason: None,
            descriptor: name.to_owned(),
            key_identities: vec![identity(Some(index), token)],
            pinned_count,
        }
    }

    #[test]
    fn health_selection_is_value_safe_and_jump_emits_the_selected_slug() {
        let mut view = HealthView::new(health_snapshot());
        assert_eq!(view.selected_issue(), Some(0));
        assert_eq!(view.reduce(HealthAction::Next), HealthEffect::None);
        assert_eq!(view.selected_issue(), Some(1));
        assert_eq!(
            view.reduce(HealthAction::Jump),
            HealthEffect::JumpToEntry("drift".to_owned())
        );
        assert_eq!(
            view.reduce(HealthAction::ActivateIssue(0)),
            HealthEffect::JumpToEntry("gone".to_owned())
        );
    }

    #[test]
    fn health_rebuild_replaces_the_whole_report_and_keeps_the_outcome() {
        let mut view = HealthView::new(health_snapshot());
        assert_eq!(view.reduce(HealthAction::Rebuild), HealthEffect::Rebuild);
        let mut refreshed = health_snapshot();
        refreshed.entry_count = 7;
        refreshed.issues.clear();
        view.reduce(HealthAction::Rebuilt {
            snapshot: Box::new(refreshed),
            outcome: HealthRebuildOutcome {
                entry_count: 7,
                problems: vec!["orphan: meta.toml is missing; skipped".to_owned()],
            },
        });
        assert_eq!(view.snapshot().entry_count, 7);
        assert_eq!(view.selected_issue(), None);
        assert_eq!(view.rebuilt().unwrap().entry_count, 7);
        assert_eq!(view.rebuilt().unwrap().problems.len(), 1);
    }

    #[test]
    fn editor_reports_each_v040_runner_command_rule_inline() {
        let cases = [
            ("", RunnerEditorError::EmptyCommand),
            ("agent", RunnerEditorError::PromptSlotCount),
            ("{{prompt}} agent", RunnerEditorError::PromptInProgram),
            (
                "agent {{prompt}} {{model}}",
                RunnerEditorError::UnsupportedHole,
            ),
            ("agent '{{prompt}}", RunnerEditorError::UnbalancedQuotes),
        ];
        for (command, expected) in cases {
            let mut editor = RunnerEditorView::new();
            editor.reduce(RunnerEditorAction::SetName("mine".to_owned()));
            editor.reduce(RunnerEditorAction::SetCommand(command.to_owned()));
            assert_eq!(
                editor.reduce(RunnerEditorAction::Submit),
                RunnerEditorEffect::None
            );
            assert_eq!(editor.error(), Some(&expected), "command={command:?}");
        }
    }

    #[test]
    fn standalone_host_refusal_keeps_typed_input_and_editor_mode() {
        let mut editor = RunnerEditorView::new();
        editor.reduce(RunnerEditorAction::SetName("mine".to_owned()));
        editor.reduce(RunnerEditorAction::SetCommand(
            "agent --message {{prompt}}".to_owned(),
        ));

        assert_eq!(editor.mode(), RunnerEditorMode::New);
        assert_eq!(
            editor.reduce(RunnerEditorAction::MutationFailed(
                "Runner config changed".to_owned(),
            )),
            RunnerEditorEffect::None
        );
        assert_eq!(editor.name(), "mine");
        assert_eq!(editor.command(), "agent --message {{prompt}}");
        assert_eq!(editor.host_error(), Some("Runner config changed"));
    }

    #[test]
    fn editor_builds_typed_argv_and_edit_keeps_the_stable_pin_key() {
        let row = valid_row("codex", 1, "old", 3);
        let mut editor = RunnerEditorView::edit(&row);
        editor.reduce(RunnerEditorAction::SetName("renamed".to_owned()));
        editor.reduce(RunnerEditorAction::Focus(RunnerEditorField::Name));
        assert_eq!(editor.focused(), RunnerEditorField::Command);
        editor.reduce(RunnerEditorAction::FocusPrevious);
        assert_eq!(editor.focused(), RunnerEditorField::Command);
        editor.reduce(RunnerEditorAction::SetCommand(
            "codex --model o3 '{{prompt}}'".to_owned(),
        ));
        let effect = editor.reduce(RunnerEditorAction::Submit);
        assert_eq!(
            effect,
            RunnerEditorEffect::Save(RunnerSaveRequest {
                name: "codex".to_owned(),
                argv: vec![
                    "codex".to_owned(),
                    "--model".to_owned(),
                    "o3".to_owned(),
                    "{{prompt}}".to_owned(),
                ],
                target: RunnerSaveTarget::Named {
                    name: "codex".to_owned(),
                    expected: vec![identity(Some(1), "old")],
                },
            })
        );
    }

    #[test]
    fn health_navigation_covers_empty_boundaries_pages_and_invalid_mouse_rows() {
        let mut empty_snapshot = health_snapshot();
        empty_snapshot.issues.clear();
        let mut empty = HealthView::new(empty_snapshot);
        let before = serde_json::to_value(&empty).unwrap();
        for action in [
            HealthAction::Previous,
            HealthAction::Next,
            HealthAction::PagePrevious(usize::MAX),
            HealthAction::PageNext(usize::MAX),
            HealthAction::Home,
            HealthAction::End,
            HealthAction::SelectIssue(9),
            HealthAction::Jump,
            HealthAction::ActivateIssue(9),
        ] {
            assert_eq!(empty.reduce(action), HealthEffect::None);
        }
        assert_eq!(serde_json::to_value(&empty).unwrap(), before);
        assert_eq!(empty.reduce(HealthAction::Back), HealthEffect::Close);

        let mut populated = HealthView::new(health_snapshot());
        populated.reduce(HealthAction::PageNext(usize::MAX));
        assert_eq!(populated.selected_issue(), Some(1));
        populated.reduce(HealthAction::PagePrevious(usize::MAX));
        assert_eq!(populated.selected_issue(), Some(0));
        populated.reduce(HealthAction::End);
        assert_eq!(populated.selected_issue(), Some(1));
        populated.reduce(HealthAction::Home);
        assert_eq!(populated.selected_issue(), Some(0));
        populated.reduce(HealthAction::SelectIssue(1));
        assert_eq!(populated.selected_issue(), Some(1));
    }

    #[test]
    fn runner_editor_and_manager_edges_are_typed_and_preserve_invalid_state() {
        for error in [
            RunnerCommandError::EmptyCommand,
            RunnerCommandError::PromptSlotCount,
            RunnerCommandError::PromptInProgram,
            RunnerCommandError::UnsupportedHole,
        ] {
            assert!(matches!(
                RunnerEditorError::from(error),
                RunnerEditorError::EmptyCommand
                    | RunnerEditorError::PromptSlotCount
                    | RunnerEditorError::PromptInProgram
                    | RunnerEditorError::UnsupportedHole
            ));
        }
        for error in [
            RunnerArgvError::EmptyCommand,
            RunnerArgvError::PromptSlotCount,
            RunnerArgvError::PromptInProgram,
            RunnerArgvError::UnsupportedHole,
        ] {
            assert_eq!(
                RunnerEditorError::from(error),
                match error {
                    RunnerArgvError::EmptyCommand => RunnerEditorError::EmptyCommand,
                    RunnerArgvError::PromptSlotCount => RunnerEditorError::PromptSlotCount,
                    RunnerArgvError::PromptInProgram => RunnerEditorError::PromptInProgram,
                    RunnerArgvError::UnsupportedHole => RunnerEditorError::UnsupportedHole,
                }
            );
        }

        let raw = RunnerRow {
            identity: identity(Some(2), "raw"),
            name: None,
            argv: Some(vec!["agent".to_owned(), "{{prompt}}".to_owned()]),
            reason: Some("name".to_owned()),
            descriptor: "raw row".to_owned(),
            key_identities: Vec::new(),
            pinned_count: 0,
        };
        let mut repair = RunnerEditorView::repair(&raw);
        assert_eq!(repair.mode(), RunnerEditorMode::Repair);
        repair.reduce(RunnerEditorAction::Focus(RunnerEditorField::Command));
        assert_eq!(repair.focused(), RunnerEditorField::Command);
        repair.reduce(RunnerEditorAction::FocusNext);
        assert_eq!(repair.focused(), RunnerEditorField::Name);
        repair.reduce(RunnerEditorAction::FocusPrevious);
        assert_eq!(repair.focused(), RunnerEditorField::Command);

        let named = RunnerEditorView::edit(&valid_row("named", 0, "named", 0));
        assert_eq!(named.mode(), RunnerEditorMode::Edit);
        let mut missing_name = RunnerEditorView::new();
        missing_name.reduce(RunnerEditorAction::SetCommand(
            "agent {{prompt}}".to_owned(),
        ));
        assert_eq!(
            missing_name.reduce(RunnerEditorAction::Submit),
            RunnerEditorEffect::None
        );
        assert_eq!(missing_name.error(), Some(&RunnerEditorError::NameRequired));
        assert_eq!(missing_name.focused(), RunnerEditorField::Name);

        let mut empty = RunnerManagerView::new(Vec::new());
        let before = serde_json::to_value(&empty).unwrap();
        for action in [
            RunnerManagerAction::Previous,
            RunnerManagerAction::Next,
            RunnerManagerAction::PagePrevious(usize::MAX),
            RunnerManagerAction::PageNext(usize::MAX),
            RunnerManagerAction::Home,
            RunnerManagerAction::End,
            RunnerManagerAction::Select(9),
            RunnerManagerAction::ActivateSelected,
            RunnerManagerAction::ActivateRow(9),
            RunnerManagerAction::EditSelected,
            RunnerManagerAction::RemoveSelected,
            RunnerManagerAction::CloseActions,
            RunnerManagerAction::Editor(RunnerEditorAction::Cancel),
            RunnerManagerAction::ConfirmRemove,
            RunnerManagerAction::CancelRemove,
        ] {
            assert_eq!(empty.reduce(action), RunnerManagerEffect::None);
        }
        assert_eq!(serde_json::to_value(&empty).unwrap(), before);
        assert_eq!(
            empty.reduce(RunnerManagerAction::Back),
            RunnerManagerEffect::Close
        );
        assert_eq!(
            empty.reduce(RunnerManagerAction::EditSelected),
            RunnerManagerEffect::None
        );
        assert_eq!(
            empty.reduce(RunnerManagerAction::RemoveSelected),
            RunnerManagerEffect::None
        );

        let rows = vec![valid_row("one", 0, "one", 0), raw.clone()];
        let mut manager = RunnerManagerView::new(rows.clone());
        manager.reduce(RunnerManagerAction::PageNext(usize::MAX));
        assert_eq!(manager.selected(), Some(1));
        manager.reduce(RunnerManagerAction::PagePrevious(usize::MAX));
        assert_eq!(manager.selected(), Some(0));
        manager.reduce(RunnerManagerAction::End);
        assert_eq!(manager.selected(), Some(1));
        manager.reduce(RunnerManagerAction::Home);
        assert_eq!(manager.selected(), Some(0));
        manager.reduce(RunnerManagerAction::Select(1));
        manager.reduce(RunnerManagerAction::ActivateRow(1));
        assert_eq!(manager.action_row(), Some(1));
        manager.reduce(RunnerManagerAction::EditSelected);
        assert_eq!(
            manager.editor().map(RunnerEditorView::mode),
            Some(RunnerEditorMode::Repair)
        );
        manager.reduce(RunnerManagerAction::CancelEditor);
        manager.reduce(RunnerManagerAction::ActivateRow(1));
        manager.reduce(RunnerManagerAction::RemoveSelected);
        assert!(matches!(
            manager.removal(),
            Some(removal) if removal.invalid_row
                && matches!(removal.request, RunnerRemoveRequest::RawRow { .. })
        ));
        assert!(matches!(
            manager.reduce(RunnerManagerAction::ConfirmRemove),
            RunnerManagerEffect::Remove(RunnerRemoveRequest::RawRow { .. })
        ));
        manager.reduce(RunnerManagerAction::CancelRemove);
        assert!(manager.removal().is_none());

        manager.reduce(RunnerManagerAction::New);
        assert!(manager.editor().is_some());
        manager.reduce(RunnerManagerAction::Editor(RunnerEditorAction::Cancel));
        assert!(manager.editor().is_none());
        manager.reduce(RunnerManagerAction::New);
        manager.reduce(RunnerManagerAction::CancelEditor);
        assert!(manager.editor().is_none());
        manager.reduce(RunnerManagerAction::MutationFailed("failed".to_owned()));
        assert_eq!(manager.status(), Some("failed"));

        manager.reduce(RunnerManagerAction::MutationSucceeded {
            rows: rows.clone(),
            selected_name: Some("one".to_owned()),
            message: "saved".to_owned(),
        });
        assert_eq!(manager.selected(), Some(0));
        assert_eq!(manager.status(), Some("saved"));
        manager.reduce(RunnerManagerAction::MutationSucceeded {
            rows: vec![raw],
            selected_name: Some("missing".to_owned()),
            message: "refreshed".to_owned(),
        });
        assert_eq!(manager.selected(), Some(0));
        manager.reduce(RunnerManagerAction::MutationSucceeded {
            rows: Vec::new(),
            selected_name: None,
            message: "empty".to_owned(),
        });
        assert_eq!(manager.selected(), None);
    }

    #[test]
    fn manager_represents_valid_invalid_anonymous_and_container_rows_without_loss() {
        let valid = valid_row("same", 0, "valid", 2);
        let duplicate = RunnerRow {
            identity: identity(Some(1), "duplicate"),
            name: Some("same".to_owned()),
            argv: Some(vec!["second".to_owned(), "{{prompt}}".to_owned()]),
            reason: Some("duplicate".to_owned()),
            descriptor: "same".to_owned(),
            key_identities: vec![identity(Some(0), "valid"), identity(Some(1), "duplicate")],
            pinned_count: 0,
        };
        let anonymous = RunnerRow {
            identity: identity(Some(2), "anonymous"),
            name: None,
            argv: Some(vec!["valuable".to_owned(), "{{prompt}}".to_owned()]),
            reason: Some("name".to_owned()),
            descriptor: "raw anonymous row".to_owned(),
            key_identities: Vec::new(),
            pinned_count: 0,
        };
        let container = RunnerRow {
            identity: identity(None, "container"),
            name: None,
            argv: None,
            reason: Some("prompt-section-not-table".to_owned()),
            descriptor: "prompt".to_owned(),
            key_identities: Vec::new(),
            pinned_count: 0,
        };
        let view = RunnerManagerView::new(vec![valid, duplicate, anonymous, container]);
        assert_eq!(view.rows().len(), 4);
        assert_eq!(view.rows()[0].pinned_count, 2);
        assert!(view.rows()[2].is_editable());
        assert!(!view.rows()[3].is_editable());
    }

    #[test]
    fn manager_routes_edit_remove_confirmation_and_stale_feedback_without_losing_input() {
        let mut row = valid_row("same", 0, "valid", 2);
        row.key_identities.push(identity(Some(1), "duplicate"));
        let mut view = RunnerManagerView::new(vec![row]);
        view.reduce(RunnerManagerAction::ActivateSelected);
        view.reduce(RunnerManagerAction::EditSelected);
        view.reduce(RunnerManagerAction::Editor(RunnerEditorAction::SetCommand(
            "mine --flag {{prompt}}".to_owned(),
        )));
        assert_eq!(
            view.reduce(RunnerManagerAction::Editor(RunnerEditorAction::Submit)),
            RunnerManagerEffect::Save(RunnerSaveRequest {
                name: "same".to_owned(),
                argv: vec![
                    "mine".to_owned(),
                    "--flag".to_owned(),
                    "{{prompt}}".to_owned()
                ],
                target: RunnerSaveTarget::Named {
                    name: "same".to_owned(),
                    expected: vec![identity(Some(0), "valid"), identity(Some(1), "duplicate"),],
                },
            })
        );
        view.reduce(RunnerManagerAction::MutationFailed(
            "Runner config changed".to_owned(),
        ));
        assert_eq!(view.editor().unwrap().command(), "mine --flag {{prompt}}");
        assert_eq!(
            view.editor().unwrap().host_error(),
            Some("Runner config changed")
        );

        view.reduce(RunnerManagerAction::CancelEditor);
        view.reduce(RunnerManagerAction::ActivateSelected);
        view.reduce(RunnerManagerAction::RemoveSelected);
        assert_eq!(view.removal().unwrap().pinned_count, 2);
        assert_eq!(
            view.reduce(RunnerManagerAction::ConfirmRemove),
            RunnerManagerEffect::Remove(RunnerRemoveRequest::Named {
                name: "same".to_owned(),
                expected: vec![identity(Some(0), "valid"), identity(Some(1), "duplicate"),],
                expected_pinned_count: 2,
            })
        );
    }

    #[test]
    fn anonymous_repair_is_exact_row_targeted_and_cannot_rename_a_stable_key() {
        let anonymous = RunnerRow {
            identity: identity(Some(4), "raw"),
            name: None,
            argv: Some(vec!["valuable".to_owned(), "{{prompt}}".to_owned()]),
            reason: Some("name".to_owned()),
            descriptor: "raw".to_owned(),
            key_identities: Vec::new(),
            pinned_count: 0,
        };
        let mut editor = RunnerEditorView::repair(&anonymous);
        editor.reduce(RunnerEditorAction::SetName("valuable".to_owned()));
        assert_eq!(
            editor.reduce(RunnerEditorAction::Submit),
            RunnerEditorEffect::Save(RunnerSaveRequest {
                name: "valuable".to_owned(),
                argv: vec!["valuable".to_owned(), "{{prompt}}".to_owned()],
                target: RunnerSaveTarget::RawRow {
                    expected: identity(Some(4), "raw")
                },
            })
        );
    }
}
