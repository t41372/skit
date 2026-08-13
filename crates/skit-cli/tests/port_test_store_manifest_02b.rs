const PYTHON:&[&str]=&[
"test_add_exe_roundtrip",
"test_add_exe_missing_file_raises",
"test_list_entries_skips_corrupt_meta",
"test_doctor_rebuild_corrupt_meta",
];
#[test] fn store_manifest_02b_shape(){assert_eq!(PYTHON.len(),4);}
