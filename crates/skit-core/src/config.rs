use std::fs;
use std::path::PathBuf;

use crate::LibraryRoots;

/// The small, parser-free subset of user configuration needed to plan ordinary
/// interpreted/JS launches.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LaunchConfig {
    pub js_runner: Option<String>,
    pub windows_bash: Option<PathBuf>,
}

/// Read launch policy from the existing Python-era `config.toml` shape.
///
/// Missing, corrupt, or hand-edited non-string values degrade to defaults exactly like
/// the current Python read-only config path. This function never rewrites the file.
#[must_use]
pub fn load_launch_config(roots: &LibraryRoots) -> LaunchConfig {
    let path = roots.config_dir().join("config.toml");
    let Ok(text) = fs::read_to_string(path) else {
        return LaunchConfig::default();
    };
    let Ok(document) = toml::from_str::<toml::Value>(&text) else {
        return LaunchConfig::default();
    };
    let Some(root) = document.as_table() else {
        return LaunchConfig::default();
    };
    LaunchConfig {
        js_runner: nested_string(root, "js", "runner").filter(|value| !value.is_empty()),
        windows_bash: nested_string(root, "shell", "bash_path")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from),
    }
}

fn nested_string(document: &toml::Table, section: &str, key: &str) -> Option<String> {
    document
        .get(section)
        .and_then(toml::Value::as_table)
        .and_then(|table| table.get(key))
        .and_then(toml::Value::as_str)
        .map(str::to_owned)
}
