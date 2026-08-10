use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write as _},
    path::{Path, PathBuf},
};

#[cfg(unix)]
use std::fs::Permissions;

use skit_application::{EntryPayload, RepositoryError, SourcePermissions};
use skit_domain::{EntryId, EntryMeta};
use skit_i18n::Message;

use crate::fs_ops::preserve_permissions_best_effort;

#[derive(Debug)]
pub(super) struct FileLock {
    _file: File,
}

pub(super) fn acquire_lock(path: &Path) -> Result<FileLock, RepositoryError> {
    let file = open_lock_file(path)?;
    file.lock().map_err(|error| io_error("lock", path, error))?;
    Ok(FileLock { _file: file })
}

pub(super) fn acquire_shared_lock(path: &Path) -> Result<FileLock, RepositoryError> {
    let file = open_lock_file(path)?;
    file.lock_shared()
        .map_err(|error| io_error("lock", path, error))?;
    Ok(FileLock { _file: file })
}

fn open_lock_file(path: &Path) -> Result<File, RepositoryError> {
    let parent = path
        .parent()
        .ok_or_else(|| invalid(Message::new("lock path has no parent directory")))?;
    create_dir_all(parent, "create")?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|error| io_error("open", path, error))?;
    if file
        .metadata()
        .map_err(|error| io_error("inspect", path, error))?
        .len()
        == 0
    {
        file.set_len(1)
            .map_err(|error| io_error("initialize", path, error))?;
    }
    Ok(file)
}

#[derive(Debug)]
pub(super) struct StagedDirectory {
    path: PathBuf,
    committed: bool,
}

impl StagedDirectory {
    pub(super) fn new(path: PathBuf) -> Self {
        Self {
            path,
            committed: false,
        }
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    pub(super) fn commit(&mut self) {
        self.committed = true;
    }
}

impl Drop for StagedDirectory {
    fn drop(&mut self) {
        if !self.committed {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

pub(super) fn write_new_file(path: &Path, payload: &EntryPayload) -> Result<(), RepositoryError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| io_error("create", path, error))?;
    file.write_all(&payload.bytes)
        .map_err(|error| io_error("write", path, error))?;
    apply_permissions(&file, payload.permissions, path)?;
    file.sync_all()
        .map_err(|error| io_error("sync", path, error))
}

pub(super) fn write_new_metadata(path: &Path, meta: &EntryMeta) -> Result<(), RepositoryError> {
    let text = toml::to_string_pretty(meta)
        .map_err(|error| invalid(Message::new("could not encode metadata: {}").with(error)))?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| io_error("create", path, error))?;
    file.write_all(text.as_bytes())
        .map_err(|error| io_error("write", path, error))?;
    file.sync_all()
        .map_err(|error| io_error("sync", path, error))
}

#[cfg(unix)]
fn apply_permissions(
    file: &File,
    source: SourcePermissions,
    path: &Path,
) -> Result<(), RepositoryError> {
    use std::os::unix::fs::PermissionsExt;

    let mode = source
        .unix_mode
        .unwrap_or(if source.readonly { 0o400 } else { 0o600 });
    file.set_permissions(Permissions::from_mode(mode & 0o777))
        .map_err(|error| io_error("chmod", path, error))
}

#[cfg(not(unix))]
fn apply_permissions(
    file: &File,
    source: SourcePermissions,
    path: &Path,
) -> Result<(), RepositoryError> {
    let mut permissions = file
        .metadata()
        .map_err(|error| io_error("inspect", path, error))?
        .permissions();
    permissions.set_readonly(source.readonly);
    file.set_permissions(permissions)
        .map_err(|error| io_error("chmod", path, error))
}

pub(super) fn atomic_write_bytes(path: &Path, bytes: &[u8]) -> Result<(), RepositoryError> {
    let parent = path
        .parent()
        .ok_or_else(|| invalid(Message::new("write path has no parent directory")))?;
    create_dir_all(parent, "create")?;
    let temp = unique_sibling(path, "tmp")?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(|error| io_error("create", &temp, error))?;
    file.write_all(bytes)
        .map_err(|error| io_error("write", &temp, error))?;
    preserve_permissions_best_effort(
        fs::metadata(path).map(|metadata| metadata.permissions()),
        |permissions| file.set_permissions(permissions),
    );
    file.sync_all()
        .map_err(|error| io_error("sync", &temp, error))?;
    drop(file);

    let result = fs::rename(&temp, path).map_err(|error| io_error("replace", path, error));
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    } else {
        let _ = sync_directory(parent);
    }
    result
}

#[cfg(unix)]
pub(super) fn sync_directory(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

#[cfg(not(unix))]
pub(super) fn sync_directory(_path: &Path) -> io::Result<()> {
    Ok(())
}

fn unique_sibling(path: &Path, suffix: &str) -> Result<PathBuf, RepositoryError> {
    let parent = path
        .parent()
        .ok_or_else(|| invalid(Message::new("temporary path has no parent directory")))?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("entry");
    Ok(parent.join(format!(
        ".{name}.{}.{}",
        EntryId::generate().as_str(),
        suffix
    )))
}

pub(super) fn create_dir_all(path: &Path, operation: &'static str) -> Result<(), RepositoryError> {
    fs::create_dir_all(path).map_err(|error| io_error(operation, path, error))
}

pub(super) fn invalid(reason: Message) -> RepositoryError {
    RepositoryError::InvalidMutation { reason }
}

pub(super) fn io_error(operation: &'static str, path: &Path, error: io::Error) -> RepositoryError {
    RepositoryError::Io {
        operation,
        path: path.display().to_string(),
        reason: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn a_failed_atomic_replace_removes_its_temporary_file() {
        let root = TempDir::new().unwrap();
        let target = root.path().join("target");
        fs::create_dir(&target).unwrap();

        assert!(atomic_write_bytes(&target, b"value").is_err());
        let names = fs::read_dir(root.path())
            .unwrap()
            .map(|item| item.unwrap().file_name())
            .collect::<Vec<_>>();
        assert_eq!(names, ["target"]);
    }

    #[test]
    fn a_real_directory_can_be_synchronized_after_a_rename() {
        let root = TempDir::new().unwrap();
        sync_directory(root.path()).unwrap();
    }
}
