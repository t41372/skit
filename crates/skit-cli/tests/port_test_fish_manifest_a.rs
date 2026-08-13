const EXPECTED: &[&str] = &[
    "test_oneline_idiom_int",
    "test_newline_continued_or",
    "test_float_and_string_defaults",
    "test_guarded_set_may_carry_scope_flags",
    "test_secret_name_flagged",
    "test_suppressed_by_plain_clobber_anywhere",
    "test_clobber_before_the_idiom_also_suppresses",
    "test_unrelated_clobber_does_not_suppress",
    "test_underscore_name_skipped",
    "test_first_occurrence_wins_on_duplicate_idiom",
    "test_query_without_following_set_is_not_a_candidate",
];
#[test] fn fish_manifest_group_a_shape(){ assert_eq!(EXPECTED.len(),11); }
