//! Exact completeness guard for Python `tests/test_dependency_write_validation.py` at
//! `main@206f9ef`.

use std::collections::BTreeSet;

use syn::{Attribute, Item};

const LANGUAGE: &str = include_str!("../../skit-language/tests/port_test_dependency_write_validation.rs");
const CLI: &str = include_str!("port_test_dependency_write_validation.rs");

const PYTHON_TESTS: &[&str] = &[
    "test_deps_garbage_dep_is_refused_and_nothing_changes",
    "test_deps_garbage_python_is_refused_and_nothing_changes",
    "test_deps_dash_python_clears_meta_and_unpins_the_block",
    "test_deps_only_edit_still_preserves_the_blocks_own_pin",
    "test_deps_none_python_clears_meta_when_nothing_to_preserve",
    "test_deps_valid_dep_and_python_still_write",
    "test_deps_refused_write_leaves_needs_untouched",
    "test_deps_npm_entry_takes_an_npm_shaped_dep_that_fails_pep508",
    "test_update_dependencies_uv_invalid_dep_raises_usage_error",
    "test_update_dependencies_uv_invalid_python_raises_usage_error",
    "test_update_dependencies_drops_a_whitespace_only_dep_at_the_chokepoint",
    "test_update_dependencies_all_whitespace_list_clears_deps",
    "test_update_dependencies_npm_flavor_skips_uv_validation",
    "test_update_dependencies_normalizes_dash_python_before_validating",
    "test_suggest_dependencies_drops_a_name_pep508_refuses",
    "test_no_input_add_of_an_illegally_named_import_writes_no_block",
    "test_inferred_exe_draft_gets_the_kind_variant",
    "test_exe_flag_on_the_same_draft_gets_the_drop_variant_naming_only_exe",
    "test_shebang_less_unclassifiable_draft_gets_the_classify_variant",
    "test_same_unclassifiable_file_outside_drafts_gets_the_full_escape",
    "test_ref_on_an_md_draft_is_refused_before_the_prompt_ask",
];

fn is_test(attributes: &[Attribute]) -> bool {
    attributes
        .iter()
        .any(|attribute| attribute.path().is_ident("test"))
}

fn tests(source: &str) -> Vec<String> {
    syn::parse_file(source)
        .expect("dependency-write parity target must parse as Rust")
        .items
        .into_iter()
        .filter_map(|item| match item {
            Item::Fn(function) if is_test(&function.attrs) => Some(function.sig.ident.to_string()),
            _ => None,
        })
        .collect()
}

#[test]
fn all_twenty_one_python_dependency_write_contracts_are_executable_once() {
    let actual = tests(LANGUAGE)
        .into_iter()
        .chain(tests(CLI))
        .collect::<Vec<_>>();
    let actual_set = actual.iter().cloned().collect::<BTreeSet<_>>();
    let expected = PYTHON_TESTS
        .iter()
        .map(|name| (*name).to_owned())
        .collect::<BTreeSet<_>>();

    assert_eq!(PYTHON_TESTS.len(), 21);
    assert_eq!(actual.len(), 21, "unexpected extra or missing dependency-write tests");
    assert_eq!(actual_set.len(), actual.len(), "duplicate names hide a missing contract");
    assert_eq!(actual_set, expected);
}
