//! Frontend-neutral health and prompt-runner management state.

use serde::{Deserialize, Serialize};
pub use skit_application::health::{
    HealthIssue, HealthIssueKind, HealthRebuildOutcome, HealthSnapshot, MirrorHealth, UvHealth,
};
use skit_application::runner_management::{
    EditableArgvDialect, RunnerArgvError, RunnerCommandError, join_editable_argv,
    split_editable_argv, validate_runner_argv,
};
pub use skit_application::runner_management::{
    RunnerRow, RunnerRowIdentity, RunnerSaveRequest, RunnerSaveTarget,
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
    /// Another agent row that the save keeps already uses the name.
    NameTaken,
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

    /// Open a new-agent editor prefilled from one staged row.
    #[must_use]
    pub fn staged(name: &str, argv: &[String]) -> Self {
        Self {
            name: name.to_owned(),
            command: join_editable_argv(argv, EditableArgvDialect::host()),
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
    ///
    /// A row that already carries a typed name reopens with it, so a repair the user started is
    /// never retyped from nothing.
    #[must_use]
    pub fn repair(row: &RunnerRow) -> Self {
        Self {
            name: row.name.clone().unwrap_or_default(),
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

    /// Refuse one validated save and keep every typed value visible.
    pub fn refuse(&mut self, error: RunnerEditorError) {
        self.host_error = None;
        self.error = Some(error);
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
            // An unpaired DOUBLE quote is UnbalancedQuotes in both argument dialects: the
            // POSIX arm's shlex refuses an unterminated quote, and the Windows arm's
            // validate_windows_quotes refuses an odd count of unescaped quotes. The editor
            // parses with the host dialect, so the rule row must use the spelling both
            // hosts refuse; each dialect's own specifics are owned by the parameterized
            // split_editable_argv owners.
            ("agent \"{{prompt}}", RunnerEditorError::UnbalancedQuotes),
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
        // Double quotes strip in both argument dialects (shlex on POSIX, the Windows
        // argv rules on Windows), so the typed argv below holds on every host; single
        // quotes strip only on POSIX.
        editor.reduce(RunnerEditorAction::SetCommand(
            "codex --model o3 \"{{prompt}}\"".to_owned(),
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
    fn runner_editor_edges_are_typed_and_preserve_invalid_state() {
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
