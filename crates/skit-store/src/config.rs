//! Preserve and update the user configuration file.

use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
};

use skit_application::runner_management::{RunnerArgvError, validate_runner_argv};
use skit_domain::EntryId;
use skit_i18n::{Locale, Localize, Message};
use thiserror::Error;
use toml::{Table, Value};

use crate::fs_ops::{acquire_lock, atomic_write_bytes};

const RUNNER_SEEDS: &[(&str, &[&str])] = &[
    ("claude", &["claude", "--", "{{prompt}}"]),
    ("codex", &["codex", "--", "{{prompt}}"]),
    ("opencode", &["opencode", "--prompt={{prompt}}"]),
    ("amp", &["amp", "-x", "{{prompt}}"]),
    (
        "antigravity",
        &["agy", "--prompt-interactive", "{{prompt}}"],
    ),
    ("copilot", &["copilot", "--interactive={{prompt}}"]),
    ("cursor", &["cursor-agent", "--", "agent", "{{prompt}}"]),
    ("pi", &["pi", "{{prompt}}"]),
];

const PYPI_PRESETS: &[(&str, &str)] = &[
    ("tsinghua", "https://pypi.tuna.tsinghua.edu.cn/simple"),
    ("aliyun", "https://mirrors.aliyun.com/pypi/simple"),
    ("ustc", "https://pypi.mirrors.ustc.edu.cn/simple"),
];
const GITHUB_PRESETS: &[(&str, &str)] = &[("nju", "https://mirror.nju.edu.cn/github-release")];
const NPM_PRESETS: &[(&str, &str)] = &[("npmmirror", "https://registry.npmmirror.com")];

/// Supported setting names in the v0.4 listing order.
pub const CONFIG_KEYS: [&str; 10] = [
    "lang",
    "editor",
    "mirror",
    "mirror.pypi",
    "mirror.github",
    "mirror.npm",
    "form",
    "after_run",
    "shell.bash_path",
    "js.runner",
];

/// Stored mirror axes and their master switch.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MirrorSettings {
    /// Apply stored mirror URLs to child processes.
    pub enabled: bool,
    /// Replacement Python package index.
    pub pypi: String,
    /// Python build download prefix for uv.
    pub python_install: String,
    /// uv binary download prefix.
    pub uv_binary: String,
    /// Replacement npm registry.
    pub npm: String,
}

/// Details for a malformed configuration that a write repaired.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigRecovery {
    /// Malformed file that the write replaced.
    pub path: PathBuf,
    /// Location of the byte-exact backup, or `None` when the backup failed.
    pub backup_path: Option<PathBuf>,
}

/// One configured prompt runner.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PromptRunner {
    /// Stable user-facing name.
    pub name: String,
    /// Direct process arguments. No shell parses these values.
    pub argv: Vec<String>,
}

/// One raw prompt-runner row for inspection and exact repair.
#[derive(Clone, Debug, PartialEq)]
pub struct PromptRunnerRow {
    /// Zero-based raw list index. A malformed enclosing value has no index.
    pub index: Option<usize>,
    /// Parsed runner name when present.
    pub name: Option<String>,
    /// Parsed argument vector when it has only strings.
    pub argv: Option<Vec<String>>,
    /// Validation reason. A valid row has no reason.
    pub reason: Option<String>,
    /// Stable display text for malformed shapes.
    pub descriptor: String,
    reason_message: Option<Message>,
    raw: Value,
}

impl PromptRunnerRow {
    /// Return the human row status in the selected locale.
    ///
    /// [`Self::reason`] stays the stable English machine token that `--json` and `doctor` report.
    /// This text is the human wording that version 0.4 shows in the same column
    /// (`src/skit/config.py:592-624` `prompt_runner_row_reason`).
    #[must_use]
    pub fn localized_reason(&self, locale: Locale) -> Option<String> {
        self.reason_message
            .as_ref()
            .map(|message| message.localize(locale))
    }

    /// Return the display label.
    ///
    /// Runner names and raw container paths are user data, so the locale does not change them.
    #[must_use]
    pub fn localized_descriptor(&self, _locale: Locale) -> String {
        self.descriptor.clone()
    }

    /// Return an opaque token for the complete raw management identity.
    ///
    /// The token includes the raw value, row/container address, and classification.
    /// Frontends can carry it without depending on TOML or inspecting future fields.
    #[must_use]
    pub fn snapshot_token(&self) -> String {
        serde_json::to_string(&serde_json::json!({
            "index": self.index,
            "reason": self.reason,
            "descriptor": self.descriptor,
            "raw": self.raw,
        }))
        .expect("a TOML management row is JSON serializable")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PromptRunnerIssue {
    PromptSectionNotTable,
    RunnersNotList,
    Empty,
    PromptSlotCount,
    PromptInBinary,
    StrayHole,
    Name,
    ArgvType,
    RowNotTable,
    Duplicate,
}

impl PromptRunnerIssue {
    const fn code(self) -> &'static str {
        match self {
            Self::PromptSectionNotTable => "prompt-section-not-table",
            Self::RunnersNotList => "runners-not-list",
            Self::Empty => "empty",
            Self::PromptSlotCount => "prompt-slot-count",
            Self::PromptInBinary => "prompt-in-binary",
            Self::StrayHole => "stray-hole",
            Self::Name => "name",
            Self::ArgvType => "argv-type",
            Self::RowNotTable => "row-not-table",
            Self::Duplicate => "duplicate",
        }
    }

    fn message(self) -> Message {
        match self {
            Self::PromptSectionNotTable => {
                Message::new("the prompt value is not a table; repair it before runner management")
            }
            Self::RunnersNotList => Message::new(
                "the prompt.runners value is not a list; repair it before runner management",
            ),
            Self::Empty => Message::new(
                "A runner needs a command — e.g. skit runner add mycli mycli run {{prompt}}",
            ),
            Self::PromptSlotCount => Message::new(
                "A runner command must contain the {{prompt}} slot exactly once — that's where the rendered prompt lands.",
            ),
            Self::PromptInBinary => Message::new(
                "{{prompt}} can't be the command itself — the first word must be the program to run.",
            ),
            Self::StrayHole => Message::new(
                "Runner commands take only the {{prompt}} slot — single-brace text is literal, and other {{holes}} aren't supported.",
            ),
            Self::Name => Message::new("A name is required."),
            Self::ArgvType => Message::new("a prompt runner argv must be a list of strings"),
            Self::RowNotTable => Message::new("the prompt runner row is not a table"),
            Self::Duplicate => Message::new("another row already uses this prompt runner name"),
        }
    }

    /// Return the human status wording that management surfaces show for one raw row.
    ///
    /// Version 0.4 keeps this set separate from the refusal wording above
    /// (`src/skit/config.py:592-624`): the machine token stays in [`Self::code`], and a human
    /// surface never shows it. The Ratatui management screen already uses these exact sentences,
    /// so the CLI table must show the same text.
    fn status_message(self) -> Message {
        match self {
            // Version 0.4 reuses the container error text here (`src/skit/config.py:603-606`).
            Self::PromptSectionNotTable | Self::RunnersNotList => self.message(),
            Self::Empty => Message::new("Type the agent's command, e.g. mycli run {{prompt}}"),
            Self::PromptSlotCount => Message::new(
                "The command needs the {{prompt}} slot exactly once — that's where the rendered prompt lands.",
            ),
            Self::PromptInBinary => Message::new(
                "{{prompt}} can't be the command itself — the first word must be the program to run.",
            ),
            Self::StrayHole => Message::new(
                "Runner commands take only the {{prompt}} slot — single-brace text is literal, and other {{holes}} aren't supported.",
            ),
            Self::Name => Message::new("A name is required."),
            Self::ArgvType => Message::new("The command must be a list of text arguments."),
            Self::RowNotTable => Message::new("This runner row isn't a table."),
            Self::Duplicate => Message::new("Another row already uses this runner name."),
        }
    }
}

/// Report a configuration read or transaction failure.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// A file operation failed.
    #[error("could not {operation} configuration at {path}: {reason}")]
    Io {
        /// Operation such as read, lock, or write.
        operation: &'static str,
        /// Affected path.
        path: String,
        /// Operating-system detail.
        reason: String,
    },
    /// Existing TOML is malformed and must not be overwritten.
    #[error("configuration at {path} is not valid TOML: {reason}")]
    Parse {
        /// Affected path.
        path: String,
        /// Parser detail.
        reason: String,
    },
    /// The updated document could not be encoded.
    #[error("could not encode configuration: {reason}")]
    Encode {
        /// Serializer detail.
        reason: String,
    },
    /// A key, value, or runner definition is invalid.
    #[error("{0}")]
    Invalid(Message),
    /// The caller supplied an unsupported key or value.
    #[error("{0}")]
    Usage(Message),
}

