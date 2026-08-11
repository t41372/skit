//! Frontend-neutral planning for application preferences.

use std::{collections::BTreeMap, path::Path};

use serde::{Deserialize, Serialize};
use skit_i18n::{Localize, Message};
use thiserror::Error;

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
    /// JavaScript runtime preference.
    pub javascript: JavascriptChoice,
    /// Windows bash path. `None` hides the Windows-only section.
    pub bash_path: Option<String>,
    /// Configured prompt-runner names.
    pub runner_names: Vec<String>,
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
    /// JavaScript runtime preference.
    pub javascript: JavascriptChoice,
    /// Windows bash path. `None` hides the section.
    pub bash_path: Option<String>,
    /// Configured prompt-runner names.
    pub runner_names: Vec<String>,
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
    initial: PreferencesInitial,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct PreferencesInitial {
    language: String,
    editor: String,
    form: InteractiveFormChoice,
    after_run: AfterRunChoice,
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
}

impl PreferencesError {
    /// Return the control that must change.
    #[must_use]
    pub const fn field(&self) -> PreferencesField {
        match self {
            Self::CustomUrlRequired { field } => *field,
            Self::GithubHttpsRequired { .. } => PreferencesField::GithubMirror,
            Self::BashPathMissing { .. } => PreferencesField::BashPath,
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
            javascript: snapshot.javascript,
            bash_path: snapshot.bash_path,
            runner_names: snapshot.runner_names,
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
            || self.javascript != self.initial.javascript
            || self.bash_path != self.initial.bash_path
            || self.mirror_master != self.initial.mirror_master
            || self.pypi != self.initial.pypi
            || self.pypi_url != self.initial.pypi_url
            || self.github != self.initial.github
            || self.github_url != self.initial.github_url
            || self.npm != self.initial.npm
            || self.npm_url != self.initial.npm_url
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

        let mirror_unchanged = self.mirror_master == self.initial.mirror_master
            && self.pypi == self.initial.pypi
            && self.pypi_url == self.initial.pypi_url
            && self.github == self.initial.github
            && self.github_url == self.initial.github_url
            && self.npm == self.initial.npm
            && self.npm_url == self.initial.npm_url;
        if github.passthrough && mirror_unchanged {
            let change = PreferencesChangeSet { settings };
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
        let change = PreferencesChangeSet { settings };
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
