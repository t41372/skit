//! Exact completeness gate for Python `tests/test_powershell.py` at `main@206f9ef`.
//!
//! Rust replaced Python's external `pwsh` JSON extractor with a parser-owned PowerShell document.
//! JSON-envelope, subprocess-timeout, and executable-discovery helper contracts therefore have no
//! Rust seam to execute. They are explicit architecture closures, never fake same-name tests.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use syn::{Attribute, Item};

const TEST_DIRS: &[&str] = &[
    "crates/skit-language/tests",
    "crates/skit-form/tests",
    "crates/skit-application/tests",
];
const PYTHON_TESTS: &[&str] = &[
    "test_string_param_with_default_and_help",
    "test_help_is_stripped_of_surrounding_whitespace",
    "test_int_and_long_map_to_int",
    "test_double_and_single_map_to_float",
    "test_switch_is_a_store_true_flag",
    "test_validate_set_becomes_choice",
    "test_unknown_static_type_degrades",
    "test_mandatory_is_required",
    "test_non_constant_default_degrades_field",
    "test_non_scalar_default_is_left_unset",
    "test_bool_default_is_carried",
    "test_secret_name_flagged",
    "test_declaration_order_is_preserved",
    "test_empty_param_block_is_a_zero_field_surface",
    "test_no_param_block_returns_none",
    "test_parse_error_returns_none",
    "test_non_dict_payload_returns_none",
    "test_missing_status_returns_none",
    "test_params_not_a_list_yields_zero_fields",
    "test_non_dict_row_is_skipped",
    "test_nameless_row_is_dropped",
    "test_no_powershell_at_all_returns_none",
    "test_nonzero_exit_returns_none",
    "test_unparseable_json_returns_none",
    "test_timeout_returns_none",
    "test_extract_passes_the_configured_timeout",
    "test_oserror_returns_none",
    "test_find_prefers_pwsh",
    "test_find_none_on_non_windows",
    "test_find_falls_back_to_powershell_exe_on_windows",
    "test_find_none_on_windows_without_powershell",
    "test_single_dash_flags_assemble",
    "test_plan_reads_powershell_param_block",
    "test_plan_none_when_reader_finds_no_surface",
    "test_integration_reads_a_real_param_block",
];
const ARCHITECTURE_CLOSED: &[&str] = &[
    "test_non_dict_payload_returns_none",
    "test_missing_status_returns_none",
    "test_params_not_a_list_yields_zero_fields",
    "test_non_dict_row_is_skipped",
    "test_nameless_row_is_dropped",
    "test_no_powershell_at_all_returns_none",
    "test_nonzero_exit_returns_none",
    "test_unparseable_json_returns_none",
    "test_timeout_returns_none",
    "test_extract_passes_the_configured_timeout",
    "test_oserror_returns_none",
    "test_find_prefers_pwsh",
    "test_find_none_on_non_windows",
    "test_find_falls_back_to_powershell_exe_on_windows",
    "test_find_none_on_windows_without_powershell",
];

fn has_test_attribute(attributes: &[Attribute]) -> bool {
    attributes
        .iter()
        .any(|attribute| attribute.path().is_ident("test"))
}

fn has_ignore_attribute(attributes: &[Attribute]) -> bool {
    attributes
        .iter()
        .any(|attribute| attribute.path().is_ident("ignore"))
}

fn sources(repo: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for relative in TEST_DIRS {
        let directory = repo.join(relative);
        for entry in fs::read_dir(&directory)
            .unwrap_or_else(|error| panic!("could not read {}: {error}", directory.display()))
        {
            let path = entry.unwrap().path();
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if name.starts_with("port_test_powershell") && name.ends_with(".rs") {
                found.push(path);
            }
        }
    }
    found.sort();
    assert_eq!(
        found.len(),
        3,
        "PowerShell parity sources changed: {found:?}"
    );
    found
}

fn executable_names(repo: &Path) -> BTreeMap<String, usize> {
    let mut names = BTreeMap::new();
    for path in sources(repo) {
        let source = fs::read_to_string(&path).unwrap();
        let file = syn::parse_file(&source)
            .unwrap_or_else(|error| panic!("could not parse {}: {error}", path.display()));
        for item in file.items {
            let Item::Fn(function) = item else {
                continue;
            };
            if !has_test_attribute(&function.attrs) || has_ignore_attribute(&function.attrs) {
                continue;
            }
            let name = function.sig.ident.to_string();
            if name.starts_with("rust_additive_") {
                continue;
            }
            *names.entry(name).or_insert(0_usize) += 1;
        }
    }
    names
}

#[test]
fn frozen_powershell_python_inventory_is_exact() {
    assert_eq!(
        PYTHON_TESTS.len(),
        35,
        "the frozen PowerShell denominator changed"
    );
    assert_eq!(
        PYTHON_TESTS.iter().copied().collect::<BTreeSet<_>>().len(),
        PYTHON_TESTS.len(),
        "duplicate Python PowerShell contract names"
    );
    let python = PYTHON_TESTS.iter().copied().collect::<BTreeSet<_>>();
    let closed = ARCHITECTURE_CLOSED.iter().copied().collect::<BTreeSet<_>>();
    assert_eq!(
        closed.len(),
        ARCHITECTURE_CLOSED.len(),
        "duplicate architecture-closed PowerShell contract names"
    );
    assert_eq!(
        closed.len(),
        15,
        "the PowerShell architecture-closure set changed"
    );
    assert!(closed.is_subset(&python));
}

#[test]
fn every_powershell_contract_is_exact_or_explicitly_architecture_closed() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("skit-cli lives under <repo>/crates/skit-cli");
    let counts = executable_names(repo);
    let duplicates = counts
        .iter()
        .filter_map(|(name, count)| (*count > 1).then_some(format!("{name} x{count}")))
        .collect::<Vec<_>>();
    assert!(
        duplicates.is_empty(),
        "duplicate PowerShell parity mappings:\n{}",
        duplicates.join("\n")
    );

    let python = PYTHON_TESTS.iter().copied().collect::<BTreeSet<_>>();
    let closed = ARCHITECTURE_CLOSED.iter().copied().collect::<BTreeSet<_>>();
    let expected = python.difference(&closed).copied().collect::<BTreeSet<_>>();
    let actual = counts.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let unexpected = actual.difference(&python).copied().collect::<Vec<_>>();
    assert!(
        unexpected.is_empty(),
        "invented parity-looking PowerShell tests must be renamed `rust_additive_*`:\n{}",
        unexpected.join("\n")
    );
    let closed_but_executable = closed.intersection(&actual).copied().collect::<Vec<_>>();
    assert!(
        closed_but_executable.is_empty(),
        "PowerShell contracts cannot be both executable and architecture-closed:\n{}",
        closed_but_executable.join("\n")
    );
    let missing = expected.difference(&actual).copied().collect::<Vec<_>>();
    assert!(
        missing.is_empty(),
        "tests/test_powershell.py has only {}/{} executable public contracts; missing:\n{}",
        expected.len() - missing.len(),
        expected.len(),
        missing.join("\n")
    );
}