impl Localize for ConfigError {
    fn message(&self) -> Message {
        match self {
            Self::Io {
                operation,
                path,
                reason,
            } => Message::new("could not {} configuration at {}: {}")
                .nested(Message::term(operation))
                .with(path)
                .with(reason),
            Self::Parse { path, reason } => {
                Message::new("configuration at {} is not valid TOML: {}")
                    .with(path)
                    .with(reason)
            }
            Self::Encode { reason } => {
                Message::new("could not encode configuration: {}").with(reason)
            }
            Self::Invalid(message) => message.clone(),
            Self::Usage(message) => message.clone(),
        }
    }
}

impl ConfigError {
    /// Report whether changing command input can correct this error.
    #[must_use]
    pub const fn is_usage(&self) -> bool {
        matches!(self, Self::Usage(_))
    }
}

/// Filesystem-backed `config.toml` adapter.
#[derive(Clone, Debug)]
pub struct FileConfigStore {
    config_dir: PathBuf,
}

#[derive(Debug)]
struct LoadedConfig {
    original: Vec<u8>,
    original_text: Option<String>,
    document: Table,
    malformed: bool,
}

impl FileConfigStore {
    /// Use one platform-resolved skit configuration directory.
    #[must_use]
    pub fn new(config_dir: impl Into<PathBuf>) -> Self {
        Self {
            config_dir: config_dir.into(),
        }
    }

    /// Return the owned configuration root.
    #[must_use]
    pub fn config_dir(&self) -> &std::path::Path {
        &self.config_dir
    }

    /// Read every supported scalar setting with stable CLI key names.
    pub fn settings(&self) -> Result<BTreeMap<String, String>, ConfigError> {
        let document = self.load();
        let mut settings = BTreeMap::from([
            (
                "lang".to_owned(),
                read_key(&document, "lang")
                    .as_deref()
                    .filter(|value| !value.eq_ignore_ascii_case("auto"))
                    .and_then(normalize_supported_language)
                    .unwrap_or_default(),
            ),
            (
                "editor".to_owned(),
                read_key(&document, "editor").unwrap_or_default(),
            ),
            (
                "form".to_owned(),
                read_key(&document, "form")
                    .filter(|value| matches!(value.as_str(), "tui" | "plain"))
                    .unwrap_or_else(|| "tui".to_owned()),
            ),
            (
                "after_run".to_owned(),
                read_key(&document, "after_run")
                    .filter(|value| matches!(value.as_str(), "exit" | "stay"))
                    .unwrap_or_else(|| "exit".to_owned()),
            ),
            (
                "shell.bash_path".to_owned(),
                read_key(&document, "shell.bash_path").unwrap_or_default(),
            ),
            (
                "js.runner".to_owned(),
                read_key(&document, "js.runner")
                    .filter(|value| matches!(value.as_str(), "deno" | "bun" | "node"))
                    .unwrap_or_default(),
            ),
        ]);
        let mirror = mirror_from_document(&document);
        settings.insert(
            "mirror".to_owned(),
            if mirror.enabled && mirror_has_urls(&mirror) {
                "on"
            } else {
                "off"
            }
            .to_owned(),
        );
        settings.insert(
            "mirror.pypi".to_owned(),
            axis_display(&mirror.pypi, PYPI_PRESETS),
        );
        settings.insert("mirror.github".to_owned(), github_display(&mirror));
        settings.insert(
            "mirror.npm".to_owned(),
            axis_display(&mirror.npm, NPM_PRESETS),
        );
        Ok(settings)
    }

    /// Read one supported scalar setting.
    pub fn get(&self, key: &str) -> Result<String, ConfigError> {
        self.settings()?
            .remove(key)
            .ok_or_else(|| unknown_setting_error(key))
    }

    /// Set one supported scalar setting without changing unknown TOML fields.
    pub fn set(&self, key: &str, value: &str) -> Result<(), ConfigError> {
        self.set_with_recovery(key, value).map(drop)
    }

    /// Set one scalar setting and report a malformed-file recovery to the frontend.
    pub fn set_with_recovery(
        &self,
        key: &str,
        value: &str,
    ) -> Result<Option<ConfigRecovery>, ConfigError> {
        let value = normalize_setting(key, value)?;
        self.update_with_recovery(|document| write_key(document, key, &value))
            .map(|(_, recovery)| recovery)
    }

    /// Validate and replace multiple settings in one configuration transaction.
    pub fn set_many(&self, settings: &BTreeMap<String, String>) -> Result<(), ConfigError> {
        self.set_many_with_recovery(settings).map(drop)
    }

    /// Replace multiple settings and report a malformed-file recovery to the frontend.
    pub fn set_many_with_recovery(
        &self,
        settings: &BTreeMap<String, String>,
    ) -> Result<Option<ConfigRecovery>, ConfigError> {
        let settings = settings
            .iter()
            .map(|(key, value)| Ok((key.clone(), normalize_setting(key, value)?)))
            .collect::<Result<BTreeMap<_, _>, ConfigError>>()?;
        self.update_with_recovery(|document| {
            for (key, value) in settings.iter().filter(|(key, _)| key.as_str() != "mirror") {
                write_key(document, key, value)?;
            }
            if let Some(value) = settings.get("mirror") {
                write_key(document, "mirror", value)?;
            }
            Ok(())
        })
        .map(|(_, recovery)| recovery)
    }

    /// Read stored mirror URLs without applying the master switch.
    pub fn mirror(&self) -> Result<MirrorSettings, ConfigError> {
        Ok(mirror_from_document(&self.load()))
    }

