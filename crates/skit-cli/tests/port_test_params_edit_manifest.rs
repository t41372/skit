//! Mechanical completeness guard for Python v0.4 `tests/test_params_edit.py`.
//!
//! This is not behavioral coverage. It only freezes the 41 Python test names and points each at a
//! real executable Rust oracle. One Python private-helper contract has no public Rust seam; that
//! entry is explicitly mapped to Python's own stronger public command-template contract rather than
//! faking an internal helper through the wrong layer.

use std::{fs, path::Path};

struct Mapping {
    python: &'static str,
    path: &'static str,
    rust: &'static str,
    architectural_note: &'static str,
}

const D: &str = "crates/skit-cli/tests/port_test_params_edit_declared.rs";
const B: &str = "crates/skit-cli/tests/port_test_params_edit_bool_pipeline.rs";
const H: &str = "crates/skit-domain/tests/port_test_params_edit_helpers.rs";
const C: &str = "crates/skit-cli/tests/port_test_declared_params_cli.rs";

const MAPPINGS: &[Mapping] = &[
    Mapping { python: "test_add_defaults_to_first_allowed_delivery_for_a_binary", path: D, rust: "test_add_defaults_to_first_allowed_delivery_for_a_binary", architectural_note: "" },
    Mapping { python: "test_add_on_a_template_placeholder_name_becomes_a_required_placeholder", path: D, rust: "test_add_on_a_template_placeholder_name_becomes_a_required_placeholder", architectural_note: "" },
    Mapping { python: "test_add_non_placeholder_name_on_a_template_uses_first_allowed_delivery", path: C, rust: "test_template_add_of_a_non_placeholder_name_creates_a_deliverable_env_row", architectural_note: "Python's edit_declared helper takes an injected allowed-delivery tuple; Rust has no public equivalent. Python's own CLI contract overrides the helper default for command templates and is the executable product oracle." },
    Mapping { python: "test_add_existing_name_warns_already_declared", path: D, rust: "test_add_existing_name_warns_already_declared", architectural_note: "" },
    Mapping { python: "test_rm_drops_the_row", path: D, rust: "test_rm_drops_the_row", architectural_note: "" },
    Mapping { python: "test_rm_unknown_name_warns_not_declared", path: D, rust: "test_rm_unknown_name_warns_not_declared", architectural_note: "" },
    Mapping { python: "test_apply_order_is_rm_then_add_then_tweak", path: D, rust: "test_apply_order_is_rm_then_add_then_tweak", architectural_note: "" },
    Mapping { python: "test_inputs_are_never_mutated", path: D, rust: "test_inputs_are_never_mutated", architectural_note: "" },
    Mapping { python: "test_delivery_tweak_within_allowed_set", path: D, rust: "test_delivery_tweak_within_allowed_set", architectural_note: "" },
    Mapping { python: "test_delivery_outside_allowed_set_warns_bad_delivery", path: D, rust: "test_delivery_outside_allowed_set_warns_bad_delivery", architectural_note: "" },
    Mapping { python: "test_placeholder_delivery_on_a_non_placeholder_name_warns", path: D, rust: "test_placeholder_delivery_on_a_non_placeholder_name_warns", architectural_note: "" },
    Mapping { python: "test_placeholder_delivery_on_a_matching_placeholder_name_is_allowed", path: D, rust: "test_placeholder_delivery_on_a_matching_placeholder_name_is_allowed", architectural_note: "" },
    Mapping { python: "test_type_tweak_valid", path: D, rust: "test_type_tweak_valid", architectural_note: "" },
    Mapping { python: "test_type_tweak_invalid_warns_bad_type", path: D, rust: "test_type_tweak_invalid_warns_bad_type", architectural_note: "" },
    Mapping { python: "test_choices_tweak_sets_the_tuple", path: D, rust: "test_choices_tweak_sets_the_tuple", architectural_note: "" },
    Mapping { python: "test_default_coerced_to_the_declared_type", path: D, rust: "test_default_coerced_to_the_declared_type", architectural_note: "" },
    Mapping { python: "test_default_type_set_in_same_call_applies_before_coercion", path: D, rust: "test_default_type_set_in_same_call_applies_before_coercion", architectural_note: "" },
    Mapping { python: "test_default_bad_value_warns_bad_default_and_keeps_old", path: D, rust: "test_default_bad_value_warns_bad_default_and_keeps_old", architectural_note: "" },
    Mapping { python: "test_flag_tweak_strips_and_sets_empty_for_positional", path: D, rust: "test_flag_tweak_strips_and_sets_empty_for_positional", architectural_note: "" },
    Mapping { python: "test_required_and_optional_tweaks", path: D, rust: "test_required_and_optional_tweaks", architectural_note: "" },
    Mapping { python: "test_help_text_and_prompt_tweaks", path: D, rust: "test_help_text_and_prompt_tweaks", architectural_note: "" },
    Mapping { python: "test_secret_and_env_source_together", path: D, rust: "test_secret_and_env_source_together", architectural_note: "" },
    Mapping { python: "test_env_source_on_a_non_secret_param_warns_and_leaves_it_unset", path: D, rust: "test_env_source_on_a_non_secret_param_warns_and_leaves_it_unset", architectural_note: "" },
    Mapping { python: "test_no_secret_clears_the_env_source", path: D, rust: "test_no_secret_clears_the_env_source", architectural_note: "" },
    Mapping { python: "test_tweak_on_unknown_name_warns_not_declared", path: D, rust: "test_tweak_on_unknown_name_warns_not_declared", architectural_note: "" },
    Mapping { python: "test_a_name_touched_by_two_ops_is_listed_once_and_both_apply", path: D, rust: "test_a_name_touched_by_two_ops_is_listed_once_and_both_apply", architectural_note: "" },
    Mapping { python: "test_choice_type_without_choices_reverts_and_warns", path: D, rust: "test_choice_type_without_choices_reverts_and_warns", architectural_note: "" },
    Mapping { python: "test_choice_type_with_choices_in_the_same_call_is_valid", path: D, rust: "test_choice_type_with_choices_in_the_same_call_is_valid", architectural_note: "" },
    Mapping { python: "test_type_tweak_to_bool_on_a_flag_sets_store_true", path: B, rust: "test_type_tweak_to_bool_on_a_flag_sets_store_true", architectural_note: "" },
    Mapping { python: "test_type_tweak_to_bool_on_a_positional_keeps_empty_action", path: B, rust: "test_type_tweak_to_bool_on_a_positional_keeps_empty_action", architectural_note: "" },
    Mapping { python: "test_type_tweak_to_bool_on_env_delivery_keeps_empty_action", path: B, rust: "test_type_tweak_to_bool_on_env_delivery_keeps_empty_action", architectural_note: "" },
    Mapping { python: "test_type_tweak_off_bool_sheds_stale_action", path: B, rust: "test_type_tweak_off_bool_sheds_stale_action", architectural_note: "" },
    Mapping { python: "test_non_type_tweak_on_a_bool_leaves_its_action_alone", path: B, rust: "test_non_type_tweak_on_a_bool_leaves_its_action_alone", architectural_note: "" },
    Mapping { python: "test_non_type_tweak_on_a_str_with_stale_action_clears_it", path: B, rust: "test_non_type_tweak_on_a_str_with_stale_action_clears_it", architectural_note: "" },
    Mapping { python: "test_coerce_default_success", path: H, rust: "test_coerce_default_success", architectural_note: "" },
    Mapping { python: "test_coerce_default_rejects_bad_values", path: H, rust: "test_coerce_default_rejects_bad_values", architectural_note: "" },
    Mapping { python: "test_coerce_default_rejects_infinity_specifically", path: H, rust: "test_coerce_default_rejects_infinity_specifically", architectural_note: "" },
    Mapping { python: "test_as_param_type_accepts_the_five", path: H, rust: "test_as_param_type_accepts_the_five", architectural_note: "Rust additionally accepts path; the frozen historical five are still asserted exactly." },
    Mapping { python: "test_as_param_type_rejects_others", path: H, rust: "test_as_param_type_rejects_others", architectural_note: "" },
    Mapping { python: "test_bool_flag_that_is_on_by_default_is_refused_not_stamped", path: B, rust: "test_bool_flag_that_is_on_by_default_is_refused_not_stamped", architectural_note: "" },
    Mapping { python: "test_bool_flag_that_is_off_by_default_still_gets_store_true", path: B, rust: "test_bool_flag_that_is_off_by_default_still_gets_store_true", architectural_note: "" },
];

