//! Final exact ownership manifest for Python v0.4 `tests/test_store_fix.py`.

use std::{collections::BTreeSet, fs, path::Path};

use syn::{Attribute, Item};

const ORACLE: [&str; 38] = [
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
    "test_add_entry_refuses_to_reuse_an_existing_nonempty_directory",
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
    "test_update_dependencies_copy_sync_swallows_read_oserror",
    "test_update_dependencies_refuses_when_a_non_utf8_copy_carries_its_own_block",
    "test_update_dependencies_python_unpin_is_refused_for_the_same_copy",
    "test_update_dependencies_untouched_axes_never_reach_the_refusal",
    "test_deps_edit_on_a_crlf_copy_keeps_one_block_and_its_params",
    "test_add_with_deps_does_not_double_block_a_crlf_script",
    "test_add_keeps_an_lf_script_lf_when_injecting_a_block",
];

const OWNER_FILES: [&str; 7] = [
    "crates/skit-store/tests/port_test_store_fix_metadata.rs",
    "crates/skit-store/tests/port_test_store_fix_filesystem.rs",
    "crates/skit-store/src/mutations.rs",
    "crates/skit-store/src/fs_ops.rs",
    "crates/skit-store/tests/mutations.rs",
    "crates/skit-cli/tests/port_test_store_fix_add_deps.rs",
    "crates/skit-cli/src/cli/tests.rs",
];

struct Closure {
    name: &'static str,
    reason: &'static str,
    stronger_owner: &'static str,
}

const CLOSURES: [Closure; 6] = [
    Closure {
        name: "test_lost_registry_name_collision_does_not_clobber_existing_script",
        reason: "Python deletes a rebuildable index before a private add. Rust derives conflicts from authoritative entry metadata under the create lock, even when a cached row is unusable.",
        stronger_owner: "crates/skit-store/tests/port_test_store.rs::test_an_entry_whose_row_was_mangled_still_defends_its_name",
    },
    Closure {
        name: "test_add_entry_refuses_to_reuse_an_existing_nonempty_directory",
        reason: "Python monkeypatches its private slug allocator to return an occupied directory. Rust has no allocator override: allocation and private staging commit form one transaction that preserves the first payload on conflict.",
        stronger_owner: "crates/skit-store/tests/mutations.rs::create_refuses_conflicts_and_path_traversal_without_partial_entries",
    },
    Closure {
        name: "test_add_entry_still_reuses_preexisting_empty_slug_dir",
        reason: "The combined Rust stale-path owner proves both halves: an empty stale directory is reusable and a regular file at the same slug remains reserved and unchanged.",
        stronger_owner: "crates/skit-store/tests/registry_edge_contracts.rs::empty_stale_directories_are_reused_but_regular_files_stay_reserved",
    },
    Closure {
        name: "test_fs_truth_ignores_stray_non_directory_entries_in_scripts_dir",
        reason: "Rust has no Python _fs_truth helper. Its combined filesystem owner places a regular file in scripts, completes a create with a deduplicated slug, and preserves the stray bytes.",
        stronger_owner: "crates/skit-store/tests/registry_edge_contracts.rs::empty_stale_directories_are_reused_but_regular_files_stay_reserved",
    },
    Closure {
        name: "test_add_python_copy_mode_explicit_workdir_override_still_respected",
        reason: "The Python store helper accepts workdir during add. Rust keeps add defaults in application policy and exposes explicit workdir changes through the frontend-neutral params/settings mutation path.",
        stronger_owner: "crates/skit-cli/tests/port_test_cli.rs::params_command_matrix_updates_every_declared_axis_and_preserves_machine_shape",
    },
    Closure {
        name: "test_corrupt_registry_recovers_fully_via_doctor_rebuild",
        reason: "The stronger store owner preserves corrupt index bytes, proves the initial empty listing, rebuilds from untouched metadata, and verifies the recovered listing.",
        stronger_owner: "crates/skit-store/tests/port_test_store.rs::test_a_corrupt_index_lists_nothing_and_preserves_the_bad_bytes",
    },
];

struct Occurrence {
    name: String,
    ignored: bool,
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
fn test_store_fix_has_32_unique_executable_owners_and_six_structured_closures() {
    let oracle = ORACLE.into_iter().collect::<BTreeSet<_>>();
    assert_eq!(ORACLE.len(), 38);
    assert_eq!(
        oracle.len(),
        ORACLE.len(),
        "oracle manifest has a duplicate"
    );

    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let occurrences = occurrences(&root);
    let executable = occurrences
        .iter()
        .map(|owner| owner.name.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(occurrences.len(), 32, "executable occurrence count drifted");
    assert_eq!(
        executable.len(),
        occurrences.len(),
        "one frozen owner occurs more than once"
    );
    assert!(
        occurrences.iter().all(|owner| !owner.ignored),
        "an executable owner became ignored"
    );

    let closures = CLOSURES
        .iter()
        .map(|closure| closure.name)
        .collect::<BTreeSet<_>>();
    assert_eq!(CLOSURES.len(), 6);
    assert_eq!(
        closures.len(),
        CLOSURES.len(),
        "closure names are not unique"
    );
    assert!(executable.is_disjoint(&closures));
    assert_eq!(
        executable
            .union(&closures)
            .copied()
            .collect::<BTreeSet<_>>(),
        oracle
    );
    for closure in CLOSURES {
        assert!(!closure.reason.trim().is_empty());
        assert!(!closure.stronger_owner.trim().is_empty());
    }
}
