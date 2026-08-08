use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write as _},
    path::Path,
};

use skit_domain::EntryId;

#[derive(Debug)]
pub(crate) struct FileLock {
    _file: File,
}

pub(crate) fn acquire_lock(path: &Path) -> io::Result<FileLock> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "lock path has no parent"))?;
    fs::create_dir_all(parent)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    if file.metadata()?.len() == 0 {
        file.set_len(1)?;
    }
    file.lock()?;
    Ok(FileLock { _file: file })
}

pub(crate) fn atomic_write_bytes(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "write path has no parent"))?;
    fs::create_dir_all(parent)?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("state");
    let temp = parent.join(format!(
        ".{name}.{}.tmp",
        EntryId::generate().as_str()
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)?;
    file.write_all(bytes)?;
    if let Ok(metadata) = fs::metadata(path) {
        file.set_permissions(metadata.permissions())?;
    }
    file.sync_all()?;
    drop(file);

    if let Err(error) = fs::rename(&temp, path) {
        let _ = fs::remove_file(&temp);
        return Err(error);
    }
    Ok(())
}
