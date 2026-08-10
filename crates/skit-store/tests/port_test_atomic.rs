//! Mechanical port of the Python oracle `tests/test_atomic.py`
//! (`/home/ubuntu/coding/skit-oracle/tests/test_atomic.py`, origin/main@206f9ef).
//!
//! The Python module unit-tests `skit.atomic` directly: `load_toml_recoverable`,
//! `advisory_file_lock` / `try_advisory_file_lock`, `atomic_write_bytes/text/toml`,
//! `atomic_write_text_keep_mode`, and the internal `_replace_with_retry` / `_try_native_lock`
//! / `_fsync_dir` seams. The Rust rewrite has NO equivalent public "atomic" module: the atomic
//! write/replace, the corrupt-TOML backup, and the file lock are internal helpers
//! (`crates/skit-store/src/fs_ops.rs`, `src/mutations/atomic.rs`, `src/config.rs`) that are only
//! reachable through the public stores `FileConfigStore` and `FileStore`. Every port therefore
//! drives the mechanism through the closest public seam. Each test keeps its exact Python name and
//! carries a WHY comment. Tests use `tempfile::TempDir`, never real user directories, matching the
//! existing skit-store test harness (`tests/mutations.rs`, `tests/config_store.rs`).
//!
//! FINDINGS the supervisor must read (see the flagged tests below):
//!   * DATA-SAFETY: `atomic_write_bytes` removes the temp file only when the rename fails, NOT when
//!     `sync_all()` fails — a temp-fsync failure leaks a `.tmp` residue, diverging from the Python
//!     contract (`test_atomic_write_bytes_temp_fsync_failure_still_cleans_up_tmp_file`). No public
//!     fsync-injection seam exists to reproduce it, so it is `#[ignore]`d and flagged.
//!   * FEATURE PARITY: the Rust writer has no Windows sharing-violation rename retry (Python's
//!     `_replace_with_retry`, issue #4) and no non-blocking `try_advisory_file_lock`. Both are
//!     absent, not merely off-seam; flagged for the superset rule.

use std::{
    fs::{self, OpenOptions},
    path::Path,
    sync::mpsc,
    thread,
    time::Duration,
};

use skit_application::{CreateEntry, EntryMutationRepository, EntryPayload, SourcePermissions};
use skit_domain::{Entry, EntryKind, EntrySettings, StorageMode};
use skit_store::{ConfigError, FileConfigStore, FileStore, content_hash};
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

/// Names in `dir` that look like the atomic writer's private temp file (`.<name>.<id>.tmp`).
///
/// A successful atomic write must leave none: the temp is renamed onto the target, never left as a
/// partial commit beside it.
fn temp_residue(dir: &Path) -> Vec<String> {
    fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains(".tmp"))
        .collect()
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

    assert_eq!(recovery.backup_path, backup);
    assert_eq!(fs::read(&backup).unwrap(), corrupt); // byte-exact preservation
}

#[test]
fn test_load_toml_recoverable_reports_none_when_backup_itself_fails() {
    // WHY: Python simulates a failing copy2 and asserts the corrupt original is NOT lost. Rust has
    // no read-time backup or copy2 seam; the write-path analog blocks the backup by making
    // `<name>.bak` a directory. Rust surfaces an Err where Python swallowed and returned None; the
    // ported data-safety invariant is the shared one — a failed backup never destroys the corrupt
    // original.
    let root = TempDir::new().unwrap();
    let path = root.path().join("config.toml");
    let backup = root.path().join("config.toml.bak");
    let corrupt = b"this is = = not valid toml";
    fs::write(&path, corrupt).unwrap();
    fs::create_dir(&backup).unwrap();
    fs::write(backup.join("owned"), "keep").unwrap();
    let store = FileConfigStore::new(root.path());

    let error = store.set("editor", "vim").unwrap_err();

    assert!(matches!(error, ConfigError::Io { .. }));
    assert_eq!(fs::read(&path).unwrap(), corrupt); // corrupt original intact, not replaced
    assert_eq!(fs::read_to_string(backup.join("owned")).unwrap(), "keep");
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

#[ignore = "UNMAPPED: crash-release-flock needs a child process that acquires skit's lock then \
            hard-exits (os._exit). There is no public API to acquire-and-hold the lock without a \
            full mutation, and no helper binary may be added. Release-on-close is a kernel \
            guarantee that std File::lock inherits; not a skit-specific behavior."]
#[test]
fn test_advisory_file_lock_is_released_by_kernel_after_process_crash() {}

#[ignore = "UNMAPPED: Windows msvcrt one-byte-seek/retry/unlock seam. The POSIX build uses flock \
            (std File::lock); there is no msvcrt path to exercise."]
#[test]
fn test_windows_locking_uses_one_byte_seek_retry_and_unlock() {}

#[ignore = "UNMAPPED: `_try_native_lock`'s errno classification (EAGAIN retryable vs EBADF/ENOSPC \
            loud) is a non-blocking-lock concept. Rust has only the blocking `File::lock()` and no \
            errno-classifying try seam."]
