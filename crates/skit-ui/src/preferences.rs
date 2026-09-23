//! Frontend-neutral application-preference state.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use skit_application::AgentTarget;
use skit_application::preferences::{
    AfterRunChoice, InteractiveFormChoice, JavascriptChoice, MirrorChoice, PreferencesChangeSet,
    PreferencesDraft, PreferencesError, PreferencesField, RunnerDraftError, RunnerDraftRow,
    github_preset_names, npm_preset_names, pypi_preset_names, runner_row_taken_by_its_key,
};
use skit_application::runner_management::RunnerSaveRequest;

use crate::management::RunnerEditorView;
use crate::{ChoicePresentation, FormInputKind};

/// One catalog key and its unformatted values.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct PreferencesDisplayText {
    /// English catalog key.
    pub key: String,
    /// Values inserted after translation.
    pub arguments: Vec<String>,
}

impl PreferencesDisplayText {
    /// Build text without replacement values.
    #[must_use]
    pub fn new(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            arguments: Vec::new(),
        }
    }

    /// Build text with one replacement value.
    #[must_use]
    pub fn with_argument(key: impl Into<String>, argument: impl Into<String>) -> Self {
        Self::with_arguments(key, [argument])
    }

    /// Build text with replacement values in source order.
    #[must_use]
    pub fn with_arguments<I, S>(key: impl Into<String>, arguments: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            key: key.into(),
            arguments: arguments.into_iter().map(Into::into).collect(),
        }
    }
}

/// Stable identity for one Preferences section.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PreferencesSectionId {
    /// Language choice and effective language.
    Language,
    /// Editor command and environment fallback.
    Editor,
    /// Terminal form presentation.
    InteractiveForm,
    /// Post-run behavior.
    AfterRun,
    /// JavaScript runtime preference.
    Javascript,
    /// Windows-only shell path.
    Bash,
    /// Prompt-runner list and its new-agent door.
    Agents,
    /// Agent Skill installation door.
    AgentSkill,
    /// Download-mirror axes.
    Mirrors,
}

/// Position of section copy relative to its interactive controls.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PreferencesTextPlacement {
    /// Show the text before the controls.
    BeforeControls,
    /// Show the text after the controls.
    #[default]
    AfterControls,
}

/// One complete Preferences section in display order.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PreferencesSection {
    /// Stable section identity.
    pub id: PreferencesSectionId,
    /// Section heading.
    pub title: PreferencesDisplayText,
    /// Supporting text below the heading. Empty means no supporting text.
    pub help: PreferencesDisplayText,
    /// Position of the supporting text.
    pub help_placement: PreferencesTextPlacement,
    /// Current effective values and other read-only facts.
    pub status: Vec<PreferencesDisplayText>,
    /// Position of the effective-value facts.
    pub status_placement: PreferencesTextPlacement,
    /// Typed controls in focus order.
    pub controls: Vec<PreferencesControl>,
}

/// Stable identity for one Preferences control or action door.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PreferencesControlId {
    /// Language picker.
    Language,
    /// Editor command input.
    Editor,
    /// Interactive form choice.
    InteractiveForm,
    /// Post-run choice.
    AfterRun,
    /// JavaScript runtime choice.
    Javascript,
    /// Windows bash path input.
    BashPath,
    /// Prompt-runner list with its own row cursor.
    Runners,
    /// Open the editor for one new prompt runner.
    NewRunner,
    /// Open Agent Skill installation.
    InstallAgentSkill,
    /// Mirror master switch.
    MirrorMaster,
    /// PyPI mirror choice.
    PypiChoice,
    /// Custom PyPI URL.
    PypiUrl,
    /// GitHub-release mirror choice.
    GithubChoice,
    /// Custom GitHub-release base URL.
    GithubUrl,
    /// npm mirror choice.
    NpmChoice,
    /// Custom npm URL.
    NpmUrl,
}

/// One stable closed-set option and its localizable label.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PreferencesOption {
    /// Value persisted through the application transaction.
    pub value: String,
    /// English catalog key shown to the user.
    pub label: String,
}

/// One Preferences text input.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PreferencesTextControl {
    /// Current unmasked value.
    pub value: String,
    /// Editing and completion policy.
    pub kind: FormInputKind,
    /// English catalog key shown when the value is empty.
    pub placeholder: String,
}

/// One Preferences choice control.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PreferencesChoiceControl {
    /// Stable values and localizable labels in presentation order.
    pub options: Vec<PreferencesOption>,
    /// Current stable value.
    pub selected: String,
    /// Visible radio group or compact picker.
    pub presentation: ChoicePresentation,
}

/// One Preferences prompt-runner list.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PreferencesRunnerListControl {
    /// Staged agent rows in configuration order.
    pub rows: Vec<RunnerDraftRow>,
    /// Row the keyboard acts on.
    pub cursor: usize,
}

/// Widget semantic for one Preferences row.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PreferencesControlKind {
    /// Cursor-aware input.
    Text(PreferencesTextControl),
    /// One closed set with separate values and translated labels.
    Choice(PreferencesChoiceControl),
    /// A discoverable action button.
    Button,
    /// The complete agent list with its own row cursor.
    RunnerList(PreferencesRunnerListControl),
}

/// One localized Preferences control description.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PreferencesControl {
    /// Stable semantic identity.
    pub id: PreferencesControlId,
    /// English catalog key.
    pub label: String,
    /// English catalog key for supporting text. Empty means no supporting line.
    pub help: String,
    /// Typed widget semantic.
    pub kind: PreferencesControlKind,
}

/// Reducer action for the Preferences workflow.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PreferencesAction {
    /// Replace the language choice.
    SetLanguage(String),
    /// Replace the editor command.
    SetEditor(String),
    /// Replace the interactive-form choice.
    SetInteractiveForm(InteractiveFormChoice),
    /// Replace the post-run choice.
    SetAfterRun(AfterRunChoice),
    /// Replace the JavaScript runtime.
    SetJavascript(JavascriptChoice),
    /// Replace the Windows bash path.
    SetBashPath(String),
    /// Enable or pause mirror URLs.
    SetMirrorMaster(bool),
    /// Replace one mirror choice.
    ChooseMirror {
        /// Mirror axis.
        field: PreferencesField,
        /// New choice.
        choice: MirrorChoice,
    },
    /// Replace one custom mirror URL.
    SetMirrorUrl {
        /// Mirror axis.
        field: PreferencesField,
        /// New URL text.
        value: String,
    },
    /// Focus one stable control.
    Focus(PreferencesControlId),
    /// Move to the previous reachable control.
    Previous,
    /// Move to the next reachable control.
    Next,
    /// Validate and request one atomic save.
    Save,
    /// Leave the screen, with a dirty guard.
    Close,
    /// Move the agent-list cursor to one exact row.
    RunnerCursor(usize),
    /// Move the agent-list cursor to the preceding row.
    RunnerCursorPrevious,
    /// Move the agent-list cursor to the next row.
    RunnerCursorNext,
    /// Open the shared runner editor on the agent-list cursor row.
    EditRunner,
    /// Open the shared runner editor for one new agent.
    NewRunner,
    /// Stage or cancel the removal of the agent-list cursor row.
    ToggleRunnerRemoval,
    /// Apply one validated runner editor result to the draft.
    RunnerStaged(RunnerSaveRequest),
    /// Open Agent Skill installation.
    InstallAgentSkill,
    /// Present the host-discovered Agent Skill targets, including an empty result.
    PresentAgentSkillTargets(Vec<AgentTarget>),
    /// Highlight one Agent Skill target.
    SelectAgentSkillTarget(usize),
    /// Install into one target activated with a pointer.
    ActivateAgentSkillTarget(usize),
    /// Install into the highlighted target.
    ConfirmAgentSkillTarget,
    /// Close the Agent Skill picker without writing.
    CloseAgentSkillTargets,
    /// Close the picker and publish the host's localized completion status.
    AgentSkillInstalled {
        /// Complete status text, including the written path.
        message: String,
    },
    /// Report a host-side validation failure, such as a missing bash file.
    ValidationFailed(PreferencesError),
}

/// Host work requested by the Preferences reducer.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PreferencesEffect {
    /// No host work.
    #[default]
    None,
    /// Persist one complete validated transaction.
    Save(PreferencesChangeSet),
    /// Close immediately.
    Close,
    /// Ask before discarding edits.
    ConfirmDiscard,
    /// Open the shared runner editor above Preferences.
    OpenRunnerEditor(Box<RunnerEditorView>),
    /// Report one runner editor result to the open editor modal.
    RunnerStaged {
        /// Refusal that keeps the editor open. `None` closes it.
        refused: Option<RunnerDraftError>,
    },
    /// Ask the host to detect existing agent directories without writing.
    DiscoverAgentSkillTargets,
    /// Install the embedded Agent Skill below one explicitly selected directory.
    InstallAgentSkill {
        /// Directory that contains named Agent Skills.
        skills_dir: PathBuf,
    },
}

/// Typed state for the Agent Skill target picker.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AgentSkillInstallView {
    targets: Vec<AgentTarget>,
    selected: Option<usize>,
}

impl AgentSkillInstallView {
    fn new(targets: Vec<AgentTarget>) -> Self {
        let selected = (!targets.is_empty()).then_some(0);
        Self { targets, selected }
    }

    /// Return every detected target in host-defined stable order.
    #[must_use]
    pub fn targets(&self) -> &[AgentTarget] {
        &self.targets
    }

