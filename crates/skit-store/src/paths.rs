//! Resolve entry payload paths from the v0.4 data layout.

use std::{fs, path::PathBuf};

use skit_application::RepositoryError;
use skit_domain::{Entry, Slug, StorageMode};
use skit_i18n::Message;

use crate::FileStore;

/// Return the stored copy name for a known entry kind.
#[must_use]
pub fn stored_filename(kind: &str) -> Option<&'static str> {
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
        if entry.meta.mode == StorageMode::Reference {
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
