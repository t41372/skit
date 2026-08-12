//! Mechanical completeness guard for Python `tests/test_effective_uv_metadata.py` at
//! `main@206f9ef`.
//!
//! This is not behavioral coverage. Every mapping points at a real behavioral Rust test; this guard
//! makes the 26-test oracle explicit and refuses fake mappings whose named function is absent or is
//! not itself a `#[test]`.

use std::{collections::BTreeSet, fs, path::Path};

use syn::{Attribute, Item};

struct Mapping {
    python: &'static str,
    path: &'static str,
    rust: &'static str,
}

const CLI: &str = "crates/skit-cli/tests/port_test_effective_uv_metadata.rs";
const SETTINGS: &str = "crates/skit-cli/tests/port_test_effective_uv_metadata_settings.rs";
const REMAINING: &str = "crates/skit-cli/tests/port_test_effective_uv_metadata_remaining.rs";
const EDIT: &str = "crates/skit-language/tests/port_test_effective_uv_metadata_edit.rs";
const READ: &str = "crates/skit-language/tests/port_test_effective_uv_metadata.rs";

const MAPPINGS: &[Mapping] = &[
    Mapping { python: "test_add_dep_then_python_pin_keeps_block_deps_end_to_end", path: CLI, rust: "test_add_dep_then_python_pin_keeps_block_deps_end_to_end" },
    Mapping { python: "test_add_dep_then_python_pin_run_command_carries_both", path: CLI, rust: "test_add_dep_then_python_pin_run_command_carries_both" },
    Mapping { python: "test_settings_prefills_deps_and_python_from_the_block", path: SETTINGS, rust: "test_settings_prefills_deps_and_python_from_the_block" },
    Mapping { python: "test_settings_deps_only_edit_preserves_the_block_pin", path: SETTINGS, rust: "test_settings_deps_only_edit_preserves_the_block_pin" },
    Mapping { python: "test_settings_clearing_python_on_block_only_entry_unpins", path: SETTINGS, rust: "test_settings_clearing_python_on_block_only_entry_unpins" },
    Mapping { python: "test_settings_untouched_save_never_touches_the_deps_axis", path: SETTINGS, rust: "test_settings_untouched_save_never_touches_the_deps_axis" },
    Mapping { python: "test_deps_read_human_reports_effective_block_only", path: CLI, rust: "test_deps_read_human_reports_effective_block_only" },
    Mapping { python: "test_deps_read_json_reports_effective_block_only", path: CLI, rust: "test_deps_read_json_reports_effective_block_only" },
    Mapping { python: "test_show_json_reports_effective_deps_for_block_only", path: CLI, rust: "test_show_json_reports_effective_deps_for_block_only" },
    Mapping { python: "test_deps_read_meta_carried_entry_is_unchanged", path: REMAINING, rust: "test_deps_read_meta_carried_entry_is_unchanged" },
    Mapping { python: "test_deps_read_js_entry_falls_through_to_meta", path: REMAINING, rust: "test_deps_read_js_entry_falls_through_to_meta" },
    Mapping { python: "test_update_dependencies_none_none_is_a_full_no_op", path: EDIT, rust: "test_update_dependencies_none_none_is_a_full_no_op" },
    Mapping { python: "test_update_dependencies_none_python_lands_pin_and_preserves_block_deps", path: EDIT, rust: "test_update_dependencies_none_python_lands_pin_and_preserves_block_deps" },
    Mapping { python: "test_update_dependencies_clear_deps_preserves_the_pin", path: EDIT, rust: "test_update_dependencies_clear_deps_preserves_the_pin" },
    Mapping { python: "test_update_dependencies_python_only_edit_syncs_block_from_meta_deps", path: EDIT, rust: "test_update_dependencies_python_only_edit_syncs_block_from_meta_deps" },
    Mapping { python: "test_update_dependencies_missing_stored_copy_still_writes_meta", path: EDIT, rust: "test_update_dependencies_missing_stored_copy_still_writes_meta" },
    Mapping { python: "test_update_dependencies_npm_none_does_not_sweep_node_modules", path: REMAINING, rust: "test_update_dependencies_npm_none_does_not_sweep_node_modules" },
    Mapping { python: "test_update_dependencies_npm_clear_does_sweep_node_modules", path: REMAINING, rust: "test_update_dependencies_npm_clear_does_sweep_node_modules" },
    Mapping { python: "test_effective_meta_carried_skips_the_block", path: READ, rust: "test_effective_meta_carried_skips_the_block" },
    Mapping { python: "test_effective_block_only_reads_both_axes_from_the_block", path: READ, rust: "test_effective_block_only_reads_both_axes_from_the_block" },
    Mapping { python: "test_effective_meta_deps_blank_constraint_reads_constraint_from_block", path: READ, rust: "test_effective_meta_deps_blank_constraint_reads_constraint_from_block" },
    Mapping { python: "test_effective_meta_constraint_blank_deps_reads_deps_from_block", path: READ, rust: "test_effective_meta_constraint_blank_deps_reads_deps_from_block" },
    Mapping { python: "test_effective_both_blank_returns_empty", path: READ, rust: "test_effective_both_blank_returns_empty" },
    Mapping { python: "test_effective_reference_mode_python_reads_meta_only", path: READ, rust: "test_effective_reference_mode_python_reads_meta_only" },
    Mapping { python: "test_effective_js_entry_reads_meta_only", path: READ, rust: "test_effective_js_entry_reads_meta_only" },
    Mapping { python: "test_effective_missing_stored_copy_reads_meta_only", path: READ, rust: "test_effective_missing_stored_copy_reads_meta_only" },
];

fn has_test_attribute(attributes: &[Attribute]) -> bool {
    attributes
        .iter()
        .any(|attribute| attribute.path().is_ident("test"))
}

#[test]
fn every_effective_uv_metadata_python_test_has_a_real_rust_test() {
    assert_eq!(
        MAPPINGS.len(),
        26,
        "frozen Python effective-UV oracle changed without an intentional mapping update"
    );
    assert_eq!(
        MAPPINGS.iter().map(|mapping| mapping.python).collect::<BTreeSet<_>>().len(),
        26,
        "duplicate Python names make the completeness count dishonest"
    );

    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("skit-cli lives under <repo>/crates/skit-cli");
    let mut failures = Vec::new();

    for mapping in MAPPINGS {
        let path = repo.join(mapping.path);
        let source = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("could not read {}: {error}", path.display()));
        let file = syn::parse_file(&source)
            .unwrap_or_else(|error| panic!("{} is not valid Rust: {error}", path.display()));
        let matched = file.items.iter().find_map(|item| match item {
            Item::Fn(function) if function.sig.ident == mapping.rust => {
                Some(has_test_attribute(&function.attrs))
            }
            _ => None,
        });
        match matched {
            Some(true) => {}
            Some(false) => failures.push(format!(
                "{} -> {}::{} exists but is not #[test]",
                mapping.python, mapping.path, mapping.rust
            )),
            None => failures.push(format!(
                "{} -> {}::{} is missing",
                mapping.python, mapping.path, mapping.rust
            )),
        }
    }

    assert!(
        failures.is_empty(),
        "effective-UV parity manifest contains fake/non-executable mappings:\n{}",
        failures.join("\n")
    );
}
