//! Mechanical port of the Python oracle `tests/test_atomic.py`
//! (`/home/ubuntu/coding/skit-oracle/tests/test_atomic.py`, origin/main@206f9ef).
//!
//! The Python module unit-tests `skit.atomic` directly: `load_toml_recoverable`,
//! `advisory_file_lock` / `try_advisory_file_lock`, `atomic_write_bytes/text/toml`,
//! `atomic_write_text_keep_mode`, and the internal `_replace_with_retry` / `_try_native_lock`
//! / `_fsync_dir` seams. The Rust rewrite has no equivalent public "atomic" module. Public outcome
//! owners use `FileConfigStore`, `FileFormStateStore`, and `FileStore`; syscall-order and fault
//! owners run beside the crate-private shared writer, using its actual algorithm with controlled
//! operations. Each test keeps its exact Python name and carries a WHY comment. Tests use
//! `tempfile::TempDir`, never real user directories, matching the existing skit-store test harness.
//!
//! FINDINGS the supervisor must read (see the flagged tests below):
//!   * DATA-SAFETY: the temp-fsync cleanup owner moved to the shared atomic primitive's unit tests,
//!     where a deterministic sync failure exercises the real cleanup path for every store adapter.
//!   * FEATURE PARITY: RESOLVED. The Windows sharing-violation rename retry (Python's
//!     `_replace_with_retry`, issue #4, A1) and the non-blocking `try_advisory_file_lock` (A2) now
//!     both exist in `fs_ops`. The retry owners moved beside the actual atomic writer, where
//!     controlled operations verify retry, cleanup, and no-clobber through the same algorithm the
//!     real wrapper calls. The try-lock is crate-private (its production caller is the read-path
//!     self-heal), so its contract is proven by `fs_ops.rs` unit tests and by the self-heal tests in
//!     `port_test_store.rs` rather than the try-lock stubs below.
//!   * ACCOUNTING: `port_test_atomic_manifest.rs` requires all 32 frozen names exactly once as 15
//!     common exact owners, 7 target-gated exact owners, and 10 structured architecture closures.

use std::{
    fs::{self, OpenOptions},
    sync::mpsc,
    thread,
    time::Duration,
};

use skit_application::{
    CreateEntry, EntryMutationRepository, EntryPayload, SourcePermissions,
    form_state::FormStateRepository,
};
use skit_domain::{Entry, EntryKind, EntrySettings, Slug, StorageMode};
use skit_store::{FileConfigStore, FileFormStateStore, FileStore};
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Shared harness helpers.
// ---------------------------------------------------------------------------

/// Create a copy-mode Python entry whose stored `script.py` carries `bytes` and `mode`.
///
/// The stored source is written through the same internal `atomic_write_bytes` the Python
/// `atomic_write_*` functions stand in for, so a later `commit_copy_edit` exercises the atomic
/// write/replace + permission-preservation path directly.
fn create_entry(root: &TempDir, name: &str, bytes: &[u8], mode: u32) -> (FileStore, Entry) {
    let store = FileStore::new(root.path());
    let create = CreateEntry {
        name: name.to_owned(),
        kind: EntryKind::parse("python").unwrap(),
        mode: StorageMode::Copy,
        source: format!("/original/{name}.py"),
        workdir: "invoke".to_owned(),
        description: String::new(),
        payload: Some(EntryPayload {
            bytes: bytes.to_vec(),
            stored_name: Some("script.py".to_owned()),
            permissions: SourcePermissions {
                readonly: false,
                unix_mode: Some(mode),
            },
        }),
        settings: EntrySettings::default(),
    };
    let entry = store.create(create).unwrap();
    (store, entry)
}

fn set_readonly(path: &std::path::Path, readonly: bool) {
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_readonly(readonly);
    fs::set_permissions(path, permissions).unwrap();
}

fn architecture_closure() -> ! {
    panic!("architecture closure; see port_test_atomic_manifest")
}

// ===========================================================================
// load_toml_recoverable — corrupt-TOML backup helper.
//
// Python's helper runs at READ time: a corrupt file is backed up byte-exact and the original is
// left untouched. Rust folds the corrupt-backup into the WRITE path
// (`FileConfigStore::update_with_recovery` -> `preserve_corrupt_backup`): the corrupt bytes are
// preserved byte-exact as `<name>.bak` at the first write, then the original is repaired in place.
// The data-safety invariant — a present-but-corrupt file is preserved, never silently wiped — is
// ported against that write seam where the read-time seam does not exist.
// ===========================================================================

#[test]
fn test_load_toml_recoverable_missing_file_returns_empty_no_backup() {
    // WHY: a missing config reads as the v0.4 empty document and never creates a file or a backup.
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());

    assert_eq!(store.get("lang").unwrap(), "");
    assert!(!root.path().join("config.toml").exists()); // a read writes nothing
    assert!(!root.path().join("config.toml.bak").exists());
}

