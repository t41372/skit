const EXPECTED: &[&str] = &[
    "test_corpus_expected_detections",
    "test_manage_then_plan_and_assemble_env_delivery",
    "test_env_overlay_overrides_default_in_real_fish",
];
#[test] fn fish_manifest_group_c4b_shape(){ assert_eq!(EXPECTED.len(),3); }
