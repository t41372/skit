//! Public-API ports of the Python v0.4 add-review dependency suggestion contract.
//!
//! The review surface suggests third-party imports only when the script does not already declare
//! authoritative PEP 723 dependencies. Syntax errors degrade to no suggestion rather than running
//! partial source analysis.

use skit_language::external_dependencies;

#[test]
fn test_python_imports_are_reported_as_sorted_dependency_suggestions() {
    let source = "import requests\nimport rich\nprint(1)\n";
    assert_eq!(external_dependencies("python", source), ["requests", "rich"]);
}

#[test]
fn test_python_import_aliases_and_from_imports_use_the_top_level_package() {
    let source = concat!(
        "import requests as rq\n",
        "from rich.console import Console\n",
        "from requests.sessions import Session\n",
    );
    assert_eq!(external_dependencies("python", source), ["requests", "rich"]);
}

#[test]
fn test_python_pep723_dependencies_are_authoritative_over_import_suggestions() {
    let source = concat!(
        "# /// script\n",
        "# dependencies = [\"requests>=2\"]\n",
        "# ///\n",
        "import requests\n",
        "import rich\n",
    );
    assert_eq!(external_dependencies("python", source), ["requests>=2"]);
}

#[test]
fn test_python_empty_declared_pep723_block_falls_back_to_detected_imports() {
    let source = concat!(
        "# /// script\n",
        "# dependencies = []\n",
        "# ///\n",
        "import requests\n",
    );
    assert_eq!(external_dependencies("python", source), ["requests"]);
}

#[test]
fn test_python_standard_library_imports_are_not_dependency_suggestions() {
    let source = "import os\nimport sys\nfrom pathlib import Path\nimport requests\n";
    assert_eq!(external_dependencies("python", source), ["requests"]);
}

#[test]
fn test_python_relative_import_is_not_an_external_dependency() {
    let source = "from .local import thing\nfrom ..pkg import other\nimport requests\n";
    assert_eq!(external_dependencies("python", source), ["requests"]);
}

#[test]
fn test_python_dependency_scan_degrades_to_empty_on_syntax_error() {
    assert!(external_dependencies("python", "from broken import\n").is_empty());
}
