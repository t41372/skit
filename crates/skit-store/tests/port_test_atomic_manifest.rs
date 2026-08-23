//! Final exact ownership manifest for Python v0.4 `tests/test_atomic.py`.

use std::{collections::BTreeSet, fs, path::Path};

use syn::{Attribute, Item};

const ORACLE: [&str; 32] = [
    "test_load_toml_recoverable_missing_file_returns_empty_no_backup",
    "test_load_toml_recoverable_valid_file_returns_doc_no_backup",
    "test_load_toml_recoverable_corrupt_file_backs_up_and_returns_empty",
    "test_load_toml_recoverable_reports_none_when_backup_itself_fails",
    "test_advisory_file_lock_keeps_a_persistent_one_byte_inode",
    "test_advisory_file_lock_serializes_two_waiting_threads",
    "test_advisory_file_lock_is_released_by_kernel_after_process_crash",
    "test_windows_locking_uses_one_byte_seek_retry_and_unlock",
    "test_native_lock_distinguishes_contention_from_unexpected_os_errors",
    "test_advisory_lock_open_failure_releases_its_thread_mutex",
    "test_advisory_lock_native_failure_closes_fd_and_releases_mutex",
    "test_atomic_write_bytes_fsyncs_before_replace",
    "test_atomic_write_text_fsyncs_before_replace",
    "test_atomic_write_toml_fsyncs_before_replace",
    "test_atomic_write_bytes_fsyncs_parent_dir_after_replace",
    "test_atomic_write_bytes_dir_fsync_failure_is_swallowed",
    "test_atomic_write_bytes_skips_dir_fsync_on_windows",
    "test_atomic_write_bytes_temp_fsync_failure_still_cleans_up_tmp_file",
    "test_atomic_write_text_keep_mode_preserves_existing_mode",
    "test_atomic_write_text_keep_mode_applies_mode_before_the_rename",
    "test_atomic_write_text_keep_mode_missing_target_skips_chmod",
    "test_atomic_write_text_keep_mode_suppresses_chmod_failure",
    "test_atomic_write_text_keep_mode_falls_back_to_chmod_on_windows",
    "test_replace_retries_through_transient_permission_error",
    "test_replace_gives_up_loudly_after_bounded_attempts",
    "test_replace_other_oserrors_are_not_retried",
    "test_keep_mode_windows_fallback_is_skipped_when_there_is_no_mode",
    "test_keep_mode_windows_fallback_suppresses_a_chmod_failure",
    "test_try_lock_acquires_when_free_and_excludes_a_second_taker",
    "test_try_lock_declines_while_the_blocking_lock_is_held",
    "test_try_lock_declines_when_only_the_native_lock_is_held",
    "test_try_lock_treats_an_unopenable_lock_file_as_not_acquired",
];

const ACTIVE_EXACT: [&str; 15] = [
    "test_load_toml_recoverable_missing_file_returns_empty_no_backup",
    "test_load_toml_recoverable_valid_file_returns_doc_no_backup",
    "test_load_toml_recoverable_corrupt_file_backs_up_and_returns_empty",
    "test_load_toml_recoverable_reports_none_when_backup_itself_fails",
    "test_advisory_file_lock_keeps_a_persistent_one_byte_inode",
    "test_advisory_file_lock_serializes_two_waiting_threads",
    "test_advisory_lock_open_failure_releases_its_thread_mutex",
    "test_atomic_write_bytes_fsyncs_before_replace",
    "test_atomic_write_text_fsyncs_before_replace",
    "test_atomic_write_toml_fsyncs_before_replace",
    "test_atomic_write_bytes_temp_fsync_failure_still_cleans_up_tmp_file",
    "test_atomic_write_text_keep_mode_preserves_existing_mode",
    "test_replace_retries_through_transient_permission_error",
    "test_replace_gives_up_loudly_after_bounded_attempts",
    "test_replace_other_oserrors_are_not_retried",
];

struct TargetGate {
    name: &'static str,
    target: &'static str,
    owner: &'static str,
}

