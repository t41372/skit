//! Exact-name accounting gate for Python v0.4 `tests/test_store.py` at `main@206f9ef`.
//!
//! Frozen denominator: 78 `def test_` functions. Seventy have public Rust behavioral seams.
//! Eight are architecture-closed only because the Python oracle directly calls/monkeypatches
//! private helpers for formatting or deterministic fault/race injection that Rust does not expose.
//! The closed names are still frozen here so they cannot silently disappear from accounting.

use std::collections::BTreeSet;
use syn::{Attribute, Item};

const EXECUTABLE: &[&str] = &[
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
    "test_a_corrupt_index_lists_nothing_and_preserves_the_bad_bytes",
    "test_exe_is_always_reference_mode",
    "test_an_entry_whose_meta_is_gone_is_not_listed",
    "test_a_corrupted_meta_drops_out_of_the_listing_like_every_other_face",
    "test_a_non_mapping_row_falls_back_instead_of_crashing",
    "test_widening_gives_up_on_a_row_it_would_reject_again",
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
    "test_a_copy_mode_exe_meta_still_reports_its_gone_binary",
    "test_add_survives_a_hand_broken_row_that_can_claim_no_name",
    "test_an_entry_whose_row_was_mangled_still_defends_its_name",
    "test_a_meta_mutator_leaves_a_row_the_next_listing_serves_untouched",
];

const ARCHITECTURE_CLOSED: &[&str] = &[
    // Python exposes `store.human_size`; Rust's equivalent `health_size_text` is a private CLI
    // composition helper. Public doctor JSON exposes raw `size_bytes`, already covered separately.
    "test_human_size_units_and_thresholds",
    // The remaining seven directly invoke or monkeypatch Python-private repair/read/write helpers
    // to inject exact races/failures. Rust has no deterministic public synchronization/fault seam;
    // recreating those helpers in tests would be a fake port rather than behavioral evidence.
    "test_repair_skips_an_entry_removed_meanwhile",
    "test_a_store_that_cannot_be_written_still_lists",
    "test_repair_keeps_a_rename_that_landed_meanwhile",
    "test_repair_adopts_a_slug_reused_by_an_older_skit_meanwhile",
    "test_repair_skips_a_meta_that_broke_or_went_unrepresentable_meanwhile",
    "test_a_mutator_whose_row_vanished_mid_write_persists_the_meta_without_resurrecting_it",
    "test_a_listing_survives_an_entry_removed_while_it_was_mid_fallback",
];

fn has_test(attributes: &[Attribute]) -> bool {
    attributes.iter().any(|attribute| attribute.path().is_ident("test"))
}

fn test_names(source: &str) -> BTreeSet<String> {
    syn::parse_file(source)
        .expect("Store port source must parse")
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
fn test_store_frozen_names_are_exactly_accounted() {
    assert_eq!(EXECUTABLE.len(), 70);
    assert_eq!(ARCHITECTURE_CLOSED.len(), 8);
    let expected = EXECUTABLE.iter().copied().collect::<BTreeSet<_>>();
    let closed = ARCHITECTURE_CLOSED.iter().copied().collect::<BTreeSet<_>>();
    assert_eq!(expected.len(), 70, "duplicate executable names corrupt Store accounting");
    assert_eq!(closed.len(), 8, "duplicate closed names corrupt Store accounting");
    assert!(expected.is_disjoint(&closed));
    assert_eq!(expected.len() + closed.len(), 78);

    let mut actual = BTreeSet::new();
    for source in [
        include_str!("port_test_store_public.rs"),
        include_str!("port_test_store_disk.rs"),
        include_str!("port_test_store_forced_exe.rs"),
        include_str!("port_test_store_windows_pathext_bit.rs"),
        include_str!("port_test_store_windows_pathext_env.rs"),
        include_str!("port_test_store_windows_pathext_default.rs"),
        include_str!("port_test_store_add_script.rs"),
        include_str!("../../skit-language/tests/port_test_store_inference.rs"),
        include_str!("../../skit-store/tests/port_test_store_index.rs"),
        include_str!("../../skit-store/tests/port_test_store_index_edges.rs"),
        include_str!("../../skit-store/tests/port_test_store_registry_repair.rs"),
        include_str!("../../skit-store/tests/port_test_store_membership.rs"),
        include_str!("../../skit-store/tests/port_test_store_mutator_freshness.rs"),
        include_str!("../../skit-runtime/tests/port_test_store_exe_target.rs"),
    ] {
        actual.extend(test_names(source));
    }
    let actual = actual.iter().map(String::as_str).collect::<BTreeSet<_>>();
    assert_eq!(
        actual, expected,
        "Store executable parity is incomplete or a non-frozen test_* name is mislabeled"
    );
    assert!(actual.is_disjoint(&closed));
}
