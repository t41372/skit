//! Frontend-neutral planning for Agent Skill installation.

use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use skit_i18n::{Localize, Message};
use thiserror::Error;

use crate::ExitClass;

/// Identify whether an Agent Skill target belongs to the user or the current project.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentScope {
    /// A target below the user's home directory.
    User,
    /// A target below the current project directory.
    Project,
}

/// Describe one existing Agent Skill target that an interactive frontend can offer.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AgentTarget {
    /// Stable convention name: `claude`, `codex`, or `agents`.
    pub name: String,
    /// Target scope.
    pub scope: AgentScope,
    /// Tool marker directory, such as `~/.claude`.
    pub base: PathBuf,
}

impl AgentTarget {
    /// Return the directory that holds named Agent Skills.
    #[must_use]
    pub fn skills_dir(&self) -> PathBuf {
        self.base.join("skills")
    }
}

/// Supply environment roots without coupling the use case to process-global state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentRoots {
    /// Current user's home directory when it can be resolved.
    pub home: Option<PathBuf>,
    /// Current project directory.
    pub cwd: PathBuf,
}

/// Capture one CLI or UI request before consent and target planning.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentInstallRequest {
    /// Explicit named convention.
    pub target: Option<String>,
    /// Explicit skills directory supplied with `--to`.
    pub directory: Option<PathBuf>,
    /// Select project scope for conventions that support both scopes.
    pub project: bool,
    /// Whether the frontend can present a picker and confirmation.
    pub interactive: bool,
}

/// Tell a frontend whether it can install now or must ask the user to choose.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AgentInstallPlan {
    /// Explicit consent resolved to one skills directory.
    Ready {
        /// Directory below which `skit/SKILL.md` is installed.
        skills_dir: PathBuf,
    },
    /// Bare interactive mode found existing tool directories.
    Choose {
        /// Stable user-first, project-second target list.
        candidates: Vec<AgentTarget>,
    },
}

/// Report a deterministic Agent Skill planning refusal.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum AgentInstallError {
    /// `--to` was combined with a named or project target.
    #[error("use a named target with optional project scope or an explicit directory, not both")]
    ConflictingSelection,
    /// An explicit convention name is not supported.
    #[error("unknown agent convention: {name}")]
    UnknownTarget {
        /// Unsupported name.
        name: String,
    },
    /// A user-scoped target or `~/` path needs a home directory.
    #[error("could not determine the user directory")]
    UserDirectoryUnavailable,
    /// Bare non-interactive mode cannot choose on the user's behalf.
    #[error("an explicit agent target or directory is required")]
    ExplicitSelectionRequired,
    /// Bare interactive mode did not find an existing tool marker.
    #[error("no agent directories were detected")]
    NoTargetsDetected,
}

impl AgentInstallError {
    /// Return the stable process classification for this refusal.
    #[must_use]
    pub const fn exit_class(&self) -> ExitClass {
        match self {
            Self::NoTargetsDetected => ExitClass::Failure,
            Self::ConflictingSelection
            | Self::UnknownTarget { .. }
            | Self::UserDirectoryUnavailable
            | Self::ExplicitSelectionRequired => ExitClass::Usage,
        }
    }
}

impl Localize for AgentInstallError {
    fn message(&self) -> Message {
        match self {
            Self::ConflictingSelection => {
                Message::new("Use a named target (with optional --project) or --to — not both.")
            }
            Self::UnknownTarget { name } => {
                Message::new("Unknown target {}. Valid targets: claude, codex, agents.").with(name)
            }
            Self::UserDirectoryUnavailable => {
                Message::new("could not determine the user directory")
            }
            Self::ExplicitSelectionRequired => Message::new(
                "Nothing installed: name a target (claude, codex, agents) or pass --to DIR.",
            ),
            Self::NoTargetsDetected => Message::new(
                "No agent directories detected (~/.claude, ~/.codex, ./.agents, …). Pass --to DIR to choose one yourself.",
            ),
        }
    }
}

