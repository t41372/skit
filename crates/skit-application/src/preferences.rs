//! Frontend-neutral planning for application preferences.

use std::{collections::BTreeMap, path::Path};

use serde::{Deserialize, Serialize};
use skit_i18n::{Localize, Message};
use thiserror::Error;

use crate::runner_management::{RunnerRow, RunnerRowIdentity, RunnerSaveRequest, RunnerSaveTarget};

/// One configured download-mirror state.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct MirrorConfiguration {
    /// Apply the stored URLs to child processes.
    pub enabled: bool,
    /// Python package index URL.
    pub pypi: String,
    /// Python-build download prefix.
    pub python_install: String,
    /// uv binary download prefix.
    pub uv_binary: String,
    /// npm registry URL.
    pub npm: String,
}

impl MirrorConfiguration {
    fn has_urls(&self) -> bool {
        !self.pypi.is_empty()
            || !self.python_install.is_empty()
            || !self.uv_binary.is_empty()
            || !self.npm.is_empty()
    }
}

/// How terminal commands collect interactive parameter values.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InteractiveFormChoice {
    /// Open the terminal form.
    #[default]
    Tui,
    /// Ask one line at a time.
    Plain,
}

impl InteractiveFormChoice {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Tui => "tui",
            Self::Plain => "plain",
        }
    }
}

/// What the library browser does after a child exits.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AfterRunChoice {
    /// Exit and leave the child output visible.
    #[default]
    Exit,
    /// Return to the library.
    Stay,
}

impl AfterRunChoice {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Exit => "exit",
            Self::Stay => "stay",
        }
    }
}

/// The palette of the interactive interface.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeChoice {
    /// The terminal's own colors.
    #[default]
    Terminal,
    /// The fixed skit palette of version 0.4.
    Skit,
}

impl ThemeChoice {
    /// The value that `config.toml` and `skit config theme` use.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Terminal => "terminal",
            Self::Skit => "skit",
        }
    }

    /// The choice that a stored value names. Every value except `skit` names the default, as
    /// the store reads a stored value that it does not know.
    #[must_use]
    pub fn from_config(value: &str) -> Self {
        if value == "skit" {
            Self::Skit
        } else {
            Self::Terminal
        }
    }
}

/// The one hue of the terminal theme.
///
/// A color name picks that color of the terminal's own palette, so the hue follows the user's
/// terminal colors. The `skit` theme ignores the choice.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AccentChoice {
    /// Cyan on a dark background, magenta on a light one, and no hue when the terminal does not
    /// say. Only this choice asks the terminal for its colors.
    #[default]
    Auto,
    /// No hue: marks use the default colors.
    None,
    /// ANSI red.
    Red,
    /// ANSI green.
    Green,
    /// ANSI yellow.
    Yellow,
    /// ANSI blue.
    Blue,
    /// ANSI magenta.
    Magenta,
    /// ANSI cyan.
    Cyan,
    /// ANSI bright red.
    BrightRed,
    /// ANSI bright green.
    BrightGreen,
    /// ANSI bright yellow.
    BrightYellow,
    /// ANSI bright blue.
    BrightBlue,
    /// ANSI bright magenta.
    BrightMagenta,
    /// ANSI bright cyan.
    BrightCyan,
}

impl AccentChoice {
    /// Every choice, in the order that `skit config` and Preferences name them.
    pub const ALL: [Self; 14] = [
        Self::Auto,
        Self::None,
        Self::Red,
        Self::Green,
        Self::Yellow,
        Self::Blue,
        Self::Magenta,
        Self::Cyan,
        Self::BrightRed,
        Self::BrightGreen,
        Self::BrightYellow,
        Self::BrightBlue,
        Self::BrightMagenta,
        Self::BrightCyan,
    ];

    /// The value that `config.toml` and `skit config accent` use.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::None => "none",
            Self::Red => "red",
            Self::Green => "green",
            Self::Yellow => "yellow",
            Self::Blue => "blue",
            Self::Magenta => "magenta",
            Self::Cyan => "cyan",
            Self::BrightRed => "bright-red",
            Self::BrightGreen => "bright-green",
            Self::BrightYellow => "bright-yellow",
            Self::BrightBlue => "bright-blue",
            Self::BrightMagenta => "bright-magenta",
            Self::BrightCyan => "bright-cyan",
        }
    }

    /// The choice that a stored value names. A value that names no choice names the default, as
    /// the store reads a stored value that it does not know.
    #[must_use]
    pub fn from_config(value: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|choice| choice.as_str() == value)
            .unwrap_or_default()
    }
}

/// Preferred JavaScript and TypeScript runtime.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JavascriptChoice {
    /// Pick the first available runtime in product order.
    #[default]
    Automatic,
    /// Use deno.
    Deno,
    /// Use bun.
    Bun,
    /// Use node.
    Node,
}

