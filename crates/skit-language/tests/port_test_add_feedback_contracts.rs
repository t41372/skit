//! Language-level ports from Python `tests/test_add_feedback_contracts.py` at `main@206f9ef`.

use skit_language::{external_dependencies_at, python_version_pin};

#[test]
fn test_micro_version_pin_unit() {
    assert_eq!(
        python_version_pin("python3.12.1"),
        Some(">=3.12.1,<3.13".to_owned())
    );
    assert_eq!(
        python_version_pin("python3.12.1.7"),
        Some(">=3.12.1.7,<3.13".to_owned())
    );
}

#[test]
fn test_resolve_python_metadata_without_script_dir_does_not_filter() {
    let dependencies = external_dependencies_at(
        "python",
        "import helpers\nimport requests\n",
        None,
    );

    assert_eq!(dependencies, ["helpers", "requests"]);
}
