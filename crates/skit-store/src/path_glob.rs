//! Filesystem-backed glob expansion for launch-time multi values and raw argument tails.

use std::path::{Path, PathBuf};

use glob::{MatchOptions, glob_with};
use skit_application::glob_expansion::GlobExpander;

/// Glob matcher scoped to one launch working directory without changing process-global cwd.
#[derive(Clone, Debug)]
pub struct FileGlobExpander {
    cwd: PathBuf,
}

impl FileGlobExpander {
    /// Match relative patterns beneath `cwd`.
    #[must_use]
    pub fn new(cwd: impl Into<PathBuf>) -> Self {
        Self { cwd: cwd.into() }
    }

    /// Return the configured launch working directory.
    #[must_use]
    pub fn cwd(&self) -> &Path {
        &self.cwd
    }
}

impl GlobExpander for FileGlobExpander {
    fn expand_piece(&self, piece: &str) -> Vec<String> {
        if !piece.contains(['*', '?', '[']) {
            return vec![piece.to_owned()];
        }

        let input = Path::new(piece);
        let absolute = input.is_absolute();
        let pattern_path = if absolute {
            input.to_path_buf()
        } else {
            self.cwd.join(input)
        };
        let pattern = pattern_path.to_string_lossy();
        let options = MatchOptions {
            case_sensitive: !cfg!(windows),
            require_literal_separator: true,
            require_literal_leading_dot: true,
        };
        let Ok(paths) = glob_with(&pattern, options) else {
            return vec![piece.to_owned()];
        };

        let matches = paths
            .filter_map(Result::ok)
            .map(|path| {
                if absolute {
                    path
                } else {
                    path.strip_prefix(&self.cwd)
                        .map_or(path.clone(), Path::to_path_buf)
                }
            })
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>();

        if matches.is_empty() {
            vec![piece.to_owned()]
        } else {
            matches
        }
    }
}
