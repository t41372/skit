//! Exact executable completeness guard for Python `tests/test_analyzer.py` at `main@206f9ef`.
//!
//! The 37 names below are the frozen Python `def test_` sequence. The behavioral target must expose
//! exactly those same tests; implementation-derived extras or missing oracle cases fail accounting.

use std::{fs, path::Path};

use syn::{Attribute, Item};

const EXPECTED: &[&str] = &[
    "test_module_level_consts",
    "test_ann_assign_and_bool_not_int",
    "test_main_guard_scanned_c4",
    "test_main_guard_reversed_form",
    "test_input_calls_ordered_b1",
    "test_secret_heuristics",
    "test_framework_detection",
    "test_syntax_error_returns_empty",
    "test_duplicate_top_level_const_is_deduped_to_one_candidate",
    "test_duplicate_top_level_const_keeps_last_occurrence_value",
    "test_duplicate_top_level_const_keeps_first_occurrence_position",
    "test_duplicate_top_level_const_mixed_ann_assign",
    "test_duplicate_const_injection_no_longer_corrupts_source",
    "test_shadowed_input_via_def_yields_no_input_candidates",
    "test_shadowed_input_via_assignment_yields_no_input_candidates",
    "test_shadowed_input_via_from_import_yields_no_input_candidates",
    "test_shadowed_input_via_plain_import_yields_no_input_candidates",
    "test_function_parameter_named_input_does_not_shadow_the_module_level_call",
    "test_call_inside_the_shadowing_function_is_not_a_candidate",
    "test_local_assignment_shadows_only_its_own_function",
    "test_module_level_binding_still_shadows_calls_nested_in_functions",
    "test_comprehension_and_lambda_bindings_stay_local",
    "test_shadowing_input_does_not_suppress_const_detection",
    "test_unshadowed_input_is_still_detected",
    "test_match_inputs_prompt_survives_position_shift",
    "test_match_inputs_falls_back_to_position_when_no_prompt_recorded",
    "test_match_inputs_flags_ambiguous_when_prompt_renamed_but_position_still_exists",
    "test_match_inputs_flags_ambiguous_when_two_call_sites_share_a_prompt",
    "test_match_inputs_missing_when_neither_prompt_nor_position_resolves",
    "test_match_inputs_duplicate_stored_prompts_never_double_bind_on_delete",
    "test_match_inputs_duplicate_stored_prompts_edit_one_flags_rebind_for_loser",
    "test_match_inputs_triple_duplicate_stored_prompts_only_one_winner",
    "test_match_capture_named_input_shadows_only_its_own_scope",
    "test_except_handler_named_input_shadows_only_its_own_scope",
    "test_dotted_import_binds_only_its_top_level_name",
    "test_star_import_is_treated_as_possibly_binding_input",
    "test_one_shadowing_scope_does_not_stop_the_scan_of_the_others",
];

fn is_test(attributes: &[Attribute]) -> bool {
    attributes
        .iter()
        .any(|attribute| attribute.path().is_ident("test"))
}

#[test]
fn analyzer_has_exactly_the_37_frozen_python_oracles() {
    assert_eq!(EXPECTED.len(), 37);
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("skit-cli lives under <repo>/crates/skit-cli");
    let path = repo.join("crates/skit-language/tests/port_test_analyzer.rs");
    let source = fs::read_to_string(&path).unwrap();
    let file = syn::parse_file(&source).expect("analyzer parity target must parse as Rust");
    let actual = file
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Fn(function) if is_test(&function.attrs) => Some(function.sig.ident.to_string()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        actual,
        EXPECTED
            .iter()
            .map(|name| (*name).to_owned())
            .collect::<Vec<_>>(),
        "analyzer parity target must be exactly the frozen Python test sequence"
    );
}