impl JavascriptChoice {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Automatic => "",
            Self::Deno => "deno",
            Self::Bun => "bun",
            Self::Node => "node",
        }
    }
}

/// One mirror-axis selection.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MirrorChoice {
    /// Use one named product preset.
    Preset(String),
    /// Use the adjacent URL field.
    Custom,
    /// Clear this axis.
    Off,
}

/// Read-only values needed to construct the complete Preferences workflow.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PreferencesSnapshot {
    /// Stored language tag. An empty value follows the system.
    pub language: String,
    /// Shipped language tags.
    pub available_languages: Vec<String>,
    /// Language currently in effect.
    pub effective_language: String,
    /// Stored editor command.
    pub editor: String,
    /// Effective environment fallback when present.
    pub editor_fallback: Option<String>,
    /// Interactive form preference.
    pub form: InteractiveFormChoice,
    /// Post-run preference.
    pub after_run: AfterRunChoice,
    /// Interface palette.
    pub theme: ThemeChoice,
    /// Hue of the terminal theme.
    pub accent: AccentChoice,
    /// JavaScript runtime preference.
    pub javascript: JavascriptChoice,
    /// Windows bash path. `None` hides the Windows-only section.
    pub bash_path: Option<String>,
    /// Complete prompt-runner management rows in config order.
    pub runners: Vec<RunnerRow>,
    /// Stored mirror state, including paused URLs.
    pub mirror: MirrorConfiguration,
}

/// Editable frontend-neutral Preferences state.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PreferencesDraft {
    /// Selected language tag or `auto`.
    pub language: String,
    /// Language choices in display order.
    pub language_options: Vec<String>,
    /// Language currently in effect.
    pub effective_language: String,
    /// Editor command.
    pub editor: String,
    /// Effective editor fallback.
    pub editor_fallback: Option<String>,
    /// Interactive form preference.
    pub form: InteractiveFormChoice,
    /// Post-run preference.
    pub after_run: AfterRunChoice,
    /// Interface palette.
    pub theme: ThemeChoice,
    /// Hue of the terminal theme.
    pub accent: AccentChoice,
    /// JavaScript runtime preference.
    pub javascript: JavascriptChoice,
    /// Windows bash path. `None` hides the section.
    pub bash_path: Option<String>,
    /// Apply or pause saved mirror URLs.
    pub mirror_master: bool,
    /// PyPI choice.
    pub pypi: MirrorChoice,
    /// Custom PyPI URL.
    pub pypi_url: String,
    /// GitHub-release choice.
    pub github: MirrorChoice,
    /// Custom GitHub-release base URL.
    pub github_url: String,
    /// npm choice.
    pub npm: MirrorChoice,
    /// Custom npm URL.
    pub npm_url: String,
    runners: Vec<RunnerDraftRow>,
    initial: PreferencesInitial,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct PreferencesInitial {
    language: String,
    editor: String,
    form: InteractiveFormChoice,
    after_run: AfterRunChoice,
    theme: ThemeChoice,
    accent: AccentChoice,
    javascript: JavascriptChoice,
    bash_path: Option<String>,
    mirror_master: bool,
    pypi: MirrorChoice,
    pypi_url: String,
    github: MirrorChoice,
    github_url: String,
    npm: MirrorChoice,
    npm_url: String,
    mirror: MirrorConfiguration,
}

/// Validated values for one atomic host transaction.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PreferencesChangeSet {
    /// Stable CLI/config keys and their final values.
    pub settings: BTreeMap<String, String>,
    /// Staged agent mutations in store-application order.
    pub runners: Vec<RunnerChange>,
}

impl PreferencesChangeSet {
    /// Validate file-backed settings through a host-supplied filesystem projection.
    ///
    /// The host can expand platform-specific path syntax before it performs the file query.
    pub fn validate_files(&self, is_file: impl Fn(&Path) -> bool) -> Result<(), PreferencesError> {
        let Some(path) = self
            .settings
            .get("shell.bash_path")
            .map(String::as_str)
            .map(str::trim)
            .filter(|path| !path.is_empty())
        else {
            return Ok(());
        };
        if is_file(Path::new(path)) {
            Ok(())
        } else {
            Err(PreferencesError::BashPathMissing {
                path: path.to_owned(),
            })
        }
    }
}

/// Control that owns a validation error.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PreferencesField {
    /// Windows bash path.
    BashPath,
    /// PyPI mirror row.
    PypiMirror,
    /// GitHub-release mirror row.
    GithubMirror,
    /// npm mirror row.
    NpmMirror,
    /// Agent (prompt runner) list.
    Runners,
}

