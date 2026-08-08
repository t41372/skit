use std::env;
use std::error::Error as StdError;
use std::fmt;
use std::path::PathBuf;

use crate::LibraryRoots;

/// A platform layout that skit supports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Linux,
    MacOs,
    Windows,
}

/// Filesystem inputs used to resolve skit's owned directories.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PathContext {
    pub home: Option<PathBuf>,
    pub local_app_data: Option<PathBuf>,
    pub xdg_data_home: Option<PathBuf>,
    pub xdg_state_home: Option<PathBuf>,
    pub xdg_config_home: Option<PathBuf>,
    pub data_override: Option<PathBuf>,
    pub state_override: Option<PathBuf>,
    pub config_override: Option<PathBuf>,
}

impl PathContext {
    fn from_process() -> Self {
        Self {
            home: environment_path("HOME"),
            local_app_data: environment_path("LOCALAPPDATA"),
            xdg_data_home: environment_path("XDG_DATA_HOME"),
            xdg_state_home: environment_path("XDG_STATE_HOME"),
            xdg_config_home: environment_path("XDG_CONFIG_HOME"),
            data_override: environment_path("SKIT_DATA_DIR"),
            state_override: environment_path("SKIT_STATE_DIR"),
            config_override: environment_path("SKIT_CONFIG_DIR"),
        }
    }
}

/// A failure to resolve a required platform directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathError {
    MissingHome,
    MissingLocalAppData,
}

impl fmt::Display for PathError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingHome => write!(formatter, "cannot find the home directory"),
            Self::MissingLocalAppData => {
                write!(formatter, "cannot find the local app data directory")
            }
        }
    }
}

impl StdError for PathError {}

/// Resolve roots from explicit platform inputs.
///
/// This function has no process-global side effects. Frontends can test or embed the
/// same path contract without changing environment variables.
///
/// # Errors
///
/// Returns an error if an axis needs a platform base directory that is unavailable.
pub fn resolve_roots(platform: Platform, context: &PathContext) -> Result<LibraryRoots, PathError> {
    if let (Some(data), Some(state), Some(config)) = (
        &context.data_override,
        &context.state_override,
        &context.config_override,
    ) {
        return Ok(LibraryRoots::new(data, state, config));
    }

    match platform {
        Platform::Linux => resolve_linux(context),
        Platform::MacOs => resolve_macos(context),
        Platform::Windows => resolve_windows(context),
    }
}

/// Resolve roots for the current process and platform.
///
/// # Errors
///
/// Returns an error if the operating system does not expose a required base directory.
pub fn discover_roots() -> Result<LibraryRoots, PathError> {
    resolve_roots(current_platform(), &PathContext::from_process())
}

fn resolve_linux(context: &PathContext) -> Result<LibraryRoots, PathError> {
    let data = if let Some(path) = &context.data_override {
        path.clone()
    } else if let Some(path) = &context.xdg_data_home {
        path.join("skit")
    } else {
        context
            .home
            .as_ref()
            .ok_or(PathError::MissingHome)?
            .join(".local/share/skit")
    };
    let state = if let Some(path) = &context.state_override {
        path.clone()
    } else if let Some(path) = &context.xdg_state_home {
        path.join("skit")
    } else {
        context
            .home
            .as_ref()
            .ok_or(PathError::MissingHome)?
            .join(".local/state/skit")
    };
    let config = if let Some(path) = &context.config_override {
        path.clone()
    } else if let Some(path) = &context.xdg_config_home {
        path.join("skit")
    } else {
        context
            .home
            .as_ref()
            .ok_or(PathError::MissingHome)?
            .join(".config/skit")
    };
    Ok(LibraryRoots::new(data, state, config))
}

fn resolve_macos(context: &PathContext) -> Result<LibraryRoots, PathError> {
    let base = if context.data_override.is_some()
        && context.state_override.is_some()
        && context.config_override.is_some()
    {
        None
    } else {
        Some(
            context
                .home
                .as_ref()
                .ok_or(PathError::MissingHome)?
                .join("Library/Application Support/skit"),
        )
    };
    let data = context
        .data_override
        .clone()
        .or_else(|| base.clone())
        .ok_or(PathError::MissingHome)?;
    let state = context
        .state_override
        .clone()
        .or_else(|| base.clone())
        .ok_or(PathError::MissingHome)?;
    let config = context
        .config_override
        .clone()
        .or(base)
        .ok_or(PathError::MissingHome)?;
    Ok(LibraryRoots::new(data, state, config))
}

fn resolve_windows(context: &PathContext) -> Result<LibraryRoots, PathError> {
    let base = if context.data_override.is_some()
        && context.state_override.is_some()
        && context.config_override.is_some()
    {
        None
    } else {
        Some(
            context
                .local_app_data
                .as_ref()
                .ok_or(PathError::MissingLocalAppData)?
                .join("skit"),
        )
    };
    let data = context
        .data_override
        .clone()
        .or_else(|| base.clone())
        .ok_or(PathError::MissingLocalAppData)?;
    let state = context
        .state_override
        .clone()
        .or_else(|| base.clone())
        .ok_or(PathError::MissingLocalAppData)?;
    let config = context
        .config_override
        .clone()
        .or(base)
        .ok_or(PathError::MissingLocalAppData)?;
    Ok(LibraryRoots::new(data, state, config))
}

fn environment_path(name: &str) -> Option<PathBuf> {
    env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

const fn current_platform() -> Platform {
    #[cfg(target_os = "windows")]
    {
        return Platform::Windows;
    }
    #[cfg(target_os = "macos")]
    {
        return Platform::MacOs;
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        Platform::Linux
    }
}
