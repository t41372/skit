const EXPECTED: &[&str] = &[
    "test_argparse_garbage_specs_are_skipped",
    "test_argparse_empty_long_falls_back_to_short",
    "test_corpus_analyze_is_total_and_reads_back",
];
#[test] fn fish_manifest_group_c4a_shape(){ assert_eq!(EXPECTED.len(),3); }