    /// Return the highlighted target index.
    #[must_use]
    pub const fn selected(&self) -> Option<usize> {
        self.selected
    }

    /// Return the highlighted target.
    #[must_use]
    pub fn selected_target(&self) -> Option<&AgentTarget> {
        self.selected.and_then(|index| self.targets.get(index))
    }
}

/// Serializable state for the complete Preferences workflow.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PreferencesView {
    draft: PreferencesDraft,
    focused: PreferencesControlId,
    error: Option<PreferencesError>,
    agent_skill_install: Option<AgentSkillInstallView>,
    #[serde(default)]
    runner_cursor: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    editing_runner_row: Option<usize>,
}

impl PreferencesView {
    /// Start on the language picker with no inline error.
    #[must_use]
    pub const fn new(draft: PreferencesDraft) -> Self {
        Self {
            draft,
            focused: PreferencesControlId::Language,
            error: None,
            agent_skill_install: None,
            runner_cursor: 0,
            editing_runner_row: None,
        }
    }

    /// Return the typed draft.
    #[must_use]
    pub const fn draft(&self) -> &PreferencesDraft {
        &self.draft
    }

    /// Return the active control.
    #[must_use]
    pub const fn focused(&self) -> PreferencesControlId {
        self.focused
    }

    /// Return the active inline validation error.
    #[must_use]
    pub const fn error(&self) -> Option<&PreferencesError> {
        self.error.as_ref()
    }

    /// Return the Agent Skill target picker when it is open.
    #[must_use]
    pub const fn agent_skill_install(&self) -> Option<&AgentSkillInstallView> {
        self.agent_skill_install.as_ref()
    }

    /// Return the agent-list row the keyboard acts on.
    #[must_use]
    pub const fn runner_cursor(&self) -> usize {
        self.runner_cursor
    }

    /// Return the draft row the open runner editor rewrites, if it has one.
    #[must_use]
    pub const fn editing_runner_row(&self) -> Option<usize> {
        self.editing_runner_row
    }

    /// Forget the row the runner editor held after the editor closes without a save.
    pub const fn runner_editor_closed(&mut self) {
        self.editing_runner_row = None;
    }

    /// Return the verb of the focused control and the action its Enter key performs.
    ///
    /// A control that Enter does not activate returns `None`, so the footer never prints a key
    /// that does nothing.
    #[must_use]
    pub fn activation(&self) -> Option<(&'static str, PreferencesAction)> {
        match self.focused {
            PreferencesControlId::Runners => self
                .draft
                .runner_rows()
                .get(self.runner_cursor)
                .is_some_and(RunnerDraftRow::is_editable)
                .then_some(("Edit", PreferencesAction::EditRunner)),
            PreferencesControlId::NewRunner => Some(("New agent…", PreferencesAction::NewRunner)),
            PreferencesControlId::InstallAgentSkill => Some((
                "Teach an AI agent skit…",
                PreferencesAction::InstallAgentSkill,
            )),
            PreferencesControlId::Language
            | PreferencesControlId::Editor
            | PreferencesControlId::InteractiveForm
            | PreferencesControlId::AfterRun
            | PreferencesControlId::Javascript
            | PreferencesControlId::BashPath
            | PreferencesControlId::MirrorMaster
            | PreferencesControlId::PypiChoice
            | PreferencesControlId::PypiUrl
            | PreferencesControlId::GithubChoice
            | PreferencesControlId::GithubUrl
            | PreferencesControlId::NpmChoice
            | PreferencesControlId::NpmUrl => None,
        }
    }

    /// Report whether any editable value changed.
    #[must_use]
    pub fn dirty(&self) -> bool {
        self.draft.dirty()
    }

    /// Build reachable controls in navigation order.
    #[must_use]
    pub fn controls(&self) -> Vec<PreferencesControl> {
        let mut controls = vec![
            control(
                PreferencesControlId::Language,
                "",
                "",
                choice_control(
                    self.draft
                        .language_options
                        .iter()
                        .map(|value| {
                            option(
                                value,
                                if value == "auto" {
                                    "Automatic (follow the system)"
                                } else {
                                    value
                                },
                            )
                        })
                        .collect(),
                    self.draft.language.clone(),
                    ChoicePresentation::Picker,
                ),
            ),
            control(
                PreferencesControlId::Editor,
                "",
                "",
                text_control(
                    &self.draft.editor,
                    FormInputKind::Text,
                    "e.g. code --wait (empty = use $VISUAL / $EDITOR)",
                ),
            ),
            control(
                PreferencesControlId::InteractiveForm,
                "",
                "",
                choice_control(
                    vec![
                        option("tui", "Mini form — opens in place, fully clickable"),
                        option(
                            "plain",
                            "Line-by-line prompts — plainest, best over slow terminals",
                        ),
                    ],
                    match self.draft.form {
                        InteractiveFormChoice::Tui => "tui",
                        InteractiveFormChoice::Plain => "plain",
                    }
                    .to_owned(),
                    ChoicePresentation::Radio,
                ),
            ),
            control(
                PreferencesControlId::AfterRun,
                "",
                "",
                choice_control(
                    vec![
                        option("exit", "Quit skit — leave the run's output in the terminal"),
                        option("stay", "Return to the Library immediately"),
                    ],
                    match self.draft.after_run {
                        AfterRunChoice::Exit => "exit",
                        AfterRunChoice::Stay => "stay",
                    }
                    .to_owned(),
                    ChoicePresentation::Radio,
                ),
            ),
            control(
                PreferencesControlId::Javascript,
                "",
                "",
                choice_control(
                    vec![
                        option("auto", "Automatic — the first of deno / bun / node found"),
                        option("deno", "deno"),
                        option("bun", "bun"),
                        option("node", "node"),
                    ],
                    match self.draft.javascript {
                        JavascriptChoice::Automatic => "auto",
                        JavascriptChoice::Deno => "deno",
                        JavascriptChoice::Bun => "bun",
                        JavascriptChoice::Node => "node",
                    }
                    .to_owned(),
                    ChoicePresentation::Radio,
                ),
            ),
        ];
        if let Some(path) = &self.draft.bash_path {
            controls.push(control(
                PreferencesControlId::BashPath,
                "",
                "",
                text_control(
                    path,
                    FormInputKind::Path,
                    r"Path to bash.exe (empty = Git Bash / WSL detection)",
                ),
            ));
        }
        if !self.draft.runner_rows().is_empty() {
            controls.push(control(
                PreferencesControlId::Runners,
                "",
                "",
                PreferencesControlKind::RunnerList(PreferencesRunnerListControl {
                    rows: self.draft.runner_rows().to_vec(),
                    cursor: self.runner_cursor,
                }),
            ));
        }
        controls.extend([
            button(PreferencesControlId::NewRunner, "New agent…", ""),
            button(
                PreferencesControlId::InstallAgentSkill,
                "Teach an AI agent skit…",
                "",
            ),
            control(
                PreferencesControlId::MirrorMaster,
                "Master switch — \"off\" pauses mirrors but keeps the saved URLs.",
                "",
                choice_control(
                    vec![option("on", "on"), option("off", "off")],
                    if self.draft.mirror_master {
                        "on"
                    } else {
                        "off"
                    }
                    .to_owned(),
                    ChoicePresentation::Radio,
                ),
            ),
            control(
                PreferencesControlId::PypiChoice,
                "PyPI index (Python packages)",
                "",
                mirror_choice_control(&self.draft.pypi, pypi_preset_names()),
            ),
        ]);
        if self.draft.custom_pypi_visible() {
            controls.push(control(
                PreferencesControlId::PypiUrl,
                "",
                "",
                text_control(&self.draft.pypi_url, FormInputKind::Text, "PyPI index URL"),
            ));
        }
        controls.push(control(
            PreferencesControlId::GithubChoice,
            "GitHub releases (Python builds, the uv binary)",
            "",
            mirror_choice_control(&self.draft.github, github_preset_names()),
        ));
        if self.draft.custom_github_visible() {
            controls.push(control(
                PreferencesControlId::GithubUrl,
                "",
                "",
                text_control(
                    &self.draft.github_url,
                    FormInputKind::Text,
                    "github-release mirror base URL",
                ),
            ));
        }
        controls.push(control(
            PreferencesControlId::NpmChoice,
            "npm registry (JS/TS packages)",
            "",
            mirror_choice_control(&self.draft.npm, npm_preset_names()),
        ));
        if self.draft.custom_npm_visible() {
            controls.push(control(
                PreferencesControlId::NpmUrl,
                "",
                "",
                text_control(&self.draft.npm_url, FormInputKind::Text, "npm registry URL"),
            ));
        }
        controls
    }