/// A Preferences draft cannot be submitted without changing one control.
#[derive(Clone, Debug, Deserialize, Eq, Error, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PreferencesError {
    /// A custom mirror row has no valid URL token.
    #[error("a custom mirror choice needs a URL")]
    CustomUrlRequired {
        /// Affected mirror row.
        field: PreferencesField,
    },
    /// The executable uv download mirror does not use HTTPS.
    #[error("the github-release mirror base does not use HTTPS: {url}")]
    GithubHttpsRequired {
        /// Rejected URL.
        url: String,
    },
    /// A configured Windows bash path does not name a file.
    #[error("bash file does not exist: {path}")]
    BashPathMissing {
        /// Rejected path.
        path: String,
    },
    /// The stored agent rows changed after Preferences read them.
    #[error("the agent rows changed before the save")]
    RunnersChanged,
    /// A staged agent name is already in use by another row the save keeps.
    #[error("another agent row already uses the name")]
    RunnerNameTaken,
    /// The prompt pins of one staged agent removal changed before the save.
    #[error("the prompt pins of agent {name} changed to {actual}")]
    RunnerPinsChanged {
        /// Stable agent key of the refused removal.
        name: String,
        /// Pin count the store found.
        actual: usize,
    },
}

impl PreferencesError {
    /// Return the control that must change.
    #[must_use]
    pub const fn field(&self) -> PreferencesField {
        match self {
            Self::CustomUrlRequired { field } => *field,
            Self::GithubHttpsRequired { .. } => PreferencesField::GithubMirror,
            Self::BashPathMissing { .. } => PreferencesField::BashPath,
            Self::RunnersChanged | Self::RunnerNameTaken | Self::RunnerPinsChanged { .. } => {
                PreferencesField::Runners
            }
        }
    }
}

impl Localize for PreferencesError {
    fn message(&self) -> Message {
        match self {
            Self::CustomUrlRequired { .. } => Message::new("A custom choice needs a URL."),
            Self::GithubHttpsRequired { url } => Message::new(
                "The uv binary is downloaded and executed, so the github-release base URL must use https:// (got: {}).",
            )
            .with(url),
            Self::BashPathMissing { path } => Message::new("No such file: {}").with(path),
            Self::RunnersChanged => Message::new(
                "The agent list changed on disk. Reopen Preferences and try again.",
            ),
            Self::RunnerNameTaken => {
                Message::new("Another row already uses this runner name.")
            }
            Self::RunnerPinsChanged { .. } => Message::new(
                "The prompt pins changed before the runner could be removed; inspect again.",
            ),
        }
    }
}

const PYPI_PRESETS: &[(&str, &str)] = &[
    ("tsinghua", "https://pypi.tuna.tsinghua.edu.cn/simple"),
    ("aliyun", "https://mirrors.aliyun.com/pypi/simple"),
    ("ustc", "https://pypi.mirrors.ustc.edu.cn/simple"),
];
const GITHUB_PRESETS: &[(&str, &str)] = &[("nju", "https://mirror.nju.edu.cn/github-release")];
const NPM_PRESETS: &[(&str, &str)] = &[("npmmirror", "https://registry.npmmirror.com")];

/// Return PyPI preset names in product order.
#[must_use]
pub fn pypi_preset_names() -> Vec<String> {
    preset_names(PYPI_PRESETS)
}

/// Return GitHub-release preset names in product order.
#[must_use]
pub fn github_preset_names() -> Vec<String> {
    preset_names(GITHUB_PRESETS)
}

/// Return npm preset names in product order.
#[must_use]
pub fn npm_preset_names() -> Vec<String> {
    preset_names(NPM_PRESETS)
}

fn preset_names(presets: &[(&str, &str)]) -> Vec<String> {
    presets.iter().map(|(name, _)| (*name).to_owned()).collect()
}

/// One staged agent mutation addressed by frontend row identities.
///
/// The store adapter resolves every identity into its raw configuration row before it writes.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunnerChange {
    /// Append one agent. A name that a row already uses is refused.
    Add {
        /// Stable agent name.
        name: String,
        /// Direct process argv.
        argv: Vec<String>,
    },
    /// Replace one stable agent key while all of its raw rows match the read.
    ReplaceNamed {
        /// Stable agent name.
        name: String,
        /// Direct process argv.
        argv: Vec<String>,
        /// Every raw row the read reported for the key.
        expected: Vec<RunnerRowIdentity>,
    },
    /// Repair one malformed raw row while its complete snapshot matches the read.
    RepairRow {
        /// Stable agent name the user typed.
        name: String,
        /// Direct process argv.
        argv: Vec<String>,
        /// Raw row from the read.
        expected: RunnerRowIdentity,
    },
    /// Remove one stable agent key while its rows and its prompt pins match the read.
    RemoveNamed {
        /// Stable agent name.
        name: String,
        /// Every raw row the read reported for the key.
        expected: Vec<RunnerRowIdentity>,
        /// Prompt entries the read found pinned to the key.
        expected_pinned_count: usize,
    },
    /// Remove one raw row, or one malformed container, while its snapshot matches the read.
    RemoveRow {
        /// Raw row from the read.
        expected: RunnerRowIdentity,
    },
}

