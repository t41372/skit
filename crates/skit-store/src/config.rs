//! Preserve and update the user configuration file.

use std::{collections::BTreeMap, fs, path::PathBuf};

use skit_i18n::{Localize, Message};
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

/// One configured prompt runner.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PromptRunner {
    /// Stable user-facing name.
    pub name: String,
    /// Direct process arguments. No shell parses these values.
    pub argv: Vec<String>,
}

/// One raw prompt-runner row for inspection and repair.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PromptRunnerRow {
    /// One-based row index.
    pub index: usize,
    /// Parsed runner name when present.
    pub name: Option<String>,
    /// Parsed argument vector when it has only strings.
    pub argv: Option<Vec<String>>,
    /// Validation reason. A valid row has no reason.
    pub reason: Option<String>,
    /// Stable display text for malformed shapes.
    pub descriptor: String,
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
}

impl Localize for ConfigError {
    fn message(&self) -> Message {
        match self {
            Self::Io {
                operation,
                path,
                reason,
            } => Message::new("could not {} configuration at {}: {}")
                .with(operation)
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
        }
    }
}

/// Filesystem-backed `config.toml` adapter.
#[derive(Clone, Debug)]
pub struct FileConfigStore {
    config_dir: PathBuf,
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
        let document = self.load()?;
        let mut settings = BTreeMap::new();
        for key in ["lang", "editor", "form", "after_run"] {
            if let Some(value) = read_key(&document, key) {
                settings.insert(key.to_owned(), value);
            }
        }
        for key in ["shell.bash_path", "js.runner"] {
            if let Some(value) = read_key(&document, key) {
                settings.insert(key.to_owned(), value);
            }
        }
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
        settings.entry("lang".to_owned()).or_default();
        settings.entry("editor".to_owned()).or_default();
        settings
            .entry("form".to_owned())
            .or_insert_with(|| "tui".to_owned());
        settings
            .entry("after_run".to_owned())
            .or_insert_with(|| "exit".to_owned());
        settings.entry("shell.bash_path".to_owned()).or_default();
        settings.entry("js.runner".to_owned()).or_default();
        Ok(settings)
    }

    /// Read one supported scalar setting.
    pub fn get(&self, key: &str) -> Result<String, ConfigError> {
        self.settings()?.remove(key).ok_or_else(|| {
            ConfigError::Invalid(Message::new("unknown configuration key: {}").with(key))
        })
    }

    /// Set one supported scalar setting without changing unknown TOML fields.
    pub fn set(&self, key: &str, value: &str) -> Result<(), ConfigError> {
        validate_setting(key, value)?;
        self.update(|document| write_key(document, key, value))
    }

    /// Validate and replace multiple settings in one configuration transaction.
    pub fn set_many(&self, settings: &BTreeMap<String, String>) -> Result<(), ConfigError> {
        for (key, value) in settings {
            validate_setting(key, value)?;
        }
        self.update(|document| {
            for (key, value) in settings.iter().filter(|(key, _)| key.as_str() != "mirror") {
                write_key(document, key, value)?;
            }
            if let Some(value) = settings.get("mirror") {
                write_key(document, "mirror", value)?;
            }
            Ok(())
        })
    }

    /// Read stored mirror URLs without applying the master switch.
    pub fn mirror(&self) -> Result<MirrorSettings, ConfigError> {
        Ok(mirror_from_document(&self.load()?))
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
        if !mirror.pypi.is_empty()
            && !base.contains_key("UV_DEFAULT_INDEX")
            && !base.contains_key("UV_INDEX_URL")
        {
            output.insert("UV_DEFAULT_INDEX".to_owned(), mirror.pypi);
        }
        if !mirror.python_install.is_empty() && !base.contains_key("UV_PYTHON_INSTALL_MIRROR") {
            output.insert("UV_PYTHON_INSTALL_MIRROR".to_owned(), mirror.python_install);
        }
        if !mirror.npm.is_empty()
            && !base.contains_key("NPM_CONFIG_REGISTRY")
            && !base.contains_key("npm_config_registry")
        {
            output.insert("NPM_CONFIG_REGISTRY".to_owned(), mirror.npm);
        }
        Ok(output)
    }

    /// Read valid prompt runners. The built-in rows are defaults until customized.
    pub fn runners(&self) -> Result<Vec<PromptRunner>, ConfigError> {
        let document = self.load()?;
        Ok(runners_from_document(&document).unwrap_or_else(seed_runners))
    }

    /// Materialize the default runner rows on an explicit management read.
    pub fn ensure_runners_seeded(&self) -> Result<(), ConfigError> {
        self.update(materialize_seed_runners)
    }

    /// Return each raw runner row, including malformed future shapes.
    pub fn runner_rows(&self) -> Result<Vec<PromptRunnerRow>, ConfigError> {
        let document = self.load()?;
        let rows = explicit_runner_rows(&document).unwrap_or(&[]);
        Ok(rows
            .iter()
            .enumerate()
            .map(|(index, value)| runner_row(index + 1, value))
            .collect())
    }

    /// Return labels for malformed prompt runner rows that normal reads ignore.
    pub fn invalid_runner_rows(&self) -> Result<Vec<String>, ConfigError> {
        let document = self.load()?;
        let Some(rows) = document
            .get("prompt")
            .and_then(Value::as_table)
            .and_then(|prompt| prompt.get("runners"))
            .and_then(Value::as_array)
        else {
            return Ok(Vec::new());
        };
        Ok(rows
            .iter()
            .enumerate()
            .filter_map(|(index, value)| {
                let runner = value.as_table().and_then(runner_from_row);
                runner
                    .as_ref()
                    .is_none_or(|runner| validate_runner(runner).is_err())
                    .then(|| {
                        value
                            .as_table()
                            .and_then(|row| row.get("name"))
                            .and_then(Value::as_str)
                            .filter(|name| !name.trim().is_empty())
                            .map(str::to_owned)
                            .unwrap_or_else(|| format!("row {}", index + 1))
                    })
            })
            .collect())
    }

    /// Add or replace one named prompt runner.
    pub fn set_runner(&self, runner: PromptRunner, replace: bool) -> Result<(), ConfigError> {
        validate_runner(&runner)?;
        self.update(|document| {
            materialize_seed_runners(document)?;
            let rows = runner_array_mut(document)?;
            let existing = rows.iter().position(|value| {
                value
                    .as_table()
                    .and_then(runner_from_row)
                    .filter(|item| validate_runner(item).is_ok())
                    .is_some_and(|item| item.name == runner.name)
            });
            if let Some(index) = existing {
                if !replace {
                    return Err(ConfigError::Invalid(
                        Message::new("prompt runner already exists: {}").with(&runner.name),
                    ));
                }
                let row = rows[index]
                    .as_table_mut()
                    .expect("a matched runner row is a table");
                write_runner_fields(row, &runner);
            } else {
                rows.push(Value::Table(runner_table(&runner)));
            }
            Ok(())
        })
    }

    /// Remove one named prompt runner and report whether it existed.
    pub fn remove_runner(&self, name: &str) -> Result<bool, ConfigError> {
        self.update(|document| {
            materialize_seed_runners(document)?;
            let rows = runner_array_mut(document)?;
            let index = rows.iter().position(|value| {
                value
                    .as_table()
                    .and_then(runner_from_row)
                    .filter(|item| validate_runner(item).is_ok())
                    .is_some_and(|item| item.name == name)
            });
            Ok(index.map(|index| rows.remove(index)).is_some())
        })
    }

    /// Remove one raw one-based row without parsing or normalizing other rows.
    pub fn remove_runner_row(&self, row: usize) -> Result<bool, ConfigError> {
        self.update(|document| {
            // `--row` addresses the raw rows that `runner_rows` reported, so seeding
            // here would renumber them under the user.
            let Some(rows) = explicit_runner_rows_mut(document) else {
                return Ok(false);
            };
            let Some(index) = row.checked_sub(1).filter(|index| *index < rows.len()) else {
                return Ok(false);
            };
            rows.remove(index);
            Ok(true)
        })
    }

    fn path(&self) -> PathBuf {
        self.config_dir.join("config.toml")
    }

    fn lock_path(&self) -> PathBuf {
        self.config_dir.join(".config.lock")
    }

    fn load(&self) -> Result<Table, ConfigError> {
        let path = self.path();
        match fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).map_err(|error| ConfigError::Parse {
                path: path.display().to_string(),
                reason: error.to_string(),
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Table::new()),
            Err(error) => Err(io_error("read", &path, error)),
        }
    }

    fn update<T>(
        &self,
        operation: impl FnOnce(&mut Table) -> Result<T, ConfigError>,
    ) -> Result<T, ConfigError> {
        let lock_path = self.lock_path();
        let _lock =
            acquire_lock(&lock_path).map_err(|error| io_error("lock", &lock_path, error))?;
        let mut document = self.load()?;
        let result = operation(&mut document)?;
        let encoded =
            toml::to_string_pretty(&document).expect("a parsed TOML value tree must serialize");
        let path = self.path();
        atomic_write_bytes(&path, encoded.as_bytes())
            .map_err(|error| io_error("write", &path, error))?;
        Ok(result)
    }
}