#[test]
fn test_load_toml_recoverable_valid_file_returns_doc_no_backup() {
    // WHY: a valid file projects its document and is left byte-exact on read, with no backup.
    let root = TempDir::new().unwrap();
    let path = root.path().join("config.toml");
    let source = "language = \"zh-CN\"\n[mirror]\nenabled = true\n";
    fs::write(&path, source).unwrap();
    let store = FileConfigStore::new(root.path());

    assert_eq!(store.get("lang").unwrap(), "zh-CN");
    assert!(store.mirror().unwrap().enabled);
    assert_eq!(fs::read_to_string(&path).unwrap(), source); // untouched on read
    assert!(!root.path().join("config.toml.bak").exists());
}

#[test]
fn test_load_toml_recoverable_corrupt_file_backs_up_and_returns_empty() {
    // WHY: Python backs the corrupt file up at READ time and leaves the original in place. Rust
    // has no read-time-backup seam; it preserves the corrupt bytes byte-exact as `<name>.bak` at
    // the first write (then repairs the original). The invariant under test — the corrupt bytes
    // are preserved, never silently wiped — is ported against that write seam.
    let root = TempDir::new().unwrap();
    let path = root.path().join("config.toml");
    let backup = root.path().join("config.toml.bak");
    let corrupt = "language = \"zh-CN\"\nthis is = = not valid toml".as_bytes();
    fs::write(&path, corrupt).unwrap();
    let store = FileConfigStore::new(root.path());

    let recovery = store
        .set_with_recovery("editor", "vim")
        .unwrap()
        .expect("a corrupt file must be preserved as a byte-exact backup, not wiped");

    assert_eq!(recovery.backup_path.as_deref(), Some(backup.as_path()));
    assert_eq!(fs::read(&backup).unwrap(), corrupt); // byte-exact preservation
}

/// The backup carries the mode and the times of the file it copies.
///
/// Version 0.4 saves the corrupt configuration with `shutil.copy2`
/// (`skit-oracle/src/skit/atomic.py:309`), which preserves the access and modification times as
/// well as the mode. A user who opens the backup to recover reads when their configuration was
/// written, not when skit found the damage.
#[test]
fn rust_additive_a_corrupt_backup_keeps_the_original_mode_and_times() {
    let root = TempDir::new().unwrap();
    let path = root.path().join("config.toml");
    let backup = root.path().join("config.toml.bak");
    fs::write(&path, b"this is = = not valid toml").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    }
    // A file written a while ago: the backup must say so too.
    let written = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_577_836_800);
    fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(
            fs::FileTimes::new()
                .set_modified(written)
                .set_accessed(written),
        )
        .unwrap();
    let before = fs::metadata(&path).unwrap();

    FileConfigStore::new(root.path())
        .set_with_recovery("editor", "vim")
        .unwrap()
        .expect("a corrupt file must be preserved");

    let saved = fs::metadata(&backup).unwrap();
    assert_eq!(saved.modified().unwrap(), before.modified().unwrap());
    assert_eq!(saved.permissions(), before.permissions());
}

#[test]
fn test_load_toml_recoverable_reports_none_when_backup_itself_fails() {
    // WHY: Python reports no backup, warns, and still applies the requested setting. A directory at
    // the nested copy target blocks the write while leaving unrelated directory contents intact.
    let root = TempDir::new().unwrap();
    let path = root.path().join("config.toml");
    let backup = root.path().join("config.toml.bak");
    let corrupt = b"this is = = not valid toml";
    fs::write(&path, corrupt).unwrap();
    fs::create_dir(&backup).unwrap();
    let blocker = backup.join("config.toml");
    fs::create_dir(&blocker).unwrap();
    fs::write(blocker.join("owned"), "keep").unwrap();
    let store = FileConfigStore::new(root.path());

    let recovery = store
        .set_with_recovery("editor", "vim")
        .unwrap()
        .expect("the malformed file must report recovery");

    assert_eq!(recovery.backup_path, None);
    assert_eq!(store.get("editor").unwrap(), "vim");
    assert_eq!(fs::read_to_string(blocker.join("owned")).unwrap(), "keep");
}

// ===========================================================================
// advisory_file_lock — persistent kernel-backed transaction serialization.
// Rust uses std `File::lock()` (blocking flock) inside `acquire_lock`, reachable through the
// config store's `config.lock`.
// ===========================================================================

