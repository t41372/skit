use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write as _},
    path::Path,
    time::Duration,
};

use skit_domain::EntryId;

// Windows can't replace a file another handle has open (sharing violation -> PermissionError);
// concurrent readers of registry.toml hold it for microseconds, so a bounded exponential backoff is
// the standard idiom (total worst-case wait ~= 1.3 s). POSIX replaces open files freely, so this
// path never fires there. Faithful translation of skit.atomic._REPLACE_RETRIES / _replace_with_retry.
const REPLACE_RETRIES: u32 = 7; // sleeps: 0.01 · 0.02 · 0.04 · 0.08 · 0.16 · 0.32 · 0.64 s
const REPLACE_BACKOFF_START: Duration = Duration::from_millis(10);

/// Injectable core of [`replace_with_retry`]: `rename` and `sleep` stand in for `fs::rename` and
/// `std::thread::sleep`, so a Linux test can drive the Windows-only sharing-violation retry/backoff
/// that the real calls otherwise never trigger on POSIX. This mirrors the oracle's
/// `monkeypatch.setattr(atomic.os, "replace", ...)` / `monkeypatch.setattr(atomic.time, "sleep", ...)`
/// (test_atomic.py ~552-608). Exposed `#[doc(hidden)] pub` (like `content_hash`) purely as that test
/// seam; it is not part of the documented API.
///
/// A transient `PermissionDenied` (the Windows sharing violation) is retried with exponential
/// backoff; after the bounded retries are exhausted the final attempt's error propagates -- a target
/// held open indefinitely (antivirus, an actual leak) must stay loud. Any other error is immediate.
#[doc(hidden)]
pub fn replace_with_retry_impl(
    mut rename: impl FnMut(&Path, &Path) -> io::Result<()>,
    mut sleep: impl FnMut(Duration),
    src: &Path,
    dst: &Path,
) -> io::Result<()> {
    let mut delay = REPLACE_BACKOFF_START;
    for _ in 0..REPLACE_RETRIES {
        match rename(src, dst) {
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
                sleep(delay);
                delay *= 2;
            }
            other => return other,
        }
    }
    rename(src, dst)
}

/// Atomic replacement that rides out transient Windows sharing violations. After the retries are
/// exhausted, the final attempt's error propagates -- a target held open indefinitely (antivirus,
/// an actual leak) must stay loud.
fn replace_with_retry(src: &Path, dst: &Path) -> io::Result<()> {
    replace_with_retry_impl(atomicwrites::replace_atomic, std::thread::sleep, src, dst)
}

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

/// `acquire_lock` that never waits: `Some(FileLock)` holding the lock, or `None` holding nothing.
///
/// For a write that rides on a READ path -- the registry self-heal under `list`, say -- where the
/// write is optional and the read's latency is not. `acquire_lock`'s blocking `File::lock` polls
/// forever, so a read that used it would freeze shell TAB completion behind any process on the lock
/// (a large add, a hung skit); a read that uses this one skips its optional write and stays a read.
///
/// Faithful translation of `skit.atomic.try_advisory_file_lock`: every failure is "not acquired",
/// never an error the read must handle. Contention (`TryLockError::WouldBlock`), an unopenable lock
/// file, or any other error all yield `None` -- the oracle's `except OSError: native_locked = False`
/// plus `_try_native_lock`'s `EACCES`/`EAGAIN` -> `False`. Rust has no per-process thread mutex, so
/// the kernel `flock` is the whole exclusion; on Linux one process's non-blocking `flock` on a
/// second open file description is still denied by a lock it holds through another, which is what
/// lets a same-process test observe the decline.
pub(crate) fn try_acquire_lock(path: &Path) -> Option<FileLock> {
    let parent = path.parent()?;
    fs::create_dir_all(parent).ok()?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .ok()?;
    if file.metadata().ok()?.len() == 0 {
        file.set_len(1).ok()?;
    }
    match file.try_lock() {
        Ok(()) => Some(FileLock { _file: file }),
        Err(_) => None,
    }
}

pub(crate) fn atomic_write_bytes(path: &Path, bytes: &[u8]) -> io::Result<()> {
    atomic_write_bytes_with(path, bytes, |_, _, error| error, File::sync_all)
}

pub(crate) fn atomic_write_bytes_with<E>(
    path: &Path,
    bytes: &[u8],
    map_error: impl Fn(&'static str, &Path, io::Error) -> E,
    sync_file: impl FnOnce(&File) -> io::Result<()>,
) -> Result<(), E> {
    let parent = path.parent().ok_or_else(|| {
        map_error(
            "write",
            path,
            io::Error::new(io::ErrorKind::InvalidInput, "write path has no parent"),
        )
    })?;
    fs::create_dir_all(parent).map_err(|error| map_error("create", parent, error))?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("state");
    let temp = parent.join(format!(".{name}.{}.tmp", EntryId::generate().as_str()));
    // Any failure after the temp file exists must remove it, not just a rename failure: a
    // write_all or sync_all (fsync) error otherwise leaks a `.tmp` beside the target. The target
    // itself always stays intact -- the rename is the only step that touches it -- so this is a
    // temp-cleanup contract, matching the oracle's atomic writer.
    let outcome = (|| -> Result<(), E> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|error| map_error("create", &temp, error))?;
        file.write_all(bytes)
            .map_err(|error| map_error("write", &temp, error))?;
        preserve_permissions_best_effort(
            fs::metadata(path).map(|metadata| metadata.permissions()),
            |permissions| file.set_permissions(permissions),
        );
        sync_file(&file).map_err(|error| map_error("sync", &temp, error))?;
        drop(file);
        replace_with_retry(&temp, path).map_err(|error| map_error("replace", path, error))
    })();
    if outcome.is_err() {
        let _ = fs::remove_file(&temp);
        return outcome;
    }
    let _ = sync_directory(parent);
    Ok(())
}

