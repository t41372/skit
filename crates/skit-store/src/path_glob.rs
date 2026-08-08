//! Expand path patterns for launch inputs.

use std::path::{Component, Path, PathBuf};

use glob::{MatchOptions, glob_with};
use skit_application::glob_expansion::GlobExpander;

/// Match glob patterns in one launch directory.
/// This type does not change the process working directory.
#[derive(Clone, Debug)]
pub struct FileGlobExpander {
    cwd: PathBuf,
}

impl FileGlobExpander {
    /// Create a matcher for one launch directory.
    #[must_use]
    pub fn new(cwd: impl Into<PathBuf>) -> Self {
        Self { cwd: cwd.into() }
    }

    /// Return the launch directory.
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
            require_literal_leading_dot: false,
        };
        let Ok(paths) = glob_with(&pattern, options) else {
            return vec![piece.to_owned()];
        };

        let mut matches = paths
            .filter_map(Result::ok)
            .filter(|path| hidden_segments_are_explicit(&pattern_path, path))
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
        matches.sort();

        if matches.is_empty() {
            vec![piece.to_owned()]
        } else {
            matches
        }
    }
}

fn hidden_segments_are_explicit(pattern: &Path, candidate: &Path) -> bool {
    let pattern = text_components(pattern);
    let candidate = text_components(candidate);
    hidden_parts_match(&pattern, &candidate)
}

fn hidden_parts_match(pattern: &[String], candidate: &[String]) -> bool {
    match (pattern.split_first(), candidate.split_first()) {
        (None, None) => true,
        (None, Some(_)) => false,
        (Some((part, rest)), _) if part == "**" => {
            hidden_parts_match(rest, candidate)
                || candidate.first().is_some_and(|value| {
                    !value.starts_with('.') && hidden_parts_match(pattern, &candidate[1..])
                })
        }
        (Some((part, rest)), Some((value, values))) => {
            (!value.starts_with('.') || part.starts_with('.')) && hidden_parts_match(rest, values)
        }
        (Some(_), None) => false,
    }
}

fn text_components(path: &Path) -> Vec<String> {
    path.components()
        .map(|component| match component {
            Component::Normal(value) => value.to_string_lossy().into_owned(),
            Component::Prefix(value) => value.as_os_str().to_string_lossy().into_owned(),
            Component::RootDir => std::path::MAIN_SEPARATOR.to_string(),
            Component::CurDir => ".".to_owned(),
            Component::ParentDir => "..".to_owned(),
        })
        .collect()
}