#[test]
fn test_advisory_file_lock_keeps_a_persistent_one_byte_inode() {
    // WHY: the lock file is created as a >=1-byte inode and is never unlinked; a later write finds
    // the same persistent inode (path replacement is what let the old lease design admit two
    // owners).
    let root = TempDir::new().unwrap();
    let lock = root.path().join("config.lock");
    let store = FileConfigStore::new(root.path());

    store.set("form", "plain").unwrap();
    assert!(lock.is_file());
    assert!(lock.metadata().unwrap().len() >= 1);

    store.set("form", "tui").unwrap();
    assert!(lock.is_file()); // never unlinked between transactions
    assert!(lock.metadata().unwrap().len() >= 1);
}

#[test]
fn test_advisory_file_lock_serializes_two_waiting_threads() {
    // WHY: a held lock forces waiters to serialize. Ported as: an externally held `config.lock`
    // blocks a store write until the holder releases it, then the waiter lands. Mirrors the
    // existing `removal_waits_for_the_dependency_transaction_lock` pattern.
    let root = TempDir::new().unwrap();
    let lock_path = root.path().join("config.lock");
    let held = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .unwrap();
    held.set_len(1).unwrap();
    held.lock().unwrap();

    let store = FileConfigStore::new(root.path());
    let (started_tx, started_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let worker = thread::spawn(move || {
        started_tx.send(()).unwrap();
        done_tx.send(store.set("form", "plain")).unwrap();
    });
    started_rx.recv().unwrap();

    // Blocked behind the held lock: the write cannot land while another owner holds it.
    assert!(done_rx.recv_timeout(Duration::from_millis(100)).is_err());
    drop(held);
    // Released: the waiter serializes in and completes.
    done_rx
        .recv_timeout(Duration::from_secs(2))
        .unwrap()
        .unwrap();
    worker.join().unwrap();
}

#[ignore = "UNMAPPED: Windows msvcrt one-byte-seek/retry/unlock seam. The POSIX build uses flock \
            (std File::lock); there is no msvcrt path to exercise."]
#[test]
fn test_windows_locking_uses_one_byte_seek_retry_and_unlock() {
    architecture_closure()
}

#[ignore = "UNMAPPED: `_try_native_lock`'s errno classification (EAGAIN retryable vs EBADF/ENOSPC \
            loud) is a non-blocking-lock concept. Rust has only the blocking `File::lock()` and no \
            errno-classifying try seam."]
#[test]
fn test_native_lock_distinguishes_contention_from_unexpected_os_errors() {
    architecture_closure()
}

#[test]
fn test_advisory_lock_open_failure_releases_its_thread_mutex() {
    // Rust has no per-path thread mutex. Its public equivalent is that a failed lock-file open
    // leaves no lock resource behind: the same store can retry after the filesystem obstruction is
    // removed.
    let root = TempDir::new().unwrap();
    let lock_path = root.path().join("config.lock");
    fs::create_dir(&lock_path).unwrap();
    let store = FileConfigStore::new(root.path());

    assert!(store.set("form", "plain").is_err());
    assert!(!root.path().join("config.toml").exists());

    fs::remove_dir(&lock_path).unwrap();
    store.set("form", "plain").unwrap();

    assert_eq!(store.get("form").unwrap(), "plain");
    assert!(lock_path.is_file());
    assert!(lock_path.metadata().unwrap().len() >= 1);
}

#[ignore = "UNMAPPED: same two-layer (thread-mutex + native) design as above. In Rust a failed \
            lock drops its File, closing the fd via RAII, and there is no thread mutex; there is \
            also no seam to force `File::lock()` itself to fail on POSIX."]
#[test]
fn test_advisory_lock_native_failure_closes_fd_and_releases_mutex() {
    architecture_closure()
}

// ===========================================================================
// atomic_write_text_keep_mode — preserve an existing file's permission bits across the replace.
// Rust's `atomic_write_bytes` reads the target's current permissions and applies them to the temp
// file BEFORE the rename (`preserve_permissions_best_effort`), reachable via `commit_copy_edit`.
// ===========================================================================