/// New values staged on one stored agent row.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RunnerDraftEdit {
    /// Stable agent name. A repaired raw row takes the name the user typed.
    pub name: String,
    /// Direct process argv.
    pub argv: Vec<String>,
}

/// Change staged on one stored agent row.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunnerDraftState {
    /// The stored row is unchanged.
    Unchanged,
    /// The row has new values.
    Edited(RunnerDraftEdit),
    /// The row is staged for removal.
    Removed {
        /// Values that come back when the user cancels the removal.
        edit: Option<RunnerDraftEdit>,
    },
}

/// Staging state shown beside one agent row.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunnerDraftMarker {
    /// The row is staged for append.
    Added,
    /// The row has new values.
    Edited,
    /// The row is staged for removal.
    Removed,
}

/// One row of the Preferences agent list.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunnerDraftRow {
    /// One row the configuration already stores.
    Existing {
        /// Row exactly as the host read it.
        row: RunnerRow,
        /// Change staged on the row.
        state: RunnerDraftState,
    },
    /// One row staged for append. It has no stored identity.
    Added {
        /// Stable agent name.
        name: String,
        /// Direct process argv.
        argv: Vec<String>,
    },
}

impl RunnerDraftRow {
    /// Return the name the row has after staging.
    #[must_use]
    pub fn name(&self) -> Option<&str> {
        match self {
            Self::Existing { row, state } => state
                .edit()
                .map_or_else(|| row.name.as_deref(), |edit| Some(edit.name.as_str())),
            Self::Added { name, .. } => Some(name.as_str()),
        }
    }

    /// Return the argv the row has after staging.
    #[must_use]
    pub fn argv(&self) -> Option<&[String]> {
        match self {
            Self::Existing { row, state } => state
                .edit()
                .map_or_else(|| row.argv.as_deref(), |edit| Some(edit.argv.as_slice())),
            Self::Added { argv, .. } => Some(argv.as_slice()),
        }
    }

    /// Return the staging marker, or `None` for a stored row without a staged change.
    #[must_use]
    pub const fn marker(&self) -> Option<RunnerDraftMarker> {
        match self {
            Self::Existing {
                state: RunnerDraftState::Unchanged,
                ..
            } => None,
            Self::Existing {
                state: RunnerDraftState::Edited(_),
                ..
            } => Some(RunnerDraftMarker::Edited),
            Self::Existing {
                state: RunnerDraftState::Removed { .. },
                ..
            } => Some(RunnerDraftMarker::Removed),
            Self::Added { .. } => Some(RunnerDraftMarker::Added),
        }
    }

    /// Report whether an editor can open on this row.
    ///
    /// A staged removal answers the Del key alone, and a row without an index or an argv list has
    /// no shape to repair.
    #[must_use]
    pub const fn is_editable(&self) -> bool {
        match self {
            Self::Added { .. } => true,
            Self::Existing { row, state } => {
                !matches!(state, RunnerDraftState::Removed { .. }) && row.is_editable()
            }
        }
    }

    /// Report whether Ctrl+S removes this row.
    #[must_use]
    pub const fn is_removed(&self) -> bool {
        matches!(
            self,
            Self::Existing {
                state: RunnerDraftState::Removed { .. },
                ..
            }
        )
    }

    /// Return the stored row with every staged value applied.
    ///
    /// An appended row has no stored identity, so it has no resolved row.
    #[must_use]
    pub fn resolved_row(&self) -> Option<RunnerRow> {
        match self {
            Self::Existing { row, state } => {
                let mut resolved = row.clone();
                if let Some(edit) = state.edit() {
                    // A raw row has no stored name, and the repair the user typed is the one the
                    // editor must show again when it reopens.
                    resolved.name = Some(edit.name.clone());
                    resolved.argv = Some(edit.argv.clone());
                }
                Some(resolved)
            }
            Self::Added { .. } => None,
        }
    }
}

impl RunnerDraftState {
    const fn edit(&self) -> Option<&RunnerDraftEdit> {
        match self {
            Self::Unchanged => None,
            Self::Edited(edit) => Some(edit),
            Self::Removed { edit } => edit.as_ref(),
        }
    }
}

/// A staged agent edit cannot join the draft.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Error, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunnerDraftError {
    /// Another row that the save keeps already uses the name.
    #[error("another agent row already uses the name")]
    DuplicateName,
}