    /// Report whether a mirror section has ever been written.
    ///
    /// This is the first-run marker, and it is deliberately not "the configuration file exists":
    /// setting a language also writes the file, and that must not suppress the mirror offer
    /// (`src/skit/config.py:178-183`).
    pub fn mirror_configured(&self) -> Result<bool, ConfigError> {
        Ok(self.load().contains_key("mirror"))
    }

    /// Write the current mirror settings back so the first-run offer never repeats.
    ///
    /// Version 0.4 persists `save_mirror(load_mirror())` after the offer, whatever the user chose,
    /// so the probe happens once (`src/skit/cli.py:5617-5618`).
    pub fn mark_mirror_configured(&self) -> Result<(), ConfigError> {
        self.update_with_recovery(|document| {
            let stored = mirror_from_document(document);
            let table = repairable_table_mut(document, "mirror");
            table.insert("enabled".to_owned(), Value::Boolean(stored.enabled));
            for (key, url) in [
                ("pypi", &stored.pypi),
                ("python_install", &stored.python_install),
                ("uv_binary", &stored.uv_binary),
                ("npm", &stored.npm),
            ] {
                if url.is_empty() {
                    table.remove(key);
                } else {
                    table.insert(key.to_owned(), Value::String(url.clone()));
                }
            }
            Ok(())
        })
        .map(drop)
    }

    /// Build environment values for a child without changing the parent environment.
    pub fn mirror_environment(
        &self,
        base: &BTreeMap<String, String>,
    ) -> Result<BTreeMap<String, String>, ConfigError> {
        let mirror = self.mirror()?;
        let mut output = BTreeMap::new();
        if !mirror.enabled {
            return Ok(output);
        }
        let has_value = |key| base.get(key).is_some_and(|value| !value.is_empty());
        if !mirror.pypi.is_empty() && !has_value("UV_DEFAULT_INDEX") && !has_value("UV_INDEX_URL") {
            output.insert("UV_DEFAULT_INDEX".to_owned(), mirror.pypi);
        }
        if !mirror.python_install.is_empty() && !has_value("UV_PYTHON_INSTALL_MIRROR") {
            output.insert("UV_PYTHON_INSTALL_MIRROR".to_owned(), mirror.python_install);
        }
        if !mirror.npm.is_empty()
            && !has_value("NPM_CONFIG_REGISTRY")
            && !has_value("npm_config_registry")
        {
            output.insert("NPM_CONFIG_REGISTRY".to_owned(), mirror.npm);
        }
        Ok(output)
    }

    /// Read valid prompt runners. The built-in rows are defaults until customized.
    pub fn runners(&self) -> Result<Vec<PromptRunner>, ConfigError> {
        let document = self.load();
        Ok(runner_rows_from_document(&document)
            .into_iter()
            .filter(|row| row.reason.is_none())
            .filter_map(|row| {
                Some(PromptRunner {
                    name: row.name?,
                    argv: row.argv?,
                })
            })
            .collect())
    }

    /// Materialize the default runner rows on an explicit management read.
    pub fn ensure_runners_seeded(&self) -> Result<(), ConfigError> {
        self.update(materialize_seed_runners)
    }

    /// Return each raw runner row, including malformed future shapes.
    pub fn runner_rows(&self) -> Result<Vec<PromptRunnerRow>, ConfigError> {
        let document = self.load();
        Ok(runner_rows_from_document(&document))
    }

    /// Return labels for malformed prompt runner rows that normal reads ignore.
    pub fn invalid_runner_rows(&self) -> Result<Vec<String>, ConfigError> {
        Ok(self
            .runner_rows()?
            .into_iter()
            .filter(|row| row.reason.is_some())
            .map(|row| row.descriptor)
            .collect())
    }

    /// Add or replace one named prompt runner.
    pub fn set_runner(&self, runner: PromptRunner, replace: bool) -> Result<bool, ConfigError> {
        validate_runner(&runner).map_err(|issue| ConfigError::Usage(issue.message()))?;
        let runner = PromptRunner {
            name: runner.name.trim().to_owned(),
            argv: runner.argv,
        };
        self.update(|document| {
            materialize_seed_runners(document)?;
            let rows = runner_array_mut(document)?;
            let existing = rows
                .iter()
                .enumerate()
                .filter_map(|(index, value)| {
                    (raw_runner_name(value) == runner.name).then_some(index)
                })
                .collect::<Vec<_>>();
            if let Some(&first) = existing.first() {
                if !replace {
                    return Err(ConfigError::Invalid(
                        Message::new(
                            "The runner {} already exists — pass --force to replace its command.",
                        )
                        .with(&runner.name),
                    ));
                }
                let row = rows[first]
                    .as_table_mut()
                    .expect("a matched runner row is a table");
                write_runner_fields(row, &runner);
                for index in existing.into_iter().skip(1).rev() {
                    rows.remove(index);
                }
                Ok(true)
            } else {
                rows.push(Value::Table(runner_table(&runner)));
                Ok(false)
            }
        })
    }

    /// Replace one stable runner key only while all of its raw rows match a prior read.
    ///
    /// Unrelated configuration edits do not block the transaction. Duplicate rows for
    /// the selected key are coalesced into the first row after a successful comparison.
    pub fn set_runner_if_unchanged(
        &self,
        runner: PromptRunner,
        expected: &[PromptRunnerRow],
    ) -> Result<bool, ConfigError> {
        validate_runner(&runner).map_err(|issue| ConfigError::Usage(issue.message()))?;
        let runner = PromptRunner {
            name: runner.name.trim().to_owned(),
            argv: runner.argv,
        };
        self.update(|document| {
            materialize_seed_runners(document)?;
            let rows = runner_array_mut(document)?;
            let matches = rows
                .iter()
                .enumerate()
                .filter_map(|(index, value)| {
                    (raw_runner_name(value) == runner.name).then_some(index)
                })
                .collect::<Vec<_>>();
            let current = matches
                .iter()
                .map(|index| rows[*index].clone())
                .collect::<Vec<_>>();
            let expected = expected
                .iter()
                .filter(|row| row.name.as_deref() == Some(runner.name.as_str()))
                .map(|row| row.raw.clone())
                .collect::<Vec<_>>();
            if matches.is_empty() || current != expected {
                return Ok(false);
            }
            let first = matches[0];
            // A matching raw runner name can only come from a table row.
            let mut replacement = rows[first].as_table().cloned().unwrap_or_default();
            write_runner_fields(&mut replacement, &runner);
            rows[first] = Value::Table(replacement);
            for index in matches.into_iter().skip(1).rev() {
                rows.remove(index);
            }
            Ok(true)
        })
    }

    /// Repair one recognizable raw row only while its complete snapshot is unchanged.
    pub fn replace_runner_row_if_unchanged(
        &self,
        runner: PromptRunner,
        expected: &PromptRunnerRow,
    ) -> Result<bool, ConfigError> {
        validate_runner(&runner).map_err(|issue| ConfigError::Usage(issue.message()))?;
        let runner = PromptRunner {
            name: runner.name.trim().to_owned(),
            argv: runner.argv,
        };
        self.update(|document| {
            let Some(index) = expected.index else {
                return Ok(false);
            };
            let Some(rows) = explicit_runner_rows_mut(document) else {
                return Ok(false);
            };
            if rows.get(index) != Some(&expected.raw) {
                return Ok(false);
            }
            if rows
                .iter()
                .enumerate()
                .any(|(current, value)| current != index && raw_runner_name(value) == runner.name)
            {
                return Err(ConfigError::Invalid(
                    Message::new("The runner {} already exists — pick another name.")
                        .with(&runner.name),
                ));
            }
            rows[index] = Value::Table(runner_table(&runner));
            mark_runners_seeded(document)?;
            Ok(true)
        })
    }