pub(crate) fn preserve_permissions_best_effort<F>(
    permissions: io::Result<fs::Permissions>,
    apply: F,
) where
    F: FnOnce(fs::Permissions) -> io::Result<()>,
{
    if let Ok(permissions) = permissions {
        let _ = apply(permissions);
    }
}

#[cfg(unix)]
pub(crate) fn sync_directory(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

#[cfg(not(unix))]
pub(crate) fn sync_directory(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        acquire_lock, atomic_write_bytes, atomic_write_bytes_with,
        preserve_permissions_best_effort, sync_directory, try_acquire_lock,
    };
    use std::{cell::Cell, io};
    use tempfile::TempDir;

    // Port of skit.atomic try_advisory_file_lock tests (test_atomic.py ~653-702), driven at the
    // fs_ops primitive rather than the crate-private read-path seam that uses it.

    #[test]
    fn try_lock_acquires_when_free_and_a_second_taker_declines() {
        // WHY (test_try_lock_acquires_when_free_and_excludes_a_second_taker): a free lock is taken;
        // while it is held a second attempt declines without waiting; once released it is free again.
        // On Linux one process's non-blocking flock on a second open file description is denied by a
        // lock it already holds through the first, so a same-process second taker declines.
        let root = TempDir::new().unwrap();
        let lock = root.path().join("x.lock");

        let held = try_acquire_lock(&lock).expect("a free lock is acquired");
        assert!(
            try_acquire_lock(&lock).is_none(),
            "a second taker declines while the lock is held, never waiting"
        );
        drop(held);
        assert!(
            try_acquire_lock(&lock).is_some(),
            "the lock is free again once fully released"
        );
    }

    #[test]
    fn try_lock_declines_while_the_blocking_lock_is_held() {
        // WHY (test_try_lock_declines_while_the_blocking_lock_is_held): the try-variant must decline a
        // lock the blocking `acquire_lock` holds, where the blocking variant would poll forever.
        let root = TempDir::new().unwrap();
        let lock = root.path().join("x.lock");

        let blocking = acquire_lock(&lock).unwrap();
        assert!(try_acquire_lock(&lock).is_none());
        drop(blocking);
        assert!(try_acquire_lock(&lock).is_some());
    }

    #[test]
    fn try_lock_treats_an_unopenable_path_as_not_acquired() {
        // WHY (test_try_lock_treats_an_unopenable_lock_file_as_not_acquired): an unopenable lock file
        // is "not acquired", never an error. Here the lock's parent is a regular file, so creating
        // the parent directory fails and the try declines.
        let root = TempDir::new().unwrap();
        let blocker = root.path().join("blocker");
        std::fs::write(&blocker, b"file").unwrap();

        assert!(try_acquire_lock(&blocker.join("child.lock")).is_none());
    }

    #[test]
    fn path_validation_and_failed_replacement_leave_no_temporary_file() {
        assert!(acquire_lock(std::path::Path::new("")).is_err());
        assert!(atomic_write_bytes(std::path::Path::new(""), b"value").is_err());

        let root = TempDir::new().unwrap();
        let target = root.path().join("target");
        std::fs::create_dir(&target).unwrap();
        assert!(atomic_write_bytes(&target, b"value").is_err());
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
    }

    #[test]
    fn test_atomic_write_bytes_temp_fsync_failure_still_cleans_up_tmp_file() {
        let root = TempDir::new().unwrap();
        let target = root.path().join("target");
        std::fs::write(&target, b"before").unwrap();

        let result = atomic_write_bytes_with(
            &target,
            b"after",
            |_, _, error| error,
            |_| {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "simulated temp fsync failure",
                ))
            },
        );

        assert!(result.is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"before");
        let names = std::fs::read_dir(root.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>();
        assert_eq!(names, ["target"]);
    }

    #[test]
    fn a_real_directory_can_be_synchronized_after_a_state_replace() {
        let root = TempDir::new().unwrap();
        sync_directory(root.path()).unwrap();
    }

    #[test]
    fn a_permission_restore_failure_does_not_turn_a_committed_write_into_an_error() {
        let root = TempDir::new().unwrap();
        let target = root.path().join("target");
        std::fs::write(&target, b"old").unwrap();
        let attempted = Cell::new(false);

        preserve_permissions_best_effort(
            std::fs::metadata(&target).map(|metadata| metadata.permissions()),
            |_: std::fs::Permissions| {
                attempted.set(true);
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "simulated chmod refusal",
                ))
            },
        );

        assert!(attempted.get());
    }
}
