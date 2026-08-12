//! Completeness guard for Python v0.4 `tests/test_store_fix.py` at `main@206f9ef`.
//!
//! Thirty-six contracts have same-named executable Rust oracles. Two Python tests deliberately
//! inject themselves below Rust's public product seams:
//!
//! * `test_add_entry_refuses_preexisting_nonempty_directory` calls private Python `_add_entry`
//!   with a preselected occupied slug, bypassing the allocator. Rust's only create API allocates a
//!   free slug first; the public occupied-directory defense is tested separately, but pretending it
//!   exercises the bypassed second defense would be weaker.
//! * `test_update_dependencies_copy_sync_swallows_read_oserror` monkeypatches exactly the source
//!   `read_bytes()` call after path resolution. Rust's composition root performs `fs::read(path).ok()`
//!   directly with no injectable read port. Permission/race tricks are platform-dependent and are
//!   not accepted as parity evidence.
//!
//! Those two names must therefore stay absent until an equivalent stable test seam exists. They are
//! accounted gaps, not fake passing tests.

use std::collections::{BTreeMap, BTreeSet};

use syn::{Attribute, Item};

const SOURCES: &[&str] = &[
    include_str!("../../skit-store/tests/port_test_store_fix.rs"),
    include_str!("../../skit-store/tests/port_test_store_fix_corruption_exact.rs"),
    include_str!("port_test_store_fix_workdir.rs"),
    include_str!("../../skit-store/tests/port_test_store_fix_corrupt_registry.rs"),
    include_str!("port_test_store_fix_python_bytes.rs"),
    include_str!("../../skit-store/tests/port_test_store_fix_atomic_rollback.rs"),
    include_str!("../../skit-store/tests/port_test_store_fix_registry_lock.rs"),
    include_str!("../../skit-store/tests/port_test_store_concurrency.rs"),
];

const EXECUTABLE: &[&str] = &[
    "test_from_toml_dict_missing_name_raises_scriptmetaerror_not_keyerror",
    "test_from_toml_dict_missing_kind_raises_scriptmetaerror_not_keyerror",
    "test_list_entries_skips_valid_toml_missing_name_key",
    "test_doctor_rebuild_reports_missing_key_instead_of_crashing",
    "test_resolve_corrupt_missing_key_meta_raises_notfounderror_not_keyerror",
    "test_from_toml_dict_scalar_dependencies_raises_scriptmetaerror_not_typeerror",
    "test_from_toml_dict_scalar_params_raises_scriptmetaerror_not_typeerror",
    "test_list_entries_skips_scalar_dependencies_meta",
    "test_doctor_rebuild_reports_scalar_params_instead_of_crashing",
    "test_resolve_scalar_dependencies_meta_raises_notfounderror_not_typeerror",
    "test_from_toml_dict_missing_key_message_is_gettext_wrapped",
    "test_from_toml_dict_invalid_type_message_is_gettext_wrapped",
    "test_lost_registry_name_collision_does_not_clobber_existing_script",
    "test_lost_registry_slug_collision_gets_deduped_not_overwritten",
    "test_add_entry_still_reuses_preexisting_empty_slug_dir",
    "test_fs_truth_ignores_stray_non_directory_entries_in_scripts_dir",
    "test_fs_truth_skips_unreadable_meta_in_unregistered_orphan_directory",
    "test_add_python_copy_mode_defaults_workdir_to_invoke",
    "test_add_python_reference_mode_still_defaults_workdir_to_origin",
    "test_add_python_copy_mode_explicit_workdir_override_still_respected",
    "test_corrupt_registry_is_backed_up_and_degrades_to_empty",
    "test_corrupt_registry_recovers_fully_via_doctor_rebuild",
    "test_add_python_non_utf8_source_skips_injection_keeps_deps_in_meta",
    "test_add_python_utf8_source_still_injects_normally",
    "test_add_python_injected_write_failure_rolls_back_entire_entry",
    "test_registry_lock_serializes_concurrent_holders",
    "test_registry_lock_uses_a_versioned_persistent_native_inode",
    "test_concurrent_add_python_both_succeed_with_distinct_slugs",
    "test_update_dependencies_copy_non_utf8_leaves_stored_copy_byte_identical",
    "test_update_dependencies_copy_utf8_syncs_block_and_stays_utf8",
    "test_update_dependencies_refuses_when_a_non_utf8_copy_carries_its_own_block",
    "test_update_dependencies_python_unpin_is_refused_for_the_same_copy",
    "test_update_dependencies_untouched_axes_never_reach_the_refusal",
    "test_deps_edit_on_a_crlf_copy_keeps_one_block_and_its_params",
    "test_add_with_deps_does_not_double_block_a_crlf_script",
    "test_add_keeps_an_lf_script_lf_when_injecting_a_block",
];

const BLOCKED: &[&str] = &[
    "test_add_entry_refuses_preexisting_nonempty_directory",
    "test_update_dependencies_copy_sync_swallows_read_oserror",
];

fn is_test(attributes: &[Attribute]) -> bool {
    attributes
        .iter()
        .any(|attribute| attribute.path().is_ident("test"))
}

fn names(source: &str) -> Vec<String> {
    syn::parse_file(source)
        .expect("store-fix parity source must parse as Rust")
        .items
        .into_iter()
        .filter_map(|item| match item {
            Item::Fn(function) if is_test(&function.attrs) => Some(function.sig.ident.to_string()),
            _ => None,
        })
        .collect()
}

#[test]
fn every_executable_store_fix_python_contract_maps_exactly_once() {
    let expected = EXECUTABLE.iter().copied().collect::<BTreeSet<_>>();
    assert_eq!(expected.len(), 36, "executable store-fix inventory drifted");

    let mut counts = BTreeMap::<String, usize>::new();
    for source in SOURCES {
        for name in names(source) {
            if expected.contains(name.as_str()) || BLOCKED.contains(&name.as_str()) {
                *counts.entry(name).or_default() += 1;
            }
        }
    }

    for name in EXECUTABLE {
        assert_eq!(
            counts.get(*name).copied().unwrap_or_default(),
            1,
            "Python store-fix contract {name} must map to exactly one executable Rust test"
        );
    }
}

#[test]
fn private_only_store_fix_contracts_are_not_faked_as_coverage() {
    let all_names = SOURCES
        .iter()
        .flat_map(|source| names(source))
        .collect::<Vec<_>>();
    for blocked in BLOCKED {
        assert!(
            !all_names.iter().any(|name| name == blocked),
            "{blocked} has no stable equivalent Rust test seam; do not paper it over with a weaker same-name test"
        );
    }
    assert_eq!(EXECUTABLE.len() + BLOCKED.len(), 38);
}