fn validate_setting(key: &str, value: &str) -> Result<(), ConfigError> {
    let allowed = match key {
        "lang" => !value.trim().is_empty(),
        "editor" => true,
        "form" => matches!(value, "tui" | "plain"),
        "after_run" => matches!(value, "exit" | "stay"),
        "shell.bash_path" => true,
        "js.runner" => value.is_empty() || matches!(value, "deno" | "bun" | "node"),
        "mirror" => matches!(value, "on" | "off"),
        "mirror.pypi" => valid_axis_value(value, PYPI_PRESETS, false),
        "mirror.github" => valid_axis_value(value, GITHUB_PRESETS, true),
        "mirror.npm" => valid_axis_value(value, NPM_PRESETS, false),
        _ => false,
    };
    if allowed {
        Ok(())
    } else {
        Err(ConfigError::Invalid(
            Message::new("invalid configuration value for {}: {}")
                .with(key)
                .with(value),
        ))
    }
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
        let table = table_mut(document, "mirror")?;
        if value == "on" && !table_has_urls(table) {
            return Err(ConfigError::Invalid(Message::new(
                "no mirror URLs are stored; set one mirror axis first",
            )));
        }
        table.insert("enabled".to_owned(), Value::Boolean(value == "on"));
        return Ok(());
    }
    if matches!(key, "mirror.pypi" | "mirror.github" | "mirror.npm") {
        let table = table_mut(document, "mirror")?;
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
    let storage_key = if key == "lang" { "language" } else { key };
    let mut parts = storage_key.split('.');
    let first = parts.next().expect("validated keys are not empty");
    if let Some(second) = parts.next() {
        let table = table_mut(document, first)?;
        table.insert(second.to_owned(), Value::String(value.to_owned()));
    } else {
        document.insert(first.to_owned(), Value::String(value.to_owned()));
    }
    Ok(())
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

fn runners_from_document(document: &Table) -> Option<Vec<PromptRunner>> {
    let prompt = document.get("prompt")?.as_table()?;
    if !prompt
        .get("runners_seeded")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return None;
    }
    let rows = prompt.get("runners")?.as_array()?;
    Some(
        rows.iter()
            .filter_map(Value::as_table)
            .filter_map(runner_from_row)
            .filter(|runner| validate_runner(runner).is_ok())
            .collect(),
    )
}