#[test]
fn test_native_lock_distinguishes_contention_from_unexpected_os_errors() {}

#[ignore = "UNMAPPED: Python layers a per-path in-process threading.Lock over flock and tests that \
            a failed lockfile open releases that mutex. Rust relies solely on kernel flock \
            (per-fd), so there is no in-process mutex layer to leak or release."]
#[test]
fn test_advisory_lock_open_failure_releases_its_thread_mutex() {}

#[ignore = "UNMAPPED: same two-layer (thread-mutex + native) design as above. In Rust a failed \
            lock drops its File, closing the fd via RAII, and there is no thread mutex; there is \
            also no seam to force `File::lock()` itself to fail on POSIX."]
#[test]
fn test_advisory_lock_native_failure_closes_fd_and_releases_mutex() {}

// ===========================================================================
// atomic_write_* — durability (fsync temp before replace, fsync dir after replace).
// The fsync/replace ORDER and the dir fsync are not observable through the public API; each test
// ports the OBSERVABLE OUTCOME of the atomic swap (exact bytes land, no partial/temp commit) and
// carries a MUST-VERIFY note pinning the source that the supervisor must confirm by inspection.
// ===========================================================================

#[test]
fn test_atomic_write_bytes_fsyncs_before_replace() {
    // WHY: outcome of the atomic swap. MUST-VERIFY (source-only): the temp file is `sync_all()`ed
    // BEFORE `fs::rename` (mutations/atomic.rs:163-167, fs_ops.rs:51-58), so a crash cannot leave
    // the target renamed-but-unflushed.
    let root = TempDir::new().unwrap();
    let (store, entry) = create_entry(&root, "Payload", b"before", 0o644);
    let dir = root.path().join("scripts/payload");

    let edited = store
        .commit_copy_edit(&entry, b"payload", &entry.meta.source_hash)
        .unwrap();

    assert_eq!(fs::read(dir.join("script.py")).unwrap(), b"payload");
    assert_eq!(edited.meta.source_hash, content_hash(b"payload"));
    assert!(temp_residue(&dir).is_empty()); // no partial commit beside the file
}

#[test]
fn test_atomic_write_text_fsyncs_before_replace() {
    // WHY: same durability guarantee for text content. MUST-VERIFY: fsync-before-rename ordering.
    let root = TempDir::new().unwrap();
    let (store, entry) = create_entry(&root, "Note", b"old", 0o644);
    let dir = root.path().join("scripts/note");

    store
        .commit_copy_edit(&entry, b"hello", &entry.meta.source_hash)
        .unwrap();

    assert_eq!(fs::read_to_string(dir.join("script.py")).unwrap(), "hello");
    assert!(temp_residue(&dir).is_empty());
}

#[test]
fn test_atomic_write_toml_fsyncs_before_replace() {
    // WHY: the TOML write lands atomically. Byte-exact serializer formatting is impl-specific, so
    // the parsed value is asserted. MUST-VERIFY: fsync-before-rename ordering (config writes go
    // through the same fs_ops::atomic_write_bytes).
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());

    store.set("lang", "en").unwrap();

    let document = fs::read_to_string(root.path().join("config.toml"))
        .unwrap()
        .parse::<toml::Table>()
        .unwrap();
    assert_eq!(document["language"].as_str(), Some("en"));
    assert!(temp_residue(root.path()).is_empty());
}

#[test]
fn test_atomic_write_bytes_fsyncs_parent_dir_after_replace() {
    // WHY: outcome of the write. MUST-VERIFY (source-only, POSIX): after the rename lands the
    // parent directory fd is `sync_all()`ed best-effort (mutations/atomic.rs:171 -> sync_directory,
    // fs_ops.rs:58), so the rename survives a lost/rolled-back directory entry across power loss.
    let root = TempDir::new().unwrap();
    let (store, entry) = create_entry(&root, "Durable", b"before", 0o644);
    let dir = root.path().join("scripts/durable");

    store
        .commit_copy_edit(&entry, b"payload", &entry.meta.source_hash)
        .unwrap();

    assert_eq!(fs::read(dir.join("script.py")).unwrap(), b"payload");
    assert!(temp_residue(&dir).is_empty());
}

