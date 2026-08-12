//! Completeness guard for Python `tests/test_review_fixes.py` at `main@206f9ef`.
//!
//! This file is accounting only: behavioral assertions live beside the public Rust seam they test.
//! Twenty-one Python regressions have executable Rust equivalents. Nine are implementation/runtime
//! seams that do not exist in the Rust architecture; they remain explicitly architecture-closed
//! rather than being impersonated by same-named tests of different behavior.

use std::{collections::{BTreeMap, BTreeSet}, fs, path::Path};

use syn::{Attribute, Item};

struct Mapping {
    python: &'static str,
    path: &'static str,
}

const L: &str = "crates/skit-language/tests/port_test_review_fixes_language.rs";
const D: &str = "crates/skit-domain/tests/port_test_review_fixes_domain.rs";
const R: &str = "crates/skit-runtime/tests/port_test_review_fixes_runtime.rs";
const S: &str = "crates/skit-store/tests/port_test_review_fixes_store.rs";
const C: &str = "crates/skit-cli/tests/port_test_review_fixes_cli.rs";

const EXECUTABLE: &[Mapping] = &[
    Mapping { python: "test_escaped_placeholder_not_substituted", path: R },
    Mapping { python: "test_escape_unescaped_even_without_params", path: R },
    Mapping { python: "test_extra_args_quoted_for_posix_shell", path: R },
    Mapping { python: "test_inject_rejects_non_finite_float", path: L },
    Mapping { python: "test_inject_accepts_normal_float", path: L },
    Mapping { python: "test_write_injected_unique_and_private", path: C },
    Mapping { python: "test_write_params_prompt_with_newline_roundtrips", path: L },
    Mapping { python: "test_set_dependencies_multiline_array_with_comment", path: L },
    Mapping { python: "test_is_supported_rejects_junk", path: S },
    Mapping { python: "test_slugify_all_special_chars_fallback", path: D },
    Mapping { python: "test_write_params_no_block_no_params", path: L },
    Mapping { python: "test_parse_block_corrupt_body_returns_none", path: L },
    Mapping { python: "test_argstate_corrupt_file_fallback", path: S },
    Mapping { python: "test_config_language_corrupt_file", path: S },
    Mapping { python: "test_set_language_with_existing_corrupt_config", path: S },
    Mapping { python: "test_slugify_leading_trailing_special", path: D },
    Mapping { python: "test_inject_annotated_assignment", path: L },
    Mapping { python: "test_unique_slug_multiple_collisions", path: S },
    Mapping { python: "test_update_dependencies_reference_mode", path: C },
    Mapping { python: "test_update_dependencies_exe_entry", path: C },
    Mapping { python: "test_build_python_only_requires_python", path: R },
];

