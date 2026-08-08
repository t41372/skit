use std::fs;

use skit_core::suggest_python_dependencies;
use tempfile::tempdir;

#[test]
fn absolute_imports_are_found_across_scopes_and_stdlib_is_excluded() {
    let source = r#"
import os
import requests

def f():
    import rich.table
    import numpy as np
    from http.client import HTTPConnection
"#;
    assert_eq!(
        suggest_python_dependencies(source, None),
        ["numpy", "requests", "rich"]
    );
}

#[test]
fn from_import_uses_top_level_module_and_relative_imports_are_ignored() {
    let source = r#"
from rich.table import Table
from requests.sessions import Session
from .helpers import x
from ..pkg import y
"#;
    assert_eq!(
        suggest_python_dependencies(source, None),
        ["requests", "rich"]
    );
}

#[test]
fn common_import_names_map_to_installable_distributions_and_dedupe() {
    let source = r#"
from PIL import Image
import cv2
import yaml
from Crypto.Cipher import AES
import Crypto.Hash
import win32api
import win32com.client
"#;
    assert_eq!(
        suggest_python_dependencies(source, None),
        [
            "Pillow",
            "PyYAML",
            "opencv-python",
            "pycryptodome",
            "pywin32"
        ]
    );
}

#[test]
fn syntax_errors_degrade_to_no_suggestions() {
    assert!(suggest_python_dependencies("def broken(:\nimport requests\n", None).is_empty());
}

#[test]
fn sibling_modules_packages_and_namespace_portions_are_local()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    fs::write(root.path().join("helpers.py"), "x = 1\n")?;
    fs::create_dir(root.path().join("pkg"))?;
    fs::write(root.path().join("pkg/__init__.py"), "x = 1\n")?;
    fs::create_dir(root.path().join("ns"))?;
    fs::write(root.path().join("ns/thing.py"), "x = 1\n")?;

    let source = "import helpers\nimport pkg\nimport ns\nimport requests\n";
    assert_eq!(
        suggest_python_dependencies(source, Some(root.path())),
        ["requests"]
    );
    Ok(())
}

#[test]
fn data_only_directory_does_not_hide_a_real_distribution()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    fs::create_dir(root.path().join("rich"))?;
    fs::write(root.path().join("rich/notes.txt"), "data\n")?;
    assert_eq!(
        suggest_python_dependencies("import rich\n", Some(root.path())),
        ["rich"]
    );
    Ok(())
}

#[test]
fn private_and_non_pep508_import_names_are_not_fabricated_as_packages() {
    assert!(suggest_python_dependencies("import _private\nimport café\n", None).is_empty());
}

#[test]
fn aliases_and_multi_import_statements_keep_each_real_module() {
    let source = "import requests as r, rich, numpy.linalg as la\n";
    assert_eq!(
        suggest_python_dependencies(source, None),
        ["numpy", "requests", "rich"]
    );
}
