const PYTHON:&[&str]=&[
"test_extract_comment_description_skips_shebang_and_blank_lines",
"test_extract_comment_description_skips_metadata_fence",
"test_extract_comment_description_empty_comment_line_continues",
"test_extract_comment_description_code_first_is_empty",
];
#[test] fn store_manifest_04b_shape(){assert_eq!(PYTHON.len(),4);}
