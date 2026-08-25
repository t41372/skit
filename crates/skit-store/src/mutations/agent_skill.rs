use std::{
    collections::BTreeSet,
    fs, io,
    path::{Path, PathBuf},
};

use skit_application::RepositoryError;

use super::atomic::{atomic_write_bytes, io_error};

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
        let destination = skills_dir.join("skit").join("SKILL.md");
        let write_path = follow_final_symlink(&destination)?;
        atomic_write_bytes(&write_path, bytes)?;
        Ok(destination)
    }
}

fn follow_final_symlink(path: &Path) -> Result<PathBuf, RepositoryError> {
    let mut current = path.to_path_buf();
    let mut visited = BTreeSet::new();
    loop {
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
