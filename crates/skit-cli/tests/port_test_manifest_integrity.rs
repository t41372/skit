//! Integrity checks for parity manifests that map Python names to Rust test functions.
//!
//! A manifest entry is not allowed to pass merely because a target file contains *some* `#[test]`
//! and *some* function with the mapped name. Each mapped Rust function itself must exist and carry
//! `#[test]`. The mapping list remains single-sourced in its original manifest; this test parses that
//! manifest text instead of copying the ledger into a second list.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use syn::{Attribute, Item};

const DECLARED_MANIFEST: &str = include_str!("port_test_declared_params_manifest.rs");
const PARAMS_EDIT_MANIFEST: &str = include_str!("port_test_params_edit_manifest.rs");

#[derive(Debug, Eq, PartialEq)]
struct Target {
    path: String,
    rust: String,
}

fn quoted_field(line: &str, field: &str) -> Option<String> {
    let marker = format!("{field}: \"");
    let tail = line.split_once(&marker)?.1;
    Some(tail.split_once('"')?.0.to_owned())
}

fn path_aliases(manifest: &str) -> BTreeMap<String, String> {
    manifest
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let tail = line.strip_prefix("const ")?;
            let (name, value) = tail.split_once(": &str = \"")?;
            let value = value.split_once('"')?.0;
            Some((name.trim().to_owned(), value.to_owned()))
        })
        .collect()
}

fn path_field(line: &str, aliases: &BTreeMap<String, String>) -> String {
    if let Some(path) = quoted_field(line, "path") {
        return path;
    }
    let tail = line
        .split_once("path: ")
        .unwrap_or_else(|| panic!("manifest mapping lacks path: {line}"))
        .1;
    let token = tail.split_once(',').map_or(tail, |(token, _)| token).trim();
    aliases
        .get(token)
        .cloned()
        .unwrap_or_else(|| panic!("manifest mapping uses unknown path alias {token}: {line}"))
}

fn mapped_targets(manifest: &str) -> Vec<Target> {
    let aliases = path_aliases(manifest);
    manifest
        .lines()
        .filter(|line| {
            line.contains("Mapping {") && line.contains(" path: ") && line.contains(" rust: ")
        })
        .map(|line| Target {
            path: path_field(line, &aliases),
            rust: quoted_field(line, "rust")
                .unwrap_or_else(|| panic!("manifest mapping lacks quoted Rust test name: {line}")),
        })
        .collect()
}

fn has_test_attribute(attributes: &[Attribute]) -> bool {
    attributes
        .iter()
        .any(|attribute| attribute.path().is_ident("test"))
}

fn assert_real_tests(manifest_name: &str, manifest: &str, expected_count: usize) {
    let targets = mapped_targets(manifest);
    assert_eq!(
        targets.len(),
        expected_count,
        "{manifest_name} mapping count changed; update the executable Python test count intentionally"
    );

    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("skit-cli lives under <repo>/crates/skit-cli");
    let mut failures = Vec::new();

    for target in targets {
        let path: PathBuf = repo.join(&target.path);
        let source = fs::read_to_string(&path).unwrap_or_else(|error| {
            panic!("could not read mapped target {}: {error}", path.display())
        });
        let file = syn::parse_file(&source).unwrap_or_else(|error| {
            panic!(
                "mapped target {} is not valid Rust: {error}",
                path.display()
            )
        });
        let matched = file.items.iter().find_map(|item| match item {
            Item::Fn(function) if function.sig.ident == target.rust => {
                Some(has_test_attribute(&function.attrs))
            }
            _ => None,
        });
        match matched {
            Some(true) => {}
            Some(false) => failures.push(format!(
                "{}::{} exists but is not #[test]",
                target.path, target.rust
            )),
            None => failures.push(format!("{}::{} does not exist", target.path, target.rust)),
        }
    }

    assert!(
        failures.is_empty(),
        "parity manifest contains fake/non-executable mappings:\n{}",
        failures.join("\n")
    );
}

#[test]
fn declared_params_manifest_targets_are_themselves_executable_tests() {
    assert_real_tests("declared params", DECLARED_MANIFEST, 52);
}

#[test]
fn params_edit_manifest_targets_are_themselves_executable_tests() {
    assert_real_tests("params edit", PARAMS_EDIT_MANIFEST, 39);
}