#[test]
fn test_atomic_write_bytes_dir_fsync_failure_is_swallowed() {
    // WHY: the post-replace directory fsync is best-effort — a failure must not fail the write.
    // MUST-VERIFY (source-only): the call is `let _ = sync_directory(parent);` so its Err is
    // discarded; the content durability was already secured by the temp-file fsync before rename.
    // The observable outcome (the write still succeeds) is ported here.
    let root = TempDir::new().unwrap();
    let (store, entry) = create_entry(&root, "BestEffort", b"before", 0o644);
    let dir = root.path().join("scripts/besteffort");

    store
        .commit_copy_edit(&entry, b"payload", &entry.meta.source_hash)
        .unwrap();

    assert_eq!(fs::read(dir.join("script.py")).unwrap(), b"payload");
}

#[ignore = "UNMAPPED: platform-cfg. On POSIX the `#[cfg(unix)]` sync_directory arm is compiled; \
            the Windows no-op (`#[cfg(not(unix))]` sync_directory -> Ok(())) is not built here and \
            has no runtime seam to observe."]
#[test]
fn test_atomic_write_bytes_skips_dir_fsync_on_windows() {}

#[ignore = "UNMAPPED + MUST-VERIFY DATA-SAFETY FINDING: Python asserts a temp-file fsync failure \
            propagates AND leaves NO temp residue (destination as if the write never started). The \
            Rust atomic_write_bytes removes the temp ONLY on rename failure (mutations/atomic.rs \
            168-169, fs_ops.rs 54-56); on `sync_all()` failure the `?` returns early and the \
            `.tmp` file LEAKS. No public fsync-injection seam exists to reproduce it. Candidate \
            temp-residue / partial-commit gap — supervisor MUST review."]
#[test]
fn test_atomic_write_bytes_temp_fsync_failure_still_cleans_up_tmp_file() {}

// ===========================================================================
// atomic_write_text_keep_mode — preserve an existing file's permission bits across the replace.
// Rust's `atomic_write_bytes` reads the target's current permissions and applies them to the temp
// file BEFORE the rename (`preserve_permissions_best_effort`), reachable via `commit_copy_edit`.
// ===========================================================================

#[cfg(unix)]
#[test]
fn test_atomic_write_text_keep_mode_preserves_existing_mode() {
    // WHY: an existing file's bits survive the atomic replace. The stored script is set to a
    // non-default 0o750; after a copy-edit the new content lands AND 0o750 is preserved exactly
    // (the writer would otherwise strand the file at the temp's create-time mode).
    use std::os::unix::fs::PermissionsExt as _;

    let root = TempDir::new().unwrap();
    let (store, entry) = create_entry(&root, "Script", b"old\n", 0o644);
    let script = root.path().join("scripts/script/script.py");
    fs::set_permissions(&script, fs::Permissions::from_mode(0o750)).unwrap();

    store
        .commit_copy_edit(&entry, b"new content\n", &entry.meta.source_hash)
        .unwrap();

    assert_eq!(fs::read(&script).unwrap(), b"new content\n");
    assert_eq!(
        fs::metadata(&script).unwrap().permissions().mode() & 0o777,
        0o750 // bits preserved exactly, not reset to the temp file's mode
    );
}

