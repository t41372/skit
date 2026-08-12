//! Public language-layer ports of the PEP 723/dependency-suggestion slice in Python
//! `tests/test_phase1.py` at `main@206f9ef`.

use std::fs;

use skit_language::{
    external_dependencies_at, has_uv_metadata_block, read_uv_metadata, write_uv_metadata,
};
use tempfile::TempDir;

const BLOCK: &str = concat!(
    "# /// script\n",
    "# requires-python = \">=3.11\"\n",
    "# dependencies = [\n",
    "#     \"requests\",\n",
    "# ]\n",
    "# ///\n",
    "import requests\n",
    "print(requests.__version__)\n",
);

#[test]
fn test_parse_block() {
    let metadata = read_uv_metadata(BLOCK).expect("PEP 723 block must parse");
    assert_eq!(metadata.dependencies, ["requests"]);
    assert_eq!(metadata.requires_python, ">=3.11");
}

#[test]
fn test_parse_no_block() {
    let text = "print('hi')\n";
    assert_eq!(read_uv_metadata(text), None);
    assert!(!has_uv_metadata_block(text));
}

#[test]
fn test_suggest_dependencies() {
    let text = "import requests\nimport os\nfrom rich.table import Table\nimport mymod.sub\n";
    let got = external_dependencies_at("python", text, None);
    assert!(got.contains(&"requests".to_owned()), "{got:?}");
    assert!(got.contains(&"rich".to_owned()), "{got:?}");
    assert!(
        !got.contains(&"os".to_owned()),
        "stdlib leaked into dependencies: {got:?}"
    );
}

#[test]
fn test_suggest_syntax_error_returns_empty() {
    assert!(external_dependencies_at("python", "def broken(:\n", None).is_empty());
}

#[test]
fn test_suggest_dependencies_maps_import_name_to_pypi_package() {
    assert_eq!(
        external_dependencies_at("python", "from PIL import Image\n", None),
        ["Pillow"]
    );
    assert_eq!(
        external_dependencies_at("python", "import cv2\n", None),
        ["opencv-python"]
    );
    assert_eq!(
        external_dependencies_at("python", "import yaml\n", None),
        ["PyYAML"]
    );
}

#[test]
fn test_suggest_dependencies_dedupes_after_mapping() {
    let source = "from Crypto.Cipher import AES\nimport Crypto.Hash\n";
    assert_eq!(
        external_dependencies_at("python", source, None),
        ["pycryptodome"]
    );
}

#[test]
fn test_suggest_dependencies_unmapped_name_unchanged() {
    assert_eq!(
        external_dependencies_at("python", "import requests\n", None),
        ["requests"]
    );
}

#[test]
fn test_suggest_dependencies_excludes_sibling_py_module() {
    let root = TempDir::new().unwrap();
    fs::write(root.path().join("helpers.py"), "X = 1\n").unwrap();
    let source = "import helpers\nimport requests\n";
    assert_eq!(
        external_dependencies_at("python", source, Some(root.path())),
        ["requests"]
    );
}

#[test]
fn test_suggest_dependencies_excludes_sibling_package_dir() {
    let root = TempDir::new().unwrap();
    let helpers = root.path().join("helpers");
    fs::create_dir(&helpers).unwrap();
    fs::write(helpers.join("__init__.py"), "x = 1\n").unwrap();
    let source = "import helpers\nimport requests\n";
    assert_eq!(
        external_dependencies_at("python", source, Some(root.path())),
        ["requests"]
    );
}

#[test]
fn test_suggest_dependencies_keeps_name_without_a_sibling() {
    let root = TempDir::new().unwrap();
    assert_eq!(
        external_dependencies_at("python", "import helpers\n", Some(root.path())),
        ["helpers"]
    );
}

#[test]
fn test_suggest_dependencies_default_script_dir_none_does_not_filter() {
    assert_eq!(
        external_dependencies_at("python", "import helpers\n", None),
        ["helpers"]
    );
}

#[test]
fn test_suggest_dependencies_from_import_sibling_excluded() {
    let root = TempDir::new().unwrap();
    fs::write(root.path().join("helpers.py"), "x = 1\n").unwrap();
    let source = "from helpers import x\nimport requests\n";
    assert_eq!(
        external_dependencies_at("python", source, Some(root.path())),
        ["requests"]
    );
}

#[test]
fn test_suggest_dependencies_submodule_of_sibling_dir_excluded() {
    let root = TempDir::new().unwrap();
    let helpers = root.path().join("helpers");
    fs::create_dir(&helpers).unwrap();
    fs::write(helpers.join("sub.py"), "x = 1\n").unwrap();
    let source = "import helpers.sub\nimport requests\n";
    assert_eq!(
        external_dependencies_at("python", source, Some(root.path())),
        ["requests"]
    );
}

#[test]
fn test_inject_preserves_body() {
    let source = "import requests\nprint('x')\n";
    let output = write_uv_metadata(source, &["requests".to_owned()], "").unwrap();
    assert!(
        output.ends_with("import requests\nprint('x')\n"),
        "PEP 723 insertion changed source body: {output}"
    );
    let metadata = read_uv_metadata(&output).unwrap();
    assert_eq!(metadata.dependencies, ["requests"]);
}
