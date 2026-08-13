//! Exact completeness gate for Python `tests/test_interpreters.py` at `main@206f9ef`.
//!
//! This gate is intentionally red until every public Rust-equivalent contract has an executable
//! exact-name port. A Python-private helper is closed only when Rust has no equivalent architectural
//! seam; it is never replaced by a weaker same-named assertion. `rust_additive_*` tests strengthen
//! coverage but never count toward the 74 frozen Python tests.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use syn::{Attribute, Item};

const TEST_DIRS: &[&str] = &[
    "crates/skit-domain/tests",
    "crates/skit-language/tests",
    "crates/skit-runtime/tests",
    "crates/skit-store/tests",
    "crates/skit-cli/tests",
];

const PYTHON_TESTS: &[&str] = &[
    "test_resolve_interpreter_found_on_path",
    "test_resolve_interpreter_missing_posix_names_the_interpreter",
    "test_resolve_bash_on_win32_uses_config_path_when_it_exists",
    "test_resolve_bash_on_win32_configured_but_missing_falls_through",
    "test_resolve_bash_on_win32_unset_names_both_escape_hatches",
    "test_resolve_nonbash_on_win32_gets_generic_message",
    "test_which_seam_is_the_real_shutil_which",
    "test_interpreter_launch_builds_argv",
    "test_interpreter_launch_meta_interpreter_beats_default",
    "test_interpreter_launch_prefix_placement",
    "test_interpreter_launch_describe_is_side_effect_free",
    "test_interpreter_launch_target_is_script_path",
    "test_interpreter_launch_preflight_missing_interpreter",
    "test_interpreter_launch_preflight_ok",
    "test_interpreter_launch_missing_script_raises_before_resolution",
    "test_runner_detection_order_prefers_deno",
    "test_runner_falls_to_bun_then_node",
    "test_runner_meta_interpreter_override",
    "test_runner_config_override",
    "test_runner_none_installed_names_candidates_and_config_key",
    "test_runner_describe_uses_preferred_name_without_path_lookup",
    "test_runner_preflight_checks_script_and_runner",
    "test_runner_target_is_script_path",
    "test_shebang_plain",
    "test_shebang_env_form",
    "test_shebang_env_dash_s_with_flags",
    "test_shebang_none_when_no_shebang",
    "test_shebang_none_when_unreadable",
    "test_shebang_none_when_empty_hashbang_line",
    "test_shebang_env_with_only_flags_is_none",
    "test_kind_for_shebang_maps_the_program_or_none",
    "test_kind_for_shebang_versioned_python_is_python",
    "test_kind_for_shebang_text_versioned_python_and_non_matches",
    "test_infer_kind_versioned_python_shebang",
    "test_infer_extension_beats_shebang",
    "test_infer_shebang_beats_exec_bit",
    "test_infer_unknown_shebang_program_falls_to_exec_bit",
    "test_infer_exec_bit_only_is_exe",
    "test_infer_plain_file_is_unknown",
    "test_infer_zsh_extension_is_shell",
    "test_infer_r_extension_is_case_insensitive",
    "test_preflight_needs_lists_only_missing",
    "test_run_entry_needs_raises_before_spawn",
    "test_missing_needs_returns_the_gap",
    "test_missing_needs_empty_when_all_present",
    "test_meta_round_trip_carries_interpreter_needs_parameters",
    "test_meta_omits_empty_needs",
    "test_update_needs_sets_and_clears",
    "test_cli_add_shell_script_records_interpreter",
    "test_cli_add_kind_forces_extensionless_file",
    "test_cli_add_kind_exe",
    "test_cli_add_kind_unknown_is_usage_error",
    "test_cli_add_kind_and_exe_conflict",
    "test_cli_add_command_kind_rejected",
    "test_deps_need_sets_the_list",
    "test_deps_need_replaces_whole_list",
    "test_deps_clear_needs",
    "test_deps_need_and_clear_needs_conflict",
    "test_deps_need_works_on_python_too",
    "test_deps_dep_on_shell_is_refused",
    "test_deps_read_view_shows_needs_for_shell",
    "test_deps_json_view_includes_needs",
    "test_deps_read_view_needs_dash_when_empty",
    "test_doctor_flags_missing_needs",
    "test_doctor_json_needs_missing",
    "test_show_human_prints_needs_line",
    "test_show_json_includes_needs",
    "test_show_interpreted_header_and_source",
    "test_edit_program_refusal_is_kind_neutral",
    "test_edit_command_refusal_is_kind_neutral",
    "test_e2e_run_shell_script",
    "test_e2e_run_shell_env_param_reaches_child",
    "test_e2e_dry_run_shows_interpreter_and_script",
    "test_e2e_run_reference_mode_shell",
];