    /// Remove one named prompt runner and report whether it existed.
    pub fn remove_runner(&self, name: &str) -> Result<bool, ConfigError> {
        self.remove_runner_with_snapshot(name, None)
    }

    /// Remove one named runner only when its complete raw rows match a management read.
    pub fn remove_runner_if_unchanged(
        &self,
        name: &str,
        expected: &[PromptRunnerRow],
    ) -> Result<bool, ConfigError> {
        self.remove_runner_with_snapshot(name, Some(expected))
    }

    fn remove_runner_with_snapshot(
        &self,
        name: &str,
        expected: Option<&[PromptRunnerRow]>,
    ) -> Result<bool, ConfigError> {
        let name = name.trim();
        if name.is_empty() {
            return Ok(false);
        }
        self.update(|document| {
            materialize_seed_runners(document)?;
            let rows = runner_array_mut(document)?;
            let current = rows
                .iter()
                .filter(|value| raw_runner_name(value) == name)
                .cloned()
                .collect::<Vec<_>>();
            if let Some(expected) = expected {
                let expected = expected
                    .iter()
                    .filter(|row| row.name.as_deref() == Some(name))
                    .map(|row| row.raw.clone())
                    .collect::<Vec<_>>();
                if current != expected {
                    return Ok(false);
                }
            }
            let before = rows.len();
            rows.retain(|value| raw_runner_name(value) != name);
            Ok(rows.len() != before)
        })
    }

    /// Remove one raw zero-based row without parsing or normalizing other rows.
    pub fn remove_runner_row(&self, row: usize) -> Result<bool, ConfigError> {
        self.update(|document| {
            // `--row` addresses the raw rows that `runner_rows` reported, so seeding
            // here would renumber them under the user.
            let Some(rows) = explicit_runner_rows_mut(document) else {
                return Ok(false);
            };
            if row >= rows.len() {
                return Ok(false);
            }
            rows.remove(row);
            Ok(true)
        })
    }

    /// Remove one raw row or malformed container only if its raw snapshot is unchanged.
    pub fn remove_runner_row_if_unchanged(
        &self,
        expected: &PromptRunnerRow,
    ) -> Result<bool, ConfigError> {
        self.update(|document| match expected.index {
            Some(index) => {
                let Some(rows) = explicit_runner_rows_mut(document) else {
                    return Ok(false);
                };
                if rows.get(index) != Some(&expected.raw) {
                    return Ok(false);
                }
                rows.remove(index);
                mark_runners_seeded(document)?;
                Ok(true)
            }
            None => remove_malformed_runner_container(document, expected),
        })
    }

    fn path(&self) -> PathBuf {
        self.config_dir.join("config.toml")
    }

    fn lock_path(&self) -> PathBuf {
        self.config_dir.join("config.lock")
    }

    fn load(&self) -> Table {
        self.load_document()
            .map_or_else(|_| Table::new(), |loaded| loaded.document)
    }

