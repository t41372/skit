//! Exact-name accounting for Python v0.4 `tests/test_atomic.py` at `main@206f9ef`.
//!
//! Frozen denominator: 32 `def test_` functions. Thirteen are exercised through real public Rust
//! consumers of atomic persistence/locking. Nineteen directly monkeypatch or inspect Python-private
//! syscall helpers (`_try_native_lock`, `_fsync_dir`, `_replace_with_retry`, fchmod/fsync/replace)
//! for deterministic fault/order injection; Rust exposes no equivalent public seam, so those names
//! are architecture-closed rather than recreated with a test-only implementation.

use std::collections::BTreeSet;
use syn::{Attribute, Item};

const EXECUTABLE: &[&str] = &[
    "test_load_toml_recoverable_missing_file_returns_empty_no_backup",
    "test_load_toml_recoverable_valid_file_returns_doc_no_backup",
    "test_load_toml_recoverable_corrupt_file_backs_up_and_returns_empty",
    "test_advisory_file_lock_keeps_a_persistent_one_byte_inode",
    "test_advisory_file_lock_serializes_two_waiting_threads",
    "test_advisory_lock_open_failure_releases_its_thread_mutex",
    "test_atomic_write_text_keep_mode_preserves_existing_mode",
    "test_atomic_write_text_keep_mode_missing_target_skips_chmod",
    "test_keep_mode_windows_fallback_is_skipped_when_there_is_no_mode",
    "test_try_lock_acquires_when_free_and_excludes_a_second_taker",
    "test_try_lock_declines_while_the_blocking_lock_is_held",
    "test_try_lock_declines_when_only_the_native_lock_is_held",
    "test_try_lock_treats_an_unopenable_lock_file_as_not_acquired",
];

const ARCHITECTURE_CLOSED: &[&str] = &[
    "test_load_toml_recoverable_reports_none_when_backup_itself_fails",
    "test_advisory_file_lock_is_released_by_kernel_after_process_crash",
    "test_windows_locking_uses_one_byte_seek_retry_and_unlock",
    "test_native_lock_distinguishes_contention_from_unexpected_os_errors",
    "test_advisory_lock_native_failure_closes_fd_and_releases_mutex",
    "test_atomic_write_bytes_fsyncs_before_replace",
    "test_atomic_write_text_fsyncs_before_replace",
    "test_atomic_write_toml_fsyncs_before_replace",
    "test_atomic_write_bytes_fsyncs_parent_dir_after_replace",
    "test_atomic_write_bytes_dir_fsync_failure_is_swallowed",
    "test_atomic_write_bytes_skips_dir_fsync_on_windows",
    "test_atomic_write_bytes_temp_fsync_failure_still_cleans_up_tmp_file",
    "test_atomic_write_text_keep_mode_applies_mode_before_the_rename",
    "test_atomic_write_text_keep_mode_suppresses_chmod_failure",
    "test_atomic_write_text_keep_mode_falls_back_to_chmod_on_windows",
    "test_replace_retries_through_transient_permission_error",
    "test_replace_gives_up_loudly_after_bounded_attempts",
    "test_replace_other_oserrors_are_not_retried",
    "test_keep_mode_windows_fallback_suppresses_a_chmod_failure",
];

fn has_test(attributes: &[Attribute]) -> bool {
    attributes.iter().any(|attribute| attribute.path().is_ident("test"))
}

fn names(source: &str) -> BTreeSet<String> {
    syn::parse_file(source)
        .expect("Atomic public port source must parse")
        .items
        .into_iter()
        .filter_map(|item| match item {
            Item::Fn(function)
                if has_test(&function.attrs) && function.sig.ident.to_string().starts_with("test_") =>
            {
                Some(function.sig.ident.to_string())
            }
            _ => None,
        })
        .collect()
}

#[test]
fn test_atomic_frozen_names_are_exactly_accounted() {
    assert_eq!(EXECUTABLE.len(), 13);
    assert_eq!(ARCHITECTURE_CLOSED.len(), 19);
    let expected = EXECUTABLE.iter().copied().collect::<BTreeSet<_>>();
    let closed = ARCHITECTURE_CLOSED.iter().copied().collect::<BTreeSet<_>>();
    assert_eq!(expected.len(), 13);
    assert_eq!(closed.len(), 19);
    assert!(expected.is_disjoint(&closed));
    assert_eq!(expected.len() + closed.len(), 32);

    let actual = names(include_str!("../../skit-store/tests/port_test_atomic_public.rs"));
    let actual = actual.iter().map(String::as_str).collect::<BTreeSet<_>>();
    assert_eq!(actual, expected, "Atomic executable parity is incomplete or mislabeled");
    assert!(actual.is_disjoint(&closed));
}