/// Report whether the change staged on the stable key of one row already takes that row.
///
/// The store keeps one row for each stable name. It removes every raw row of a key it deletes,
/// and it folds the duplicate rows of a key it edits into the first row of that key. A row the
/// save takes this way carries no change of its own, so the frontend says what happens to it and
/// offers it no command.
///
/// One row of each key answers for the key: a named save marks the first row the save keeps, and
/// only the valid row of a key can carry its removal. Two rows of one key that both carry a change
/// would take each other, and the change set would lose both.
#[must_use]
pub fn runner_row_taken_by_its_key(rows: &[RunnerDraftRow], index: usize) -> bool {
    let Some(RunnerDraftRow::Existing { row, state }) = rows.get(index) else {
        return false;
    };
    let Some(name) = row.name.as_deref() else {
        return false;
    };
    // The row that carries the key removal speaks for the key, so nothing takes it.
    if matches!(state, RunnerDraftState::Removed { .. }) && row.is_valid() {
        return false;
    }
    rows.iter()
        .enumerate()
        .filter(|(other, _)| *other != index)
        .any(|(_, other)| match other {
            RunnerDraftRow::Existing { row: stored, state }
                if stored.name.as_deref() == Some(name) =>
            {
                match state {
                    RunnerDraftState::Unchanged => false,
                    // The edit keeps one row of the key and folds the others into it.
                    RunnerDraftState::Edited(_) => true,
                    // The key removal takes every raw row. One raw row leaves its twins.
                    RunnerDraftState::Removed { .. } => stored.is_valid(),
                }
            }
            RunnerDraftRow::Existing { .. } | RunnerDraftRow::Added { .. } => false,
        })
}

impl PreferencesDraft {
    /// Build the full workflow from stored and effective configuration values.
    #[must_use]
    pub fn from_snapshot(snapshot: PreferencesSnapshot) -> Self {
        let language = if snapshot.language.is_empty() {
            "auto".to_owned()
        } else {
            snapshot.language.clone()
        };
        let mut language_options = vec!["auto".to_owned()];
        for option in &snapshot.available_languages {
            if !language_options.contains(option) {
                language_options.push(option.clone());
            }
        }
        if !language_options.contains(&language) {
            language_options.push(language.clone());
        }
        let mirror_master = snapshot.mirror.enabled || !snapshot.mirror.has_urls();
        let pypi = axis_choice(&snapshot.mirror.pypi, PYPI_PRESETS);
        let github = github_choice(&snapshot.mirror);
        let npm = axis_choice(&snapshot.mirror.npm, NPM_PRESETS);
        let pypi_url = snapshot.mirror.pypi.clone();
        let github_url = github_base(&snapshot.mirror);
        let npm_url = snapshot.mirror.npm.clone();
        let initial = PreferencesInitial {
            language: language.clone(),
            editor: snapshot.editor.clone(),
            form: snapshot.form,
            after_run: snapshot.after_run,
            theme: snapshot.theme,
            accent: snapshot.accent,
            javascript: snapshot.javascript,
            bash_path: snapshot.bash_path.clone(),
            mirror_master,
            pypi: pypi.clone(),
            pypi_url: pypi_url.clone(),
            github: github.clone(),
            github_url: github_url.clone(),
            npm: npm.clone(),
            npm_url: npm_url.clone(),
            mirror: snapshot.mirror,
        };
        Self {
            language,
            language_options,
            effective_language: snapshot.effective_language,
            editor: snapshot.editor,
            editor_fallback: snapshot.editor_fallback,
            form: snapshot.form,
            after_run: snapshot.after_run,
            theme: snapshot.theme,
            accent: snapshot.accent,
            javascript: snapshot.javascript,
            bash_path: snapshot.bash_path,
            runners: snapshot
                .runners
                .into_iter()
                .map(|row| RunnerDraftRow::Existing {
                    row,
                    state: RunnerDraftState::Unchanged,
                })
                .collect(),
            mirror_master,
            pypi,
            pypi_url,
            github,
            github_url,
            npm,
            npm_url,
            initial,
        }
    }

    /// Return every agent row in configuration order.
    #[must_use]
    pub fn runner_rows(&self) -> &[RunnerDraftRow] {
        &self.runners
    }