const TARGET_GATED_EXACT: [TargetGate; 7] = [
    TargetGate {
        name: "test_advisory_file_lock_is_released_by_kernel_after_process_crash",
        target: "Unix: the frozen oracle explicitly exercises POSIX flock crash release",
        owner: "crates/skit-store/src/fs_ops.rs::test_advisory_file_lock_is_released_by_kernel_after_process_crash",
    },
    TargetGate {
        name: "test_atomic_write_bytes_fsyncs_parent_dir_after_replace",
        target: "Unix: production synchronizes the parent directory only on Unix",
        owner: "crates/skit-store/src/fs_ops.rs::test_atomic_write_bytes_fsyncs_parent_dir_after_replace",
    },
    TargetGate {
        name: "test_atomic_write_bytes_dir_fsync_failure_is_swallowed",
        target: "Unix: only the Unix writer performs the best-effort parent directory sync",
        owner: "crates/skit-store/src/fs_ops.rs::test_atomic_write_bytes_dir_fsync_failure_is_swallowed",
    },
    TargetGate {
        name: "test_atomic_write_bytes_skips_dir_fsync_on_windows",
        target: "non-Unix: the production writer omits the Unix directory-fsync operation",
        owner: "crates/skit-store/src/fs_ops.rs::test_atomic_write_bytes_skips_dir_fsync_on_windows",
    },
    TargetGate {
        name: "test_atomic_write_text_keep_mode_applies_mode_before_the_rename",
        target: "Unix: this exact owner observes Unix mode bits on the temp file",
        owner: "crates/skit-store/src/fs_ops.rs::test_atomic_write_text_keep_mode_applies_mode_before_the_rename",
    },
    TargetGate {
        name: "test_atomic_write_text_keep_mode_missing_target_skips_chmod",
        target: "Unix: this frozen branch is paired with a native Windows additive owner",
        owner: "crates/skit-store/src/fs_ops.rs::test_atomic_write_text_keep_mode_missing_target_skips_chmod",
    },
    TargetGate {
        name: "test_atomic_write_text_keep_mode_suppresses_chmod_failure",
        target: "Unix: this frozen branch is paired with a native Windows additive owner",
        owner: "crates/skit-store/src/fs_ops.rs::test_atomic_write_text_keep_mode_suppresses_chmod_failure",
    },
];

struct Closure {
    name: &'static str,
    reason: &'static str,
    stronger_owner: &'static str,
}

const CLOSURES: [Closure; 10] = [
    Closure {
        name: "test_windows_locking_uses_one_byte_seek_retry_and_unlock",
        reason: "Python specifies its msvcrt call sequence. Rust uses std File locking and the Windows LockFileEx implementation instead.",
        stronger_owner: "crates/skit-store/src/fs_ops.rs::windows_native_lock_blocks_then_resumes_and_keeps_the_sentinel",
    },
    Closure {
        name: "test_native_lock_distinguishes_contention_from_unexpected_os_errors",
        reason: "Rust's blocking lock API does not expose Python's private errno classifier. Its nonblocking read repair has a typed availability classifier.",
        stronger_owner: "crates/skit-store/src/fs_ops.rs::existing_read_lock_degrades_only_when_the_same_writer_lock_cannot_work",
    },
    Closure {
        name: "test_advisory_lock_native_failure_closes_fd_and_releases_mutex",
        reason: "Rust has no Python thread-mutex layer. A failed File lock drops its owned descriptor by RAII, and public retry after an open failure is executable.",
        stronger_owner: "crates/skit-store/tests/port_test_atomic.rs::test_advisory_lock_open_failure_releases_its_thread_mutex",
    },
    Closure {
        name: "test_atomic_write_text_keep_mode_falls_back_to_chmod_on_windows",
        reason: "Python restores mode after replace because it lacks fchmod. Rust applies the Windows readonly permission to the temp handle before replace, which removes the post-commit crash window.",
        stronger_owner: "crates/skit-store/src/fs_ops.rs::windows_native_atomic_write_preserves_existing_readonly_attribute",
    },
    Closure {
        name: "test_keep_mode_windows_fallback_is_skipped_when_there_is_no_mode",
        reason: "Rust has no post-replace chmod fallback. Its native Windows operation seam proves that a missing target never calls permission apply.",
        stronger_owner: "crates/skit-store/src/fs_ops.rs::windows_native_missing_target_skips_permission_apply",
    },
    Closure {
        name: "test_keep_mode_windows_fallback_suppresses_a_chmod_failure",
        reason: "Rust applies permissions before replace and treats failure as best effort. It does not expose Python's post-replace chmod branch.",
        stronger_owner: "crates/skit-store/src/fs_ops.rs::windows_native_permission_apply_failure_is_best_effort_and_cleans_temp",
    },
    Closure {
        name: "test_try_lock_acquires_when_free_and_excludes_a_second_taker",
        reason: "The production try-lock is crate-private because only registry self-heal uses it.",
        stronger_owner: "crates/skit-store/src/fs_ops.rs::try_lock_acquires_when_free_and_a_second_taker_declines",
    },
    Closure {
        name: "test_try_lock_declines_while_the_blocking_lock_is_held",
        reason: "The production try-lock is crate-private and is exercised beside its blocking lock implementation.",
        stronger_owner: "crates/skit-store/src/fs_ops.rs::try_lock_declines_while_the_blocking_lock_is_held",
    },
    Closure {
        name: "test_try_lock_declines_when_only_the_native_lock_is_held",
        reason: "A public listing owns the observable consequence: registry repair never waits on a held native lock.",
        stronger_owner: "crates/skit-store/tests/port_test_store.rs::test_a_listing_never_blocks_on_the_registry_lock",
    },
    Closure {
        name: "test_try_lock_treats_an_unopenable_lock_file_as_not_acquired",
        reason: "The production try-lock is crate-private and its unopenable-path behavior runs in its unit owner.",
        stronger_owner: "crates/skit-store/src/fs_ops.rs::try_lock_treats_an_unopenable_path_as_not_acquired",
    },
];

