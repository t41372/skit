//! Exact-name/order completeness guard for Python v0.4 `tests/test_default_name_resolution.py`.
//!
//! Frozen oracle: `origin/main@206f9ef946fc45835cb2479593794431f2620c32`.
//! This does not count as behavioral coverage. The 42 behavioral tests live in `skit-form`; this
//! guard only prevents omissions, renames, reorder drift, or extra fake parity tests.

use syn::{Attribute, Item};

const SOURCE: &str = include_str!("../../skit-form/tests/port_test_default_name_resolution.rs");

const PYTHON_TESTS: &[&str] = &[
    "test_argparse_string_constant_default_resolves",
    "test_argparse_int_and_bool_constant_defaults_resolve",
    "test_argparse_augmented_assigned_name_does_not_resolve",
    "test_argparse_loop_reassigned_name_does_not_resolve",
    "test_argparse_non_store_value_binding_does_not_resolve",
    "test_argparse_star_import_makes_every_constant_default_opaque",
    "test_argparse_unknown_name_default_degrades",
    "test_argparse_call_default_still_degrades",
    "test_argparse_constant_used_twice_resolves_in_both_fields",
    "test_argparse_conditional_rebinding_does_not_resolve",
    "test_argparse_try_except_rebinding_does_not_resolve",
    "test_argparse_with_block_rebinding_does_not_resolve",
    "test_argparse_function_local_assignment_blocks_resolution",
    "test_argparse_function_parameter_shadow_blocks_resolution",
    "test_argparse_secret_constant_never_resolves",
    "test_argparse_password_and_token_constants_never_resolve",
    "test_argparse_constant_bound_twice_does_not_resolve",
    "test_click_constant_default_resolves",
    "test_click_constant_also_read_inside_the_body_still_resolves",
    "test_click_secret_constant_default_degrades",
    "test_click_unknown_name_default_degrades",
    "test_typer_legacy_option_constant_default_resolves",
    "test_typer_annotated_signature_constant_default_resolves",
    "test_typer_bare_signature_constant_default_resolves",
    "test_typer_unknown_signature_default_degrades",
    "test_js_constant_default_resolves",
    "test_js_let_binding_default_does_not_resolve",
    "test_js_reassigned_const_default_does_not_resolve",
    "test_js_unknown_identifier_default_degrades",
    "test_js_function_local_const_shadow_does_not_resolve",
    "test_js_function_parameter_shadow_does_not_resolve",
    "test_js_constant_read_as_a_parameter_default_still_resolves",
    "test_ts_typed_parameter_default_reads_the_constant_without_declaring_it",
    "test_ts_destructured_parameter_default_is_also_only_a_read",
    "test_ts_typed_parameter_with_a_default_still_shadows_by_its_bound_name",
    "test_js_parameter_with_a_default_still_shadows_by_its_bound_name",
    "test_ts_typed_function_parameter_shadow_does_not_resolve",
    "test_js_secret_constant_never_resolves",
    "test_js_non_parameter_value_binding_does_not_resolve",
    "test_js_import_binding_does_not_resolve",
    "test_js_nonbinding_shapes_leave_constant_resolution_intact",
    "test_ts_constant_default_resolves",
];

fn has_test_attribute(attributes: &[Attribute]) -> bool {
    attributes.iter().any(|attribute| attribute.path().is_ident("test"))
}

fn rust_test_names() -> Vec<String> {
    let file = syn::parse_file(SOURCE).expect("default-name parity source must parse as Rust");
    file.items
        .into_iter()
        .filter_map(|item| match item {
            Item::Fn(function) if has_test_attribute(&function.attrs) => Some(function.sig.ident.to_string()),
            _ => None,
        })
        .collect()
}

#[test]
fn test_default_name_resolution_port_has_all_42_python_tests_in_exact_order() {
    let actual = rust_test_names();
    let expected = PYTHON_TESTS
        .iter()
        .map(|name| (*name).to_owned())
        .collect::<Vec<_>>();
    assert_eq!(actual, expected);
    assert_eq!(PYTHON_TESTS.len(), 42);
}
