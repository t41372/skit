//! Completeness guard for Python v0.4 `tests/test_show.py`.
//! Behavioral coverage lives in `port_test_show.rs`.

use syn::{Attribute, Item};

const PYTHON_TESTS: &[&str] = &[
    "test_show_json_argparse_full_schema",
    "test_show_json_stable_shape",
    "test_show_json_repeat_true_for_a_click_multiple_option",
    "test_show_json_inject_secret_and_state",
    "test_show_json_command_kind",
    "test_show_json_deps_and_missing_reference",
    "test_show_json_degraded_parser",
    "test_show_json_drift",
    "test_show_human_argparse_table",
    "test_show_human_masks_secret_default_and_names_env_source",
    "test_show_human_secret_without_env_source",
    "test_show_human_command_kind",
    "test_show_human_no_fields_exe",
    "test_show_human_description_deps_presets_and_drift",
    "test_show_human_degraded_parser_notice",
    "test_show_human_missing_marker",
    "test_show_not_found_exits_1",
];

fn is_test(attributes: &[Attribute]) -> bool {
    attributes
        .iter()
        .any(|attribute| attribute.path().is_ident("test"))
}

#[test]
fn every_python_show_test_has_the_same_named_executable_rust_oracle() {
    let parsed = syn::parse_file(include_str!("port_test_show.rs")).unwrap();
    let actual = parsed
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Fn(function) if is_test(&function.attrs) => Some(function.sig.ident.to_string()),
            _ => None,
        })
        .collect::<Vec<_>>();
    let expected = PYTHON_TESTS
        .iter()
        .map(|name| (*name).to_owned())
        .collect::<Vec<_>>();
    assert_eq!(PYTHON_TESTS.len(), 17);
    assert_eq!(actual, expected);
}