const OWNER_FILES: [&str; 2] = [
    "crates/skit-store/tests/port_test_atomic.rs",
    "crates/skit-store/src/fs_ops.rs",
];

struct Occurrence {
    name: String,
    ignored: bool,
    target_gated: bool,
    empty: bool,
}

fn has_attribute(attributes: &[Attribute], name: &str) -> bool {
    attributes
        .iter()
        .any(|attribute| attribute.path().is_ident(name))
}

fn occurrences(root: &Path) -> Vec<Occurrence> {
    let oracle = ORACLE.into_iter().collect::<BTreeSet<_>>();
    OWNER_FILES
        .iter()
        .flat_map(|file| {
            fn collect(items: Vec<Item>, oracle: &BTreeSet<&str>, output: &mut Vec<Occurrence>) {
                for item in items {
                    match item {
                        Item::Fn(function) if has_attribute(&function.attrs, "test") => {
                            let name = function.sig.ident.to_string();
                            if oracle.contains(name.as_str()) {
                                output.push(Occurrence {
                                    name,
                                    ignored: has_attribute(&function.attrs, "ignore"),
                                    target_gated: has_attribute(&function.attrs, "cfg"),
                                    empty: function.block.stmts.is_empty(),
                                });
                            }
                        }
                        Item::Mod(module) => {
                            if let Some((_, items)) = module.content {
                                collect(items, oracle, output);
                            }
                        }
                        _ => {}
                    }
                }
            }

            let source = fs::read_to_string(root.join(file)).unwrap();
            let mut output = Vec::new();
            collect(
                syn::parse_file(&source).unwrap().items,
                &oracle,
                &mut output,
            );
            output
        })
        .collect()
}

#[test]
fn atomic_oracle_has_exact_unique_executable_gated_and_structured_ownership() {
    let oracle = ORACLE.into_iter().collect::<BTreeSet<_>>();
    assert_eq!(ORACLE.len(), 32);
    assert_eq!(
        oracle.len(),
        ORACLE.len(),
        "oracle manifest has a duplicate"
    );

    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let occurrences = occurrences(&root);
    let occurrence_names = occurrences
        .iter()
        .map(|owner| owner.name.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(occurrences.len(), 32, "frozen occurrence count drifted");
    assert_eq!(
        occurrence_names.len(),
        occurrences.len(),
        "one frozen owner occurs more than once"
    );
    assert_eq!(occurrence_names, oracle, "the frozen name set changed");

    let active = ACTIVE_EXACT.into_iter().collect::<BTreeSet<_>>();
    let gated = TARGET_GATED_EXACT
        .iter()
        .map(|owner| owner.name)
        .collect::<BTreeSet<_>>();
    let closures = CLOSURES
        .iter()
        .map(|closure| closure.name)
        .collect::<BTreeSet<_>>();
    assert_eq!(active.len(), ACTIVE_EXACT.len());
    assert_eq!(gated.len(), TARGET_GATED_EXACT.len());
    assert_eq!(closures.len(), CLOSURES.len());
    assert!(active.is_disjoint(&gated));
    assert!(active.is_disjoint(&closures));
    assert!(gated.is_disjoint(&closures));
    assert_eq!(
        active
            .union(&gated)
            .copied()
            .collect::<BTreeSet<_>>()
            .union(&closures)
            .copied()
            .collect::<BTreeSet<_>>(),
        oracle
    );

    for occurrence in &occurrences {
        assert!(!occurrence.empty, "empty frozen owner: {}", occurrence.name);
        if active.contains(occurrence.name.as_str()) {
            assert!(!occurrence.ignored);
            assert!(!occurrence.target_gated);
        } else if gated.contains(occurrence.name.as_str()) {
            assert!(!occurrence.ignored);
            assert!(occurrence.target_gated);
        } else {
            assert!(occurrence.ignored);
        }
    }
    for gate in TARGET_GATED_EXACT {
        assert!(!gate.target.trim().is_empty());
        assert!(!gate.owner.trim().is_empty());
    }
    for closure in CLOSURES {
        assert!(!closure.reason.trim().is_empty());
        assert!(!closure.stronger_owner.trim().is_empty());
    }
}
