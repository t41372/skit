//! Executable Rust equivalents for the distribution-facing contracts in Python
//! `tests/test_packaging.py` at `main@206f9ef`.
//!
//! Python's importlib/module-hook contracts have no Rust runtime equivalent and stay
//! architecture-closed in the companion manifest. These tests cover the three packaging facts a
//! Rust binary distribution can observe directly: no public PEP 621 extras, no catalog source files
//! in the binary wheel inputs, and one version across PyPI metadata, Cargo metadata, and the binary.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("skit-cli lives under <repo>/crates/skit-cli")
        .to_path_buf()
}

fn read_toml(path: &Path) -> toml::Value {
    fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("could not read {}: {error}", path.display()))
        .parse()
        .unwrap_or_else(|error| panic!("could not parse {} as TOML: {error}", path.display()))
}

fn collect_catalog_sources(directory: &Path, output: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("could not scan {}: {error}", directory.display()))
    {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect_catalog_sources(&path, output);
        } else if matches!(
            path.extension().and_then(|extension| extension.to_str()),
            Some("po") | Some("pot")
        ) {
            output.push(path);
        }
    }
}

#[test]
fn test_no_dead_optional_dependencies() {
    let root = repo_root();
    let pyproject = read_toml(&root.join("pyproject.toml"));
    let project = pyproject
        .get("project")
        .and_then(toml::Value::as_table)
        .expect("pyproject.toml must have [project]");

    assert!(
        !project.contains_key("optional-dependencies"),
        "the public skit-cli distribution must not expose dead optional extras: {project:#?}"
    );
}

#[test]
fn test_wheel_excludes_catalog_sources() {
    let root = repo_root();
    let pyproject = read_toml(&root.join("pyproject.toml"));
    let maturin = pyproject
        .get("tool")
        .and_then(|tool| tool.get("maturin"))
        .and_then(toml::Value::as_table)
        .expect("pyproject.toml must configure [tool.maturin]");

    assert_eq!(
        maturin.get("bindings").and_then(toml::Value::as_str),
        Some("bin"),
        "the wheel must remain a binary wheel rather than packaging a Python runtime tree"
    );
    let includes = maturin
        .get("include")
        .and_then(toml::Value::as_array)
        .expect("maturin include rows must be explicit");
    assert!(!includes.is_empty(), "the sdist include contract disappeared");
    for row in includes {
        let table = row.as_table().expect("each maturin include row is a table");
        assert_eq!(
            table.get("format").and_then(toml::Value::as_str),
            Some("sdist"),
            "an explicit package-data include became a wheel include: {table:#?}"
        );
    }

    let mut catalog_sources = Vec::new();
    collect_catalog_sources(&root.join("crates/skit-i18n"), &mut catalog_sources);
    assert!(
        catalog_sources.is_empty(),
        "the Rust distribution must compile translations instead of shipping .po/.pot sources: {catalog_sources:#?}"
    );
}

#[test]
fn test_version_is_single_sourced_from_the_distribution() {
    let root = repo_root();
    let pyproject = read_toml(&root.join("pyproject.toml"));
    let pyproject_version = pyproject
        .get("project")
        .and_then(|project| project.get("version"))
        .and_then(toml::Value::as_str)
        .expect("pyproject.toml must declare project.version");
    let workspace = read_toml(&root.join("Cargo.toml"));
    let workspace_version = workspace
        .get("workspace")
        .and_then(|workspace| workspace.get("package"))
        .and_then(|package| package.get("version"))
        .and_then(toml::Value::as_str)
        .expect("Cargo.toml must declare workspace.package.version");

    assert_eq!(workspace_version, pyproject_version, "Cargo and PyPI versions drifted");
    assert_eq!(
        env!("CARGO_PKG_VERSION"),
        pyproject_version,
        "the compiled skit-cli package version drifted from distribution metadata"
    );

    let output = Command::new(env!("CARGO_BIN_EXE_skit"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "skit --version failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        output.stdout,
        format!("skit {pyproject_version}\n").as_bytes(),
        "the installed binary reports a different version"
    );
    assert!(
        output.stderr.is_empty(),
        "skit --version wrote stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