    /// Stage one validated agent editor result.
    ///
    /// `edited_row` is the appended row the editor opened, so the "new agent" door never
    /// overwrites the row the cursor happens to rest on. A save on a stable name marks the first
    /// row of that name, because the store coalesces every raw row of one key into it.
    pub fn stage_runner(
        &mut self,
        request: RunnerSaveRequest,
        edited_row: Option<usize>,
    ) -> Result<(), RunnerDraftError> {
        let target = match &request.target {
            RunnerSaveTarget::New => edited_row.filter(|index| {
                matches!(self.runners.get(*index), Some(RunnerDraftRow::Added { .. }))
            }),
            RunnerSaveTarget::Named { name, .. } => {
                // The store rewrites the first row of the key and folds the others into it, so
                // the edit marks the first row this save keeps and never covers a removal.
                let Some(index) = self.runners.iter().position(|row| {
                    matches!(row, RunnerDraftRow::Existing { row: stored, .. } if stored.name.as_deref() == Some(name.as_str()))
                        && !row.is_removed()
                }) else {
                    return Ok(());
                };
                Some(index)
            }
            RunnerSaveTarget::RawRow { expected } => {
                let Some(index) = self.runners.iter().position(|row| {
                    matches!(row, RunnerDraftRow::Existing { row, .. } if &row.identity == expected)
                }) else {
                    return Ok(());
                };
                Some(index)
            }
        };
        let key = match &request.target {
            RunnerSaveTarget::Named { name, .. } => Some(name.as_str()),
            RunnerSaveTarget::New | RunnerSaveTarget::RawRow { .. } => None,
        };
        if self.runner_name_taken(&request.name, target, key) {
            return Err(RunnerDraftError::DuplicateName);
        }
        let edit = RunnerDraftEdit {
            name: request.name,
            argv: request.argv,
        };
        match (target.and_then(|index| self.runners.get_mut(index)), edit) {
            (Some(RunnerDraftRow::Added { name, argv }), edit) => {
                *name = edit.name;
                *argv = edit.argv;
            }
            (Some(RunnerDraftRow::Existing { state, .. }), edit) => {
                *state = RunnerDraftState::Edited(edit);
            }
            (None, edit) => self.runners.push(RunnerDraftRow::Added {
                name: edit.name,
                argv: edit.argv,
            }),
        }
        Ok(())
    }

    /// Stage or cancel the removal of one agent row.
    ///
    /// An appended row has nothing on disk, so removing it drops the row. A cancelled removal
    /// brings the name back, so it is refused while another live row already uses that name.
    pub fn toggle_runner_removal(&mut self, index: usize) -> Result<(), RunnerDraftError> {
        if self.restore_repeats_a_live_name(index) {
            return Err(RunnerDraftError::DuplicateName);
        }
        let Some(row) = self.runners.get_mut(index) else {
            return Ok(());
        };
        match row {
            RunnerDraftRow::Added { .. } => {
                self.runners.remove(index);
            }
            RunnerDraftRow::Existing { state, .. } => {
                *state = match std::mem::replace(state, RunnerDraftState::Unchanged) {
                    RunnerDraftState::Unchanged => RunnerDraftState::Removed { edit: None },
                    RunnerDraftState::Edited(edit) => {
                        RunnerDraftState::Removed { edit: Some(edit) }
                    }
                    RunnerDraftState::Removed { edit: Some(edit) } => {
                        RunnerDraftState::Edited(edit)
                    }
                    RunnerDraftState::Removed { edit: None } => RunnerDraftState::Unchanged,
                };
            }
        }
        Ok(())
    }

    /// Report whether any agent row carries a staged change.
    #[must_use]
    pub fn runners_staged(&self) -> bool {
        self.runners
            .iter()
            .any(|row| RunnerDraftRow::marker(row).is_some())
    }

    /// Report whether cancelling the removal of one row would repeat a live name.
    fn restore_repeats_a_live_name(&self, index: usize) -> bool {
        self.runners.get(index).is_some_and(|row| {
            let RunnerDraftRow::Existing { row: stored, .. } = row else {
                return false;
            };
            row.is_removed()
                && row.name().is_some_and(|name| {
                    self.runner_name_taken(name, Some(index), stored.name.as_deref())
                })
        })
    }

    /// Report whether one live row other than the save's own target already uses `name`.
    ///
    /// `key` is the stable name the save rewrites. The store coalesces every raw row of that key
    /// into one, so a name stored twice does not collide with the save that repairs it.
    fn runner_name_taken(&self, name: &str, exclude: Option<usize>, key: Option<&str>) -> bool {
        self.runners.iter().enumerate().any(|(index, row)| {
            // A row the save removes frees its name, and the save removes before it adds.
            if Some(index) == exclude
                || row.is_removed()
                || runner_row_taken_by_its_key(&self.runners, index)
            {
                return false;
            }
            if let RunnerDraftRow::Existing { row: stored, .. } = row
                && key.is_some()
                && stored.name.as_deref() == key
            {
                return false;
            }
            row.name() == Some(name)
        })
    }