    /// Build complete sections with effective-value facts and typed controls.
    #[must_use]
    pub fn sections(&self) -> Vec<PreferencesSection> {
        let controls = self.controls();
        let mut sections = vec![
            section(
                PreferencesSectionId::Language,
                "Interface language",
                "",
                vec![PreferencesDisplayText::with_argument(
                    "Currently in effect: {}",
                    &self.draft.effective_language,
                )],
                controls_for(&controls, &[PreferencesControlId::Language]),
            ),
            section(
                PreferencesSectionId::Editor,
                "Editor",
                "",
                self.draft
                    .editor_fallback
                    .as_ref()
                    .map_or_else(Vec::new, |fallback| {
                        vec![PreferencesDisplayText::with_argument(
                            "Empty means: {} (from $VISUAL / $EDITOR)",
                            fallback,
                        )]
                    }),
                controls_for(&controls, &[PreferencesControlId::Editor]),
            ),
            section(
                PreferencesSectionId::InteractiveForm,
                "Interactive form",
                "Used by terminal runs: `skit run` parameter prompts and the `skit add` review panel.",
                Vec::new(),
                controls_for(&controls, &[PreferencesControlId::InteractiveForm]),
            ),
            section(
                PreferencesSectionId::AfterRun,
                "After a run (from this menu)",
                "",
                Vec::new(),
                controls_for(&controls, &[PreferencesControlId::AfterRun]),
            ),
            section(
                PreferencesSectionId::Javascript,
                "JavaScript runtime",
                "Runs js/ts entries that don't pin their own runtime.",
                Vec::new(),
                controls_for(&controls, &[PreferencesControlId::Javascript]),
            ),
        ];
        if self.draft.bash_path.is_some() {
            sections.push(section(
                PreferencesSectionId::Bash,
                "Shell on Windows",
                "Shell scripts need an explicit bash here.",
                Vec::new(),
                controls_for(&controls, &[PreferencesControlId::BashPath]),
            ));
        }
        sections.extend([
            section(
                PreferencesSectionId::Agents,
                "Agents (prompt runners)",
                "The AI agents that run prompt entries.",
                if self.draft.runner_rows().is_empty() {
                    vec![PreferencesDisplayText::new("No agents configured.")]
                } else {
                    Vec::new()
                },
                controls_for(
                    &controls,
                    &[
                        PreferencesControlId::Runners,
                        PreferencesControlId::NewRunner,
                    ],
                ),
            ),
            section(
                PreferencesSectionId::AgentSkill,
                "Agent Skill",
                "Install the skit Agent Skill into an AI agent's skills directory (Claude Code, Codex, …).",
                Vec::new(),
                controls_for(&controls, &[PreferencesControlId::InstallAgentSkill]),
            ),
            section(
                PreferencesSectionId::Mirrors,
                "Download mirrors (mainland-China acceleration)",
                "Each ecosystem is its own choice — mirror vendors differ per axis.",
                Vec::new(),
                controls_for(
                    &controls,
                    &[
                        PreferencesControlId::MirrorMaster,
                        PreferencesControlId::PypiChoice,
                        PreferencesControlId::PypiUrl,
                        PreferencesControlId::GithubChoice,
                        PreferencesControlId::GithubUrl,
                        PreferencesControlId::NpmChoice,
                        PreferencesControlId::NpmUrl,
                    ],
                ),
            ),
        ]);
        sections
    }

    /// Return one reachable control by stable identity.
    #[must_use]
    pub fn control(&self, id: PreferencesControlId) -> Option<PreferencesControl> {
        self.controls().into_iter().find(|control| control.id == id)
    }

    /// Report whether a conditional control is reachable.
    #[must_use]
    pub fn has_control(&self, id: PreferencesControlId) -> bool {
        self.controls().iter().any(|control| control.id == id)
    }

    /// Apply one semantic action and return host work.
    pub fn update(&mut self, action: PreferencesAction) -> PreferencesEffect {
        match action {
            PreferencesAction::SetLanguage(value) => {
                self.focused = PreferencesControlId::Language;
                self.draft.language = value;
            }
            PreferencesAction::SetEditor(value) => {
                self.focused = PreferencesControlId::Editor;
                self.draft.editor = value;
            }
            PreferencesAction::SetInteractiveForm(value) => {
                self.focused = PreferencesControlId::InteractiveForm;
                self.draft.form = value;
            }
            PreferencesAction::SetAfterRun(value) => {
                self.focused = PreferencesControlId::AfterRun;
                self.draft.after_run = value;
            }
            PreferencesAction::SetJavascript(value) => {
                self.focused = PreferencesControlId::Javascript;
                self.draft.javascript = value;
            }
            PreferencesAction::SetBashPath(value) => {
                self.focused = PreferencesControlId::BashPath;
                self.draft.bash_path = Some(value);
                self.clear_error_for(PreferencesField::BashPath);
            }
            PreferencesAction::SetMirrorMaster(value) => {
                self.focused = PreferencesControlId::MirrorMaster;
                self.draft.mirror_master = value;
            }
            PreferencesAction::ChooseMirror { field, choice } => {
                match field {
                    PreferencesField::PypiMirror => {
                        self.focused = PreferencesControlId::PypiChoice;
                        self.draft.pypi = choice;
                    }
                    PreferencesField::GithubMirror => {
                        self.focused = PreferencesControlId::GithubChoice;
                        self.draft.github = choice;
                    }
                    PreferencesField::NpmMirror => {
                        self.focused = PreferencesControlId::NpmChoice;
                        self.draft.npm = choice;
                    }
                    PreferencesField::BashPath | PreferencesField::Runners => {
                        return PreferencesEffect::None;
                    }
                }
                self.clear_error_for(field);
            }
            PreferencesAction::SetMirrorUrl { field, value } => {
                let (focused, url) = match field {
                    PreferencesField::PypiMirror => {
                        (PreferencesControlId::PypiUrl, &mut self.draft.pypi_url)
                    }
                    PreferencesField::GithubMirror => {
                        (PreferencesControlId::GithubUrl, &mut self.draft.github_url)
                    }
                    PreferencesField::NpmMirror => {
                        (PreferencesControlId::NpmUrl, &mut self.draft.npm_url)
                    }
                    PreferencesField::BashPath | PreferencesField::Runners => {
                        return PreferencesEffect::None;
                    }
                };
                self.focused = focused;
                *url = value;
                self.clear_error_for(field);
            }
            PreferencesAction::Focus(id) => {
                if self.has_control(id) {
                    self.focused = id;
                }
            }
            PreferencesAction::Previous => self.move_focus(-1),
            PreferencesAction::Next => self.move_focus(1),
            PreferencesAction::Save => {
                self.error = None;
                match self.draft.resolve(|_| true) {
                    Ok(change) => return PreferencesEffect::Save(change),
                    Err(error) => self.set_error(error),
                }
            }
            PreferencesAction::Close => {
                self.editing_runner_row = None;
                return if self.dirty() {
                    PreferencesEffect::ConfirmDiscard
                } else {
                    PreferencesEffect::Close
                };
            }
            PreferencesAction::RunnerCursor(index) => self.move_runner_cursor(Some(index)),
            PreferencesAction::RunnerCursorPrevious => {
                self.move_runner_cursor(self.runner_cursor.checked_sub(1));
            }
            PreferencesAction::RunnerCursorNext => {
                self.move_runner_cursor(Some(self.runner_cursor.saturating_add(1)));
            }
            PreferencesAction::EditRunner => return self.open_runner_editor(),
            PreferencesAction::NewRunner => {
                self.editing_runner_row = None;
                return PreferencesEffect::OpenRunnerEditor(Box::default());
            }
            PreferencesAction::ToggleRunnerRemoval => {
                if self.has_control(PreferencesControlId::Runners)
                    && self.cursor_speaks_for_itself()
                {
                    self.focused = PreferencesControlId::Runners;
                    self.clear_error_for(PreferencesField::Runners);
                    if self
                        .draft
                        .toggle_runner_removal(self.runner_cursor)
                        .is_err()
                    {
                        self.set_error(PreferencesError::RunnerNameTaken);
                    }
                    self.clamp_runner_cursor();
                }
            }
            PreferencesAction::RunnerStaged(request) => {
                self.clear_error_for(PreferencesField::Runners);
                let refused = self
                    .draft
                    .stage_runner(request, self.editing_runner_row)
                    .err();
                // The editor keeps the refusal too, so the reason survives its own dismissal.
                match refused {
                    None => {
                        self.editing_runner_row = None;
                        self.clamp_runner_cursor();
                    }
                    Some(RunnerDraftError::DuplicateName) => {
                        self.set_error(PreferencesError::RunnerNameTaken);
                    }
                }
                return PreferencesEffect::RunnerStaged { refused };
            }
            PreferencesAction::InstallAgentSkill => {
                return PreferencesEffect::DiscoverAgentSkillTargets;
            }
            PreferencesAction::PresentAgentSkillTargets(targets) => {
                self.agent_skill_install = Some(AgentSkillInstallView::new(targets));
            }
            PreferencesAction::SelectAgentSkillTarget(index) => {
                if let Some(picker) = &mut self.agent_skill_install
                    && index < picker.targets.len()
                {
                    picker.selected = Some(index);
                }
            }
            PreferencesAction::ActivateAgentSkillTarget(index) => {
                if let Some(target) = self
                    .agent_skill_install
                    .as_ref()
                    .and_then(|picker| picker.targets.get(index))
                {
                    return PreferencesEffect::InstallAgentSkill {
                        skills_dir: target.skills_dir(),
                    };
                }
            }
            PreferencesAction::ConfirmAgentSkillTarget => {
                if let Some(target) = self
                    .agent_skill_install
                    .as_ref()
                    .and_then(AgentSkillInstallView::selected_target)
                {
                    return PreferencesEffect::InstallAgentSkill {
                        skills_dir: target.skills_dir(),
                    };
                }
            }
            PreferencesAction::CloseAgentSkillTargets
            | PreferencesAction::AgentSkillInstalled { .. } => {
                self.agent_skill_install = None;
            }
            PreferencesAction::ValidationFailed(error) => self.set_error(error),
        }
        PreferencesEffect::None
    }

    fn move_runner_cursor(&mut self, index: Option<usize>) {
        if !self.has_control(PreferencesControlId::Runners) {
            return;
        }
        self.focused = PreferencesControlId::Runners;
        if let Some(index) = index {
            self.runner_cursor = index;
        }
        self.clamp_runner_cursor();
    }

