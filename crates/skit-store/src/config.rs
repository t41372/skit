//! Preserve and update the user configuration file.

use std::{collections::BTreeMap, fs, path::PathBuf};

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

/// One configured prompt runner.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PromptRunner {
    /// Stable user-facing name.
    pub name: String,
    /// Direct process arguments. No shell parses these values.
    pub argv: Vec<String>,
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
    Invalid(String),
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
        for key in [
            "shell.bash_path",
            "js.runner",
            "mirror.pypi",
            "mirror.github",
            "mirror.npm",
        ] {
            if let Some(value) = read_key(&document, key) {
                settings.insert(key.to_owned(), value);
            }
        }
        let mirror_enabled = document
            .get("mirror")
            .and_then(Value::as_table)
            .and_then(|table| table.get("enabled"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        settings.insert(
            "mirror".to_owned(),
            if mirror_enabled { "on" } else { "off" }.to_owned(),
        );
        settings
            .entry("lang".to_owned())
            .or_insert_with(|| "auto".to_owned());
        settings.entry("editor".to_owned()).or_default();
        settings
            .entry("form".to_owned())
            .or_insert_with(|| "tui".to_owned());
        settings
            .entry("after_run".to_owned())
            .or_insert_with(|| "exit".to_owned());
        settings.entry("shell.bash_path".to_owned()).or_default();
        settings.entry("js.runner".to_owned()).or_default();
        settings.entry("mirror.pypi".to_owned()).or_default();
        settings.entry("mirror.github".to_owned()).or_default();
        settings.entry("mirror.npm".to_owned()).or_default();
        Ok(settings)
    }

    /// Read one supported scalar setting.
    pub fn get(&self, key: &str) -> Result<String, ConfigError> {
        self.settings()?
            .remove(key)
            .ok_or_else(|| ConfigError::Invalid(format!("unknown configuration key: {key}")))
    }

    /// Set one supported scalar setting without changing unknown TOML fields.
    pub fn set(&self, key: &str, value: &str) -> Result<(), ConfigError> {
        validate_setting(key, value)?;
        self.update(|document| write_key(document, key, value))
    }

    /// Read valid prompt runners. The built-in rows are defaults until customized.
    pub fn runners(&self) -> Result<Vec<PromptRunner>, ConfigError> {
        let document = self.load()?;
        Ok(runners_from_document(&document).unwrap_or_else(seed_runners))
    }

    /// Add or replace one named prompt runner.
    pub fn set_runner(&self, runner: PromptRunner, replace: bool) -> Result<(), ConfigError> {
        validate_runner(&runner)?;
        self.update(|document| {
            let mut runners = runners_from_document(document).unwrap_or_else(seed_runners);
            if let Some(index) = runners.iter().position(|item| item.name == runner.name) {
                if !replace {
                    return Err(ConfigError::Invalid(format!(
                        "prompt runner already exists: {}",
                        runner.name
                    )));
                }
                runners[index] = runner;
            } else {
                runners.push(runner);
            }
            write_runners(document, &runners);
            Ok(())
        })
    }

    /// Remove one named prompt runner and report whether it existed.
    pub fn remove_runner(&self, name: &str) -> Result<bool, ConfigError> {
        self.update(|document| {
            let mut runners = runners_from_document(document).unwrap_or_else(seed_runners);
            let old_len = runners.len();
            runners.retain(|item| item.name != name);
            let removed = runners.len() != old_len;
            if removed {
                write_runners(document, &runners);
            }
            Ok(removed)
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
        let encoded = toml::to_string_pretty(&document).map_err(|error| ConfigError::Encode {
            reason: error.to_string(),
        })?;
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
        "mirror.pypi" | "mirror.github" | "mirror.npm" => true,
        _ => false,
    };
    if allowed {
        Ok(())
    } else {
        Err(ConfigError::Invalid(format!(
            "invalid configuration value for {key}: {value}"
        )))
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
        table.insert("enabled".to_owned(), Value::Boolean(value == "on"));
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

fn table_mut<'a>(document: &'a mut Table, key: &str) -> Result<&'a mut Table, ConfigError> {
    if !document.contains_key(key) {
        document.insert(key.to_owned(), Value::Table(Table::new()));
    }
    document
        .get_mut(key)
        .and_then(Value::as_table_mut)
        .ok_or_else(|| ConfigError::Invalid(format!("configuration section is not a table: {key}")))
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
            .filter_map(|row| {
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
            })
            .collect(),
    )
}

fn write_runners(document: &mut Table, runners: &[PromptRunner]) {
    let mut prompt = document
        .remove("prompt")
        .and_then(|value| value.as_table().cloned())
        .unwrap_or_default();
    prompt.insert("runners_seeded".to_owned(), Value::Boolean(true));
    prompt.insert(
        "runners".to_owned(),
        Value::Array(
            runners
                .iter()
                .map(|runner| {
                    Value::Table(Table::from_iter([
                        ("name".to_owned(), Value::String(runner.name.clone())),
                        (
                            "argv".to_owned(),
                            Value::Array(runner.argv.iter().cloned().map(Value::String).collect()),
                        ),
                    ]))
                })
                .collect(),
        ),
    );
    document.insert("prompt".to_owned(), Value::Table(prompt));
}

fn validate_runner(runner: &PromptRunner) -> Result<(), ConfigError> {
    if runner.name.trim().is_empty() || runner.argv.is_empty() {
        return Err(ConfigError::Invalid(
            "a prompt runner needs a name and command".to_owned(),
        ));
    }
    let slots = runner
        .argv
        .iter()
        .enumerate()
        .flat_map(|(index, token)| token.match_indices("{{prompt}}").map(move |_| index))
        .collect::<Vec<_>>();
    if slots.len() != 1 || slots[0] == 0 {
        return Err(ConfigError::Invalid(
            "a prompt runner command needs {{prompt}} exactly once after the program".to_owned(),
        ));
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