#[ignore = "UNMAPPED seam-spy (+MUST-VERIFY): Python spies on the temp file's mode at rename time \
            to prove the bits ride along BEFORE the swap (no crash window at the temp's default \
            mode). Rust applies the mode to the temp fd before drop/rename \
            (mutations/atomic.rs:159-167, fs_ops.rs:47-54) with no post-rename chmod, so there is \
            no rename-seam to observe the pre-rename mode; the preserved-final-mode outcome is \
            covered by test_atomic_write_text_keep_mode_preserves_existing_mode."]
#[test]
fn test_atomic_write_text_keep_mode_applies_mode_before_the_rename() {}

#[test]
fn test_atomic_write_text_keep_mode_missing_target_skips_chmod() {
    // WHY: with no existing target, preserve_permissions_best_effort's fs::metadata(path) fails, so
    // no mode is captured or applied and the fresh write still lands. Exercised via the first
    // config write to a non-existent config.toml.
    let root = TempDir::new().unwrap();
    let path = root.path().join("config.toml");
    assert!(!path.exists());
    let store = FileConfigStore::new(root.path());

    store.set("editor", "vim").unwrap();

    assert_eq!(store.get("editor").unwrap(), "vim");
    assert!(path.is_file());
}

#[ignore = "UNMAPPED (+MUST-VERIFY): a chmod failure is best-effort — the write still succeeds. \
            Rust swallows it as `let _ = apply(permissions)` in preserve_permissions_best_effort \
            (fs_ops.rs:68-70); this exact swallow is proven in-module by \
            `a_permission_restore_failure_does_not_turn_a_committed_write_into_an_error`. No public \
            seam forces set_permissions to fail on the temp fd."]
#[test]
fn test_atomic_write_text_keep_mode_suppresses_chmod_failure() {}

#[ignore = "UNMAPPED: Windows-only. Python restores bits with a post-rename os.chmod because \
            Windows lacks os.fchmod. Rust uses one cross-platform path — File::set_permissions on \
            the temp handle before the rename — with no fchmod-vs-chmod split to exercise on POSIX."]
#[test]
fn test_atomic_write_text_keep_mode_falls_back_to_chmod_on_windows() {}

// ===========================================================================
// _replace_with_retry — Windows sharing-violation backoff (issue #4).
// The Rust atomic_write_bytes calls `fs::rename` directly with NO retry, so the transient-retry
// contract is absent, not merely off-seam.
// ===========================================================================

#[ignore = "UNMAPPED + FEATURE-PARITY FINDING: Rust atomic_write_bytes has no Windows \
            sharing-violation rename retry/backoff (Python's `_replace_with_retry`, issue #4). \
            fs::rename is called once; a transient PermissionError would propagate immediately. \
            Feature loss vs the oracle (superset rule) — supervisor should confirm intent."]
#[test]
fn test_replace_retries_through_transient_permission_error() {}

#[ignore = "UNMAPPED + FEATURE-PARITY FINDING: bounded-attempts-then-loud is part of the missing \
            `_replace_with_retry` above; Rust surfaces the first rename error with no retry loop."]
#[test]
fn test_replace_gives_up_loudly_after_bounded_attempts() {}

#[ignore = "UNMAPPED: no rename-injection seam. Rust surfaces every rename error immediately (no \
            retry logic to distinguish transient from permanent); immediate failure with no temp \
            residue is covered by the in-module `a_failed_atomic_replace_removes_its_temporary_file`."]
#[test]
fn test_replace_other_oserrors_are_not_retried() {}

#[ignore = "UNMAPPED: Windows-only fallback guard (skip the post-rename chmod when the target \
            vanished, so None is never handed to os.chmod). Rust has no post-rename Windows chmod \
            branch on POSIX to exercise."]
#[test]
fn test_keep_mode_windows_fallback_is_skipped_when_there_is_no_mode() {}

#[ignore = "UNMAPPED: Windows-only. The post-rename restore's best-effort suppression is the \
            Windows chmod branch, not built or reachable on POSIX; the POSIX swallow is covered in \
            the keep_mode suppression note above."]
#[test]
fn test_keep_mode_windows_fallback_suppresses_a_chmod_failure() {}

// ===========================================================================
// try_advisory_file_lock — the never-waits variant for read-path writes.
// Rust exposes only the blocking `File::lock()` / `File::lock_shared()`; there is no non-blocking
// try-lock variant. Absent, not off-seam — flagged for the superset rule.
// ===========================================================================

#[ignore = "UNMAPPED + FEATURE-PARITY FINDING: Rust has no non-blocking try_advisory_file_lock; \
            acquire_lock always blocks (File::lock). The never-waits read-path variant is absent \
            (superset rule) — supervisor should confirm whether the read path needs it."]
#[test]
fn test_try_lock_acquires_when_free_and_excludes_a_second_taker() {}

#[ignore = "UNMAPPED: no non-blocking try-lock variant in Rust (see above)."]
#[test]
fn test_try_lock_declines_while_the_blocking_lock_is_held() {}

#[ignore = "UNMAPPED: no non-blocking try-lock variant in Rust (see above); the cross-process \
            decline-when-native-lock-held path has no equivalent."]
#[test]
fn test_try_lock_declines_when_only_the_native_lock_is_held() {}

#[ignore = "UNMAPPED: no non-blocking try-lock variant in Rust (see above)."]
#[test]
fn test_try_lock_treats_an_unopenable_lock_file_as_not_acquired() {}