    fn clamp_runner_cursor(&mut self) {
        let rows = self.draft.runner_rows().len();
        self.runner_cursor = self.runner_cursor.min(rows.saturating_sub(1));
        if rows == 0 && self.focused == PreferencesControlId::Runners {
            self.focused = PreferencesControlId::NewRunner;
        }
    }

    fn open_runner_editor(&mut self) -> PreferencesEffect {
        if !self.has_control(PreferencesControlId::Runners) {
            return PreferencesEffect::None;
        }
        self.focused = PreferencesControlId::Runners;
        let Some((editing, view)) = self.cursor_editor() else {
            return PreferencesEffect::None;
        };
        self.editing_runner_row = editing;
        PreferencesEffect::OpenRunnerEditor(Box::new(view))
    }

    /// Build the editor of the cursor row and the draft row its save rewrites.
    ///
    /// An appended row has no stored identity, so its save goes back to the same draft row.
    fn cursor_editor(&self) -> Option<(Option<usize>, RunnerEditorView)> {
        if !self.cursor_speaks_for_itself() {
            return None;
        }
        let row = self
            .draft
            .runner_rows()
            .get(self.runner_cursor)
            .filter(|row| row.is_editable())?;
        match row {
            RunnerDraftRow::Added { name, argv } => Some((
                Some(self.runner_cursor),
                RunnerEditorView::staged(name, argv),
            )),
            // A raw row has no stable key that prompts pin, so its editor repairs the row in
            // place even after the user typed a name into it.
            RunnerDraftRow::Existing { row: stored, .. } => {
                let resolved = row.resolved_row()?;
                Some((
                    None,
                    if stored.name.is_some() {
                        RunnerEditorView::edit(&resolved)
                    } else {
                        RunnerEditorView::repair(&resolved)
                    },
                ))
            }
        }
    }

    /// Report whether the cursor row carries its own commands.
    ///
    /// A duplicate row of a key that the draft already changes goes with that key. It offers no
    /// command, so a key or a click on it does nothing.
    fn cursor_speaks_for_itself(&self) -> bool {
        !runner_row_taken_by_its_key(self.draft.runner_rows(), self.runner_cursor)
    }

    fn clear_error_for(&mut self, field: PreferencesField) {
        if self
            .error
            .as_ref()
            .is_some_and(|error| error.field() == field)
        {
            self.error = None;
        }
    }

    fn move_focus(&mut self, delta: isize) {
        let controls = self.controls();
        let current = controls
            .iter()
            .position(|control| control.id == self.focused)
            .unwrap_or_default();
        let next = current
            .saturating_add_signed(delta)
            .min(controls.len().saturating_sub(1));
        if let Some(control) = controls.get(next) {
            self.focused = control.id;
        }
    }

    fn set_error(&mut self, error: PreferencesError) {
        self.focused = self.control_for(error.field());
        self.error = Some(error);
    }

    /// Return the control the current refusal belongs to.
    ///
    /// A refusal outlives the keystroke that raised it, so the frontend prints it under the
    /// control it names, not under the control the user moved on to.
    #[must_use]
    pub fn error_control(&self) -> Option<PreferencesControlId> {
        self.error
            .as_ref()
            .map(|error| self.control_for(error.field()))
    }

    /// Return the control one refused field belongs to.
    fn control_for(&self, field: PreferencesField) -> PreferencesControlId {
        match field {
            PreferencesField::BashPath => PreferencesControlId::BashPath,
            PreferencesField::PypiMirror if self.draft.custom_pypi_visible() => {
                PreferencesControlId::PypiUrl
            }
            PreferencesField::PypiMirror => PreferencesControlId::PypiChoice,
            PreferencesField::GithubMirror if self.draft.custom_github_visible() => {
                PreferencesControlId::GithubUrl
            }
            PreferencesField::GithubMirror => PreferencesControlId::GithubChoice,
            PreferencesField::NpmMirror if self.draft.custom_npm_visible() => {
                PreferencesControlId::NpmUrl
            }
            PreferencesField::NpmMirror => PreferencesControlId::NpmChoice,
            PreferencesField::Runners if self.has_control(PreferencesControlId::Runners) => {
                PreferencesControlId::Runners
            }
            PreferencesField::Runners => PreferencesControlId::NewRunner,
        }
    }
}

fn control(
    id: PreferencesControlId,
    label: &str,
    help: &str,
    kind: PreferencesControlKind,
) -> PreferencesControl {
    PreferencesControl {
        id,
        label: label.to_owned(),
        help: help.to_owned(),
        kind,
    }
}

fn button(id: PreferencesControlId, label: &str, help: &str) -> PreferencesControl {
    PreferencesControl {
        id,
        label: label.to_owned(),
        help: help.to_owned(),
        kind: PreferencesControlKind::Button,
    }
}

fn text_control(value: &str, kind: FormInputKind, placeholder: &str) -> PreferencesControlKind {
    PreferencesControlKind::Text(PreferencesTextControl {
        value: value.to_owned(),
        kind,
        placeholder: placeholder.to_owned(),
    })
}

fn choice_control(
    options: Vec<PreferencesOption>,
    selected: String,
    presentation: ChoicePresentation,
) -> PreferencesControlKind {
    PreferencesControlKind::Choice(PreferencesChoiceControl {
        options,
        selected,
        presentation,
    })
}

fn option(value: impl Into<String>, label: impl Into<String>) -> PreferencesOption {
    PreferencesOption {
        value: value.into(),
        label: label.into(),
    }
}

fn mirror_choice_control(
    choice_value: &MirrorChoice,
    mut presets: Vec<String>,
) -> PreferencesControlKind {
    presets.extend(["custom".to_owned(), "off".to_owned()]);
    let selected = match choice_value {
        MirrorChoice::Preset(name) => name.clone(),
        MirrorChoice::Custom => "custom".to_owned(),
        MirrorChoice::Off => "off".to_owned(),
    };
    choice_control(
        presets
            .into_iter()
            .map(|value| option(value.clone(), value))
            .collect(),
        selected,
        ChoicePresentation::Radio,
    )
}

fn section(
    id: PreferencesSectionId,
    title: &str,
    help: &str,
    status: Vec<PreferencesDisplayText>,
    controls: Vec<PreferencesControl>,
) -> PreferencesSection {
    let help_placement = if id == PreferencesSectionId::Mirrors {
        PreferencesTextPlacement::BeforeControls
    } else {
        PreferencesTextPlacement::AfterControls
    };
    let status_placement = if id == PreferencesSectionId::Agents {
        PreferencesTextPlacement::BeforeControls
    } else {
        PreferencesTextPlacement::AfterControls
    };
    PreferencesSection {
        id,
        title: PreferencesDisplayText::new(title),
        help: PreferencesDisplayText::new(help),
        help_placement,
        status,
        status_placement,
        controls,
    }
}