    fn runner_changes(&self) -> Vec<RunnerChange> {
        let removals = self.runners.iter().enumerate().filter_map(|(index, row)| {
            let RunnerDraftRow::Existing {
                row,
                state: RunnerDraftState::Removed { .. },
            } = row
            else {
                return None;
            };
            // The change on the key already takes this row, so a second removal would find
            // nothing and refuse the complete save.
            if runner_row_taken_by_its_key(&self.runners, index) {
                return None;
            }
            Some(if row.is_valid() {
                RunnerChange::RemoveNamed {
                    name: row.name.clone().unwrap_or_default(),
                    expected: row.key_identities.clone(),
                    expected_pinned_count: row.pinned_count,
                }
            } else {
                RunnerChange::RemoveRow {
                    expected: row.identity.clone(),
                }
            })
        });
        let edits = self.runners.iter().enumerate().filter_map(|(index, row)| {
            let RunnerDraftRow::Existing {
                row,
                state: RunnerDraftState::Edited(edit),
            } = row
            else {
                return None;
            };
            if runner_row_taken_by_its_key(&self.runners, index) {
                return None;
            }
            Some(if row.name.is_some() {
                RunnerChange::ReplaceNamed {
                    name: edit.name.clone(),
                    argv: edit.argv.clone(),
                    expected: row.key_identities.clone(),
                }
            } else {
                RunnerChange::RepairRow {
                    name: edit.name.clone(),
                    argv: edit.argv.clone(),
                    expected: row.identity.clone(),
                }
            })
        });
        let adds = self.runners.iter().filter_map(|row| {
            let RunnerDraftRow::Added { name, argv } = row else {
                return None;
            };
            Some(RunnerChange::Add {
                name: name.clone(),
                argv: argv.clone(),
            })
        });
        removals.chain(edits).chain(adds).collect()
    }

    /// Report whether the PyPI URL input is reachable.
    #[must_use]
    pub const fn custom_pypi_visible(&self) -> bool {
        matches!(self.pypi, MirrorChoice::Custom)
    }

    /// Report whether the GitHub URL input is reachable.
    #[must_use]
    pub const fn custom_github_visible(&self) -> bool {
        matches!(self.github, MirrorChoice::Custom)
    }

    /// Report whether the npm URL input is reachable.
    #[must_use]
    pub const fn custom_npm_visible(&self) -> bool {
        matches!(self.npm, MirrorChoice::Custom)
    }

    /// Report whether an editable value differs from its initial value.
    #[must_use]
    pub fn dirty(&self) -> bool {
        self.language != self.initial.language
            || self.editor != self.initial.editor
            || self.form != self.initial.form
            || self.after_run != self.initial.after_run
            || self.theme != self.initial.theme
            || self.accent != self.initial.accent
            || self.javascript != self.initial.javascript
            || self.bash_path != self.initial.bash_path
            || self.mirror_master != self.initial.mirror_master
            || self.pypi != self.initial.pypi
            || self.pypi_url != self.initial.pypi_url
            || self.github != self.initial.github
            || self.github_url != self.initial.github_url
            || self.npm != self.initial.npm
            || self.npm_url != self.initial.npm_url
            || self.runners_staged()
    }

    /// Validate every section before returning one atomic configuration transaction.
    pub fn resolve(
        &self,
        is_file: impl Fn(&Path) -> bool,
    ) -> Result<PreferencesChangeSet, PreferencesError> {
        let bash_path = self.bash_path.as_deref().map(str::trim);
        let pypi = resolve_axis(
            &self.pypi,
            &self.pypi_url,
            PYPI_PRESETS,
            PreferencesField::PypiMirror,
        )?;
        let npm = resolve_axis(
            &self.npm,
            &self.npm_url,
            NPM_PRESETS,
            PreferencesField::NpmMirror,
        )?;
        let github = self.resolve_github()?;

        let mut settings = BTreeMap::from([
            (
                "lang".to_owned(),
                if self.language == "auto" {
                    String::new()
                } else {
                    self.language.clone()
                },
            ),
            ("editor".to_owned(), self.editor.trim().to_owned()),
            ("form".to_owned(), self.form.as_str().to_owned()),
            ("after_run".to_owned(), self.after_run.as_str().to_owned()),
            ("js.runner".to_owned(), self.javascript.as_str().to_owned()),
        ]);
        if let Some(path) = bash_path {
            settings.insert("shell.bash_path".to_owned(), path.to_owned());
        }
        // Version 0.4 has no theme setting. A save writes it only when it changed, so a clean
        // save keeps the historical bytes of `config.toml`.
        if self.theme != self.initial.theme {
            settings.insert("theme".to_owned(), self.theme.as_str().to_owned());
        }
        if self.accent != self.initial.accent {
            settings.insert("accent".to_owned(), self.accent.as_str().to_owned());
        }

        let mirror_unchanged = self.mirror_master == self.initial.mirror_master
            && self.pypi == self.initial.pypi
            && self.pypi_url == self.initial.pypi_url
            && self.github == self.initial.github
            && self.github_url == self.initial.github_url
            && self.npm == self.initial.npm
            && self.npm_url == self.initial.npm_url;
        if github.passthrough && mirror_unchanged {
            let change = PreferencesChangeSet {
                settings,
                runners: self.runner_changes(),
            };
            change.validate_files(&is_file)?;
            return Ok(change);
        }

        settings.insert("mirror.pypi".to_owned(), pypi);
        settings.insert("mirror.npm".to_owned(), npm);
        if let Some(value) = github.setting {
            settings.insert("mirror.github".to_owned(), value);
        }
        let any_urls =
            settings["mirror.pypi"] != "off" || settings["mirror.npm"] != "off" || github.has_urls;
        settings.insert(
            "mirror".to_owned(),
            if self.mirror_master && any_urls {
                "on"
            } else {
                "off"
            }
            .to_owned(),
        );
        let change = PreferencesChangeSet {
            settings,
            runners: self.runner_changes(),
        };
        change.validate_files(is_file)?;
        Ok(change)
    }

