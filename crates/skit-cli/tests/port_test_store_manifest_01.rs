const PYTHON:&[&str]=&[
"test_add_copy_preserves_original_verbatim",
"test_add_reference_points_to_origin",
"test_name_conflict_rejected",
"test_slug_dedup",
"test_resolve_and_remove",
"test_remove_copy_does_not_touch_original",
"test_add_command_entry",
"test_command_requires_nonempty_template",
];
#[test] fn store_manifest_01_shape(){assert_eq!(PYTHON.len(),8);}