    fn load_document(&self) -> io::Result<LoadedConfig> {
        let path = self.path();
        match fs::read(&path) {
            Ok(original) => {
                let original_text = String::from_utf8(original.clone()).ok();
                let parsed = original_text
                    .as_deref()
                    .and_then(|text| toml::from_str(text).ok());
                let malformed = parsed.is_none();
                Ok(LoadedConfig {
                    original,
                    original_text,
                    document: parsed.unwrap_or_default(),
                    malformed,
                })
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(LoadedConfig {
                original: Vec::new(),
                original_text: Some(String::new()),
                document: Table::new(),
                malformed: false,
            }),
            Err(error) => Err(error),
        }
    }

    fn update<T>(
        &self,
        operation: impl FnOnce(&mut Table) -> Result<T, ConfigError>,
    ) -> Result<T, ConfigError> {
        self.update_with_recovery(operation)
            .map(|(result, _)| result)
    }

    fn update_with_recovery<T>(
        &self,
        operation: impl FnOnce(&mut Table) -> Result<T, ConfigError>,
    ) -> Result<(T, Option<ConfigRecovery>), ConfigError> {
        let lock_path = self.lock_path();
        let _lock =
            acquire_lock(&lock_path).map_err(|error| io_error("lock", &lock_path, error))?;
        let path = self.path();
        let mut loaded = self
            .load_document()
            .map_err(|error| io_error("read", &path, error))?;
        let before = loaded.document.clone();
        let result = operation(&mut loaded.document)?;
        if loaded.document == before && !loaded.malformed {
            return Ok((result, None));
        }
        let desired = toml::to_string_pretty(&loaded.document)
            .expect("a configuration table contains only TOML values");
        let encoded = if loaded.malformed {
            desired.into_bytes()
        } else {
            crate::toml_document::merge_update(
                loaded
                    .original_text
                    .as_deref()
                    .expect("a valid TOML document is UTF-8"),
                &desired,
                &before,
                &loaded.document,
            )
            .map_err(|reason| ConfigError::Encode { reason })?
            .into_bytes()
        };
        let recovery = loaded.malformed.then(|| ConfigRecovery {
            path: path.clone(),
            backup_path: preserve_corrupt_backup(&path, &loaded.original).ok(),
        });
        atomic_write_bytes(&path, &encoded).map_err(|error| io_error("write", &path, error))?;
        Ok((result, recovery))
    }
}

fn normalize_setting(key: &str, value: &str) -> Result<String, ConfigError> {
    match key {
        "lang" if value.is_empty() => Ok(String::new()),
        "lang" if value.trim().eq_ignore_ascii_case("auto") => Ok(String::new()),
        "lang" => normalize_supported_language(value).ok_or_else(|| {
            ConfigError::Usage(
                Message::new("Unknown language: {}. Available: {}")
                    .with(value)
                    .with(skit_i18n::available_locale_tags().join(", ")),
            )
        }),
        "editor" => Ok(value.trim().to_owned()),
        "form" if matches!(value, "tui" | "plain") => Ok(value.to_owned()),
        "form" => Err(ConfigError::Usage(
            Message::new("Unknown form style: {}. Choose from: tui, plain").with(value),
        )),
        "after_run" if matches!(value, "exit" | "stay") => Ok(value.to_owned()),
        "after_run" => Err(ConfigError::Usage(
            Message::new("Unknown after-run behavior: {}. Choose from: exit, stay").with(value),
        )),
        "shell.bash_path" => Ok(value.trim().to_owned()),
        "js.runner" if value.trim().is_empty() => Ok(String::new()),
        "js.runner" if matches!(value, "deno" | "bun" | "node") => Ok(value.to_owned()),
        "js.runner" => Err(ConfigError::Usage(
            Message::new("Unknown JS runner: {}. Choose from: {}")
                .with(value)
                .with("deno, bun, node"),
        )),
        "mirror" if matches!(value, "on" | "off") => Ok(value.to_owned()),
        "mirror" => Err(ConfigError::Usage(
            Message::new(
                "Unknown mirror value: {}. \"mirror\" is the master switch (on / off); mirrors are picked per ecosystem: mirror.pypi ({}), mirror.github ({}), mirror.npm ({}) — each also takes a URL or \"off\".",
            )
            .with(value)
            .with(preset_names(PYPI_PRESETS))
            .with(preset_names(GITHUB_PRESETS))
            .with(preset_names(NPM_PRESETS)),
        )),
        "mirror.pypi" | "mirror.npm" => {
            let presets = if key == "mirror.pypi" {
                PYPI_PRESETS
            } else {
                NPM_PRESETS
            };
            if valid_axis_value(value, presets, false) {
                Ok(value.to_owned())
            } else {
                Err(ConfigError::Usage(
                    Message::new("Unknown {} value: {}. Choose from: {}, off — or give a full URL.")
                        .with(key)
                        .with(value)
                        .with(preset_names(presets)),
                ))
            }
        }
        "mirror.github" if valid_axis_value(value, GITHUB_PRESETS, true) => Ok(value.to_owned()),
        "mirror.github" => Err(ConfigError::Usage(
            Message::new(
                "Unknown mirror.github value: {}. Choose from: {}, off — or give an https:// github-release base URL (the uv binary is downloaded and executed, so https:// is required).",
            )
            .with(value)
            .with(preset_names(GITHUB_PRESETS)),
        )),
        _ => Err(unknown_setting_error(key)),
    }
}

fn preset_names(presets: &[(&str, &str)]) -> String {
    presets
        .iter()
        .map(|(name, _)| *name)
        .collect::<Vec<_>>()
        .join(", ")
}

fn unknown_setting_error(key: &str) -> ConfigError {
    ConfigError::Usage(
        Message::new("Unknown setting: {}. Available: {}")
            .with(key)
            .with(CONFIG_KEYS.join(", ")),
    )
}

fn normalize_supported_language(value: &str) -> Option<String> {
    let without_codeset = value.split('.').next().unwrap_or_default();
    let without_modifier = without_codeset.split('@').next().unwrap_or_default();
    let normalized_input = without_modifier.trim().replace('_', "-");
    if normalized_input.is_empty() {
        return None;
    }
    let mut parts = normalized_input.split('-');
    let language = parts.next()?.to_lowercase();
    let mut output = vec![language.clone()];
    for part in parts {
        let characters = part.chars().count();
        let canonical = if characters == 2 {
            part.to_uppercase()
        } else if characters == 4 {
            let mut chars = part.chars();
            let first = chars.next()?.to_uppercase().collect::<String>();
            format!("{first}{}", chars.as_str().to_lowercase())
        } else {
            part.to_lowercase()
        };
        output.push(canonical);
    }
    let normalized = output.join("-");
    (matches!(language.as_str(), "en" | "zh") || normalized.eq_ignore_ascii_case("x-pseudo"))
        .then_some(normalized)
}

fn read_key(document: &Table, key: &str) -> Option<String> {
    let storage_key = if key == "lang" { "language" } else { key };
    let mut parts = storage_key.split('.');
    let first = parts.next()?;
    let second = parts.next();
    match second {
        None => document
            .get(first)
            .and_then(Value::as_str)
            .map(str::to_owned),
        Some(second) => document
            .get(first)
            .and_then(Value::as_table)
            .and_then(|table| table.get(second))
            .and_then(Value::as_str)
            .map(str::to_owned),
    }
}

fn write_key(document: &mut Table, key: &str, value: &str) -> Result<(), ConfigError> {
    if key == "mirror" {
        let table = repairable_table_mut(document, "mirror");
        if value == "on" && !table_has_urls(table) {
            return Err(ConfigError::Usage(Message::new(
                "Nothing to enable: no mirror URLs are saved. Set an axis first: mirror.pypi / mirror.github / mirror.npm.",
            )));
        }
        table.insert("enabled".to_owned(), Value::Boolean(value == "on"));
        return Ok(());
    }
    if matches!(key, "mirror.pypi" | "mirror.github" | "mirror.npm") {
        let table = repairable_table_mut(document, "mirror");
        let was_paused = !table
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(false)
            && table_has_urls(table);
        if key == "mirror.pypi" {
            let url = resolve_axis(value, PYPI_PRESETS);
            table.insert("pypi".to_owned(), Value::String(url));
        } else if key == "mirror.npm" {
            let url = resolve_axis(value, NPM_PRESETS);
            table.insert("npm".to_owned(), Value::String(url));
        } else {
            let base = resolve_axis(value, GITHUB_PRESETS);
            let (python, uv) = github_urls(&base);
            table.insert("python_install".to_owned(), Value::String(python));
            table.insert("uv_binary".to_owned(), Value::String(uv));
            table.remove("github");
        }
        table.insert(
            "enabled".to_owned(),
            Value::Boolean(!was_paused && table_has_urls(table)),
        );
        return Ok(());
    }
    if matches!(key, "lang" | "editor") {
        let storage_key = if key == "lang" { "language" } else { key };
        if value.is_empty() {
            document.remove(storage_key);
        } else {
            document.insert(storage_key.to_owned(), Value::String(value.to_owned()));
        }
        return Ok(());
    }
    let storage_key = if key == "lang" { "language" } else { key };
    let mut parts = storage_key.split('.');
    let first = parts.next().expect("validated keys are not empty");
    if let Some(second) = parts.next() {
        write_nested_string(document, first, second, value);
    } else {
        document.insert(first.to_owned(), Value::String(value.to_owned()));
    }
    Ok(())
}

fn write_nested_string(document: &mut Table, section: &str, key: &str, value: &str) {
    if value.is_empty() {
        if let Some(table) = document.get_mut(section).and_then(Value::as_table_mut) {
            table.remove(key);
            if table.is_empty() {
                document.remove(section);
            }
        } else {
            document.remove(section);
        }
        return;
    }
    repairable_table_mut(document, section).insert(key.to_owned(), Value::String(value.to_owned()));
}

fn repairable_table_mut<'a>(document: &'a mut Table, key: &str) -> &'a mut Table {
    if document.get(key).is_none_or(|value| !value.is_table()) {
        document.insert(key.to_owned(), Value::Table(Table::new()));
    }
    document
        .get_mut(key)
        .and_then(Value::as_table_mut)
        .expect("the setting section was repaired as a table")
}

fn valid_axis_value(value: &str, presets: &[(&str, &str)], https_only: bool) -> bool {
    value == "off" || presets.iter().any(|(name, _)| *name == value) || valid_url(value, https_only)
}

fn valid_url(value: &str, https_only: bool) -> bool {
    let scheme = if https_only {
        value.starts_with("https://")
    } else {
        value.starts_with("https://") || value.starts_with("http://")
    };
    scheme && !value.chars().any(char::is_whitespace) && !value.contains('·')
}

fn resolve_axis(value: &str, presets: &[(&str, &str)]) -> String {
    if value == "off" {
        String::new()
    } else {
        presets
            .iter()
            .find_map(|(name, url)| (*name == value).then(|| (*url).to_owned()))
            .unwrap_or_else(|| value.trim_end_matches('/').to_owned())
    }
}