const ARCHITECTURE_CLOSED: &[(&str, &str)] = &[
    (
        "test_edit_specs_does_not_mutate_input",
        "Python exposes a pure edit_specs(list)->new-list helper. Rust separates read-only parser reconciliation from repository mutation and has no public in-memory edit-list seam whose caller-owned Vec could be mutated.",
    ),
    (
        "test_atomic_write_bytes_cleanup_on_error",
        "The Python oracle monkeypatches os.fdopen after temporary-file allocation. Rust's atomic writer has no public fault-injection seam at that exact post-allocation boundary; substituting another I/O failure would not prove cleanup of the same branch.",
    ),
    (
        "test_available_locales_missing_dir",
        "Python discovers compiled locale data from a runtime directory. Rust embeds a static catalog, so there is no locale-data directory whose absence can be observed at runtime.",
    ),
    (
        "test_normalize_four_char_subtag",
        "Python exposes a private string normalizer that returns zh-Hant-TW. Rust's public locale API resolves directly to a Locale enum and does not expose a canonicalized BCP-47 string for this input.",
    ),
    (
        "test_detect_locale_locale_module_error",
        "Python catches ValueError/TypeError from locale.getlocale(). Rust delegates platform locale discovery to sys_locale and has no equivalent exception-producing locale-module seam.",
    ),
    (
        "test_preamble_appended_when_only_future_imports",
        "Python injection materializes a # skit:shim preamble. Rust performs parser-owned direct source edits and has no shim preamble artifact; testing the marker would assert Python implementation shape rather than behavior.",
    ),
    (
        "test_preamble_appends_newline_when_missing",
        "Same Python-only shim-preamble architecture: Rust emits no preamble, so there is no preamble-newline branch to exercise without fabricating an implementation detail.",
    ),
    (
        "test_find_uv_private_bin_exe_variant",
        "Python monkeypatches Windows private-bin lookup on any host. Rust's managed private-uv path is compile-time cfg-selected; constructing a Windows UvTarget would test asset naming, not the actual current-host private-bin discovery branch.",
    ),
    (
        "test_ensure_uv_downloaded_success",
        "Python injects a fake network fetch into uv installation. Rust's public ensure_managed_uv owns the real downloader and exposes no fetch-injection seam; using the network or testing archive helpers would weaken the exact success-path contract.",
    ),
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

fn load_targets(repo: &Path) -> BTreeMap<&'static str, BTreeSet<String>> {
    [L, D, R, S, C]
        .into_iter()
        .map(|path| {
            let source = fs::read_to_string(repo.join(path)).unwrap();
            (path, test_names(&source))
        })
        .collect()
}

#[test]
fn every_executable_review_fix_has_exactly_one_rust_oracle() {
    assert_eq!(EXECUTABLE.len(), 21);
    assert_eq!(ARCHITECTURE_CLOSED.len(), 9);
    assert_eq!(EXECUTABLE.len() + ARCHITECTURE_CLOSED.len(), 30);
    assert_eq!(
        EXECUTABLE
            .iter()
            .map(|mapping| mapping.python)
            .chain(ARCHITECTURE_CLOSED.iter().map(|(name, _)| *name))
            .collect::<BTreeSet<_>>()
            .len(),
        30,
        "duplicate accounting would hide an unmigrated Python regression"
    );

    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap();
    let actual_by_path = load_targets(repo);
    let expected = EXECUTABLE
        .iter()
        .map(|mapping| mapping.python.to_owned())
        .collect::<BTreeSet<_>>();
    let actual_union = actual_by_path
        .values()
        .flat_map(|names| names.iter().cloned())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        actual_union, expected,
        "review-fixes target files contain an unmapped or missing executable test"
    );

    let mut wrong_target = Vec::new();
    let mut duplicated = Vec::new();
    for mapping in EXECUTABLE {
        if !actual_by_path[mapping.path].contains(mapping.python) {
            wrong_target.push(format!("{} -> {}", mapping.python, mapping.path));
        }
        let occurrences = actual_by_path
            .values()
            .filter(|names| names.contains(mapping.python))
            .count();
        if occurrences != 1 {
            duplicated.push(format!("{} occurs {occurrences} times", mapping.python));
        }
    }
    assert!(
        wrong_target.is_empty(),
        "review-fixes executable mappings moved to the wrong seam:\n{}",
        wrong_target.join("\n")
    );
    assert!(
        duplicated.is_empty(),
        "review-fixes executable mappings are not one-to-one:\n{}",
        duplicated.join("\n")
    );
}

#[test]
fn architecture_closed_review_fixes_are_not_impersonated_by_weaker_tests() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap();
    let actual_by_path = load_targets(repo);
    let executable_names = actual_by_path
        .values()
        .flat_map(|names| names.iter().cloned())
        .collect::<BTreeSet<_>>();

    for (name, reason) in ARCHITECTURE_CLOSED {
        assert!(!reason.trim().is_empty(), "{name} needs a concrete architectural reason");
        assert!(
            !executable_names.contains(*name),
            "{name} is architecture-closed ({reason}); do not add a same-named weaker stand-in"
        );
    }
}
