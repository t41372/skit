use std::env;
use std::path::{Path, PathBuf};

use crate::{Platform, ProgramResolver};

/// Process-executable search policy. The path list and PATHEXT are explicit so tests,
/// CLI, and a future GUI all share one lookup rule without mutating process globals.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgramSearch {
    platform: Platform,
    paths: Vec<PathBuf>,
    pathext: Vec<String>,
}

impl ProgramSearch {
    /// Create an explicit search context.
    #[must_use]
    pub fn new(
        platform: Platform,
        paths: Vec<PathBuf>,
        pathext: impl IntoIterator<Item = String>,
    ) -> Self {
        let pathext = pathext
            .into_iter()
            .filter(|extension| !extension.is_empty())
            .map(|extension| {
                if extension.starts_with('.') {
                    extension
                } else {
                    format!(".{extension}")
                }
            })
            .collect();
        Self {
            platform,
            paths,
            pathext,
        }
    }

    /// Snapshot executable search inputs from the current process.
    #[must_use]
    pub fn from_environment(platform: Platform) -> Self {
        let paths = env::var_os("PATH")
            .map(|value| env::split_paths(&value).collect())
            .unwrap_or_default();
        let pathext = if platform == Platform::Windows {
            env::var("PATHEXT")
                .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_owned())
                .split(';')
                .map(str::to_owned)
                .collect()
        } else {
            Vec::new()
        };
        Self::new(platform, paths, pathext)
    }

    fn candidates(&self, name: &str) -> Vec<PathBuf> {
        let name_path = Path::new(name);
        let has_parent = name_path
            .parent()
            .is_some_and(|parent| !parent.as_os_str().is_empty());
        let bases = if name_path.is_absolute() || has_parent {
            vec![name_path.to_owned()]
        } else {
            self.paths
                .iter()
                .map(|directory| directory.join(name_path))
                .collect()
        };
        if self.platform != Platform::Windows {
            return bases;
        }

        let extensions = if self.pathext.is_empty() {
            vec![
                ".COM".to_owned(),
                ".EXE".to_owned(),
                ".BAT".to_owned(),
                ".CMD".to_owned(),
            ]
        } else {
            self.pathext.clone()
        };
        let existing_extension = name_path.extension().and_then(|value| value.to_str());
        let explicitly_executable = existing_extension.is_some_and(|extension| {
            extensions
                .iter()
                .any(|item| item.trim_start_matches('.').eq_ignore_ascii_case(extension))
        });

        let mut output = Vec::new();
        for base in bases {
            if explicitly_executable {
                output.push(base.clone());
            }
            for extension in &extensions {
                let mut candidate = base.as_os_str().to_os_string();
                candidate.push(extension);
                output.push(PathBuf::from(candidate));
            }
        }
        output
    }
}

impl ProgramResolver for ProgramSearch {
    fn resolve(&self, name: &str) -> Option<PathBuf> {
        self.candidates(name)
            .into_iter()
            .find(|candidate| executable(candidate, self.platform))
    }
}

fn executable(path: &Path, platform: Platform) -> bool {
    if !path.is_file() {
        return false;
    }
    if platform == Platform::Windows {
        return true;
    }
    posix_executable(path)
}

#[cfg(unix)]
fn posix_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    path.metadata()
        .is_ok_and(|metadata| metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn posix_executable(path: &Path) -> bool {
    path.is_file()
}