fn mirror_from_document(document: &Table) -> MirrorSettings {
    let table = document.get("mirror").and_then(Value::as_table);
    let string = |key: &str| {
        table
            .and_then(|value| value.get(key))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let uv_binary = string("uv_binary");
    MirrorSettings {
        enabled: table
            .and_then(|value| value.get("enabled"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        pypi: string("pypi"),
        python_install: string("python_install"),
        uv_binary: if uv_binary.starts_with("https://") {
            uv_binary
        } else {
            String::new()
        },
        npm: string("npm"),
    }
}

fn mirror_has_urls(mirror: &MirrorSettings) -> bool {
    !mirror.pypi.is_empty()
        || !mirror.python_install.is_empty()
        || !mirror.uv_binary.is_empty()
        || !mirror.npm.is_empty()
}

fn table_has_urls(table: &Table) -> bool {
    ["pypi", "python_install", "uv_binary", "npm"]
        .into_iter()
        .any(|key| {
            table
                .get(key)
                .and_then(Value::as_str)
                .is_some_and(|value| !value.is_empty())
        })
}

fn axis_display(value: &str, presets: &[(&str, &str)]) -> String {
    if value.is_empty() {
        "off".to_owned()
    } else {
        presets
            .iter()
            .find_map(|(name, url)| (*url == value).then(|| (*name).to_owned()))
            .unwrap_or_else(|| value.to_owned())
    }
}

fn github_urls(base: &str) -> (String, String) {
    if base.is_empty() {
        return (String::new(), String::new());
    }
    let base = base.trim_end_matches('/');
    (
        format!("{base}/astral-sh/python-build-standalone/"),
        format!("{base}/astral-sh/uv"),
    )
}

fn github_display(mirror: &MirrorSettings) -> String {
    if mirror.python_install.is_empty() && mirror.uv_binary.is_empty() {
        return "off".to_owned();
    }
    for (name, base) in GITHUB_PRESETS {
        let (python, uv) = github_urls(base);
        if mirror.python_install == python && mirror.uv_binary == uv {
            return (*name).to_owned();
        }
    }
    let suffix = "/astral-sh/python-build-standalone/";
    if let Some(base) = mirror.python_install.strip_suffix(suffix) {
        let (_, uv) = github_urls(base);
        if mirror.uv_binary == uv {
            return base.to_owned();
        }
    }
    "custom".to_owned()
}

fn table_mut<'a>(document: &'a mut Table, key: &str) -> Result<&'a mut Table, ConfigError> {
    if !document.contains_key(key) {
        document.insert(key.to_owned(), Value::Table(Table::new()));
    }
    document
        .get_mut(key)
        .and_then(Value::as_table_mut)
        .ok_or_else(|| {
            ConfigError::Invalid(Message::new("configuration section is not a table: {}").with(key))
        })
}

fn seed_runners() -> Vec<PromptRunner> {
    RUNNER_SEEDS
        .iter()
        .map(|(name, argv)| PromptRunner {
            name: (*name).to_owned(),
            argv: argv.iter().map(|value| (*value).to_owned()).collect(),
        })
        .collect()
}

fn runners_are_configured(document: &Table) -> bool {
    let Some(prompt) = document.get("prompt") else {
        return false;
    };
    let Some(prompt) = prompt.as_table() else {
        return true;
    };
    prompt.contains_key("runners")
        || prompt
            .get("runners_seeded")
            .and_then(Value::as_bool)
            .unwrap_or(false)
}

/// Return the stored runner rows, or `None` when the file declares no list.
fn explicit_runner_rows_mut(document: &mut Table) -> Option<&mut Vec<Value>> {
    document
        .get_mut("prompt")?
        .as_table_mut()?
        .get_mut("runners")?
        .as_array_mut()
}

fn runner_array_mut(document: &mut Table) -> Result<&mut Vec<Value>, ConfigError> {
    if !document.contains_key("prompt") {
        document.insert("prompt".to_owned(), Value::Table(Table::new()));
    }
    let prompt = document
        .get_mut("prompt")
        .and_then(Value::as_table_mut)
        .ok_or_else(|| ConfigError::Invalid(PromptRunnerIssue::PromptSectionNotTable.message()))?;
    if !prompt.contains_key("runners") {
        prompt.insert("runners".to_owned(), Value::Array(Vec::new()));
    }
    prompt
        .get_mut("runners")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| ConfigError::Invalid(PromptRunnerIssue::RunnersNotList.message()))
}

fn runner_rows_from_document(document: &Table) -> Vec<PromptRunnerRow> {
    if !runners_are_configured(document) {
        return seed_runners()
            .into_iter()
            .enumerate()
            .map(|(index, runner)| {
                let raw = Value::Table(runner_table(&runner));
                PromptRunnerRow {
                    index: Some(index),
                    name: Some(runner.name.clone()),
                    argv: Some(runner.argv),
                    reason: None,
                    descriptor: runner.name,
                    reason_message: None,
                    raw,
                }
            })
            .collect();
    }

    // `runners_are_configured` returns true only when the prompt key exists.
    let prompt_value = &document["prompt"];
    let Some(prompt) = prompt_value.as_table() else {
        return vec![container_runner_row(
            PromptRunnerIssue::PromptSectionNotTable,
            "prompt",
            prompt_value,
        )];
    };
    let Some(rows_value) = prompt.get("runners") else {
        return Vec::new();
    };
    let Some(rows) = rows_value.as_array() else {
        return vec![container_runner_row(
            PromptRunnerIssue::RunnersNotList,
            "prompt.runners",
            rows_value,
        )];
    };

    let mut seen = std::collections::BTreeSet::new();
    rows.iter()
        .enumerate()
        .map(|(index, value)| runner_row(index, value, &mut seen))
        .collect()
}

fn container_runner_row(
    issue: PromptRunnerIssue,
    descriptor: &str,
    value: &Value,
) -> PromptRunnerRow {
    PromptRunnerRow {
        index: None,
        name: None,
        argv: None,
        reason: Some(issue.code().to_owned()),
        descriptor: descriptor.to_owned(),
        reason_message: Some(issue.status_message()),
        raw: value.clone(),
    }
}

fn runner_row(
    index: usize,
    value: &Value,
    seen: &mut std::collections::BTreeSet<String>,
) -> PromptRunnerRow {
    let table = value.as_table();
    let raw_name = table
        .and_then(|row| row.get("name"))
        .and_then(Value::as_str);
    let name = raw_name.map(str::trim).map(str::to_owned);
    let valid_name = name.as_ref().filter(|name| !name.is_empty());
    let argv = table
        .and_then(|row| row.get("argv"))
        .and_then(Value::as_array)
        .and_then(|items| {
            items
                .iter()
                .map(Value::as_str)
                .map(|value| value.map(str::to_owned))
                .collect::<Option<Vec<_>>>()
        });
    let mut issue = if table.is_none() {
        Some(PromptRunnerIssue::RowNotTable)
    } else if valid_name.is_none() {
        Some(PromptRunnerIssue::Name)
    } else if argv.is_none() {
        Some(PromptRunnerIssue::ArgvType)
    } else {
        validate_runner(&PromptRunner {
            name: valid_name.expect("the name was checked").clone(),
            argv: argv.clone().expect("argv was checked"),
        })
        .err()
    };
    if issue.is_none() {
        let normalized = valid_name.expect("a valid row has a name");
        if !seen.insert(normalized.clone()) {
            issue = Some(PromptRunnerIssue::Duplicate);
        }
    }
    let descriptor = if issue == Some(PromptRunnerIssue::Duplicate) {
        valid_name.expect("a duplicate row has a name").clone()
    } else {
        raw_name
            .filter(|name| !name.trim().is_empty())
            .map_or_else(|| legacy_value_descriptor(value), str::to_owned)
    };
    let reason_message = issue.map(PromptRunnerIssue::status_message);
    PromptRunnerRow {
        index: Some(index),
        name,
        argv,
        reason: issue.map(|issue| issue.code().to_owned()),
        descriptor,
        reason_message,
        raw: value.clone(),
    }
}