fn controls_for(
    controls: &[PreferencesControl],
    ids: &[PreferencesControlId],
) -> Vec<PreferencesControl> {
    ids.iter()
        .filter_map(|id| controls.iter().find(|control| control.id == *id).cloned())
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use skit_application::preferences::{
        AfterRunChoice, InteractiveFormChoice, JavascriptChoice, MirrorChoice, MirrorConfiguration,
        PreferencesDraft, PreferencesError, PreferencesField, PreferencesSnapshot,
        RunnerDraftError, RunnerDraftMarker,
    };
    use skit_application::runner_management::{
        EditableArgvDialect, RunnerRow, RunnerRowIdentity, RunnerSaveRequest, RunnerSaveTarget,
        join_editable_argv,
    };
    use skit_application::{AgentScope, AgentTarget};

    use super::{
        PreferencesAction, PreferencesControlId, PreferencesControlKind, PreferencesEffect,
        PreferencesSectionId, PreferencesView,
    };
    use crate::management::{RunnerEditorError, RunnerEditorMode, RunnerEditorView};

    fn runner_row(index: usize, name: &str) -> RunnerRow {
        let identity = RunnerRowIdentity {
            index: Some(index),
            snapshot_token: format!("token-{index}"),
        };
        RunnerRow {
            key_identities: vec![identity.clone()],
            identity,
            name: Some(name.to_owned()),
            argv: Some(vec![name.to_owned(), "{{prompt}}".to_owned()]),
            reason: None,
            descriptor: format!("prompt.runners[{index}]"),
            pinned_count: 0,
        }
    }

    /// Return the editable command one argv gets on this host, with both dialects pinned.
    ///
    /// The editor renders its command through [`EditableArgvDialect::host()`], so the quoting
    /// follows the platform. The two assertions keep the POSIX text and the Windows text of the
    /// same argv under test on every host, and the return value is the text this host paints.
    fn host_editable_command(argv: &[&str], posix: &str, windows: &str) -> String {
        let argv: Vec<String> = argv.iter().map(|word| (*word).to_owned()).collect();
        assert_eq!(join_editable_argv(&argv, EditableArgvDialect::Posix), posix);
        assert_eq!(
            join_editable_argv(&argv, EditableArgvDialect::Windows),
            windows
        );
        join_editable_argv(&argv, EditableArgvDialect::host())
    }

    /// Return one row without the shape an editor needs.
    fn malformed_row(index: usize) -> RunnerRow {
        RunnerRow {
            identity: RunnerRowIdentity {
                index: Some(index),
                snapshot_token: format!("token-{index}"),
            },
            name: None,
            argv: None,
            reason: Some("row-not-table".to_owned()),
            descriptor: format!("prompt.runners[{index}]"),
            key_identities: Vec::new(),
            pinned_count: 0,
        }
    }

    fn snapshot(windows: bool, runners: Vec<RunnerRow>) -> PreferencesSnapshot {
        PreferencesSnapshot {
            language: String::new(),
            available_languages: vec!["en".to_owned(), "zh-CN".to_owned(), "zh-TW".to_owned()],
            effective_language: "en".to_owned(),
            editor: String::new(),
            editor_fallback: Some("vim".to_owned()),
            form: InteractiveFormChoice::Tui,
            after_run: AfterRunChoice::Exit,
            javascript: JavascriptChoice::Automatic,
            bash_path: windows.then(String::new),
            runners,
            mirror: MirrorConfiguration::default(),
        }
    }

    fn view(windows: bool) -> PreferencesView {
        PreferencesView::new(PreferencesDraft::from_snapshot(snapshot(
            windows,
            vec![runner_row(0, "claude"), runner_row(1, "codex")],
        )))
    }

    /// Return the editor an effect opens, or `None` for every other outcome.
    fn opened_editor(effect: &PreferencesEffect) -> Option<&RunnerEditorView> {
        match effect {
            PreferencesEffect::OpenRunnerEditor(view) => Some(view),
            _ => None,
        }
    }

    fn empty_view() -> PreferencesView {
        PreferencesView::new(PreferencesDraft::from_snapshot(snapshot(false, Vec::new())))
    }

    #[test]
    fn preferences_have_typed_controls_even_when_the_config_omits_every_key() {
        let view = view(false);
        let ids = view
            .controls()
            .into_iter()
            .map(|control| control.id)
            .collect::<Vec<_>>();

        assert_eq!(
            ids,
            [
                PreferencesControlId::Language,
                PreferencesControlId::Editor,
                PreferencesControlId::InteractiveForm,
                PreferencesControlId::AfterRun,
                PreferencesControlId::Javascript,
                PreferencesControlId::Runners,
                PreferencesControlId::NewRunner,
                PreferencesControlId::InstallAgentSkill,
                PreferencesControlId::MirrorMaster,
                PreferencesControlId::PypiChoice,
                PreferencesControlId::GithubChoice,
                PreferencesControlId::NpmChoice,
            ]
        );
        assert_eq!(view.focused(), PreferencesControlId::Language);
        assert!(!view.dirty());
        assert_eq!(view.draft().runner_rows().len(), 2);
        assert_eq!(view.runner_cursor(), 0);
        assert!(matches!(
            view.control(PreferencesControlId::Runners).map(|control| control.kind),
            Some(PreferencesControlKind::RunnerList(list))
                if list.cursor == 0 && list.rows.len() == 2
        ));
        assert!(!empty_view().has_control(PreferencesControlId::Runners));
    }

    #[test]
    fn windows_and_custom_axes_reveal_only_their_real_input_controls() {
        let mut view = view(true);
        assert!(view.has_control(PreferencesControlId::BashPath));
        assert!(!view.has_control(PreferencesControlId::PypiUrl));
        assert!(!view.has_control(PreferencesControlId::GithubUrl));
        assert!(!view.has_control(PreferencesControlId::NpmUrl));

        view.update(PreferencesAction::ChooseMirror {
            field: PreferencesField::PypiMirror,
            choice: MirrorChoice::Custom,
        });

        assert!(view.has_control(PreferencesControlId::PypiUrl));
        assert!(!view.has_control(PreferencesControlId::GithubUrl));
        assert!(!view.has_control(PreferencesControlId::NpmUrl));
        assert!(view.dirty());
    }

    #[test]
    fn validation_is_frontend_neutral_and_focuses_the_refused_control() {
        let mut view = view(false);
        view.update(PreferencesAction::SetEditor("micro".to_owned()));
        view.update(PreferencesAction::ChooseMirror {
            field: PreferencesField::GithubMirror,
            choice: MirrorChoice::Custom,
        });
        view.update(PreferencesAction::SetMirrorUrl {
            field: PreferencesField::GithubMirror,
            value: "http://mirror.example/gh".to_owned(),
        });

        let effect = view.update(PreferencesAction::Save);

        assert_eq!(effect, PreferencesEffect::None);
        assert_eq!(view.focused(), PreferencesControlId::GithubUrl);
        assert_eq!(
            view.error().map(|error| error.field()),
            Some(PreferencesField::GithubMirror)
        );
        assert!(view.dirty());
    }

    #[test]
    fn save_skill_and_dirty_close_are_distinct_typed_effects() {
        let mut view = view(false);
        assert_eq!(
            view.update(PreferencesAction::Close),
            PreferencesEffect::Close
        );
        assert_eq!(
            view.update(PreferencesAction::InstallAgentSkill),
            PreferencesEffect::DiscoverAgentSkillTargets
        );

        view.update(PreferencesAction::SetEditor("micro".to_owned()));
        assert_eq!(
            view.update(PreferencesAction::Close),
            PreferencesEffect::ConfirmDiscard
        );
        assert!(matches!(
            view.update(PreferencesAction::Save),
            PreferencesEffect::Save(_)
        ));
    }

    #[test]
    fn sections_preserve_the_complete_latest_main_surface_and_stable_option_values() {
        let view = view(false);
        let sections = view.sections();

        assert_eq!(
            sections
                .iter()
                .map(|section| section.id)
                .collect::<Vec<_>>(),
            [
                PreferencesSectionId::Language,
                PreferencesSectionId::Editor,
                PreferencesSectionId::InteractiveForm,
                PreferencesSectionId::AfterRun,
                PreferencesSectionId::Javascript,
                PreferencesSectionId::Agents,
                PreferencesSectionId::AgentSkill,
                PreferencesSectionId::Mirrors,
            ]
        );
        assert_eq!(sections[0].title.key, "Interface language");
        assert_eq!(
            sections[0].status,
            [super::PreferencesDisplayText::with_argument(
                "Currently in effect: {}",
                "en",
            )]
        );
        assert_eq!(
            sections[1].status,
            [super::PreferencesDisplayText::with_argument(
                "Empty means: {} (from $VISUAL / $EDITOR)",
                "vim",
            )]
        );
        assert_eq!(
            sections[7].title.key,
            "Download mirrors (mainland-China acceleration)"
        );
        assert_eq!(
            sections[7].help.key,
            "Each ecosystem is its own choice — mirror vendors differ per axis."
        );
        assert_eq!(
            sections[7].help_placement,
            super::PreferencesTextPlacement::BeforeControls
        );
        assert_eq!(
            sections[5].help.key,
            "The AI agents that run prompt entries."
        );
        assert!(sections[5].status.is_empty());
        assert_eq!(sections[6].title.key, "Agent Skill");
        assert_eq!(
            sections[6].help.key,
            "Install the skit Agent Skill into an AI agent's skills directory (Claude Code, Codex, …)."
        );
        assert_eq!(
            empty_view().sections()[5].status,
            [super::PreferencesDisplayText::new("No agents configured.")]
        );
        assert_eq!(
            sections[5].status_placement,
            super::PreferencesTextPlacement::BeforeControls
        );
        assert_eq!(
            sections[0].status_placement,
            super::PreferencesTextPlacement::AfterControls
        );

        let form = view
            .control(PreferencesControlId::InteractiveForm)
            .expect("interactive-form control");
        assert!(matches!(
            &form.kind,
            PreferencesControlKind::Choice(choice)
                if choice.options.iter().map(|option| (option.value.as_str(), option.label.as_str())).collect::<Vec<_>>()
                    == [
                        ("tui", "Mini form — opens in place, fully clickable"),
                        ("plain", "Line-by-line prompts — plainest, best over slow terminals"),
                    ]
        ));
        let after = view
            .control(PreferencesControlId::AfterRun)
            .expect("after-run control");
        assert!(matches!(
            &after.kind,
            PreferencesControlKind::Choice(choice)
                if choice.options.iter().map(|option| option.label.as_str()).collect::<Vec<_>>()
                    == [
                        "Quit skit — leave the run's output in the terminal",
                        "Return to the Library immediately",
                    ]
        ));
    }

    #[test]
    fn every_preference_action_keeps_typed_focus_validation_and_serialization() {
        let mut view = view(true);
        assert_eq!(view.sections()[5].id, PreferencesSectionId::Bash);

        for (action, focused) in [
            (
                PreferencesAction::SetLanguage("zh-TW".to_owned()),
                PreferencesControlId::Language,
            ),
            (
                PreferencesAction::SetInteractiveForm(InteractiveFormChoice::Plain),
                PreferencesControlId::InteractiveForm,
            ),
            (
                PreferencesAction::SetAfterRun(AfterRunChoice::Stay),
                PreferencesControlId::AfterRun,
            ),
            (
                PreferencesAction::SetJavascript(JavascriptChoice::Deno),
                PreferencesControlId::Javascript,
            ),
            (
                PreferencesAction::SetBashPath("C:/Git/bin/bash.exe".to_owned()),
                PreferencesControlId::BashPath,
            ),
            (
                PreferencesAction::SetMirrorMaster(false),
                PreferencesControlId::MirrorMaster,
            ),
            (
                PreferencesAction::RunnerCursorNext,
                PreferencesControlId::Runners,
            ),
            (
                PreferencesAction::RunnerCursorPrevious,
                PreferencesControlId::Runners,
            ),
            (
                PreferencesAction::RunnerCursor(1),
                PreferencesControlId::Runners,
            ),
            (
                PreferencesAction::ToggleRunnerRemoval,
                PreferencesControlId::Runners,
            ),
        ] {
            assert_eq!(view.update(action), PreferencesEffect::None);
            assert_eq!(view.focused(), focused);
            let encoded = serde_json::to_vec(&view).unwrap();
            assert_eq!(
                serde_json::from_slice::<PreferencesView>(&encoded).unwrap(),
                view
            );
        }
        assert_eq!(view.draft().language, "zh-TW");
        assert_eq!(view.draft().form, InteractiveFormChoice::Plain);
        assert_eq!(view.draft().after_run, AfterRunChoice::Stay);
        assert_eq!(view.draft().javascript, JavascriptChoice::Deno);
        assert!(!view.draft().mirror_master);
        assert_eq!(view.runner_cursor(), 1);
        assert_eq!(
            view.draft().runner_rows()[1].marker(),
            Some(RunnerDraftMarker::Removed)
        );
        view.update(PreferencesAction::ToggleRunnerRemoval);
        let controls = view.controls();
        assert!(matches!(
            controls.iter().find(|control| control.id == PreferencesControlId::InteractiveForm).map(|control| &control.kind),
            Some(PreferencesControlKind::Choice(choice)) if choice.selected == "plain"
        ));
        assert!(matches!(
            controls.iter().find(|control| control.id == PreferencesControlId::AfterRun).map(|control| &control.kind),
            Some(PreferencesControlKind::Choice(choice)) if choice.selected == "stay"
        ));
        assert!(matches!(
            controls.iter().find(|control| control.id == PreferencesControlId::Javascript).map(|control| &control.kind),
            Some(PreferencesControlKind::Choice(choice)) if choice.selected == "deno"
        ));
        assert!(matches!(
            controls.iter().find(|control| control.id == PreferencesControlId::MirrorMaster).map(|control| &control.kind),
            Some(PreferencesControlKind::Choice(choice)) if choice.selected == "off"
        ));

        for (runtime, expected) in [
            (JavascriptChoice::Bun, "bun"),
            (JavascriptChoice::Node, "node"),
        ] {
            view.update(PreferencesAction::SetJavascript(runtime));
            assert!(matches!(
                view.control(PreferencesControlId::Javascript).map(|control| control.kind),
                Some(PreferencesControlKind::Choice(choice)) if choice.selected == expected
            ));
        }

        view.update(PreferencesAction::ChooseMirror {
            field: PreferencesField::NpmMirror,
            choice: MirrorChoice::Custom,
        });
        assert_eq!(view.focused(), PreferencesControlId::NpmChoice);
        assert!(view.has_control(PreferencesControlId::NpmUrl));
        view.update(PreferencesAction::SetMirrorUrl {
            field: PreferencesField::NpmMirror,
            value: "https://npm.example".to_owned(),
        });
        assert_eq!(view.focused(), PreferencesControlId::NpmUrl);
        assert_eq!(view.draft().npm_url, "https://npm.example");
        view.update(PreferencesAction::ChooseMirror {
            field: PreferencesField::NpmMirror,
            choice: MirrorChoice::Off,
        });
        assert_eq!(view.focused(), PreferencesControlId::NpmChoice);
        assert!(!view.has_control(PreferencesControlId::NpmUrl));

        view.update(PreferencesAction::ChooseMirror {
            field: PreferencesField::PypiMirror,
            choice: MirrorChoice::Preset("tsinghua".to_owned()),
        });
        assert!(matches!(
            view.control(PreferencesControlId::PypiChoice).map(|control| control.kind),
            Some(PreferencesControlKind::Choice(choice)) if choice.selected == "tsinghua"
        ));
        view.update(PreferencesAction::ChooseMirror {
            field: PreferencesField::PypiMirror,
            choice: MirrorChoice::Custom,
        });
        view.update(PreferencesAction::SetMirrorUrl {
            field: PreferencesField::PypiMirror,
            value: "https://pypi.example/simple".to_owned(),
        });
        assert_eq!(view.focused(), PreferencesControlId::PypiUrl);
        assert_eq!(view.draft().pypi_url, "https://pypi.example/simple");
        view.update(PreferencesAction::SetMirrorUrl {
            field: PreferencesField::GithubMirror,
            value: "https://github.example".to_owned(),
        });
        assert_eq!(view.draft().github_url, "https://github.example");

        let before = serde_json::to_value(&view).unwrap();
        assert_eq!(
            view.update(PreferencesAction::ChooseMirror {
                field: PreferencesField::BashPath,
                choice: MirrorChoice::Off,
            }),
            PreferencesEffect::None
        );
        assert_eq!(serde_json::to_value(&view).unwrap(), before);
        assert_eq!(
            view.update(PreferencesAction::SetMirrorUrl {
                field: PreferencesField::BashPath,
                value: "ignored".to_owned(),
            }),
            PreferencesEffect::None
        );
        assert_eq!(serde_json::to_value(&view).unwrap(), before);

        for (field, choice, expected_focus) in [
            (
                PreferencesField::PypiMirror,
                MirrorChoice::Custom,
                PreferencesControlId::PypiUrl,
            ),
            (
                PreferencesField::GithubMirror,
                MirrorChoice::Off,
                PreferencesControlId::GithubChoice,
            ),
            (
                PreferencesField::NpmMirror,
                MirrorChoice::Custom,
                PreferencesControlId::NpmUrl,
            ),
        ] {
            view.update(PreferencesAction::ChooseMirror { field, choice });
            assert_eq!(
                view.update(PreferencesAction::ValidationFailed(
                    PreferencesError::CustomUrlRequired { field },
                )),
                PreferencesEffect::None
            );
            assert_eq!(view.focused(), expected_focus);
            assert_eq!(view.error().map(PreferencesError::field), Some(field));
        }
        for (field, expected_focus) in [
            (
                PreferencesField::PypiMirror,
                PreferencesControlId::PypiChoice,
            ),
            (PreferencesField::NpmMirror, PreferencesControlId::NpmChoice),
        ] {
            view.update(PreferencesAction::ChooseMirror {
                field,
                choice: MirrorChoice::Off,
            });
            view.update(PreferencesAction::ValidationFailed(
                PreferencesError::CustomUrlRequired { field },
            ));
            assert_eq!(view.focused(), expected_focus);
        }
        assert_eq!(
            view.update(PreferencesAction::ValidationFailed(
                PreferencesError::BashPathMissing {
                    path: "C:/missing/bash.exe".to_owned(),
                },
            )),
            PreferencesEffect::None
        );
        assert_eq!(view.focused(), PreferencesControlId::BashPath);
        view.update(PreferencesAction::SetBashPath(
            "C:/Git/bin/bash.exe".to_owned(),
        ));
        assert!(view.error().is_none());

        view.update(PreferencesAction::Next);
        assert_ne!(view.focused(), PreferencesControlId::BashPath);
    }

    #[test]
    fn the_agent_cursor_moves_inside_the_list_and_clamps_at_both_ends() {
        let mut view = view(false);

        assert_eq!(
            view.update(PreferencesAction::RunnerCursorPrevious),
            PreferencesEffect::None
        );
        assert_eq!(view.runner_cursor(), 0);
        view.update(PreferencesAction::RunnerCursorNext);
        assert_eq!(view.runner_cursor(), 1);
        view.update(PreferencesAction::RunnerCursorNext);
        assert_eq!(view.runner_cursor(), 1);
        view.update(PreferencesAction::RunnerCursor(9));
        assert_eq!(view.runner_cursor(), 1);
        assert_eq!(view.focused(), PreferencesControlId::Runners);

        let mut empty = empty_view();
        assert_eq!(
            empty.update(PreferencesAction::RunnerCursor(0)),
            PreferencesEffect::None
        );
        assert_eq!(empty.focused(), PreferencesControlId::Language);
        assert_eq!(
            empty.update(PreferencesAction::Focus(PreferencesControlId::Runners)),
            PreferencesEffect::None
        );
        assert_eq!(empty.focused(), PreferencesControlId::Language);
        assert_eq!(
            empty.update(PreferencesAction::ToggleRunnerRemoval),
            PreferencesEffect::None
        );
        let opened = empty.update(PreferencesAction::EditRunner);
        assert_eq!(opened, PreferencesEffect::None);
        assert_eq!(opened_editor(&opened), None);
        assert!(!empty.dirty());
    }

    #[test]
    fn dropping_the_only_agent_row_hides_the_list_and_rehomes_the_focus() {
        let mut view = empty_view();
        assert_eq!(
            view.update(PreferencesAction::NewRunner),
            PreferencesEffect::OpenRunnerEditor(Box::default())
        );
        view.update(PreferencesAction::RunnerStaged(RunnerSaveRequest {
            name: "only".to_owned(),
            argv: vec!["only".to_owned(), "{{prompt}}".to_owned()],
            target: RunnerSaveTarget::New,
        }));
        view.update(PreferencesAction::RunnerCursor(0));
        assert_eq!(view.focused(), PreferencesControlId::Runners);

        view.update(PreferencesAction::ToggleRunnerRemoval);

        assert!(view.draft().runner_rows().is_empty());
        assert!(!view.has_control(PreferencesControlId::Runners));
        assert_eq!(view.focused(), PreferencesControlId::NewRunner);
        assert_eq!(view.runner_cursor(), 0);
        assert!(!view.dirty());
    }

    /// The footer verb, the row chip and the editor door read one predicate.
    #[test]
    fn the_focused_control_publishes_the_verb_that_enter_performs() {
        let mut view = PreferencesView::new(PreferencesDraft::from_snapshot(snapshot(
            false,
            vec![runner_row(0, "claude"), malformed_row(1)],
        )));

        assert_eq!(view.activation(), None);
        view.update(PreferencesAction::Focus(PreferencesControlId::NewRunner));
        assert_eq!(
            view.activation(),
            Some(("New agent…", PreferencesAction::NewRunner))
        );
        view.update(PreferencesAction::Focus(
            PreferencesControlId::InstallAgentSkill,
        ));
        assert_eq!(
            view.activation(),
            Some((
                "Teach an AI agent skit…",
                PreferencesAction::InstallAgentSkill
            ))
        );

        view.update(PreferencesAction::RunnerCursor(0));
        assert_eq!(
            view.activation(),
            Some(("Edit", PreferencesAction::EditRunner))
        );

        // A row Enter cannot open advertises no verb: a staged removal and a shapeless row.
        view.update(PreferencesAction::ToggleRunnerRemoval);
        assert_eq!(view.activation(), None);
        view.update(PreferencesAction::ToggleRunnerRemoval);
        assert_eq!(
            view.activation(),
            Some(("Edit", PreferencesAction::EditRunner))
        );
        view.update(PreferencesAction::RunnerCursor(1));
        assert_eq!(view.activation(), None);
        assert_eq!(
            view.update(PreferencesAction::EditRunner),
            PreferencesEffect::None
        );
    }

    /// The door labels the footer prints must be the labels the doors paint.
    #[test]
    fn every_activation_verb_is_the_label_of_its_own_control() {
        let mut view = view(false);
        for id in [
            PreferencesControlId::NewRunner,
            PreferencesControlId::InstallAgentSkill,
        ] {
            view.update(PreferencesAction::Focus(id));
            let (verb, _) = view.activation().expect("a door advertises its verb");
            assert_eq!(
                verb,
                view.control(id).expect("the door is reachable").label,
                "{id:?}"
            );
        }
    }

    #[test]
    fn a_cancelled_editor_leaves_the_staged_row_and_the_cursor_alone() {
        let mut view = empty_view();
        view.update(PreferencesAction::NewRunner);
        view.update(PreferencesAction::RunnerStaged(RunnerSaveRequest {
            name: "only".to_owned(),
            argv: vec!["only".to_owned(), "{{prompt}}".to_owned()],
            target: RunnerSaveTarget::New,
        }));
        view.update(PreferencesAction::RunnerCursor(0));
        let opened = view.update(PreferencesAction::EditRunner);
        assert!(opened_editor(&opened).is_some());
        assert_eq!(view.editing_runner_row(), Some(0));

        view.runner_editor_closed();

        assert_eq!(view.editing_runner_row(), None);
        assert_eq!(view.runner_cursor(), 0);
        assert_eq!(view.draft().runner_rows().len(), 1);

        // A second new agent must append instead of rewriting the row the cancelled editor held.
        view.update(PreferencesAction::NewRunner);
        view.update(PreferencesAction::RunnerStaged(RunnerSaveRequest {
            name: "second".to_owned(),
            argv: vec!["second".to_owned(), "{{prompt}}".to_owned()],
            target: RunnerSaveTarget::New,
        }));
        assert_eq!(view.draft().runner_rows().len(), 2);

        view.update(PreferencesAction::RunnerCursor(0));
        let opened = view.update(PreferencesAction::EditRunner);
        assert!(opened_editor(&opened).is_some());
        assert_eq!(view.editing_runner_row(), Some(0));
        view.update(PreferencesAction::Close);
        assert_eq!(view.editing_runner_row(), None);
    }

    /// The key removal owns every duplicate row of that key, so those rows take no command.
    #[test]
    fn a_row_the_key_change_takes_answers_no_key_of_its_own() {
        let mut duplicate = runner_row(1, "claude");
        duplicate.reason = Some("duplicate".to_owned());
        let mut view = PreferencesView::new(PreferencesDraft::from_snapshot(snapshot(
            false,
            vec![runner_row(0, "claude"), duplicate],
        )));
        view.update(PreferencesAction::RunnerCursor(0));
        view.update(PreferencesAction::ToggleRunnerRemoval);
        view.update(PreferencesAction::RunnerCursor(1));

        assert_eq!(
            view.update(PreferencesAction::EditRunner),
            PreferencesEffect::None
        );
        assert_eq!(
            view.update(PreferencesAction::ToggleRunnerRemoval),
            PreferencesEffect::None
        );

        assert_eq!(view.runner_cursor(), 1);
        assert!(view.draft().runner_rows()[0].is_removed());
        assert!(!view.draft().runner_rows()[1].is_removed());
        assert_eq!(view.error(), None);

        // The restored key gives every duplicate its own commands back.
        view.update(PreferencesAction::RunnerCursor(0));
        view.update(PreferencesAction::ToggleRunnerRemoval);
        view.update(PreferencesAction::RunnerCursor(1));
        assert!(opened_editor(&view.update(PreferencesAction::EditRunner)).is_some());
    }

    /// A cancelled removal brings a name back, so it is refused while another row holds it.
    #[test]
    fn a_refused_restore_keeps_the_removal_and_names_the_agent_list() {
        let mut view = view(false);
        view.update(PreferencesAction::RunnerCursor(0));
        view.update(PreferencesAction::ToggleRunnerRemoval);
        view.update(PreferencesAction::NewRunner);
        view.update(PreferencesAction::RunnerStaged(RunnerSaveRequest {
            name: "claude".to_owned(),
            argv: vec![
                "claude".to_owned(),
                "--new".to_owned(),
                "{{prompt}}".to_owned(),
            ],
            target: RunnerSaveTarget::New,
        }));
        view.update(PreferencesAction::RunnerCursor(0));

        assert_eq!(
            view.update(PreferencesAction::ToggleRunnerRemoval),
            PreferencesEffect::None
        );

        assert_eq!(view.error(), Some(&PreferencesError::RunnerNameTaken));
        assert_eq!(
            view.error().map(PreferencesError::field),
            Some(PreferencesField::Runners)
        );
        assert_eq!(view.focused(), PreferencesControlId::Runners);
        assert!(view.draft().runner_rows()[0].is_removed());
        assert_eq!(view.draft().runner_rows().len(), 3);

        // Dropping the appended row frees the name, and the next restore clears the refusal.
        view.update(PreferencesAction::RunnerCursor(2));
        view.update(PreferencesAction::ToggleRunnerRemoval);
        view.update(PreferencesAction::RunnerCursor(0));
        view.update(PreferencesAction::ToggleRunnerRemoval);
        assert_eq!(view.error(), None);
        assert!(!view.draft().runner_rows()[0].is_removed());
    }

    #[test]
    fn a_forged_agent_cursor_opens_no_editor() {
        let view = view(false);
        let mut value = serde_json::to_value(&view).unwrap();
        value["runner_cursor"] = serde_json::json!(99);
        let mut forged: PreferencesView = serde_json::from_value(value).unwrap();

        assert_eq!(
            forged.update(PreferencesAction::EditRunner),
            PreferencesEffect::None
        );
        assert_eq!(forged.focused(), PreferencesControlId::Runners);
    }

    #[test]
    fn removing_the_last_agent_row_moves_focus_to_the_new_agent_door() {
        let mut view = PreferencesView::new(PreferencesDraft::from_snapshot(snapshot(
            false,
            vec![runner_row(0, "claude")],
        )));
        view.update(PreferencesAction::NewRunner);
        view.update(PreferencesAction::RunnerStaged(RunnerSaveRequest {
            name: "extra".to_owned(),
            argv: vec!["extra".to_owned(), "{{prompt}}".to_owned()],
            target: RunnerSaveTarget::New,
        }));
        view.update(PreferencesAction::RunnerCursor(1));
        assert_eq!(view.runner_cursor(), 1);

        view.update(PreferencesAction::ToggleRunnerRemoval);
        assert_eq!(view.runner_cursor(), 0);
        assert_eq!(view.focused(), PreferencesControlId::Runners);

        view.update(PreferencesAction::ToggleRunnerRemoval);
        view.update(PreferencesAction::ValidationFailed(
            PreferencesError::RunnersChanged,
        ));
        assert_eq!(view.focused(), PreferencesControlId::Runners);
    }

    #[test]
    fn an_empty_agent_list_takes_the_save_refusal_on_the_new_agent_door() {
        let mut view = empty_view();

        assert_eq!(
            view.update(PreferencesAction::ValidationFailed(
                PreferencesError::RunnerPinsChanged {
                    name: "claude".to_owned(),
                    actual: 2,
                },
            )),
            PreferencesEffect::None
        );

        assert_eq!(view.focused(), PreferencesControlId::NewRunner);
        assert_eq!(
            view.error().map(PreferencesError::field),
            Some(PreferencesField::Runners)
        );
    }

    #[test]
    fn the_new_agent_door_and_an_edited_row_open_distinct_typed_editors() {
        let mut view = view(false);

        let opened = view.update(PreferencesAction::NewRunner);
        let editor = opened_editor(&opened).expect("the door opens the shared editor");
        assert_eq!(editor.mode(), RunnerEditorMode::New);
        assert_eq!(editor.name(), "");

        view.update(PreferencesAction::RunnerCursor(1));
        let opened = view.update(PreferencesAction::EditRunner);
        let editor = opened_editor(&opened).expect("the cursor row opens the shared editor");
        assert_eq!(editor.mode(), RunnerEditorMode::Edit);
        assert_eq!(editor.name(), "codex");
        assert_eq!(
            editor.command(),
            host_editable_command(
                &["codex", "{{prompt}}"],
                "codex '{{prompt}}'",
                "codex {{prompt}}"
            )
        );
    }

    #[test]
    fn a_staged_row_reopens_prefilled_and_its_save_rewrites_only_that_row() {
        let mut view = view(false);
        view.update(PreferencesAction::NewRunner);
        assert_eq!(
            view.update(PreferencesAction::RunnerStaged(RunnerSaveRequest {
                name: "extra".to_owned(),
                argv: vec!["extra".to_owned(), "{{prompt}}".to_owned()],
                target: RunnerSaveTarget::New,
            })),
            PreferencesEffect::RunnerStaged { refused: None }
        );
        assert_eq!(view.draft().runner_rows().len(), 3);

        view.update(PreferencesAction::RunnerCursor(2));
        let opened = view.update(PreferencesAction::EditRunner);
        let editor = opened_editor(&opened).expect("a staged row reopens its editor");
        assert_eq!(editor.mode(), RunnerEditorMode::New);
        assert_eq!(editor.name(), "extra");

        view.update(PreferencesAction::RunnerStaged(RunnerSaveRequest {
            name: "renamed".to_owned(),
            argv: vec!["renamed".to_owned(), "{{prompt}}".to_owned()],
            target: RunnerSaveTarget::New,
        }));
        assert_eq!(view.draft().runner_rows().len(), 3);
        assert_eq!(view.draft().runner_rows()[2].name(), Some("renamed"));

        // One staged result answers one editor. The next `New` request appends again.
        view.update(PreferencesAction::RunnerStaged(RunnerSaveRequest {
            name: "second".to_owned(),
            argv: vec!["second".to_owned(), "{{prompt}}".to_owned()],
            target: RunnerSaveTarget::New,
        }));
        assert_eq!(view.draft().runner_rows().len(), 4);
        assert_eq!(view.draft().runner_rows()[2].name(), Some("renamed"));
        assert_eq!(view.draft().runner_rows()[3].name(), Some("second"));

        assert_eq!(
            view.update(PreferencesAction::RunnerStaged(RunnerSaveRequest {
                name: "claude".to_owned(),
                argv: vec!["claude".to_owned(), "{{prompt}}".to_owned()],
                target: RunnerSaveTarget::New,
            })),
            PreferencesEffect::RunnerStaged {
                refused: Some(RunnerDraftError::DuplicateName),
            }
        );
        assert_eq!(view.draft().runner_rows().len(), 4);
    }

    #[test]
    fn a_row_staged_for_removal_and_a_malformed_container_have_no_editor() {
        let mut view = view(false);
        view.update(PreferencesAction::ToggleRunnerRemoval);
        assert_eq!(
            view.update(PreferencesAction::EditRunner),
            PreferencesEffect::None
        );

        let mut container = PreferencesView::new(PreferencesDraft::from_snapshot(snapshot(
            false,
            vec![RunnerRow {
                identity: RunnerRowIdentity {
                    index: None,
                    snapshot_token: "container".to_owned(),
                },
                name: None,
                argv: None,
                reason: Some("row-not-table".to_owned()),
                descriptor: "prompt.runners".to_owned(),
                key_identities: Vec::new(),
                pinned_count: 0,
            }],
        )));
        assert_eq!(
            container.update(PreferencesAction::EditRunner),
            PreferencesEffect::None
        );
        assert_eq!(container.focused(), PreferencesControlId::Runners);
    }

    #[test]
    fn a_repairable_malformed_row_opens_the_repair_editor_with_its_staged_values() {
        let mut view = PreferencesView::new(PreferencesDraft::from_snapshot(snapshot(
            false,
            vec![RunnerRow {
                identity: RunnerRowIdentity {
                    index: Some(3),
                    snapshot_token: "raw".to_owned(),
                },
                name: None,
                argv: Some(vec!["broken".to_owned(), "{{prompt}}".to_owned()]),
                reason: Some("name".to_owned()),
                descriptor: "prompt.runners[3]".to_owned(),
                key_identities: Vec::new(),
                pinned_count: 0,
            }],
        )));

        let opened = view.update(PreferencesAction::EditRunner);
        let editor = opened_editor(&opened).expect("a repairable row opens the repair editor");
        assert_eq!(editor.mode(), RunnerEditorMode::Repair);
        assert_eq!(
            editor.command(),
            host_editable_command(
                &["broken", "{{prompt}}"],
                "broken '{{prompt}}'",
                "broken {{prompt}}"
            )
        );
        assert_eq!(editor.name(), "");

        view.update(PreferencesAction::RunnerStaged(RunnerSaveRequest {
            name: "repaired".to_owned(),
            argv: vec![
                "repaired".to_owned(),
                "--fast".to_owned(),
                "{{prompt}}".to_owned(),
            ],
            target: RunnerSaveTarget::RawRow {
                expected: RunnerRowIdentity {
                    index: Some(3),
                    snapshot_token: "raw".to_owned(),
                },
            },
        }));
        let opened = view.update(PreferencesAction::EditRunner);
        let editor = opened_editor(&opened).expect("the staged repair reopens with its own values");
        assert_eq!(
            editor.command(),
            host_editable_command(
                &["repaired", "--fast", "{{prompt}}"],
                "repaired --fast '{{prompt}}'",
                "repaired --fast {{prompt}}"
            )
        );
        // A raw row keeps its raw target, and the name the user typed comes back with it.
        assert_eq!(editor.name(), "repaired");
        assert_eq!(editor.mode(), RunnerEditorMode::Repair);
        assert!(view.dirty());
    }

    #[test]
    fn a_typed_editor_error_replaces_any_earlier_host_refusal() {
        let mut editor = RunnerEditorView::new();
        editor.reduce(crate::management::RunnerEditorAction::MutationFailed(
            "an earlier host refusal".to_owned(),
        ));
        assert_eq!(editor.host_error(), Some("an earlier host refusal"));

        editor.refuse(RunnerEditorError::NameTaken);

        assert_eq!(editor.error(), Some(&RunnerEditorError::NameTaken));
        assert_eq!(editor.host_error(), None);
    }

    #[test]
    fn hiding_a_custom_url_rehomes_focus_and_unrelated_navigation_keeps_the_error() {
        let mut view = view(false);
        view.update(PreferencesAction::ChooseMirror {
            field: PreferencesField::GithubMirror,
            choice: MirrorChoice::Custom,
        });
        view.update(PreferencesAction::SetMirrorUrl {
            field: PreferencesField::GithubMirror,
            value: "http://mirror.example/gh".to_owned(),
        });
        assert_eq!(
            view.update(PreferencesAction::Save),
            PreferencesEffect::None
        );
        assert!(view.error().is_some());

        view.update(PreferencesAction::Previous);
        assert!(view.error().is_some());
        view.update(PreferencesAction::Focus(PreferencesControlId::GithubUrl));
        view.update(PreferencesAction::ChooseMirror {
            field: PreferencesField::GithubMirror,
            choice: MirrorChoice::Off,
        });

        assert_eq!(view.focused(), PreferencesControlId::GithubChoice);
        assert!(!view.has_control(PreferencesControlId::GithubUrl));
        assert!(view.error().is_none());
    }

    #[test]
    fn agent_skill_install_is_a_typed_picker_and_explicit_target_transaction() {
        let mut view = view(false);
        assert_eq!(
            view.update(PreferencesAction::InstallAgentSkill),
            PreferencesEffect::DiscoverAgentSkillTargets
        );
        view.update(PreferencesAction::PresentAgentSkillTargets(vec![
            AgentTarget {
                name: "claude".to_owned(),
                scope: AgentScope::User,
                base: PathBuf::from("/home/demo/.claude"),
            },
            AgentTarget {
                name: "codex".to_owned(),
                scope: AgentScope::Project,
                base: PathBuf::from("/work/.codex"),
            },
        ]));
        assert_eq!(view.agent_skill_install().unwrap().targets().len(), 2);
        assert_eq!(view.agent_skill_install().unwrap().selected(), Some(0));

        assert_eq!(
            view.update(PreferencesAction::ActivateAgentSkillTarget(0)),
            PreferencesEffect::InstallAgentSkill {
                skills_dir: PathBuf::from("/home/demo/.claude/skills"),
            }
        );

        view.update(PreferencesAction::SelectAgentSkillTarget(1));
        assert_eq!(
            view.update(PreferencesAction::ConfirmAgentSkillTarget),
            PreferencesEffect::InstallAgentSkill {
                skills_dir: PathBuf::from("/work/.codex/skills"),
            }
        );
        view.update(PreferencesAction::AgentSkillInstalled {
            message: "Installed".to_owned(),
        });
        assert!(view.agent_skill_install().is_none());
    }

    #[test]
    fn no_detected_agent_target_still_opens_a_mouse_and_keyboard_closable_modal() {
        let mut view = view(false);
        view.update(PreferencesAction::PresentAgentSkillTargets(Vec::new()));
        assert!(view.agent_skill_install().is_some());
        assert_eq!(
            view.update(PreferencesAction::ConfirmAgentSkillTarget),
            PreferencesEffect::None
        );
        assert!(view.agent_skill_install().is_some());
        view.update(PreferencesAction::CloseAgentSkillTargets);
        assert!(view.agent_skill_install().is_none());
    }
}
