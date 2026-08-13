const EXPECTED: &[&str] = &[
    "test_idiom_inside_every_block_kind_is_ignored",
    "test_toplevel_after_a_closed_block_is_detected",
    "test_nested_clobber_does_not_suppress_toplevel_idiom",
];
#[test] fn fish_manifest_group_b2a_shape(){ assert_eq!(EXPECTED.len(),3); }
