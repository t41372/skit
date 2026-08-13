const EXPECTED: &[&str] = &[
    "test_argparse_dummy_short_yields_long_only",
    "test_argparse_numeric_hash_degrades",
    "test_argparse_validator_is_stripped",
    "test_argparse_secret_name",
    "test_argparse_skips_own_options",
];
#[test] fn fish_manifest_group_c2_shape(){ assert_eq!(EXPECTED.len(),5); }
