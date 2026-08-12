//! Exact completeness guard for Python `tests/test_add_validation_contracts.py` at
//! `main@206f9ef`.

use std::collections::BTreeSet;

use syn::{Attribute, Item};

const LANGUAGE: &str =
    include_str!("../../skit-language/tests/port_test_add_validation_contracts.rs");
const UI: &str = include_str!("../../skit-ui/tests/port_test_add_validation_contracts.rs");
const CLI: &str = include_str!("port_test_add_validation_contracts.rs");
const EDITOR: &str = include_str!("port_test_add_validation_editor.rs");

const PYTHON_TESTS: &[&str] = &[
    "test_requires_python_error_is_none_for_valid_constraints",
    "test_requires_python_error_localizes_a_message_for_an_invalid_constraint",
    "test_requires_python_error_rejects_a_bare_version_without_operator",
    "test_requirement_error_is_none_for_valid_requirements",
    "test_requirement_error_localizes_a_message_for_an_invalid_requirement",
    "test_validate_python_flags_passes_valid_and_normalizes_the_constraint",
    "test_validate_python_flags_normalizes_dash_and_none_to_empty",
    "test_validate_python_flags_returns_none_when_no_python_given",
    "test_validate_python_flags_treats_an_empty_python_as_empty",
    "test_validate_python_flags_skips_empty_dep_strings",
    "test_validate_python_flags_exits_2_on_a_bad_dep",
    "test_validate_python_flags_exits_2_on_a_bad_python",
    "test_interactive_deps_reask_then_python_reask_then_accept",
    "test_interactive_valid_deps_accepted_first_try",
    "test_exe_flag_on_a_kept_draft_is_refused_naming_only_exe",
    "test_kind_exe_on_a_kept_draft_is_refused_naming_only_kind_exe",
    "test_inferred_exe_on_a_kept_draft_is_refused_and_keeps_it",
    "test_ref_flag_on_a_kept_draft_is_refused_naming_only_ref",
    "test_a_normal_draft_resume_still_adds_as_a_copy",
    "test_stdin_garbage_python_exits_2_and_leaves_the_drafts_dir_empty",
    "test_stdin_garbage_dep_exits_2_and_leaves_the_drafts_dir_empty",
    "test_stdin_dash_python_is_automatic",
    "test_stdin_valid_python_lands_in_the_stored_block",
    "test_editor_lane_refuses_bad_python_before_opening_the_editor",
    "test_editor_lane_refuses_bad_dep_before_opening_the_editor",
    "test_kind_for_draft_single_prompt_extension_outranks_the_shebang",
    "test_kind_for_draft_extensionless_falls_through_to_the_shebang",
    "test_kind_for_draft_script_suffix_stays_shebang_first",
    "test_prompt_single_extension_draft_resumes_as_prompt_end_to_end",
    "test_nondraft_awk_shebang_refusal_offers_the_exe_escape",
    "test_kept_draft_awk_shebang_refusal_offers_only_kind",
];

fn is_test(attributes: &[Attribute]) -> bool {
    attributes
        .iter()
        .any(|attribute| attribute.path().is_ident("test"))
}

fn tests(source: &str) -> Vec<String> {
    syn::parse_file(source)
        .expect("add-validation parity target must parse as Rust")
        .items
        .into_iter()
        .filter_map(|item| match item {
            Item::Fn(function) if is_test(&function.attrs) => Some(function.sig.ident.to_string()),
            _ => None,
        })
        .collect()
}

#[test]
fn all_thirty_one_python_add_validation_contracts_are_executable_once() {
    let actual = tests(LANGUAGE)
        .into_iter()
        .chain(tests(UI))
        .chain(tests(CLI))
        .chain(tests(EDITOR))
        .collect::<Vec<_>>();
    let actual_set = actual.iter().cloned().collect::<BTreeSet<_>>();
    let expected = PYTHON_TESTS
        .iter()
        .map(|name| (*name).to_owned())
        .collect::<BTreeSet<_>>();

    assert_eq!(PYTHON_TESTS.len(), 31);
    assert_eq!(
        actual.len(),
        31,
        "unexpected extra or missing add-validation tests"
    );
    assert_eq!(
        actual_set.len(),
        actual.len(),
        "duplicate names hide a missing contract"
    );
    assert_eq!(actual_set, expected);
}
