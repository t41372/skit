//! One temporary directory that resolves its path when it is made.
//!
//! The oracle's pytest `tmp_path` is already resolved, so every oracle fixture compared a resolved
//! path with a resolved path. A Rust `TempDir` keeps the spelling of `$TMPDIR`, and on macOS that
//! spelling goes through the `/var` link to `/private/var`. A port that derives its expectations
//! from an unresolved root therefore compares two different spellings of one directory.
//!
//! Resolving the root once, before any path comes off it, gives the tests the same ground the
//! oracle had: inside a sandbox the given spelling and the resolved spelling are the same on every
//! platform.

use std::{
    fs,
    path::{Path, PathBuf},
};

use tempfile::TempDir;

/// A sandbox root directory with one spelling.
pub(crate) struct TempRoot {
    /// Kept alive only to remove the directory when the test ends.
    _guard: TempDir,
    resolved: PathBuf,
}

impl TempRoot {
    pub(crate) fn new() -> Self {
        let guard = TempDir::new().unwrap();
        let resolved = fs::canonicalize(guard.path()).unwrap();
        Self {
            _guard: guard,
            resolved,
        }
    }

    pub(crate) fn path(&self) -> &Path {
        &self.resolved
    }
}
