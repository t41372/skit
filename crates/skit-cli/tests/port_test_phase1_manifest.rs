//! Completeness guard for Python `tests/test_phase1.py` at `main@206f9ef`.
//! All 27 frozen contracts have executable Rust oracles. No architecture exceptions are needed.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

use syn::{Attribute, Item};

struct Mapping {
    python: &'static str,
    path: &'static str,
}

const LANG: &str = "crates/skit-language/tests/port_test_phase1_language.rs";
const UV_REVIEW: &str = "crates/skit-ui/tests/port_test_phase1_uv_review.rs";
const PLACEHOLDERS: &str = "crates/skit-language/tests/port_test_phase1_placeholders.rs";
const RUNTIME: &str = "crates/skit-runtime/tests/port_test_phase1_runtime.rs";
const STORE_STATE: &str = "crates/skit-cli/tests/port_test_phase1_store_state.rs";

const EXPECTED: &[Mapping] = &[
    Mapping {
        python: "test_parse_block",
        path: LANG,
    },
    Mapping {
        python: "test_parse_no_block",
        path: LANG,
    },
    Mapping {
        python: "test_suggest_dependencies",
        path: LANG,
    },
    Mapping {
        python: "test_suggest_syntax_error_returns_empty",
        path: LANG,
    },
    Mapping {
        python: "test_suggest_dependencies_maps_import_name_to_pypi_package",
        path: LANG,
    },
    Mapping {
        python: "test_suggest_dependencies_dedupes_after_mapping",
        path: LANG,
    },
    Mapping {
        python: "test_suggest_dependencies_unmapped_name_unchanged",
        path: LANG,
    },
    Mapping {
        python: "test_suggest_dependencies_excludes_sibling_py_module",
        path: LANG,
    },
    Mapping {
        python: "test_suggest_dependencies_excludes_sibling_package_dir",
        path: LANG,
    },
    Mapping {
        python: "test_suggest_dependencies_keeps_name_without_a_sibling",
        path: LANG,
    },
    Mapping {
        python: "test_suggest_dependencies_default_script_dir_none_does_not_filter",
        path: LANG,
    },
    Mapping {
        python: "test_suggest_dependencies_from_import_sibling_excluded",
        path: LANG,
    },
    Mapping {
        python: "test_suggest_dependencies_submodule_of_sibling_dir_excluded",
        path: LANG,
    },
    Mapping {
        python: "test_inject_block_roundtrip",
        path: UV_REVIEW,
    },
    Mapping {
        python: "test_inject_preserves_body",
        path: LANG,
    },
    Mapping {
        python: "test_add_python_copy_injects_pep723",
        path: STORE_STATE,
    },
    Mapping {
        python: "test_add_python_reference_records_in_meta",
        path: STORE_STATE,
    },
    Mapping {
        python: "test_add_python_existing_block_not_touched",
        path: STORE_STATE,
    },
    Mapping {
        python: "test_build_command_reference_deps",
        path: RUNTIME,
    },
    Mapping {
        python: "test_extract_placeholders",
        path: PLACEHOLDERS,
    },
    Mapping {
        python: "test_command_params_fill_and_escape",
        path: RUNTIME,
    },
    Mapping {
        python: "test_command_missing_values_raises",
        path: RUNTIME,
    },
    Mapping {
        python: "test_argstate_roundtrip_and_forget",
        path: STORE_STATE,
    },
    Mapping {
        python: "test_remove_clears_argstate",
        path: STORE_STATE,
    },
    Mapping {
        python: "test_uv_download_url_shape",
        path: RUNTIME,
    },
    Mapping {
        python: "test_uv_triple_current_platform",
        path: RUNTIME,
    },
    Mapping {
        python: "test_ensure_uv_downloaded_skips_when_present",
        path: RUNTIME,
    },
];

fn has_test_attribute(attributes: &[Attribute]) -> bool {
    attributes
        .iter()
        .any(|attribute| attribute.path().is_ident("test"))
}

fn test_names(source: &str) -> BTreeSet<String> {
    syn::parse_file(source)
        .unwrap()
        .items
        .into_iter()
        .filter_map(|item| match item {
            Item::Fn(function) if has_test_attribute(&function.attrs) => {
                Some(function.sig.ident.to_string())
            }
            _ => None,
        })
        .collect()
}

#[test]
fn every_phase1_contract_has_exactly_one_rust_oracle() {
    assert_eq!(EXPECTED.len(), 27);
    assert_eq!(
        EXPECTED
            .iter()
            .map(|mapping| mapping.python)
            .collect::<BTreeSet<_>>()
            .len(),
        27,
        "duplicate accounting could hide an unmigrated Phase 1 contract"
    );

    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap();
    let targets = [LANG, UV_REVIEW, PLACEHOLDERS, RUNTIME, STORE_STATE]
        .into_iter()
        .map(|path| {
            let source = fs::read_to_string(repo.join(path)).unwrap();
            (path, test_names(&source))
        })
        .collect::<BTreeMap<_, _>>();
    let expected = EXPECTED
        .iter()
        .map(|mapping| mapping.python.to_owned())
        .collect::<BTreeSet<_>>();
    let actual = targets
        .values()
        .flat_map(|names| names.iter().cloned())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        actual, expected,
        "Phase 1 executable test inventory drifted"
    );

    let mut misplaced = Vec::new();
    let mut duplicated = Vec::new();
    for mapping in EXPECTED {
        if !targets[mapping.path].contains(mapping.python) {
            misplaced.push(format!("{} -> {}", mapping.python, mapping.path));
        }
        let count = targets
            .values()
            .filter(|names| names.contains(mapping.python))
            .count();
        if count != 1 {
            duplicated.push(format!("{} occurs {count} times", mapping.python));
        }
    }
    assert!(
        misplaced.is_empty(),
        "Phase 1 tests moved away from their audited public seam:\n{}",
        misplaced.join("\n")
    );
    assert!(
        duplicated.is_empty(),
        "Phase 1 tests are not one-to-one with Python:\n{}",
        duplicated.join("\n")
    );
}