#[test]
fn test_atomic_write_text_keep_mode_preserves_existing_mode() {
    // Both public adapters use the shared atomic writer. A readonly state file and a readonly
    // stored copy must keep that portable permission after replacement while the new bytes land.
    let root = TempDir::new().unwrap();

    let state_root = root.path().join("state");
    let state_store = FileFormStateStore::new(&state_root);
    let state_slug = Slug::parse("permission-state").unwrap();
    state_store
        .update(&state_slug, |state| {
            state.values.insert("before".to_owned(), "1".to_owned());
        })
        .unwrap();
    // The oracle probes preservation with chmod(0o755) and notes that Windows maps POSIX modes
    // to nothing (test_atomic.py:445-460): its replace always lands on a WRITABLE file there,
    // and CPython's os.replace requires that -- a read-only destination is refused with
    // PermissionError on Windows. Probe with a read-only file only where the platform can
    // replace one, and assert preservation of whatever the platform reports, exactly as the
    // oracle does.
    let probe_readonly = cfg!(unix);
    let state_path = state_root.join("values/permission-state.toml");
    set_readonly(&state_path, probe_readonly);

    state_store
        .update(&state_slug, |state| {
            state.values.insert("after".to_owned(), "2".to_owned());
        })
        .unwrap();

    assert_eq!(
        fs::metadata(&state_path).unwrap().permissions().readonly(),
        probe_readonly
    );
    assert_eq!(
        state_store
            .load(&state_slug)
            .values
            .get("after")
            .map(String::as_str),
        Some("2")
    );

    let (store, entry) = create_entry(&root, "Script", b"old\n", 0o644);
    let script = root.path().join("scripts/script/script.py");
    set_readonly(&script, probe_readonly);

    store
        .commit_copy_edit(&entry, b"new content\n", &entry.meta.source_hash)
        .unwrap();

    assert_eq!(fs::read(&script).unwrap(), b"new content\n");
    assert_eq!(
        fs::metadata(&script).unwrap().permissions().readonly(),
        probe_readonly
    );

    // Windows does not remove readonly files during recursive cleanup. Restore only after the
    // assertions so this temporary fixture cannot leak.
    set_readonly(&state_path, false);
    set_readonly(&script, false);
}

#[ignore = "UNMAPPED: Windows-only. Python restores bits with a post-rename os.chmod because \
            Windows lacks os.fchmod. Rust uses one cross-platform path — File::set_permissions on \
            the temp handle before the rename — with no fchmod-vs-chmod split to exercise on POSIX."]
#[test]
fn test_atomic_write_text_keep_mode_falls_back_to_chmod_on_windows() {
    architecture_closure()
}

#[ignore = "UNMAPPED: Windows-only fallback guard (skip the post-rename chmod when the target \
            vanished, so None is never handed to os.chmod). Rust has no post-rename Windows chmod \
            branch on POSIX to exercise."]
#[test]
fn test_keep_mode_windows_fallback_is_skipped_when_there_is_no_mode() {
    architecture_closure()
}

#[ignore = "UNMAPPED: Windows-only. The post-rename restore's best-effort suppression is the \
            Windows chmod branch, not built or reachable on POSIX; the POSIX swallow is covered in \
            the keep_mode suppression note above."]
#[test]
fn test_keep_mode_windows_fallback_suppresses_a_chmod_failure() {
    architecture_closure()
}

// ===========================================================================
// try_advisory_file_lock — the never-waits variant for read-path writes.
// RESOLVED (A2): `fs_ops::try_acquire_lock` now exists (non-blocking `File::try_lock`). It is
// crate-private -- its production caller is the read-path self-heal (`FileStore::repair_rows`) -- so
// its contract is ported as `fs_ops.rs` UNIT tests (`try_lock_acquires_when_free_and_a_second_taker_
// declines`, `try_lock_declines_while_the_blocking_lock_is_held`, `try_lock_treats_an_unopenable_
// path_as_not_acquired`) and its read-path use in `port_test_store.rs`
// (`test_a_listing_never_blocks_on_the_registry_lock`, `test_a_store_that_cannot_be_written_still_
// lists`), which reach the primitive rather than these external stubs.
// ===========================================================================

#[ignore = "RESOLVED (A2) -> fs_ops.rs unit test try_lock_acquires_when_free_and_a_second_taker_\
            declines. try_acquire_lock is crate-private (read-path self-heal caller), so the \
            acquire/second-taker-declines contract is proven in-crate, not through this external stub."]
#[test]
fn test_try_lock_acquires_when_free_and_excludes_a_second_taker() {
    architecture_closure()
}

#[ignore = "RESOLVED (A2) -> fs_ops.rs unit test try_lock_declines_while_the_blocking_lock_is_held \
            (crate-private try_acquire_lock)."]
#[test]
fn test_try_lock_declines_while_the_blocking_lock_is_held() {
    architecture_closure()
}

#[ignore = "RESOLVED (A2): the cross-process decline-when-native-lock-held path is proven through \
            the read-path self-heal in port_test_store.rs::test_a_listing_never_blocks_on_the_\
            registry_lock (a held flock makes the listing's try_acquire_lock decline)."]
#[test]
fn test_try_lock_declines_when_only_the_native_lock_is_held() {
    architecture_closure()
}

#[ignore = "RESOLVED (A2) -> fs_ops.rs unit test try_lock_treats_an_unopenable_path_as_not_acquired \
            (crate-private try_acquire_lock)."]
#[test]
fn test_try_lock_treats_an_unopenable_lock_file_as_not_acquired() {
    architecture_closure()
}
