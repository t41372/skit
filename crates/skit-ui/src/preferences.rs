//! Frontend-neutral application-preference state.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use skit_application::AgentTarget;
use skit_application::preferences::{
    AfterRunChoice, InteractiveFormChoice, JavascriptChoice, MirrorChoice, PreferencesChangeSet,
    PreferencesDraft, PreferencesError, PreferencesField, github_preset_names, npm_preset_names,
    pypi_preset_names,
};

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
    /// Prompt-runner and Agent Skill doors.
    Agents,
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
    /// Open prompt-runner management.
    ManageAgents,
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
    /// Open the prompt-runner manager.
    ManageAgents,
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
    /// Open prompt-runner management.
    ManageAgents,
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
        controls.extend([
            button(
                PreferencesControlId::ManageAgents,
                "Manage agents…",
                "Agents (prompt runners)",
            ),
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
                "",
                vec![self.agent_summary()],
                controls_for(
                    &controls,
                    &[
                        PreferencesControlId::ManageAgents,
                        PreferencesControlId::InstallAgentSkill,
                    ],
                ),
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

    /// Build the localized agent count and name summary.
    #[must_use]
    pub fn agent_summary(&self) -> PreferencesDisplayText {
        match self.draft.runner_names.as_slice() {
            [] => PreferencesDisplayText::new("No agents configured."),
            [name] => PreferencesDisplayText::with_arguments(
                "{} agent configured: {}",
                ["1", name.as_str()],
            ),
            names => PreferencesDisplayText::with_arguments(
                "{} agents configured: {}",
                [names.len().to_string(), names.join(", ")],
            ),
        }
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
                    PreferencesField::BashPath => return PreferencesEffect::None,
                }
                self.clear_error_for(field);
                self.rehome_hidden_focus(field);
            }
            PreferencesAction::SetMirrorUrl { field, value } => {
                match field {
                    PreferencesField::PypiMirror => {
                        self.focused = PreferencesControlId::PypiUrl;
                        self.draft.pypi_url = value;
                    }
                    PreferencesField::GithubMirror => {
                        self.focused = PreferencesControlId::GithubUrl;
                        self.draft.github_url = value;
                    }
                    PreferencesField::NpmMirror => {
                        self.focused = PreferencesControlId::NpmUrl;
                        self.draft.npm_url = value;
                    }
                    PreferencesField::BashPath => return PreferencesEffect::None,
                }
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
                return if self.dirty() {
                    PreferencesEffect::ConfirmDiscard
                } else {
                    PreferencesEffect::Close
                };
            }
            PreferencesAction::ManageAgents => return PreferencesEffect::ManageAgents,
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

    fn clear_error_for(&mut self, field: PreferencesField) {
        if self
            .error
            .as_ref()
            .is_some_and(|error| error.field() == field)
        {
            self.error = None;
        }
    }

    fn rehome_hidden_focus(&mut self, field: PreferencesField) {
        let (url, choice) = match field {
            PreferencesField::PypiMirror => (
                PreferencesControlId::PypiUrl,
                PreferencesControlId::PypiChoice,
            ),
            PreferencesField::GithubMirror => (
                PreferencesControlId::GithubUrl,
                PreferencesControlId::GithubChoice,
            ),
            PreferencesField::NpmMirror => (
                PreferencesControlId::NpmUrl,
                PreferencesControlId::NpmChoice,
            ),
            PreferencesField::BashPath => return,
        };
        if self.focused == url && !self.has_control(url) {
            self.focused = choice;
        }
    }

    fn move_focus(&mut self, delta: isize) {
        let controls = self.controls();
        let current = controls
            .iter()
            .position(|control| control.id == self.focused)
            .unwrap_or_default();
        let next = if delta.is_negative() {
            current.saturating_sub(delta.unsigned_abs())
        } else {
            current
                .saturating_add(delta as usize)
                .min(controls.len().saturating_sub(1))
        };
        if let Some(control) = controls.get(next) {
            self.focused = control.id;
        }
    }

    fn set_error(&mut self, error: PreferencesError) {
        self.focused = match error.field() {
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
        };
        self.error = Some(error);
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
        PreferencesDraft, PreferencesField, PreferencesSnapshot,
    };
    use skit_application::{AgentScope, AgentTarget};

    use super::{
        PreferencesAction, PreferencesControlId, PreferencesControlKind, PreferencesEffect,
        PreferencesSectionId, PreferencesView,
    };

    fn view(windows: bool) -> PreferencesView {
        PreferencesView::new(PreferencesDraft::from_snapshot(PreferencesSnapshot {
            language: String::new(),
            available_languages: vec!["en".to_owned(), "zh-CN".to_owned(), "zh-TW".to_owned()],
            effective_language: "en".to_owned(),
            editor: String::new(),
            editor_fallback: Some("vim".to_owned()),
            form: InteractiveFormChoice::Tui,
            after_run: AfterRunChoice::Exit,
            javascript: JavascriptChoice::Automatic,
            bash_path: windows.then(String::new),
            runner_names: vec!["claude".to_owned(), "codex".to_owned()],
            mirror: MirrorConfiguration::default(),
        }))
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
                PreferencesControlId::ManageAgents,
                PreferencesControlId::InstallAgentSkill,
                PreferencesControlId::MirrorMaster,
                PreferencesControlId::PypiChoice,
                PreferencesControlId::GithubChoice,
                PreferencesControlId::NpmChoice,
            ]
        );
        assert_eq!(view.focused(), PreferencesControlId::Language);
        assert!(!view.dirty());
        assert_eq!(view.draft().runner_names, ["claude", "codex"]);
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
    fn save_manage_agents_skill_and_dirty_close_are_distinct_typed_effects() {
        let mut view = view(false);
        assert_eq!(
            view.update(PreferencesAction::Close),
            PreferencesEffect::Close
        );
        assert_eq!(
            view.update(PreferencesAction::ManageAgents),
            PreferencesEffect::ManageAgents
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
            sections[6].title.key,
            "Download mirrors (mainland-China acceleration)"
        );
        assert_eq!(
            sections[6].help.key,
            "Each ecosystem is its own choice — mirror vendors differ per axis."
        );
        assert_eq!(
            sections[6].help_placement,
            super::PreferencesTextPlacement::BeforeControls
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
        let PreferencesControlKind::Choice(choice) = &form.kind else {
            panic!("interactive form must be a choice");
        };
        assert_eq!(
            choice
                .options
                .iter()
                .map(|option| (option.value.as_str(), option.label.as_str()))
                .collect::<Vec<_>>(),
            [
                ("tui", "Mini form — opens in place, fully clickable"),
                (
                    "plain",
                    "Line-by-line prompts — plainest, best over slow terminals",
                ),
            ]
        );
        let after = view
            .control(PreferencesControlId::AfterRun)
            .expect("after-run control");
        let PreferencesControlKind::Choice(choice) = &after.kind else {
            panic!("after-run must be a choice");
        };
        assert_eq!(
            choice
                .options
                .iter()
                .map(|option| option.label.as_str())
                .collect::<Vec<_>>(),
            [
                "Quit skit — leave the run's output in the terminal",
                "Return to the Library immediately",
            ]
        );
    }

    #[test]
    fn agent_summary_preserves_empty_singular_and_plural_product_copy() {
        let empty = view(false);
        assert_eq!(
            empty.agent_summary(),
            super::PreferencesDisplayText::with_arguments(
                "{} agents configured: {}",
                ["2", "claude, codex"],
            )
        );

        let mut singular = view(false);
        singular.draft.runner_names = vec!["solo".to_owned()];
        assert_eq!(
            singular.agent_summary(),
            super::PreferencesDisplayText::with_arguments("{} agent configured: {}", ["1", "solo"],)
        );

        singular.draft.runner_names.clear();
        assert_eq!(
            singular.agent_summary(),
            super::PreferencesDisplayText::new("No agents configured.")
        );
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
