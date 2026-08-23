//! Exact ownership manifest for the 78 tests in Python 0.4 `test_store.py`.
//!
//! Seventy-three names have executable Rust owners. Five names remain as semantic or version
//! closures. The closures keep their ignored bodies so the accounting cannot silently discard an
//! oracle contract.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const ORACLE_NAMES: [&str; 78] = [
    "test_add_copy_preserves_original_verbatim",
    "test_add_reference_points_to_origin",
    "test_name_conflict_rejected",
    "test_slug_dedup",
    "test_resolve_and_remove",
    "test_remove_copy_does_not_touch_original",
    "test_add_command_entry",
    "test_command_requires_nonempty_template",
    "test_doctor_rebuild_from_meta",
    "test_doctor_reports_missing_reference",
    "test_syntax_error_script_still_addable",
    "test_add_python_missing_file_raises",
    "test_add_exe_roundtrip",
    "test_add_exe_missing_file_raises",
    "test_list_entries_skips_corrupt_meta",
    "test_doctor_rebuild_corrupt_meta",
    "test_update_dependencies_copy_mode",
    "test_resolve_not_found_raises",
    "test_dir_size_sums_only_files_recursively",
    "test_dir_size_missing_dir_is_zero",
    "test_dir_size_on_a_file_is_zero",
    "test_human_size_units_and_thresholds",
    "test_infer_kind_python_and_forced_exe",
    "test_infer_kind_posix_uses_execute_bit",
    "test_infer_kind_windows_uses_pathext_not_execute_bit",
    "test_infer_kind_windows_reads_pathext_env",
    "test_infer_kind_windows_falls_back_to_default_pathext",
    "test_extract_comment_description_first_comment_line_wins",
    "test_extract_comment_description_skips_shebang_and_blank_lines",
    "test_extract_comment_description_skips_metadata_fence",
    "test_extract_comment_description_empty_comment_line_continues",
    "test_extract_comment_description_code_first_is_empty",
    "test_extract_comment_description_only_shebang_is_empty",
    "test_extract_comment_description_lua_double_dash_prefix",
    "test_add_script_copy_is_byte_identical_and_records_hash",
    "test_add_script_reference_points_to_origin",
    "test_add_script_explicit_workdir_override",
    "test_add_script_explicit_name_and_description",
    "test_add_script_records_interpreter",
    "test_add_script_unknown_kind_raises",
    "test_add_script_non_interpreted_kind_raises",
    "test_add_script_missing_file_raises",
    "test_add_script_lua_uses_double_dash_description",
    "test_summaries_match_full_entries_field_for_field",
    "test_summaries_serve_from_the_index_without_parsing_metas",
    "test_a_row_an_older_skit_wrote_falls_back_to_its_meta",
    "test_a_hand_broken_row_falls_back_instead_of_inventing_a_summary",
    "test_a_broken_row_over_a_corrupt_meta_is_skipped_like_list_entries",
    "test_rename_and_describe_keep_the_index_in_step",
    "test_an_older_registry_is_widened_the_first_time_it_is_listed",
    "test_repair_never_drops_an_entry_added_meanwhile",
    "test_repair_skips_an_entry_removed_meanwhile",
    "test_a_store_that_cannot_be_written_still_lists",
    "test_a_corrupt_index_lists_nothing_and_preserves_the_bad_bytes",
    "test_exe_is_always_reference_mode",
    "test_an_entry_whose_meta_is_gone_is_not_listed",
    "test_a_corrupted_meta_drops_out_of_the_listing_like_every_other_face",
    "test_a_non_mapping_row_falls_back_instead_of_crashing",
    "test_widening_gives_up_on_a_row_it_would_reject_again",
    "test_repair_keeps_a_rename_that_landed_meanwhile",
    "test_repair_adopts_a_slug_reused_by_an_older_skit_meanwhile",
    "test_a_renamed_legacy_row_is_upgraded_not_patched",
    "test_a_reference_row_without_a_target_falls_back_to_its_meta",
    "test_a_command_row_keeps_an_empty_target",
    "test_a_hand_edited_meta_shows_up_on_the_next_listing",
    "test_a_listing_never_blocks_on_the_registry_lock",
    "test_a_reference_row_that_lost_its_target_is_repaired_once",
    "test_an_emptied_target_on_a_file_kind_falls_back_to_the_meta",
    "test_resolve_survives_a_hand_broken_row",
    "test_a_fresh_stamped_row_with_broken_fields_falls_back",
    "test_an_index_whose_entries_key_is_not_a_table_reads_empty",
    "test_repair_skips_a_meta_that_broke_or_went_unrepresentable_meanwhile",
    "test_a_copy_mode_exe_meta_still_reports_its_gone_binary",
    "test_add_survives_a_hand_broken_row_that_can_claim_no_name",
    "test_an_entry_whose_row_was_mangled_still_defends_its_name",
    "test_a_meta_mutator_leaves_a_row_the_next_listing_serves_untouched",
    "test_a_mutator_whose_row_vanished_mid_write_persists_the_meta_without_resurrecting_it",
    "test_a_listing_survives_an_entry_removed_while_it_was_mid_fallback",
];

// Include the former duplicate location so a regression to the oracle name fails this manifest.
const OWNER_FILES: [&str; 10] = [
    "crates/skit-application/tests/payload_policy.rs",
    "crates/skit-cli/src/cli/tests.rs",
    "crates/skit-cli/tests/port_test_cli.rs",
    "crates/skit-cli/tests/port_test_store_add.rs",
    "crates/skit-cli/tests/port_test_store_forced_exe.rs",
    "crates/skit-language/tests/port_test_store_inference.rs",
    "crates/skit-runtime/tests/port_test_store_exe_target.rs",
    "crates/skit-store/src/read.rs",
    "crates/skit-store/tests/port_test_store.rs",
    "crates/skit-store/tests/registry_fast_read.rs",
];

