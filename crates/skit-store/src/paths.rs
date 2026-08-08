//! Resolve entry payload paths from the v0.4 data layout.

use std::{fs, path::PathBuf};

use skit_application::RepositoryError;
use skit_domain::{Entry, Slug, StorageMode};

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
        if let Some(name) = stored_filename(entry.meta.kind.as_str()) {
            let path = directory.join(name);
            if path.is_file() {
                return Ok(path);
            }
        }

        let reader = fs::read_dir(&directory).map_err(|error| io_error("scan", &directory, error))?;
        let mut files = Vec::new();
        for item in reader {
            let item = item.map_err(|error| io_error("scan", &directory, error))?;
            if item.file_name().to_string_lossy() == "meta.toml" {
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
                reason: "copy entry has no stored payload".to_owned(),
            }),
            _ => Err(RepositoryError::InvalidMutation {
                reason: "copy entry has more than one possible stored payload".to_owned(),
            }),
        }
    }
}

fn io_error(operation: &'static str, path: &std::path::Path, error: std::io::Error) -> RepositoryError {
    RepositoryError::Io {
        operation,
        path: path.display().to_string(),
        reason: error.to_string(),
    }
}
