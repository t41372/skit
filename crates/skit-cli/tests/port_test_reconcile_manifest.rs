//! Mechanical completeness guard for Python v0.4 `tests/test_reconcile.py`.
//!
//! The 27 frozen tests deliberately split across two Rust layers: 14 source-reconciliation facts
//! live in skit-language; 13 rendering/edit/resync facts execute at the CLI/storage boundary. This
//! manifest is not behavioral coverage and cannot substitute for either target.

use std::{fs, path::Path};

use syn::{Attribute, Item};

const LANGUAGE: &[&str] = &[
    "test_all_ok_no_drift",
    "test_const_missing_by_name",
    "test_const_renamed_is_missing_plus_new",
    "test_const_type_changed_still_usable",
    "test_input_matched_by_order_not_position_in_file",
    "test_input_removed_is_missing",
    "test_new_input_call_reported_as_new_only",
    "test_input_prompt_match_survives_an_earlier_insertion_no_drift",
    "test_input_deleted_earlier_call_flags_rebind_instead_of_silent_ok",
    "test_input_rebind_flagged_when_prompt_can_no_longer_disambiguate",
    "test_unselected_candidates_are_new_but_not_drift",
    "test_input_duplicate_prompt_surplus_is_missing_not_ok_on_delete",
    "test_input_duplicate_prompt_surplus_is_rebind_not_ok_when_position_edited",
    "test_syntax_error_marks_all_missing",
];

const HIGHER: &[&str] = &[
    "test_drift_lines_mention_rebind",
    "test_resync_reanchors_rebound_input_order_and_prompt",
    "test_drift_lines_mention_old_and_new_type",
    "test_edit_specs_not_managed_in_secret_warning",
    "test_edit_specs_not_managed_in_no_secret_warning",
    "test_edit_specs_not_managed_in_prompts_warning",
    "test_resync_on_unparseable_script_leaves_definitions_untouched",
    "test_resync_syntax_error_does_not_also_apply_other_edits_incorrectly",
    "test_render_warning_resync_skipped",
    "test_edit_specs_remove_with_duplicate_names_does_not_crash",
    "test_edit_specs_resync_drop_with_duplicate_names_does_not_crash",
    "test_edit_specs_dedups_duplicate_names_even_when_untouched",
    "test_no_secret_also_clears_the_env_source",
];

fn has_test_attribute(attributes: &[Attribute]) -> bool {
    attributes.iter().any(|attribute| attribute.path().is_ident("test"))
}

fn missing(path: &Path, names: &[&str]) -> Vec<String> {
    let source = fs::read_to_string(path).unwrap();
    let file = syn::parse_file(&source)
        .unwrap_or_else(|error| panic!("{} is not valid Rust: {error}", path.display()));
    names
        .iter()
        .filter_map(|name| {
            let executable = file.items.iter().any(|item| match item {
                Item::Fn(function) => {
                    function.sig.ident == *name && has_test_attribute(&function.attrs)
                }
                _ => false,
            });
            (!executable).then(|| (*name).to_owned())
        })
        .collect()
}

#[test]
fn every_python_reconcile_test_has_an_executable_rust_oracle() {
    assert_eq!(LANGUAGE.len() + HIGHER.len(), 27);
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap();
    let mut absent = missing(
        &repo.join("crates/skit-language/tests/port_test_reconcile.rs"),
        LANGUAGE,
    );
    absent.extend(missing(
        &repo.join("crates/skit-cli/tests/port_test_reconcile_edit.rs"),
        HIGHER,
    ));
    assert!(
        absent.is_empty(),
        "frozen Python reconcile tests are missing or not executable #[test] functions:\n{}",
        absent.join("\n")
    );
}

#[test]
fn higher_layer_reconcile_contracts_are_not_backfilled_with_ignore_stubs() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap();
    let source = fs::read_to_string(repo.join("crates/skit-language/tests/port_test_reconcile.rs"))
        .unwrap();
    let file = syn::parse_file(&source).unwrap();
    for name in HIGHER {
        assert!(
            !file.items.iter().any(|item| match item {
                Item::Fn(function) => function.sig.ident == *name,
                _ => false,
            }),
            "{name} belongs to the executable higher-layer target, not a language placeholder"
        );
    }
}
