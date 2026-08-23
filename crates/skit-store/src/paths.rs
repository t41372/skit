//! Resolve entry payload paths from the v0.4 data layout.

use std::{
    borrow::Cow,
    fs,
    path::{Path, PathBuf},
};

use skit_application::{RepositoryError, canonical_stored_filename};
use skit_domain::{Entry, Slug, StorageMode};
use skit_i18n::Message;

use crate::FileStore;

/// Expand the current user's leading `~` with the platform home-directory adapter.
///
/// Other text stays byte-for-byte unchanged. Environment-variable expansion is not part of this
/// path contract.
#[must_use]
pub fn expand_user_path(path: &Path) -> PathBuf {
    path.to_str().map_or_else(
        || path.to_path_buf(),
        |value| PathBuf::from(expand_leading_tilde(value).as_ref()),
    )
}

/// Expand a leading `~` with the home directory this host names.
#[cfg(not(windows))]
fn expand_leading_tilde(value: &str) -> Cow<'_, str> {
    shellexpand::tilde(value)
}

/// Expand a leading `~` the way version 0.4 expands it on this host.
///
/// Version 0.4 expands with CPython's `os.path.expanduser`. Its Windows form reads USERPROFILE, and
/// then HOMEDRIVE together with HOMEPATH, from the environment. The default adapter asks the shell
/// for the profile folder instead, and no environment can redirect that answer, so a caller that
/// names a home directory was ignored. Read what version 0.4 reads first, and keep the shell answer
/// as the last resort.
#[cfg(windows)]
fn expand_leading_tilde(value: &str) -> Cow<'_, str> {
    windows_home().map_or_else(
        || shellexpand::tilde(value),
        |home| shellexpand::tilde_with_context(value, || Some(home)),
    )
}

/// The home directory the environment names, in the order version 0.4 reads it.
#[cfg(windows)]
fn windows_home() -> Option<String> {
    fn named(variable: &str) -> Option<String> {
        std::env::var(variable)
            .ok()
            .filter(|value| !value.is_empty())
    }

    named("USERPROFILE").or_else(|| Some(format!("{}{}", named("HOMEDRIVE")?, named("HOMEPATH")?)))
}

/// Return the stored copy name for a known entry kind.
#[must_use]
pub fn stored_filename(kind: &str) -> Option<&'static str> {
    canonical_stored_filename(kind)
}

/// Return all payload filenames accepted for one known entry kind.
#[must_use]
pub fn stored_filenames(kind: &str) -> &'static [&'static str] {
    match kind {
        "js" => &["script.js", "script.mjs", "script.cjs"],
        "ts" => &["script.ts", "script.mts", "script.cts"],
        "python" => &["script.py"],
        "shell" => &["script.sh"],
        "fish" => &["script.fish"],
        "powershell" => &["script.ps1"],
        "ruby" => &["script.rb"],
        "perl" => &["script.pl"],
        "lua" => &["script.lua"],
        "r" => &["script.r"],
        "prompt" => &["prompt.md"],
        "exe" | "command" => &[],
        _ => &["payload"],
    }
}

impl FileStore {
    /// Return the directory that owns one entry.
    #[must_use]
    pub fn entry_dir_path(&self, slug: &Slug) -> PathBuf {
        self.data_dir().join("scripts").join(slug.as_str())
    }

    /// Return the current launch or edit payload path.
    pub fn payload_path(&self, entry: &Entry) -> Result<PathBuf, RepositoryError> {
        if entry.meta.kind.as_str() == "exe" || entry.meta.mode == StorageMode::Reference {
            return Ok(PathBuf::from(&entry.meta.source));
        }

        let directory = self.entry_dir_path(&entry.slug);
        for name in stored_filenames(entry.meta.kind.as_str()) {
            let path = directory.join(name);
            if path.is_file() {
                return Ok(path);
            }
        }

        let reader =
            fs::read_dir(&directory).map_err(|error| io_error("scan", &directory, error))?;
        let mut files = Vec::new();
        for item in reader {
            let item = item.map_err(|error| io_error("scan", &directory, error))?;
            if is_support_file(&item.file_name().to_string_lossy()) {
                continue;
            }
            let file_type = item
                .file_type()
                .map_err(|error| io_error("inspect", &item.path(), error))?;
            if file_type.is_file() {
                files.push(item.path());
            }
        }
        files.sort();
        match files.as_slice() {
            [path] => Ok(path.clone()),
            [] => Err(RepositoryError::InvalidMutation {
                reason: Message::new("copy entry has no stored payload"),
            }),
            _ => Err(RepositoryError::InvalidMutation {
                reason: Message::new("copy entry has more than one possible stored payload"),
            }),
        }
    }
}

/// Whether skit itself owns this name inside an entry directory.
///
/// A stored payload is never one of these, so the payload scan must skip them. Missing one
/// makes a private file look like a second payload and blocks every launch of that entry.
pub(crate) fn is_support_file(name: &str) -> bool {
    matches!(
        name,
        "meta.toml" | "package.json" | "package-lock.json" | "bun.lock" | "bun.lockb" | "deno.lock"
    )
        // the dependency stamp, its crash backup, and its staging directories
        || name.starts_with(".skit-deps")
        // one run's staged injected source
        || name.starts_with(".run-")
        || name.starts_with(".injected-")
        // an atomic replacement sibling left by an interrupted write
        || (name.starts_with('.') && name.ends_with(".tmp"))
}

fn io_error(
    operation: &'static str,
    path: &std::path::Path,
    error: std::io::Error,
) -> RepositoryError {
    RepositoryError::Io {
        operation,
        path: path.display().to_string(),
        reason: error.to_string(),
    }
}