struct Closure {
    name: &'static str,
    reason: &'static str,
    stronger_owner: &'static str,
}

const CLOSURES: [Closure; 5] = [
    Closure {
        name: "test_add_script_explicit_workdir_override",
        reason: "The Python private add helper fuses orchestration with persistence. Rust keeps the lane policy above the store and round-trips the selected workdir below it.",
        stronger_owner: "skit-application/tests/payload_policy.rs::add_workdir_keeps_script_prompt_executable_and_command_lanes_distinct",
    },
    Closure {
        name: "test_add_script_records_interpreter",
        reason: "Interpreter selection is add orchestration. FileStore accepts and round-trips the typed setting without selecting it.",
        stronger_owner: "skit-cli/tests/port_test_add_validation_contracts.rs::test_cli_add_shell_script_records_interpreter + skit-store/tests/mutations.rs::create_is_atomic_mints_identity_and_preserves_payload_bytes",
    },
    Closure {
        name: "test_add_script_non_interpreted_kind_raises",
        reason: "The Python private helper rejects executable entries, but the Rust public add lane intentionally supports --kind exe while keeping explicit authoring kinds closed.",
        stronger_owner: "skit-cli/tests/port_test_add_validation_contracts.rs::test_cli_add_kind_exe + skit-application/tests/payload_policy.rs::forced_add_kinds_are_closed_without_closing_stored_entry_kinds",
    },
    Closure {
        name: "test_summaries_serve_from_the_index_without_parsing_metas",
        reason: "Version 0.4 trusts mtime alone. Rust uses a stronger content-hashed cache proof, so forged mtime with corrupt bytes must fall back instead of serving stale data.",
        stronger_owner: "skit-store/src/read.rs::private_tests::a_verified_cache_hit_does_not_call_the_authoritative_reader",
    },
    Closure {
        name: "test_widening_gives_up_on_a_row_it_would_reject_again",
        reason: "Rust rejects an invalid enum in authoritative metadata and isolates that entry. Its repair path still proves one-time convergence for representable rows.",
        stronger_owner: "skit-store/tests/port_test_store.rs::test_an_older_registry_is_widened_the_first_time_it_is_listed + test_a_reference_row_that_lost_its_target_is_repaired_once",
    },
];

#[derive(Debug)]
struct Occurrence {
    name: String,
    file: &'static str,
    ignored: bool,
}

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn test_name(line: &str) -> Option<&str> {
    let declaration = line.trim_start().strip_prefix("fn ")?.split('(').next()?;
    declaration.starts_with("test_").then_some(declaration)
}

fn occurrences() -> Vec<Occurrence> {
    let expected: BTreeSet<&str> = ORACLE_NAMES.into_iter().collect();
    let root = repository_root();
    let mut found = Vec::new();

    for file in OWNER_FILES {
        let source = fs::read_to_string(root.join(file)).unwrap();
        let mut ignored = false;
        for line in source.lines() {
            if line.trim_start().starts_with("#[ignore") {
                ignored = true;
            }
            if let Some(name) = test_name(line) {
                if expected.contains(name) {
                    found.push(Occurrence {
                        name: name.to_owned(),
                        file,
                        ignored,
                    });
                }
                ignored = false;
            }
        }
    }
    found
}

#[test]
fn store_oracle_has_78_occurrences_78_unique_names_and_the_exact_expected_set() {
    let expected: BTreeSet<String> = ORACLE_NAMES.iter().map(|name| (*name).to_owned()).collect();
    assert_eq!(
        expected.len(),
        78,
        "the manifest itself must not contain duplicates"
    );

    let occurrences = occurrences();
    let mut counts = BTreeMap::<&str, Vec<&str>>::new();
    for occurrence in &occurrences {
        counts
            .entry(&occurrence.name)
            .or_default()
            .push(occurrence.file);
    }
    assert_eq!(
        occurrences.len(),
        78,
        "oracle names must occur exactly 78 times; actual ownership: {counts:#?}"
    );

    let unique: BTreeSet<String> = occurrences
        .iter()
        .map(|occurrence| occurrence.name.clone())
        .collect();
    assert_eq!(
        unique.len(),
        78,
        "every oracle name must have one owner; actual ownership: {counts:#?}"
    );
    assert_eq!(unique, expected, "the owner set must equal the oracle set");
}

#[test]
fn store_oracle_accounts_for_73_executable_owners_and_five_honest_closures() {
    let occurrences = occurrences();
    let ignored: BTreeSet<&str> = occurrences
        .iter()
        .filter(|occurrence| occurrence.ignored)
        .map(|occurrence| occurrence.name.as_str())
        .collect();
    let closures: BTreeSet<&str> = CLOSURES.iter().map(|closure| closure.name).collect();

    assert_eq!(CLOSURES.len(), 5);
    assert_eq!(closures.len(), 5, "closure names must be unique");
    assert_eq!(
        ignored, closures,
        "only the five documented closures may be ignored"
    );
    assert_eq!(occurrences.len() - ignored.len(), 73);

    for closure in CLOSURES {
        assert!(!closure.reason.is_empty());
        assert!(!closure.stronger_owner.is_empty());
    }
}