fn legacy_value_descriptor(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Integer(value) => value.to_string(),
        Value::Float(value) => value.to_string(),
        Value::Boolean(value) => if *value { "True" } else { "False" }.to_owned(),
        Value::Datetime(value) => value.to_string(),
        Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(legacy_nested_value)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Value::Table(values) => format!(
            "{{{}}}",
            values
                .iter()
                .map(|(key, value)| format!(
                    "{}: {}",
                    python_string(key),
                    legacy_nested_value(value)
                ))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn legacy_nested_value(value: &Value) -> String {
    match value {
        Value::String(value) => python_string(value),
        _ => legacy_value_descriptor(value),
    }
}

fn python_string(value: &str) -> String {
    let mut output = String::with_capacity(value.len() + 2);
    output.push('\'');
    for character in value.chars() {
        match character {
            '\\' => output.push_str("\\\\"),
            '\'' => output.push_str("\\'"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            character if character.is_control() => {
                use std::fmt::Write as _;
                let _ = write!(output, "\\u{:04x}", character as u32);
            }
            character => output.push(character),
        }
    }
    output.push('\'');
    output
}

fn runner_table(runner: &PromptRunner) -> Table {
    let mut table = Table::new();
    write_runner_fields(&mut table, runner);
    table
}

fn write_runner_fields(row: &mut Table, runner: &PromptRunner) {
    row.insert("name".to_owned(), Value::String(runner.name.clone()));
    row.insert(
        "argv".to_owned(),
        Value::Array(runner.argv.iter().cloned().map(Value::String).collect()),
    );
}

fn raw_runner_name(value: &Value) -> &str {
    value
        .as_table()
        .and_then(|row| row.get("name"))
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
}

/// Record the default runner rows once, keeping any rows the user already wrote.
///
/// A hand-written `[[prompt.runners]]` list is authoritative. Seeding only adds the
/// defaults when the file has no list at all.
fn materialize_seed_runners(document: &mut Table) -> Result<(), ConfigError> {
    if runners_are_configured(document) {
        return Ok(());
    }
    let prompt = table_mut(document, "prompt")?;
    prompt.entry("runners".to_owned()).or_insert_with(|| {
        Value::Array(
            seed_runners()
                .iter()
                .map(|runner| Value::Table(runner_table(runner)))
                .collect(),
        )
    });
    prompt.insert("runners_seeded".to_owned(), Value::Boolean(true));
    Ok(())
}

fn mark_runners_seeded(document: &mut Table) -> Result<(), ConfigError> {
    let prompt = table_mut(document, "prompt")?;
    prompt.insert("runners_seeded".to_owned(), Value::Boolean(true));
    Ok(())
}

fn remove_malformed_runner_container(
    document: &mut Table,
    expected: &PromptRunnerRow,
) -> Result<bool, ConfigError> {
    match document.get_mut("prompt") {
        Some(Value::Table(prompt)) => {
            let Some(runners) = prompt.get("runners") else {
                return Ok(false);
            };
            if runners.is_array()
                || expected.reason.as_deref() != Some(PromptRunnerIssue::RunnersNotList.code())
                || runners != &expected.raw
            {
                return Ok(false);
            }
            prompt.insert("runners_seeded".to_owned(), Value::Boolean(true));
            prompt.insert("runners".to_owned(), Value::Array(Vec::new()));
            Ok(true)
        }
        Some(prompt) => {
            if expected.reason.as_deref() != Some(PromptRunnerIssue::PromptSectionNotTable.code())
                || prompt != &expected.raw
            {
                return Ok(false);
            }
            let mut repaired = Table::new();
            repaired.insert("runners_seeded".to_owned(), Value::Boolean(true));
            repaired.insert("runners".to_owned(), Value::Array(Vec::new()));
            *prompt = Value::Table(repaired);
            Ok(true)
        }
        None => Ok(false),
    }
}

fn validate_runner(runner: &PromptRunner) -> Result<(), PromptRunnerIssue> {
    if runner.name.trim().is_empty() {
        return Err(PromptRunnerIssue::Name);
    }
    validate_runner_argv(&runner.argv).map_err(|error| match error {
        RunnerArgvError::EmptyCommand => PromptRunnerIssue::Empty,
        RunnerArgvError::PromptSlotCount => PromptRunnerIssue::PromptSlotCount,
        RunnerArgvError::PromptInProgram => PromptRunnerIssue::PromptInBinary,
        RunnerArgvError::UnsupportedHole => PromptRunnerIssue::StrayHole,
    })
}

fn preserve_corrupt_backup(path: &Path, original: &[u8]) -> Result<PathBuf, ConfigError> {
    let backup = path.with_file_name(format!(
        "{}.bak",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("config.toml")
    ));
    let backup_is_directory = fs::symlink_metadata(&backup)
        .map(|metadata| metadata.file_type().is_dir())
        .unwrap_or(false);
    let target = if backup_is_directory {
        backup.join(
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("config.toml"),
        )
    } else {
        backup.clone()
    };
    if target.exists() && !target.is_file() {
        return Err(io_error(
            "backup",
            &target,
            io::Error::new(
                io::ErrorKind::InvalidData,
                "the backup path is not a regular file",
            ),
        ));
    }
    let saved = if !target.exists() {
        atomic_write_bytes(&target, original)
            .map_err(|error| io_error("backup", &target, error))?;
        target
    } else {
        replace_existing_backup(&target, original, atomic_write_bytes, rename_path)?
    };
    let original_metadata = fs::metadata(path).map_err(|error| io_error("backup", path, error))?;
    fs::set_permissions(&saved, original_metadata.permissions())
        .map_err(|error| io_error("backup", &saved, error))?;
    copy_file_times(&original_metadata, &saved)
        .map_err(|error| io_error("backup", &saved, error))?;
    Ok(backup)
}

/// Give a saved copy the times of the file it copies.
///
/// Version 0.4 keeps a corrupt configuration with `shutil.copy2`, which carries the access and
/// modification times as well as the mode (`skit-oracle/src/skit/atomic.py:309`). A backup a user
/// opens to recover from should say when the file it holds was written, not when skit noticed the
/// damage.
fn copy_file_times(original: &fs::Metadata, saved: &Path) -> io::Result<()> {
    let mut times = fs::FileTimes::new();
    if let Ok(modified) = original.modified() {
        times = times.set_modified(modified);
    }
    if let Ok(accessed) = original.accessed() {
        times = times.set_accessed(accessed);
    }
    fs::OpenOptions::new()
        .write(true)
        .open(saved)?
        .set_times(times)
}

fn rename_path(previous: &Path, target: &Path) -> io::Result<()> {
    fs::rename(previous, target)
}

fn replace_existing_backup(
    backup: &Path,
    original: &[u8],
    write_backup: impl FnOnce(&Path, &[u8]) -> io::Result<()>,
    restore_previous: impl FnOnce(&Path, &Path) -> io::Result<()>,
) -> Result<PathBuf, ConfigError> {
    let previous = backup.with_file_name(format!(
        ".{}.{}.previous",
        backup
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("config.toml.bak"),
        EntryId::generate().as_str(),
    ));
    fs::rename(backup, &previous).map_err(|error| io_error("backup", backup, error))?;
    if let Err(error) = write_backup(backup, original) {
        if let Err(rollback) = restore_previous(&previous, backup) {
            return Err(io_error(
                "backup",
                backup,
                io::Error::new(
                    rollback.kind(),
                    format!("{error}; could not restore the previous backup: {rollback}"),
                ),
            ));
        }
        return Err(io_error("backup", backup, error));
    }
    fs::remove_file(&previous).map_err(|error| io_error("backup", &previous, error))?;
    Ok(backup.to_path_buf())
}

fn io_error(operation: &'static str, path: &std::path::Path, error: std::io::Error) -> ConfigError {
    ConfigError::Io {
        operation,
        path: path.display().to_string(),
        reason: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        PromptRunnerIssue, atomic_write_bytes, container_runner_row, legacy_value_descriptor,
        remove_malformed_runner_container, rename_path, replace_existing_backup, runner_array_mut,
        runner_rows_from_document, table_mut,
    };
    use skit_i18n::Locale;
    use std::{fs, io};
    use tempfile::TempDir;
    use toml::{Table, Value};

    #[test]
    fn raw_runner_issue_messages_keep_their_stable_machine_and_human_meanings() {
        for (issue, code, message) in [
            (
                PromptRunnerIssue::ArgvType,
                "argv-type",
                "a prompt runner argv must be a list of strings",
            ),
            (
                PromptRunnerIssue::RowNotTable,
                "row-not-table",
                "the prompt runner row is not a table",
            ),
            (
                PromptRunnerIssue::Duplicate,
                "duplicate",
                "another row already uses this prompt runner name",
            ),
        ] {
            assert_eq!(issue.code(), code);
            assert_eq!(issue.message().localize(Locale::En), message);
        }
    }

    #[test]
    fn runner_document_helpers_cover_fresh_missing_and_future_container_shapes() {
        let mut fresh = Table::new();
        assert!(runner_array_mut(&mut fresh).unwrap().is_empty());
        assert!(fresh["prompt"]["runners"].as_array().unwrap().is_empty());

        let mut future = Table::new();
        future.insert("prompt".to_owned(), Value::String("future".to_owned()));
        let error = table_mut(&mut future, "prompt").unwrap_err();
        assert!(
            error
                .to_string()
                .contains("configuration section is not a table: prompt")
        );

        let mut seeded_without_rows = Table::new();
        seeded_without_rows.insert(
            "prompt".to_owned(),
            Value::Table(Table::from_iter([(
                "runners_seeded".to_owned(),
                Value::Boolean(true),
            )])),
        );
        assert!(runner_rows_from_document(&seeded_without_rows).is_empty());

        let expected = container_runner_row(
            PromptRunnerIssue::RunnersNotList,
            "prompt.runners",
            &Value::String("old".to_owned()),
        );
        assert_eq!(
            expected.localized_reason(Locale::En).as_deref(),
            Some("the prompt.runners value is not a list; repair it before runner management")
        );
        assert_eq!(
            expected.localized_descriptor(Locale::ZhTw),
            "prompt.runners"
        );
        assert!(!remove_malformed_runner_container(&mut seeded_without_rows, &expected).unwrap());

        let mut malformed = Table::from_iter([(
            "prompt".to_owned(),
            Value::Table(Table::from_iter([(
                "runners".to_owned(),
                Value::String("old".to_owned()),
            )])),
        )]);
        let stale = container_runner_row(
            PromptRunnerIssue::RunnersNotList,
            "prompt.runners",
            &Value::String("newer".to_owned()),
        );
        assert!(!remove_malformed_runner_container(&mut malformed, &stale).unwrap());
        assert!(remove_malformed_runner_container(&mut malformed, &expected).unwrap());
        assert!(
            malformed["prompt"]["runners"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(malformed["prompt"]["runners_seeded"].as_bool().unwrap());

        let mut absent = Table::new();
        assert!(!remove_malformed_runner_container(&mut absent, &expected).unwrap());
    }

    #[test]
    fn legacy_runner_descriptors_use_python_scalar_and_string_spelling() {
        let datetime = "1979-05-27T07:32:00Z".parse().unwrap();
        assert_eq!(legacy_value_descriptor(&Value::Float(1.25)), "1.25");
        assert_eq!(legacy_value_descriptor(&Value::Boolean(true)), "True");
        assert_eq!(legacy_value_descriptor(&Value::Boolean(false)), "False");
        assert_eq!(
            legacy_value_descriptor(&Value::Datetime(datetime)),
            "1979-05-27T07:32:00Z"
        );
        assert_eq!(
            legacy_value_descriptor(&Value::Array(vec![
                Value::String("slash\\quote'".to_owned()),
                Value::String("line\nreturn\rtab\t\u{7}".to_owned()),
            ])),
            "['slash\\\\quote\\'', 'line\\nreturn\\rtab\\t\\u0007']"
        );
    }

    #[test]
    fn backup_replacement_restores_the_previous_backup_when_the_new_write_fails() {
        let root = TempDir::new().unwrap();
        let backup = root.path().join("config.toml.bak");
        fs::write(&backup, b"previous").unwrap();

        let error = replace_existing_backup(
            &backup,
            b"current corrupt bytes",
            |_, _| Err(io::Error::other("new backup failed")),
            rename_path,
        )
        .unwrap_err();

        assert!(error.to_string().contains("new backup failed"));
        assert_eq!(fs::read(&backup).unwrap(), b"previous");
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    }

    #[test]
    fn backup_replacement_publishes_new_bytes_after_saving_the_previous_file() {
        let root = TempDir::new().unwrap();
        let backup = root.path().join("config.toml.bak");
        fs::write(&backup, b"previous").unwrap();

        replace_existing_backup(&backup, b"current", atomic_write_bytes, rename_path).unwrap();

        assert_eq!(fs::read(&backup).unwrap(), b"current");
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    }

    #[test]
    fn backup_replacement_reports_both_failures_when_rollback_also_fails() {
        let root = TempDir::new().unwrap();
        let backup = root.path().join("config.toml.bak");
        fs::write(&backup, b"previous").unwrap();

        let error = replace_existing_backup(
            &backup,
            b"current corrupt bytes",
            |_, _| Err(io::Error::other("new backup failed")),
            |_, _| Err(io::Error::other("restore failed")),
        )
        .unwrap_err();

        let message = error.to_string();
        assert!(message.contains("new backup failed"), "{message}");
        assert!(message.contains("restore failed"), "{message}");
        assert!(!backup.exists());
        let names = fs::read_dir(root.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(names.len(), 1);
        assert!(names[0].ends_with(".previous"), "{names:?}");
    }
}
