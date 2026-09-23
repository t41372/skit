use std::{
    collections::BTreeSet,
    fs,
    fs::File,
    io,
    path::{Path, PathBuf},
};

use skit_application::RepositoryError;

use super::atomic::io_error;

/// One low-level Agent Skill installation checkpoint.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AgentSkillInstallPoint {
    /// Before the installer inspects one possible symbolic-link path.
    Inspect,
    /// Before the installer reads one confirmed symbolic link.
    ReadLink,
    /// Before the installer tries to create one missing parent directory.
    BeforeCreateDirectory,
    /// After the temporary file is durable and before it replaces the target.
    BeforeReplace,
}

/// Install the bundled Agent Skill through the filesystem adapter's atomic write discipline.
#[derive(Clone, Copy, Debug, Default)]
pub struct FileAgentSkillStore;

impl FileAgentSkillStore {
    /// Write `bytes` as `<skills_dir>/skit/SKILL.md` and return that logical path.
    ///
    /// Reinstallation is an atomic replacement. If the final file is a symbolic link, skit
    /// preserves the link and updates its target, matching the operating system behavior of the
    /// Python implementation's ordinary file write.
    pub fn install(self, skills_dir: &Path, bytes: &[u8]) -> Result<PathBuf, RepositoryError> {
        self.install_with_checkpoint(skills_dir, bytes, |_, _| Ok(()))
    }

    /// Install through the production pipeline with a raw low-level fault checkpoint.
    pub fn install_with_checkpoint(
        self,
        skills_dir: &Path,
        bytes: &[u8],
        mut checkpoint: impl FnMut(AgentSkillInstallPoint, &Path) -> io::Result<()>,
    ) -> Result<PathBuf, RepositoryError> {
        let destination = skills_dir.join("skit").join("SKILL.md");
        let write_path = follow_final_symlink(&destination, &mut checkpoint)?;
        let created_directories = create_parent_directories(&write_path, &mut checkpoint)?;
        let outcome = crate::fs_ops::atomic_write_bytes_with_before_replace(
            &write_path,
            bytes,
            io_error,
            File::sync_all,
            |path| checkpoint(AgentSkillInstallPoint::BeforeReplace, path),
        );
        if let Err(error) = outcome {
            remove_created_empty_directories(&created_directories);
            return Err(error);
        }
        Ok(destination)
    }
}

fn follow_final_symlink(
    path: &Path,
    checkpoint: &mut impl FnMut(AgentSkillInstallPoint, &Path) -> io::Result<()>,
) -> Result<PathBuf, RepositoryError> {
    let mut current = path.to_path_buf();
    let mut visited = BTreeSet::new();
    loop {
        checkpoint(AgentSkillInstallPoint::Inspect, &current)
            .map_err(|error| io_error("inspect", &current, error))?;
        let metadata = match fs::symlink_metadata(&current) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(current),
            Err(error) => return Err(io_error("inspect", &current, error)),
        };
        if !metadata.file_type().is_symlink() {
            return Ok(current);
        }
        if !visited.insert(current.clone()) {
            return Err(RepositoryError::Io {
                operation: "resolve",
                path: path.display().to_string(),
                reason: "symbolic link cycle".to_owned(),
            });
        }
        checkpoint(AgentSkillInstallPoint::ReadLink, &current)
            .map_err(|error| io_error("read link", &current, error))?;
        let target =
            fs::read_link(&current).map_err(|error| io_error("read link", &current, error))?;
        current = if target.is_absolute() {
            target
        } else {
            current
                .parent()
                .expect("an Agent Skill file path has a parent")
                .join(target)
        };
    }
}

struct OwnedDirectory {
    path: PathBuf,
    identity: same_file::Handle,
}

fn create_parent_directories(
    path: &Path,
    checkpoint: &mut impl FnMut(AgentSkillInstallPoint, &Path) -> io::Result<()>,
) -> Result<Vec<OwnedDirectory>, RepositoryError> {
    let mut ancestors = path
        .parent()
        .into_iter()
        .flat_map(Path::ancestors)
        .collect::<Vec<_>>();
    ancestors.reverse();
    let mut owned = Vec::new();
    for directory in ancestors {
        if directory.as_os_str().is_empty() {
            continue;
        }
        match fs::metadata(directory) {
            Ok(metadata) if metadata.is_dir() => continue,
            Ok(_) => {
                remove_created_empty_directories(&owned);
                return Err(io_error(
                    "create",
                    directory,
                    io::Error::new(io::ErrorKind::AlreadyExists, "the path is not a directory"),
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                remove_created_empty_directories(&owned);
                return Err(io_error("inspect", directory, error));
            }
        }
        if let Err(error) = checkpoint(AgentSkillInstallPoint::BeforeCreateDirectory, directory) {
            remove_created_empty_directories(&owned);
            return Err(io_error("create", directory, error));
        }
        match fs::create_dir(directory) {
            Ok(()) => match same_file::Handle::from_path(directory) {
                Ok(identity) => owned.push(OwnedDirectory {
                    path: directory.to_path_buf(),
                    identity,
                }),
                Err(error) => {
                    let _ = fs::remove_dir(directory);
                    remove_created_empty_directories(&owned);
                    return Err(io_error("inspect", directory, error));
                }
            },
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                if !fs::metadata(directory).is_ok_and(|metadata| metadata.is_dir()) {
                    remove_created_empty_directories(&owned);
                    return Err(io_error("create", directory, error));
                }
            }
            Err(error) => {
                remove_created_empty_directories(&owned);
                return Err(io_error("create", directory, error));
            }
        }
    }
    Ok(owned)
}

fn remove_created_empty_directories(directories: &[OwnedDirectory]) {
    for directory in directories.iter().rev() {
        let Ok(current) = same_file::Handle::from_path(&directory.path) else {
            break;
        };
        if current != directory.identity || fs::remove_dir(&directory.path).is_err() {
            break;
        }
    }
}
