const EXPECTED: &[&str] = &[
    "test_hint_ignores_commented_argv",
    "test_reconcile_ok_then_drift",
    "test_argparse_short_long_and_valueless_bool",
    "test_argparse_value_suffixes",
    "test_argparse_long_only_and_short_only",
];
#[test] fn fish_manifest_group_c1_shape(){ assert_eq!(EXPECTED.len(),5); }
