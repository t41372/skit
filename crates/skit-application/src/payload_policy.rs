//! Frontend-neutral add-time payload and working-directory policy.

use std::path::Path;

use skit_domain::{EntryKind, StorageMode};

const DEFAULT_WINDOWS_PATHEXT: &str = ".COM;.EXE;.BAT;.CMD";

/// Host executable convention used for add-time source inference.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecutableDialect<'a> {
    /// A real file is executable when any Unix execute bit is set.
    Posix,
    /// A real file is executable when its suffix occurs in PATHEXT.
    Windows {
        /// The host PATHEXT value. Unset and empty values use the Windows default.
        pathext: Option<&'a str>,
    },
}

/// Complete filesystem and host facts for direct-executable inference.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExecutableSourceFacts<'a> {
    /// Source path whose final suffix is inspected on Windows.
    pub path: &'a Path,
    /// Whether the resolved source is a regular file.
    pub is_file: bool,
    /// Unix permission bits when the host supplies them.
    pub unix_mode: Option<u32>,
    /// Host executable convention.
    pub dialect: ExecutableDialect<'a>,
}

/// Return whether one real source file can be inferred as a direct executable.
///
/// Language extension and shebang classification stays in `skit-language` and must run before
/// this fallback. This policy uses only facts supplied by a filesystem adapter.
#[must_use]
pub fn source_is_executable(facts: ExecutableSourceFacts<'_>) -> bool {
    if !facts.is_file {
        return false;
    }
    match facts.dialect {
        ExecutableDialect::Posix => facts.unix_mode.is_some_and(|mode| mode & 0o111 != 0),
        ExecutableDialect::Windows { pathext } => {
            let pathext = pathext
                .filter(|value| !value.is_empty())
                .unwrap_or(DEFAULT_WINDOWS_PATHEXT);
            let Some(extension) = facts.path.extension().and_then(|value| value.to_str()) else {
                return false;
            };
            let extension = format!(".{extension}");
            pathext
                .split(';')
                .filter(|candidate| !candidate.is_empty())
                .any(|candidate| candidate.eq_ignore_ascii_case(&extension))
        }
    }
}

/// One entry kind that an explicit `add --kind` request can author.
///
/// Prompt files and command entries use dedicated authoring lanes. The stdin composition keeps a
/// version 0.4 `--kind prompt` compatibility alias. Stored [`EntryKind`] values stay open so a
/// newer skit version's metadata remains readable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ForcedAddKind {
    /// fish source.
    Fish,
    /// JavaScript source.
    JavaScript,
    /// Lua source.
    Lua,
    /// Perl source.
    Perl,
    /// PowerShell source.
    PowerShell,
    /// Python source.
    Python,
    /// R source.
    R,
    /// Ruby source.
    Ruby,
    /// POSIX-family shell source.
    Shell,
    /// TypeScript source.
    TypeScript,
    /// Directly launched program.
    Executable,
}

impl ForcedAddKind {
    /// Every forceable kind in the version 0.4 presentation order.
    pub const ALL: &'static [Self] = &[
        Self::Fish,
        Self::JavaScript,
        Self::Lua,
        Self::Perl,
        Self::PowerShell,
        Self::Python,
        Self::R,
        Self::Ruby,
        Self::Shell,
        Self::TypeScript,
        Self::Executable,
    ];

    /// Parse one exact authoring spelling.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|kind| kind.as_str() == value)
    }

    /// Return the stable registry spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Fish => "fish",
            Self::JavaScript => "js",
            Self::Lua => "lua",
            Self::Perl => "perl",
            Self::PowerShell => "powershell",
            Self::Python => "python",
            Self::R => "r",
            Self::Ruby => "ruby",
            Self::Shell => "shell",
            Self::TypeScript => "ts",
            Self::Executable => "exe",
        }
    }
}

/// Return the canonical stored copy name for one kind.
///
/// JavaScript and TypeScript callers can use [`payload_stored_name`] to keep a module-specific
/// extension from the source path.
#[must_use]
pub fn canonical_stored_filename(kind: &str) -> Option<&'static str> {
    match kind {
        "python" => Some("script.py"),
        "shell" => Some("script.sh"),
        "js" => Some("script.js"),
        "ts" => Some("script.ts"),
        "fish" => Some("script.fish"),
        "powershell" => Some("script.ps1"),
        "ruby" => Some("script.rb"),
        "perl" => Some("script.pl"),
        "lua" => Some("script.lua"),
        "r" => Some("script.r"),
        "prompt" => Some("prompt.md"),
        "exe" | "command" => None,
        _ => Some("payload"),
    }
}

/// Choose the stored payload name for one exact source path.
#[must_use]
pub fn payload_stored_name(kind: &EntryKind, source: &Path) -> String {
    if matches!(kind.as_str(), "js" | "ts")
        && let Some(extension) = source.extension().and_then(|value| value.to_str())
    {
        let extension = extension.to_ascii_lowercase();
        if matches!(
            extension.as_str(),
            "js" | "mjs" | "cjs" | "ts" | "mts" | "cts"
        ) {
            return format!("script.{extension}");
        }
    }
    canonical_stored_filename(kind.as_str()).map_or_else(
        || {
            source
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("script")
                .to_owned()
        },
        str::to_owned,
    )
}

/// Return the latest-main default work-directory policy for an add request.
#[must_use]
pub fn add_workdir(kind: &EntryKind, mode: StorageMode) -> &'static str {
    match kind.as_str() {
        "prompt" | "command" => "invoke",
        "exe" => "origin",
        _ if mode == StorageMode::Reference => "origin",
        _ => "invoke",
    }
}

/// Report whether one known kind lets the user choose copy or reference storage.
///
/// Unknown kinds stay open-ended, but this version cannot promise their storage semantics.
#[must_use]
pub fn supports_storage_modes(kind: &EntryKind) -> bool {
    matches!(
        kind.as_str(),
        "python"
            | "shell"
            | "fish"
            | "js"
            | "ts"
            | "powershell"
            | "ruby"
            | "perl"
            | "lua"
            | "r"
            | "prompt"
    )
}