const ARCHITECTURE_CLOSED: &[&str] = &[
    "test_which_seam_is_the_real_shutil_which",
    "test_interpreter_launch_target_is_script_path",
    "test_runner_target_is_script_path",
    "test_shebang_none_when_unreadable",
    "test_missing_needs_returns_the_gap",
    "test_missing_needs_empty_when_all_present",
];

fn has_test_attribute(attributes: &[Attribute]) -> bool {
    attributes.iter().any(|attribute| attribute.path().is_ident("test"))
}

fn interpreter_sources(repo: &Path) -> Vec<PathBuf> {
    let mut sources = Vec::new();
    for relative in TEST_DIRS {
        let directory = repo.join(relative);
        let Ok(entries) = fs::read_dir(&directory) else { continue };
        for entry in entries {
            let entry = entry.unwrap_or_else(|error| panic!("could not read an entry under {}: {error}", directory.display()));
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else { continue };
            if name.starts_with("port_test_interpreters")
                && name.ends_with(".rs")
                && name != "port_test_interpreters_manifest.rs"
            {
                sources.push(path);
            }
        }
    }
    sources.sort();
    assert!(!sources.is_empty(), "no interpreter port sources were discovered");
    sources
}

fn executable_test_names(repo: &Path) -> BTreeMap<String, usize> {
    let mut names = BTreeMap::<String, usize>::new();
    for path in interpreter_sources(repo) {
        let source = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("could not read {}: {error}", path.display()));
        let file = syn::parse_file(&source)
            .unwrap_or_else(|error| panic!("could not parse {}: {error}", path.display()));
        for item in file.items {
            let Item::Fn(function) = item else { continue };
            if !has_test_attribute(&function.attrs) { continue; }
            let name = function.sig.ident.to_string();
            if name.starts_with("rust_additive_") { continue; }
            *names.entry(name).or_default() += 1;
        }
    }
    names
}

#[test]
fn frozen_interpreters_python_inventory_is_exact() {
    assert_eq!(PYTHON_TESTS.len(), 74, "the frozen interpreter denominator changed");
    assert_eq!(PYTHON_TESTS.iter().copied().collect::<BTreeSet<_>>().len(), 74, "duplicate Python names make interpreter accounting dishonest");
    let python = PYTHON_TESTS.iter().copied().collect::<BTreeSet<_>>();
    let closed = ARCHITECTURE_CLOSED.iter().copied().collect::<BTreeSet<_>>();
    assert_eq!(closed.len(), ARCHITECTURE_CLOSED.len(), "duplicate closed contracts");
    assert!(closed.is_subset(&python), "architecture-closed names must come from the frozen Python inventory");
}

#[test]
fn every_interpreters_contract_is_exact_or_explicitly_architecture_closed() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("skit-cli lives under <repo>/crates/skit-cli");
    let counts = executable_test_names(repo);
    let duplicates = counts.iter().filter_map(|(name, count)| (*count > 1).then_some(format!("{name} x{count}"))).collect::<Vec<_>>();
    assert!(duplicates.is_empty(), "duplicate exact-name interpreter mappings are not allowed:\n{}", duplicates.join("\n"));

    let python = PYTHON_TESTS.iter().copied().collect::<BTreeSet<_>>();
    let closed = ARCHITECTURE_CLOSED.iter().copied().collect::<BTreeSet<_>>();
    let expected_executable = python.difference(&closed).copied().collect::<BTreeSet<_>>();
    let actual = counts.keys().map(String::as_str).collect::<BTreeSet<_>>();

    let unexpected = actual.difference(&python).copied().collect::<Vec<_>>();
    assert!(unexpected.is_empty(), "parity-shaped `test_*` names without a frozen Python oracle must be renamed `rust_additive_*`:\n{}", unexpected.join("\n"));

    let missing = expected_executable.difference(&actual).copied().collect::<Vec<_>>();
    let closed_but_executable = closed.intersection(&actual).copied().collect::<Vec<_>>();
    assert!(closed_but_executable.is_empty(), "a contract cannot be both executable and architecture-closed:\n{}", closed_but_executable.join("\n"));
    assert!(missing.is_empty(), "tests/test_interpreters.py is not fully ported: {}/{} executable contracts present; missing:\n{}", expected_executable.len() - missing.len(), expected_executable.len(), missing.join("\n"));
}
