//! Language-owned dependency suggestion contract from Python
//! `tests/test_dependency_write_validation.py` at `main@206f9ef`.

use skit_language::external_dependencies_at;

#[test]
fn test_suggest_dependencies_drops_a_name_pep508_refuses() {
    let suggested = external_dependencies_at(
        "python",
        "import café\nimport requests\nprint(1)\n",
        None,
    );

    assert_eq!(suggested, ["requests"]);
}