/// Resolve explicit consent or return candidates for a bare interactive request.
pub fn plan_agent_install(
    request: &AgentInstallRequest,
    roots: &AgentRoots,
    is_directory: impl Fn(&Path) -> bool,
) -> Result<AgentInstallPlan, AgentInstallError> {
    if request.directory.is_some() && (request.target.is_some() || request.project) {
        return Err(AgentInstallError::ConflictingSelection);
    }
    if let Some(directory) = &request.directory {
        return Ok(AgentInstallPlan::Ready {
            skills_dir: expand_current_user(directory, roots.home.as_deref())?,
        });
    }
    if let Some(name) = &request.target {
        let target = named_target(name, request.project, roots)?;
        return Ok(AgentInstallPlan::Ready {
            skills_dir: target.skills_dir(),
        });
    }
    if !request.interactive {
        return Err(AgentInstallError::ExplicitSelectionRequired);
    }

    let candidates = detect_agent_targets(roots, is_directory);
    if candidates.is_empty() {
        Err(AgentInstallError::NoTargetsDetected)
    } else {
        Ok(AgentInstallPlan::Choose { candidates })
    }
}

/// Detect every existing Agent Skill target without choosing or writing one.
#[must_use]
pub fn detect_agent_targets(
    roots: &AgentRoots,
    is_directory: impl Fn(&Path) -> bool,
) -> Vec<AgentTarget> {
    let mut candidates = Vec::new();
    if let Some(home) = &roots.home {
        add_existing_target(
            &mut candidates,
            "claude",
            AgentScope::User,
            home.join(".claude"),
            &is_directory,
        );
        add_existing_target(
            &mut candidates,
            "codex",
            AgentScope::User,
            home.join(".codex"),
            &is_directory,
        );
    }
    add_existing_target(
        &mut candidates,
        "claude",
        AgentScope::Project,
        roots.cwd.join(".claude"),
        &is_directory,
    );
    add_existing_target(
        &mut candidates,
        "codex",
        AgentScope::Project,
        roots.cwd.join(".codex"),
        &is_directory,
    );
    add_existing_target(
        &mut candidates,
        "agents",
        AgentScope::Project,
        roots.cwd.join(".agents"),
        &is_directory,
    );
    candidates
}

fn named_target(
    name: &str,
    project: bool,
    roots: &AgentRoots,
) -> Result<AgentTarget, AgentInstallError> {
    if name == "agents" {
        return Ok(AgentTarget {
            name: name.to_owned(),
            scope: AgentScope::Project,
            base: roots.cwd.join(".agents"),
        });
    }
    let marker = match name {
        "claude" => ".claude",
        "codex" => ".codex",
        _ => {
            return Err(AgentInstallError::UnknownTarget {
                name: name.to_owned(),
            });
        }
    };
    let (scope, root) = if project {
        (AgentScope::Project, roots.cwd.as_path())
    } else {
        (
            AgentScope::User,
            roots
                .home
                .as_deref()
                .ok_or(AgentInstallError::UserDirectoryUnavailable)?,
        )
    };
    Ok(AgentTarget {
        name: name.to_owned(),
        scope,
        base: root.join(marker),
    })
}

fn add_existing_target(
    candidates: &mut Vec<AgentTarget>,
    name: &str,
    scope: AgentScope,
    base: PathBuf,
    is_directory: &impl Fn(&Path) -> bool,
) {
    if is_directory(&base) {
        candidates.push(AgentTarget {
            name: name.to_owned(),
            scope,
            base,
        });
    }
}

fn expand_current_user(path: &Path, home: Option<&Path>) -> Result<PathBuf, AgentInstallError> {
    let mut components = path.components();
    if !matches!(components.next(), Some(Component::Normal(first)) if first == "~") {
        return Ok(path.to_path_buf());
    }
    let home = home.ok_or(AgentInstallError::UserDirectoryUnavailable)?;
    Ok(components.fold(home.to_path_buf(), |expanded, part| expanded.join(part)))
}