fn runners_are_seeded(document: &Table) -> bool {
    document
        .get("prompt")
        .and_then(Value::as_table)
        .and_then(|prompt| prompt.get("runners_seeded"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn explicit_runner_rows(document: &Table) -> Option<&[Value]> {
    document
        .get("prompt")?
        .as_table()?
        .get("runners")?
        .as_array()
        .map(Vec::as_slice)
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
    let prompt = table_mut(document, "prompt")?;
    if !prompt.contains_key("runners") {
        prompt.insert("runners".to_owned(), Value::Array(Vec::new()));
    }
    prompt
        .get_mut("runners")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| ConfigError::Invalid(Message::new("prompt runners is not an array")))
}

fn runner_row(index: usize, value: &Value) -> PromptRunnerRow {
    let table = value.as_table();
    let name = table
        .and_then(|row| row.get("name"))
        .and_then(Value::as_str)
        .map(str::to_owned);
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
    let parsed = table.and_then(runner_from_row);
    let reason = parsed.as_ref().map_or_else(
        || Some("runner row needs a name and a string argv array".to_owned()),
        |runner| validate_runner(runner).err().map(|error| error.to_string()),
    );
    let descriptor = name.clone().unwrap_or_else(|| {
        value
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| format!("row {index}"))
    });
    PromptRunnerRow {
        index,
        name,
        argv,
        reason,
        descriptor,
    }
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

fn runner_from_row(row: &Table) -> Option<PromptRunner> {
    let name = row.get("name")?.as_str()?.trim();
    let argv = row
        .get("argv")?
        .as_array()?
        .iter()
        .map(Value::as_str)
        .collect::<Option<Vec<_>>>()?;
    (!name.is_empty()).then(|| PromptRunner {
        name: name.to_owned(),
        argv: argv.into_iter().map(str::to_owned).collect(),
    })
}

/// Record the default runner rows once, keeping any rows the user already wrote.
///
/// A hand-written `[[prompt.runners]]` list is authoritative. Seeding only adds the
/// defaults when the file has no list at all.
fn materialize_seed_runners(document: &mut Table) -> Result<(), ConfigError> {
    if runners_are_seeded(document) {
        return Ok(());
    }
    let prompt = table_mut(document, "prompt")?;
    if !prompt.contains_key("runners") {
        prompt.insert(
            "runners".to_owned(),
            Value::Array(
                seed_runners()
                    .iter()
                    .map(|runner| Value::Table(runner_table(runner)))
                    .collect(),
            ),
        );
    }
    prompt.insert("runners_seeded".to_owned(), Value::Boolean(true));
    Ok(())
}

fn validate_runner(runner: &PromptRunner) -> Result<(), ConfigError> {
    if runner.name.trim().is_empty() || runner.argv.is_empty() {
        return Err(ConfigError::Invalid(Message::new(
            "a prompt runner needs a name and command",
        )));
    }
    let slots = runner
        .argv
        .iter()
        .enumerate()
        .flat_map(|(index, token)| token.match_indices("{{prompt}}").map(move |_| index))
        .collect::<Vec<_>>();
    if slots.len() != 1 || slots[0] == 0 {
        return Err(ConfigError::Invalid(Message::new(
            "a prompt runner command needs {{prompt}} exactly once after the program",
        )));
    }
    Ok(())
}

fn io_error(operation: &'static str, path: &std::path::Path, error: std::io::Error) -> ConfigError {
    ConfigError::Io {
        operation,
        path: path.display().to_string(),
        reason: error.to_string(),
    }
}
