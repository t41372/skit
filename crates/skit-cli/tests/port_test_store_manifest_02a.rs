const PYTHON:&[&str]=&[
"test_doctor_rebuild_from_meta",
"test_doctor_reports_missing_reference",
"test_syntax_error_script_still_addable",
"test_add_python_missing_file_raises",
];
#[test] fn store_manifest_02a_shape(){assert_eq!(PYTHON.len(),4);}
