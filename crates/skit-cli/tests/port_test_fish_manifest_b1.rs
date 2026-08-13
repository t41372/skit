const EXPECTED: &[&str] = &[
    "test_query_with_no_name_is_ignored",
    "test_conditional_set_without_value_is_not_a_candidate",
    "test_mismatched_names_are_not_an_idiom",
    "test_unconditional_set_after_query_is_not_an_idiom",
    "test_idiom_inside_function_is_not_toplevel",
];
#[test] fn fish_manifest_group_b1_shape(){ assert_eq!(EXPECTED.len(),5); }
