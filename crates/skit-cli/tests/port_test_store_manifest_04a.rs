const PYTHON:&[&str]=&[
"test_infer_kind_windows_uses_pathext_not_execute_bit",
"test_infer_kind_windows_reads_pathext_env",
"test_infer_kind_windows_falls_back_to_default_pathext",
"test_extract_comment_description_first_comment_line_wins",
];
#[test] fn store_manifest_04a_shape(){assert_eq!(PYTHON.len(),4);}