#[test]
fn every_python_params_edit_test_has_an_executable_rust_oracle() {
    assert_eq!(MAPPINGS.len(), 41, "frozen Python module has 41 top-level tests");
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap();
    let mut missing = Vec::new();
    for mapping in MAPPINGS {
        let source = fs::read_to_string(repo.join(mapping.path)).unwrap();
        let needle = format!("fn {}(", mapping.rust);
        if !source.contains(&needle) {
            missing.push(format!(
                "{} -> {}::{}{}",
                mapping.python,
                mapping.path,
                mapping.rust,
                if mapping.architectural_note.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", mapping.architectural_note)
                }
            ));
        }
    }
    assert!(
        missing.is_empty(),
        "mapped Python params-edit tests disappeared:\n{}",
        missing.join("\n")
    );
}

#[test]
fn architectural_exceptions_are_explicit_and_narrow() {
    let exceptions = MAPPINGS
        .iter()
        .filter(|mapping| !mapping.architectural_note.is_empty())
        .collect::<Vec<_>>();
    assert_eq!(exceptions.len(), 2);
    assert_eq!(
        exceptions
            .iter()
            .map(|mapping| mapping.python)
            .collect::<Vec<_>>(),
        [
            "test_add_non_placeholder_name_on_a_template_uses_first_allowed_delivery",
            "test_as_param_type_accepts_the_five",
        ]
    );
}
