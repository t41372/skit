const EXPECTED: &[&str] = &[
    "test_argparse_attached_own_option_does_not_consume",
    "test_argparse_after_conditional_prefix_is_found",
    "test_argparse_empty_specs_is_zero_field_surface",
    "test_no_argparse_returns_none",
    "test_argparse_variable_specs_degrade_to_dynamic",
    "test_argparse_command_substitution_specs_degrade_to_dynamic",
];
#[test] fn fish_manifest_group_c3_shape(){ assert_eq!(EXPECTED.len(),6); }
