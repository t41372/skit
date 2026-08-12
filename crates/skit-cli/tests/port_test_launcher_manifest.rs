//! Completeness guard for Python v0.4 `tests/test_launcher.py` at `main@206f9ef`.
//!
//! Thirty-seven contracts have same-named executable integration/runtime ports. The remaining
//! `test_python_without_uv_auto_downloads` maps to the already-existing private composition-root
//! bootstrap test: that seam injects a successful installer, proves it is called with the selected
//! mirror/data root, and proves its returned private uv path is pinned into both settings and entry
//! metadata. Re-exposing that private function just to manufacture a same-named integration test
//! would weaken the architecture rather than improve coverage.

use std::collections::{BTreeMap, BTreeSet};

use syn::{Attribute, Item};

const RUNTIME: &str = include_str!("../../skit-runtime/tests/port_test_launcher.rs");
const PREFLIGHT: &str = include_str!("../../skit-runtime/tests/port_test_launcher_preflight.rs");
const WORKDIR: &str = include_str!("../../skit-runtime/tests/port_test_launcher_workdir_more.rs");
const REFERENCE_PREFLIGHT: &str =
    include_str!("../../skit-runtime/tests/port_test_launcher_preflight_reference.rs");
const TARGET_HEALTH: &str = include_str!("port_test_launcher_target_health.rs");
const PROCESS: &str = include_str!("port_test_launcher_process.rs");
const UV_ABSENCE: &str = include_str!("port_test_launcher_uv_absence.rs");
const COMPOSITION_ROOT: &str = include_str!("../src/run/command.rs");

const AUTO_DOWNLOAD_PYTHON: &str = "test_python_without_uv_auto_downloads";
const AUTO_DOWNLOAD_RUST: &str =
    "a_completed_bootstrap_pins_the_installed_uv_in_settings_and_metadata";

const SAME_NAMED: &[&str] = &[
    "test_python_command_uses_uv_run_script",
    "test_python_uv_download_failure_raises",
    "test_command_template_appends_extra_args",
    "test_workdir_origin_is_source_parent",
    "test_workdir_store_and_invoke",
    "test_run_entry_real_execution",
    "test_find_uv_private_bin_fallback",
    "test_find_uv_returns_none_when_absent",
    "test_workdir_origin_no_source_falls_back_to_cwd",
    "test_workdir_absolute_path_used_directly",
    "test_python_with_deps_and_python_version",
    "test_exe_missing_source_raises",
    "test_exe_directory_source_refused_as_not_executable",
    "test_preflight_refuses_exe_directory_source",
    "test_build_command_unknown_kind_raises",
    "test_run_entry_missing_workdir_raises",
    "test_run_entry_command_entry",
    "test_run_entry_injects_mirror_env",
    "test_run_entry_no_mirror_env_when_disabled",
    "test_run_entry_keeps_user_index_when_mirror_enabled",
    "test_target_missing_false_for_healthy_python_entry",
    "test_target_missing_true_when_python_copy_deleted",
    "test_target_missing_true_when_python_reference_source_deleted",
    "test_target_missing_true_when_exe_deleted",
    "test_target_missing_never_true_for_command_entries",
    "test_preflight_passes_for_healthy_entry",
    "test_preflight_raises_for_missing_python_script",
    "test_preflight_raises_for_missing_exe",
    "test_preflight_raises_for_missing_workdir",
    "test_preflight_does_not_invoke_uv",
    "test_preflight_passes_for_command_entry_without_workdir_or_target_issues",
    "test_resolve_workdir_copy_mode_falls_back_when_origin_gone",
    "test_preflight_succeeds_for_copy_mode_entry_with_deleted_origin",
    "test_run_entry_succeeds_for_copy_mode_entry_with_deleted_origin",
    "test_resolve_workdir_reference_mode_not_masked_when_origin_gone",
    "test_preflight_reference_mode_still_raises_on_missing_script_when_origin_gone",
    "test_describe_command_isolates_like_build_command",
];

fn is_test(attributes: &[Attribute]) -> bool {
    attributes
        .iter()
        .any(|attribute| attribute.path().is_ident("test"))
}

fn top_level_test_names(source: &str) -> Vec<String> {
    syn::parse_file(source)
        .expect("launcher parity source must parse as Rust")
        .items
        .into_iter()
        .filter_map(|item| match item {
            Item::Fn(function) if is_test(&function.attrs) => Some(function.sig.ident.to_string()),
            _ => None,
        })
        .collect()
}

fn collect_nested_tests(items: &[Item], names: &mut Vec<String>) {
    for item in items {
        match item {
            Item::Fn(function) if is_test(&function.attrs) => {
                names.push(function.sig.ident.to_string());
            }
            Item::Mod(module) => {
                if let Some((_, items)) = &module.content {
                    collect_nested_tests(items, names);
                }
            }
            _ => {}
        }
    }
}

#[test]
fn launcher_has_exactly_one_executable_oracle_for_each_of_37_same_named_python_contracts() {
    let expected = SAME_NAMED.iter().copied().collect::<BTreeSet<_>>();
    assert_eq!(
        expected.len(),
        37,
        "same-named Python launcher inventory drifted"
    );

    let mut counts = BTreeMap::<String, usize>::new();
    for source in [
        RUNTIME,
        PREFLIGHT,
        WORKDIR,
        REFERENCE_PREFLIGHT,
        TARGET_HEALTH,
        PROCESS,
        UV_ABSENCE,
    ] {
        for name in top_level_test_names(source) {
            *counts.entry(name).or_default() += 1;
        }
    }

    assert_eq!(
        counts.len(),
        37,
        "launcher parity targets contain extra, missing, or aggregate-named tests: {counts:#?}"
    );
    for name in SAME_NAMED {
        assert_eq!(
            counts.get(*name).copied().unwrap_or_default(),
            1,
            "Python launcher contract {name} must map to exactly one executable Rust test"
        );
    }
    assert_eq!(
        counts.keys().map(String::as_str).collect::<BTreeSet<_>>(),
        expected
    );
}

#[test]
fn python_auto_download_contract_maps_to_the_real_injectable_bootstrap_success_oracle() {
    assert_eq!(
        AUTO_DOWNLOAD_PYTHON,
        "test_python_without_uv_auto_downloads"
    );
    let parsed =
        syn::parse_file(COMPOSITION_ROOT).expect("run composition root must parse as Rust");
    let mut names = Vec::new();
    collect_nested_tests(&parsed.items, &mut names);

    assert_eq!(
        names
            .iter()
            .filter(|name| name.as_str() == AUTO_DOWNLOAD_RUST)
            .count(),
        1,
        "the audited successful-installer bootstrap oracle disappeared or became non-executable"
    );
    assert!(
        !SAME_NAMED.contains(&AUTO_DOWNLOAD_PYTHON),
        "architecture-mapped auto-download contract must not be double-counted as a fake same-name test"
    );
    assert_eq!(SAME_NAMED.len() + 1, 38);
}