    fn resolve_github(&self) -> Result<ResolvedGithub, PreferencesError> {
        match &self.github {
            MirrorChoice::Off => Ok(ResolvedGithub {
                setting: Some("off".to_owned()),
                has_urls: false,
                passthrough: false,
            }),
            MirrorChoice::Preset(name) => {
                let base = preset_value(name, GITHUB_PRESETS).ok_or(
                    PreferencesError::CustomUrlRequired {
                        field: PreferencesField::GithubMirror,
                    },
                )?;
                Ok(ResolvedGithub {
                    setting: Some(name.clone()),
                    has_urls: !base.is_empty(),
                    passthrough: false,
                })
            }
            MirrorChoice::Custom => {
                let base = self.github_url.trim();
                if base.is_empty()
                    && self.initial.github == MirrorChoice::Custom
                    && self.initial.github_url.is_empty()
                    && (!self.initial.mirror.python_install.is_empty()
                        || !self.initial.mirror.uv_binary.is_empty())
                {
                    return Ok(ResolvedGithub {
                        setting: None,
                        has_urls: true,
                        passthrough: true,
                    });
                }
                if !valid_url_token(base) {
                    return Err(PreferencesError::CustomUrlRequired {
                        field: PreferencesField::GithubMirror,
                    });
                }
                if !base.starts_with("https://") {
                    return Err(PreferencesError::GithubHttpsRequired {
                        url: base.to_owned(),
                    });
                }
                Ok(ResolvedGithub {
                    setting: Some(base.trim_end_matches('/').to_owned()),
                    has_urls: true,
                    passthrough: false,
                })
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ResolvedGithub {
    setting: Option<String>,
    has_urls: bool,
    passthrough: bool,
}

fn resolve_axis(
    choice: &MirrorChoice,
    custom: &str,
    presets: &[(&str, &str)],
    field: PreferencesField,
) -> Result<String, PreferencesError> {
    match choice {
        MirrorChoice::Off => Ok("off".to_owned()),
        MirrorChoice::Preset(name) if preset_value(name, presets).is_some() => Ok(name.clone()),
        MirrorChoice::Preset(_) => Err(PreferencesError::CustomUrlRequired { field }),
        MirrorChoice::Custom => {
            let value = custom.trim();
            if valid_url_token(value) {
                Ok(value.trim_end_matches('/').to_owned())
            } else {
                Err(PreferencesError::CustomUrlRequired { field })
            }
        }
    }
}

fn preset_value<'a>(name: &str, presets: &'a [(&str, &str)]) -> Option<&'a str> {
    presets
        .iter()
        .find_map(|(candidate, value)| (*candidate == name).then_some(*value))
}

fn valid_url_token(value: &str) -> bool {
    (value.starts_with("https://") || value.starts_with("http://"))
        && !value.chars().any(char::is_whitespace)
        && !value.contains('·')
}

fn axis_choice(value: &str, presets: &[(&str, &str)]) -> MirrorChoice {
    if value.is_empty() {
        MirrorChoice::Off
    } else {
        presets
            .iter()
            .find_map(|(name, url)| {
                (*url == value).then(|| MirrorChoice::Preset((*name).to_owned()))
            })
            .unwrap_or(MirrorChoice::Custom)
    }
}

fn github_choice(mirror: &MirrorConfiguration) -> MirrorChoice {
    if mirror.python_install.is_empty() && mirror.uv_binary.is_empty() {
        return MirrorChoice::Off;
    }
    GITHUB_PRESETS
        .iter()
        .find_map(|(name, base)| {
            (github_urls(base) == (mirror.python_install.clone(), mirror.uv_binary.clone()))
                .then(|| MirrorChoice::Preset((*name).to_owned()))
        })
        .unwrap_or(MirrorChoice::Custom)
}

fn github_base(mirror: &MirrorConfiguration) -> String {
    let Some(base) = mirror.uv_binary.strip_suffix("/astral-sh/uv") else {
        return String::new();
    };
    let pair = github_urls(base);
    if pair == (mirror.python_install.clone(), mirror.uv_binary.clone()) {
        base.to_owned()
    } else {
        String::new()
    }
}

fn github_urls(base: &str) -> (String, String) {
    let base = base.trim_end_matches('/');
    (
        format!("{base}/astral-sh/python-build-standalone/"),
        format!("{base}/astral-sh/uv"),
    )
}
