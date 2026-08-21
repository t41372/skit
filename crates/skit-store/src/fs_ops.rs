use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write as _},
    path::Path,
    time::Duration,
};

use skit_domain::EntryId;

// Windows cannot replace a file another handle has open (sharing violation -> PermissionError);
// concurrent readers of registry.toml hold it for microseconds, so a bounded exponential backoff is
// the standard idiom (total worst-case wait ~= 1.3 s). POSIX replaces open files freely, so this
// path never fires there. Faithful translation of skit.atomic._REPLACE_RETRIES / _replace_with_retry.
const REPLACE_RETRIES: u32 = 7; // sleeps: 0.01 · 0.02 · 0.04 · 0.08 · 0.16 · 0.32 · 0.64 s
const REPLACE_BACKOFF_START: Duration = Duration::from_millis(10);

/// Core of the replace retry: `rename` and `sleep` stand in for the operating-system operations.
///
/// The atomic writer passes the real replace and sleep functions. Unit tests pass controlled
/// operations through the same writer to verify retry, cleanup, and no-clobber behavior.
///
/// Controlled `rename` and `sleep` operations let a Linux test drive the Windows-only
/// sharing-violation retry/backoff that the real calls otherwise never trigger on POSIX. This
/// mirrors the oracle's
/// `monkeypatch.setattr(atomic.os, "replace", ...)` / `monkeypatch.setattr(atomic.time, "sleep", ...)`
/// (test_atomic.py ~552-608).
///
/// A transient `PermissionDenied` (the Windows sharing violation) is retried with exponential
/// backoff; after the bounded retries are exhausted the final attempt's error propagates -- a target
/// held open indefinitely (antivirus, an actual leak) must stay loud. Any other error is immediate.
pub(crate) fn replace_with_retry_impl(
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

/// Acquire a persistent lock file only when a writer has already created it.
///
/// Read paths use this form so observing an entry does not create skit data. `None` means that no
/// skit writer has used this lock path yet or the filesystem prevents both this reader and a
/// writer from opening or locking it.
pub(crate) fn acquire_existing_lock(path: &Path) -> io::Result<Option<FileLock>> {
    let file = match OpenOptions::new().read(true).write(true).open(path) {
        Ok(file) => file,
        Err(error) if existing_lock_is_unavailable(error.kind()) => return Ok(None),
        Err(error) => return Err(error),
    };
    match file.lock() {
        Ok(()) => Ok(Some(FileLock { _file: file })),
        Err(error) if existing_lock_is_unavailable(error.kind()) => Ok(None),
        Err(error) => Err(error),
    }
}

fn existing_lock_is_unavailable(kind: io::ErrorKind) -> bool {
    matches!(
        kind,
        io::ErrorKind::NotFound
            | io::ErrorKind::PermissionDenied
            | io::ErrorKind::ReadOnlyFilesystem
            | io::ErrorKind::Unsupported
    )
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
    atomic_write_bytes_with_ops(
        path,
        bytes,
        map_error,
        |path| fs::metadata(path).map(|metadata| metadata.permissions()),
        File::set_permissions,
        sync_file,
        atomicwrites::replace_atomic,
        std::thread::sleep,
        sync_directory,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn atomic_write_bytes_with_ops<E>(
    path: &Path,
    bytes: &[u8],
    map_error: impl Fn(&'static str, &Path, io::Error) -> E,
    read_permissions: impl FnOnce(&Path) -> io::Result<fs::Permissions>,
    apply_permissions: impl FnOnce(&File, fs::Permissions) -> io::Result<()>,
    sync_file: impl FnOnce(&File) -> io::Result<()>,
    rename: impl FnMut(&Path, &Path) -> io::Result<()>,
    sleep: impl FnMut(Duration),
    sync_parent: impl FnOnce(&Path) -> io::Result<()>,
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
        preserve_permissions_best_effort(read_permissions(path), |permissions| {
            apply_permissions(&file, permissions)
        });
        sync_file(&file).map_err(|error| map_error("sync", &temp, error))?;
        drop(file);
        replace_with_retry_impl(rename, sleep, &temp, path)
            .map_err(|error| map_error("replace", path, error))
    })();
    if outcome.is_err() {
        let _ = fs::remove_file(&temp);
        return outcome;
    }
    #[cfg(unix)]
    let _ = sync_parent(parent);
    #[cfg(not(unix))]
    drop(sync_parent);
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
        acquire_lock, atomic_write_bytes, atomic_write_bytes_with_ops,
        existing_lock_is_unavailable, preserve_permissions_best_effort, sync_directory,
        try_acquire_lock,
    };
    use std::{
        cell::{Cell, RefCell},
        fs::File,
        io,
        path::Path,
        sync::{Arc, Barrier, mpsc},
        thread,
        time::Duration,
    };
    use tempfile::TempDir;

    // Port of skit.atomic try_advisory_file_lock tests (test_atomic.py ~653-702), driven at the
    // fs_ops primitive rather than the crate-private read-path seam that uses it.

    #[test]
    fn existing_read_lock_degrades_only_when_the_same_writer_lock_cannot_work() {
        for kind in [
            io::ErrorKind::NotFound,
            io::ErrorKind::PermissionDenied,
            io::ErrorKind::ReadOnlyFilesystem,
            io::ErrorKind::Unsupported,
        ] {
            assert!(existing_lock_is_unavailable(kind), "{kind:?}");
        }
        for kind in [
            io::ErrorKind::InvalidInput,
            io::ErrorKind::InvalidData,
            io::ErrorKind::Other,
        ] {
            assert!(!existing_lock_is_unavailable(kind), "{kind:?}");
        }
    }

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
    fn test_registry_lock_serializes_concurrent_holders() {
        let root = TempDir::new().unwrap();
        let path = root.path().join("registry.native.lock");
        let first = acquire_lock(&path).unwrap();
        let checkpoint = Arc::new(Barrier::new(2));
        let worker_checkpoint = Arc::clone(&checkpoint);
        let (attempting_tx, attempting_rx) = mpsc::sync_channel(1);
        let (entered_tx, entered_rx) = mpsc::sync_channel(1);
        let worker = thread::spawn(move || {
            worker_checkpoint.wait();
            attempting_tx.send(()).unwrap();
            let second = acquire_lock(&path).unwrap();
            entered_tx.send(()).unwrap();
            drop(second);
        });

        checkpoint.wait();
        attempting_rx.recv().unwrap();
        assert!(
            entered_rx.recv_timeout(Duration::from_millis(100)).is_err(),
            "the second holder must not enter its critical section"
        );
        drop(first);
        entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        worker.join().unwrap();
    }

    #[test]
    fn test_registry_lock_uses_a_versioned_persistent_native_inode() {
        let root = TempDir::new().unwrap();
        let path = root.path().join("registry.native.lock");

        let first = acquire_lock(&path).unwrap();
        let first_metadata = std::fs::metadata(&path).unwrap();
        assert_eq!(first_metadata.len(), 1);
        drop(first);

        let second = acquire_lock(&path).unwrap();
        let second_metadata = std::fs::metadata(&path).unwrap();
        assert_eq!(second_metadata.len(), 1);
        drop(second);

        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt as _;
            assert_eq!(first_metadata.ino(), second_metadata.ino());
        }
        #[cfg(windows)]
        {
            assert_eq!(std::fs::read(&path).unwrap(), [0]);
        }

        assert!(path.is_file());
        assert!(!root.path().join("registry.lock").exists());
    }

    #[cfg(windows)]
    #[test]
    fn windows_native_lock_blocks_then_resumes_and_keeps_the_sentinel() {
        let root = TempDir::new().unwrap();
        let path = root.path().join("windows.native.lock");
        let first = acquire_lock(&path).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), [0]);

        let worker_path = path.clone();
        let (attempting_tx, attempting_rx) = mpsc::sync_channel(1);
        let (entered_tx, entered_rx) = mpsc::sync_channel(1);
        let worker = thread::spawn(move || {
            attempting_tx.send(()).unwrap();
            let second = acquire_lock(&worker_path).unwrap();
            entered_tx.send(()).unwrap();
            drop(second);
        });

        attempting_rx.recv().unwrap();
        assert!(entered_rx.recv_timeout(Duration::from_millis(100)).is_err());
        drop(first);
        entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        worker.join().unwrap();

        assert!(path.is_file());
        assert_eq!(std::fs::read(path).unwrap(), [0]);
    }

    #[cfg(unix)]
    #[test]
    fn test_advisory_file_lock_is_released_by_kernel_after_process_crash() {
        const LOCK_ENV: &str = "SKIT_ATOMIC_CRASH_LOCK";
        const READY_ENV: &str = "SKIT_ATOMIC_CRASH_READY";

        if let (Some(lock), Some(ready)) = (std::env::var_os(LOCK_ENV), std::env::var_os(READY_ENV))
        {
            let _held = acquire_lock(Path::new(&lock)).unwrap();
            std::fs::write(ready, b"locked").unwrap();
            std::process::exit(23);
        }

        let root = TempDir::new().unwrap();
        let lock = root.path().join("config.lock");
        let ready = root.path().join("child-ready");
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("fs_ops::tests::test_advisory_file_lock_is_released_by_kernel_after_process_crash")
            .arg("--nocapture")
            .env(LOCK_ENV, &lock)
            .env(READY_ENV, &ready)
            .status()
            .unwrap();

        assert_eq!(status.code(), Some(23));
        assert_eq!(std::fs::read(&ready).unwrap(), b"locked");

        let store = crate::FileConfigStore::new(root.path());
        let (done_tx, done_rx) = mpsc::sync_channel(1);
        thread::spawn(move || done_tx.send(store.set("form", "plain")).unwrap());
        done_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("the kernel must release the crashed child's lock immediately")
            .unwrap();

        assert!(lock.is_file());
        assert_eq!(std::fs::metadata(lock).unwrap().len(), 1);
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

    fn atomic_with_ops(
        path: &Path,
        bytes: &[u8],
        sync_file: impl FnOnce(&File) -> io::Result<()>,
        rename: impl FnMut(&Path, &Path) -> io::Result<()>,
        sleep: impl FnMut(Duration),
        sync_parent: impl FnOnce(&Path) -> io::Result<()>,
    ) -> io::Result<()> {
        atomic_write_bytes_with_ops(
            path,
            bytes,
            |_, _, error| error,
            |path| std::fs::metadata(path).map(|metadata| metadata.permissions()),
            File::set_permissions,
            sync_file,
            rename,
            sleep,
            sync_parent,
        )
    }

    fn atomic_with_permission_ops(
        path: &Path,
        bytes: &[u8],
        read_permissions: impl FnOnce(&Path) -> io::Result<std::fs::Permissions>,
        apply_permissions: impl FnOnce(&File, std::fs::Permissions) -> io::Result<()>,
        rename: impl FnMut(&Path, &Path) -> io::Result<()>,
        sync_parent: impl FnOnce(&Path) -> io::Result<()>,
    ) -> io::Result<()> {
        atomic_write_bytes_with_ops(
            path,
            bytes,
            |_, _, error| error,
            read_permissions,
            apply_permissions,
            File::sync_all,
            rename,
            |_| unreachable!("a real successful replace does not retry"),
            sync_parent,
        )
    }

    fn names(root: &TempDir) -> Vec<String> {
        let mut names = std::fs::read_dir(root.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        names.sort();
        names
    }

    fn assert_sync_precedes_replace(bytes: &[u8]) {
        let root = TempDir::new().unwrap();
        let target = root.path().join("target");
        std::fs::write(&target, b"before").unwrap();
        let events = RefCell::new(Vec::new());

        atomic_with_ops(
            &target,
            bytes,
            |file| {
                events.borrow_mut().push("sync-file");
                assert_eq!(std::fs::read(&target).unwrap(), b"before");
                file.sync_all()
            },
            |source, destination| {
                events.borrow_mut().push("replace");
                assert_eq!(std::fs::read(&target).unwrap(), b"before");
                atomicwrites::replace_atomic(source, destination)
            },
            |_| unreachable!("a real successful replace does not retry"),
            |parent| {
                events.borrow_mut().push("sync-parent");
                assert_eq!(parent, root.path());
                assert_eq!(std::fs::read(&target).unwrap(), bytes);
                sync_directory(parent)
            },
        )
        .unwrap();

        assert_eq!(*events.borrow(), ["sync-file", "replace", "sync-parent"]);
        assert_eq!(std::fs::read(&target).unwrap(), bytes);
        assert_eq!(names(&root), ["target"]);
    }

    #[test]
    fn test_atomic_write_bytes_fsyncs_before_replace() {
        assert_sync_precedes_replace(b"payload");
    }

    #[test]
    fn test_atomic_write_text_fsyncs_before_replace() {
        let text = "hello\n";
        assert_sync_precedes_replace(text.as_bytes());
    }

    #[test]
    fn test_atomic_write_toml_fsyncs_before_replace() {
        let document = toml::Table::from_iter([
            ("language".to_owned(), toml::Value::String("en".to_owned())),
            (
                "future".to_owned(),
                toml::Value::Table(toml::Table::from_iter([(
                    "enabled".to_owned(),
                    toml::Value::Boolean(true),
                )])),
            ),
        ]);
        let encoded = toml::to_string_pretty(&document).unwrap();
        assert_sync_precedes_replace(encoded.as_bytes());
    }

    #[cfg(unix)]
    #[test]
    fn test_atomic_write_bytes_fsyncs_parent_dir_after_replace() {
        let root = TempDir::new().unwrap();
        let target = root.path().join("target");
        std::fs::write(&target, b"before").unwrap();
        let parent_calls = Cell::new(0_u32);

        atomic_with_ops(
            &target,
            b"after",
            File::sync_all,
            atomicwrites::replace_atomic,
            |_| unreachable!("a real successful replace does not retry"),
            |parent| {
                parent_calls.set(parent_calls.get() + 1);
                assert_eq!(parent, root.path());
                assert_eq!(std::fs::read(&target).unwrap(), b"after");
                sync_directory(parent)
            },
        )
        .unwrap();

        assert_eq!(parent_calls.get(), 1);
        assert_eq!(names(&root), ["target"]);
    }

    #[cfg(unix)]
    #[test]
    fn test_atomic_write_bytes_dir_fsync_failure_is_swallowed() {
        let root = TempDir::new().unwrap();
        let target = root.path().join("target");
        std::fs::write(&target, b"before").unwrap();
        let parent_called = Cell::new(false);

        atomic_with_ops(
            &target,
            b"after",
            File::sync_all,
            atomicwrites::replace_atomic,
            |_| unreachable!("a real successful replace does not retry"),
            |_| {
                parent_called.set(true);
                Err(io::Error::other("injected directory sync failure"))
            },
        )
        .unwrap();

        assert!(parent_called.get());
        assert_eq!(std::fs::read(&target).unwrap(), b"after");
        assert_eq!(names(&root), ["target"]);
    }

    #[cfg(not(unix))]
    #[test]
    fn test_atomic_write_bytes_skips_dir_fsync_on_windows() {
        let root = TempDir::new().unwrap();
        let target = root.path().join("target");
        let parent_called = Cell::new(false);

        atomic_with_ops(
            &target,
            b"after",
            File::sync_all,
            atomicwrites::replace_atomic,
            |_| unreachable!("a real successful replace does not retry"),
            |_| {
                parent_called.set(true);
                Ok(())
            },
        )
        .unwrap();

        assert!(!parent_called.get());
        assert_eq!(std::fs::read(&target).unwrap(), b"after");
        assert_eq!(names(&root), ["target"]);
    }

    #[test]
    fn test_atomic_write_bytes_temp_fsync_failure_still_cleans_up_tmp_file() {
        let root = TempDir::new().unwrap();
        let target = root.path().join("target");
        std::fs::write(&target, b"before").unwrap();

        let result = atomic_with_ops(
            &target,
            b"after",
            |_| {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "simulated temp fsync failure",
                ))
            },
            |_, _| unreachable!("replace must not run after temp sync failure"),
            |_| unreachable!("replace retry must not run after temp sync failure"),
            |_| unreachable!("parent sync must not run before replace"),
        );

        assert!(result.is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"before");
        assert_eq!(names(&root), ["target"]);
    }

    #[test]
    fn test_replace_retries_through_transient_permission_error() {
        let root = TempDir::new().unwrap();
        let target = root.path().join("registry.toml");
        std::fs::write(&target, b"old = true\n").unwrap();
        let attempts = Cell::new(0_u32);
        let sleeps = RefCell::new(Vec::new());

        atomic_with_ops(
            &target,
            b"new = true\n",
            File::sync_all,
            |source, destination| {
                attempts.set(attempts.get() + 1);
                if attempts.get() <= 2 {
                    Err(io::Error::from(io::ErrorKind::PermissionDenied))
                } else {
                    atomicwrites::replace_atomic(source, destination)
                }
            },
            |delay| sleeps.borrow_mut().push(delay),
            sync_directory,
        )
        .unwrap();

        assert_eq!(attempts.get(), 3);
        assert_eq!(
            *sleeps.borrow(),
            [10, 20].map(Duration::from_millis).to_vec()
        );
        assert_eq!(std::fs::read(&target).unwrap(), b"new = true\n");
        assert_eq!(names(&root), ["registry.toml"]);
    }

    #[test]
    fn test_replace_gives_up_loudly_after_bounded_attempts() {
        let root = TempDir::new().unwrap();
        let target = root.path().join("registry.toml");
        let original = b"# keep\nfuture = { enabled = true }\n";
        std::fs::write(&target, original).unwrap();
        let attempts = Cell::new(0_u32);
        let sleeps = RefCell::new(Vec::new());

        let error = atomic_with_ops(
            &target,
            b"language = \"en\"\n",
            File::sync_all,
            |_, _| {
                attempts.set(attempts.get() + 1);
                Err(io::Error::from(io::ErrorKind::PermissionDenied))
            },
            |delay| sleeps.borrow_mut().push(delay),
            |_| unreachable!("parent sync must not run after replace failure"),
        )
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(attempts.get(), 8);
        assert_eq!(
            *sleeps.borrow(),
            [10, 20, 40, 80, 160, 320, 640]
                .map(Duration::from_millis)
                .to_vec()
        );
        assert_eq!(std::fs::read(&target).unwrap(), original);
        assert_eq!(names(&root), ["registry.toml"]);
    }

    #[test]
    fn test_replace_other_oserrors_are_not_retried() {
        let root = TempDir::new().unwrap();
        let target = root.path().join("registry.toml");
        let original = b"future = 7\n";
        std::fs::write(&target, original).unwrap();
        let attempts = Cell::new(0_u32);
        let sleeps = Cell::new(0_u32);

        let error = atomic_with_ops(
            &target,
            b"language = \"en\"\n",
            File::sync_all,
            |_, _| {
                attempts.set(attempts.get() + 1);
                Err(io::Error::other("is a directory"))
            },
            |_| sleeps.set(sleeps.get() + 1),
            |_| unreachable!("parent sync must not run after replace failure"),
        )
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::Other);
        assert_eq!(attempts.get(), 1);
        assert_eq!(sleeps.get(), 0);
        assert_eq!(std::fs::read(&target).unwrap(), original);
        assert_eq!(names(&root), ["registry.toml"]);
    }

    #[cfg(unix)]
    #[test]
    fn test_atomic_write_text_keep_mode_applies_mode_before_the_rename() {
        use std::os::unix::fs::PermissionsExt as _;

        let root = TempDir::new().unwrap();
        let target = root.path().join("target");
        std::fs::write(&target, b"old\n").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o750)).unwrap();
        let events = RefCell::new(Vec::new());

        atomic_with_permission_ops(
            &target,
            b"new\n",
            |path| {
                events.borrow_mut().push("read-mode");
                Ok(std::fs::metadata(path)?.permissions())
            },
            |file, permissions| {
                events.borrow_mut().push("apply-mode");
                assert_eq!(std::fs::read(&target).unwrap(), b"old\n");
                assert_eq!(permissions.mode() & 0o777, 0o750);
                file.set_permissions(permissions)
            },
            |source, destination| {
                events.borrow_mut().push("replace");
                assert_eq!(std::fs::read(&target).unwrap(), b"old\n");
                assert_eq!(
                    std::fs::metadata(source).unwrap().permissions().mode() & 0o777,
                    0o750
                );
                atomicwrites::replace_atomic(source, destination)
            },
            |parent| {
                events.borrow_mut().push("sync-parent");
                sync_directory(parent)
            },
        )
        .unwrap();

        assert_eq!(
            *events.borrow(),
            ["read-mode", "apply-mode", "replace", "sync-parent"]
        );
        assert_eq!(std::fs::read(&target).unwrap(), b"new\n");
        assert_eq!(names(&root), ["target"]);
    }

    #[cfg(unix)]
    #[test]
    fn test_atomic_write_text_keep_mode_missing_target_skips_chmod() {
        let root = TempDir::new().unwrap();
        let target = root.path().join("target");
        let reads = Cell::new(0_u32);

        atomic_with_permission_ops(
            &target,
            b"new\n",
            |path| {
                reads.set(reads.get() + 1);
                std::fs::metadata(path).map(|metadata| metadata.permissions())
            },
            |_, _| unreachable!("a missing target has no permissions to apply"),
            atomicwrites::replace_atomic,
            sync_directory,
        )
        .unwrap();

        assert_eq!(reads.get(), 1);
        assert_eq!(std::fs::read(&target).unwrap(), b"new\n");
        assert_eq!(names(&root), ["target"]);
    }

    #[cfg(unix)]
    #[test]
    fn test_atomic_write_text_keep_mode_suppresses_chmod_failure() {
        use std::os::unix::fs::PermissionsExt as _;

        let root = TempDir::new().unwrap();
        let target = root.path().join("target");
        std::fs::write(&target, b"old\n").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o711)).unwrap();
        let attempted_mode = Cell::new(None);

        atomic_with_permission_ops(
            &target,
            b"new\n",
            |path| Ok(std::fs::metadata(path)?.permissions()),
            |_, permissions| {
                attempted_mode.set(Some(permissions.mode() & 0o777));
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "injected permission apply failure",
                ))
            },
            atomicwrites::replace_atomic,
            sync_directory,
        )
        .unwrap();

        assert_eq!(attempted_mode.get(), Some(0o711));
        assert_eq!(std::fs::read(&target).unwrap(), b"new\n");
        assert_eq!(names(&root), ["target"]);
    }

    #[cfg(windows)]
    #[test]
    fn windows_native_atomic_write_preserves_existing_readonly_attribute() {
        let root = TempDir::new().unwrap();
        let target = root.path().join("target");
        std::fs::write(&target, b"old\n").unwrap();
        let mut permissions = std::fs::metadata(&target).unwrap().permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(&target, permissions).unwrap();

        atomic_write_bytes(&target, b"new\n").unwrap();

        assert_eq!(std::fs::read(&target).unwrap(), b"new\n");
        assert!(std::fs::metadata(&target).unwrap().permissions().readonly());
        assert_eq!(names(&root), ["target"]);
        let mut permissions = std::fs::metadata(&target).unwrap().permissions();
        permissions.set_readonly(false);
        std::fs::set_permissions(target, permissions).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn windows_native_missing_target_skips_permission_apply() {
        let root = TempDir::new().unwrap();
        let target = root.path().join("target");
        let applied = Cell::new(false);

        atomic_with_permission_ops(
            &target,
            b"new\n",
            |path| std::fs::metadata(path).map(|metadata| metadata.permissions()),
            |_, _| {
                applied.set(true);
                Ok(())
            },
            atomicwrites::replace_atomic,
            |_| unreachable!("Windows does not synchronize the parent directory"),
        )
        .unwrap();

        assert!(!applied.get());
        assert_eq!(std::fs::read(&target).unwrap(), b"new\n");
        assert_eq!(names(&root), ["target"]);
    }

    #[cfg(windows)]
    #[test]
    fn windows_native_permission_apply_failure_is_best_effort_and_cleans_temp() {
        let root = TempDir::new().unwrap();
        let target = root.path().join("target");
        std::fs::write(&target, b"old\n").unwrap();
        let attempted = Cell::new(false);

        atomic_with_permission_ops(
            &target,
            b"new\n",
            |path| Ok(std::fs::metadata(path)?.permissions()),
            |_, _| {
                attempted.set(true);
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "injected Windows permission apply failure",
                ))
            },
            atomicwrites::replace_atomic,
            |_| unreachable!("Windows does not synchronize the parent directory"),
        )
        .unwrap();

        assert!(attempted.get());
        assert_eq!(std::fs::read(&target).unwrap(), b"new\n");
        assert_eq!(names(&root), ["target"]);
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
