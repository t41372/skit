//! Master migration-accounting gate for the Python behavior suite at `main@206f9ef`.
//!
//! The frozen Python suite has 175 test modules. Seventy-two mutation modules and nineteen coverage
//! fillers are toolchain-instrumentation work (cargo-mutants / llvm-cov), not line-for-line behavior
//! ports. The remaining **84 behavior/contract modules contain 3,018 `def test_` functions** and are
//! the granular migration surface this branch must account for before it is merge-ready.
//!
//! A module is marked accounted here only after it owns an audited executable completeness guard.
//! Existing Rust tests without such a guard deliberately remain `None`: implementation-authored
//! tests are not proof that every Python oracle was read. This test is expected to stay red while the
//! migration is incomplete; its failure is dynamic inventory, not a fixed-failure placeholder.

use std::{collections::BTreeSet, fs, path::Path};
use syn::{Attribute, Item};

#[derive(Clone, Copy)]
struct Module { python: &'static str, tests: usize, guard: Option<&'static str> }
const SMALL: &str = "crates/skit-cli/tests/port_test_small_behavior_manifests.rs";
const MODULES: &[Module] = &[
    Module { python: "test_analyzer.py", tests: 37, guard: Some("crates/skit-cli/tests/port_test_analyzer_manifest.rs") },
    Module { python: "test_analyzer_signals.py", tests: 9, guard: Some(SMALL) },
    Module { python: "test_argspec.py", tests: 34, guard: Some("crates/skit-cli/tests/port_test_argspec_manifest.rs") },
    Module { python: "test_argspec_click_typer.py", tests: 67, guard: Some("crates/skit-cli/tests/port_test_argspec_click_typer_manifest.rs") },
    Module { python: "test_callmatch.py", tests: 9, guard: Some(SMALL) },
    Module { python: "test_reconcile.py", tests: 27, guard: Some("crates/skit-cli/tests/port_test_reconcile_manifest.rs") },
    Module { python: "test_shell_analyzer.py", tests: 92, guard: Some("crates/skit-cli/tests/port_test_shell_analyzer_manifest.rs") },
    Module { python: "test_shell_inject.py", tests: 87, guard: Some("crates/skit-cli/tests/port_test_shell_inject_manifest.rs") },
    Module { python: "test_shell_getopts.py", tests: 11, guard: Some("crates/skit-cli/tests/port_test_shell_getopts_manifest.rs") },
    Module { python: "test_fish.py", tests: 64, guard: Some("crates/skit-cli/tests/port_test_fish_manifest.rs") },
    Module { python: "test_powershell.py", tests: 35, guard: Some("crates/skit-cli/tests/port_test_powershell_manifest.rs") },
    Module { python: "test_js_analyzer.py", tests: 67, guard: Some("crates/skit-cli/tests/port_test_js_analyzer_manifest.rs") },
    Module { python: "test_js_inject.py", tests: 37, guard: Some("crates/skit-cli/tests/port_test_js_inject_manifest.rs") },
    Module { python: "test_js_deps.py", tests: 151, guard: Some("crates/skit-cli/tests/port_test_js_deps_manifest.rs") },
    Module { python: "test_interpreters.py", tests: 74, guard: Some("crates/skit-cli/tests/port_test_interpreters_manifest.rs") },
    Module { python: "test_langs.py", tests: 21, guard: Some("crates/skit-cli/tests/port_test_langs_manifest.rs") },
    Module { python: "test_kindnames.py", tests: 5, guard: Some(SMALL) },
    Module { python: "test_tokens.py", tests: 21, guard: Some("crates/skit-cli/tests/port_test_tokens_manifest.rs") },
    Module { python: "test_pep723_split.py", tests: 24, guard: Some("crates/skit-cli/tests/port_test_pep723_split_manifest.rs") },
    Module { python: "test_metawriter.py", tests: 24, guard: Some("crates/skit-cli/tests/port_test_metawriter_manifest.rs") },
    Module { python: "test_template_context_quoting.py", tests: 44, guard: Some("crates/skit-cli/tests/port_test_template_context_quoting_manifest.rs") },
    Module { python: "test_declared_params.py", tests: 52, guard: Some("crates/skit-cli/tests/port_test_declared_params_manifest.rs") },
    Module { python: "test_source_default_semantics.py", tests: 19, guard: Some("crates/skit-cli/tests/port_test_source_default_semantics_manifest.rs") },
    Module { python: "test_default_semantics_review_fixes.py", tests: 18, guard: Some("crates/skit-cli/tests/port_test_default_review_manifest.rs") },
    Module { python: "test_effective_uv_metadata.py", tests: 26, guard: Some("crates/skit-cli/tests/port_test_effective_uv_metadata_manifest.rs") },
    Module { python: "test_uv_metadata_views.py", tests: 6, guard: Some("crates/skit-cli/tests/port_test_uv_metadata_views_manifest.rs") },
    Module { python: "test_uv_metadata_unpinning.py", tests: 4, guard: Some("crates/skit-cli/tests/port_test_uv_metadata_unpinning_manifest.rs") },
    Module { python: "test_path_type.py", tests: 14, guard: Some("crates/skit-cli/tests/port_test_path_type_manifest.rs") },
    Module { python: "test_corpus.py", tests: 11, guard: Some(SMALL) },
    Module { python: "test_raw.py", tests: 5, guard: Some("crates/skit-cli/tests/port_test_raw_manifest.rs") },
    Module { python: "test_rewrite.py", tests: 2, guard: Some(SMALL) },
    Module { python: "test_argv_text.py", tests: 1, guard: Some(SMALL) },
    Module { python: "test_store.py", tests: 78, guard: Some("crates/skit-cli/tests/port_test_store_manifest.rs") },
    Module { python: "test_store_fix.py", tests: 38, guard: Some("crates/skit-cli/tests/port_test_store_fix_manifest.rs") },
    Module { python: "test_atomic.py", tests: 32, guard: Some("crates/skit-cli/tests/port_test_atomic_manifest.rs") },
    Module { python: "test_flows.py", tests: 102, guard: Some("crates/skit-cli/tests/port_test_flows_manifest.rs") },
    Module { python: "test_uvman.py", tests: 36, guard: Some("crates/skit-cli/tests/port_test_uvman_manifest.rs") },
    Module { python: "test_launcher.py", tests: 38, guard: Some("crates/skit-cli/tests/port_test_launcher_manifest.rs") },
    Module { python: "test_launcher_fix.py", tests: 12, guard: Some("crates/skit-cli/tests/port_test_launcher_fix_manifest.rs") },
    Module { python: "test_shim.py", tests: 38, guard: Some("crates/skit-cli/tests/port_test_shim_manifest.rs") },
    Module { python: "test_entrypoint.py", tests: 10, guard: Some("crates/skit-cli/tests/port_test_entrypoint_manifest.rs") },
    Module { python: "test_cli.py", tests: 140, guard: Some("crates/skit-cli/tests/port_test_cli_manifest.rs") },
    Module { python: "test_prompt_cli.py", tests: 150, guard: Some("crates/skit-cli/tests/port_test_prompt_cli_manifest.rs") },
    Module { python: "test_prompt_kind.py", tests: 115, guard: Some("crates/skit-cli/tests/port_test_prompt_kind_manifest.rs") },
    Module { python: "test_config_cmd.py", tests: 75, guard: Some("crates/skit-cli/tests/port_test_config_cmd_manifest.rs") },
    Module { python: "test_add_no_source.py", tests: 68, guard: Some("crates/skit-cli/tests/port_test_add_no_source_manifest.rs") },
    Module { python: "test_config.py", tests: 60, guard: Some("crates/skit-cli/tests/port_test_config_manifest.rs") },
    Module { python: "test_editor.py", tests: 50, guard: Some("crates/skit-cli/tests/port_test_editor_manifest.rs") },
    Module { python: "test_default_name_resolution.py", tests: 42, guard: Some("crates/skit-cli/tests/port_test_default_name_resolution_manifest.rs") },
    Module { python: "test_params_edit.py", tests: 41, guard: Some("crates/skit-cli/tests/port_test_params_edit_manifest.rs") },
    Module { python: "test_add_validation_contracts.py", tests: 31, guard: Some("crates/skit-cli/tests/port_test_add_validation_contracts_manifest.rs") },
    Module { python: "test_review_fixes.py", tests: 30, guard: Some("crates/skit-cli/tests/port_test_review_fixes_manifest.rs") },
    Module { python: "test_run_set.py", tests: 27, guard: Some("crates/skit-cli/tests/port_test_run_set_manifest.rs") },
    Module { python: "test_draft_inference_and_reader_cli.py", tests: 27, guard: Some("crates/skit-cli/tests/port_test_draft_inference_and_reader_cli_manifest.rs") },
    Module { python: "test_agent_install.py", tests: 22, guard: Some("crates/skit-cli/tests/port_test_agent_install_manifest.rs") },
    Module { python: "test_dependency_write_validation.py", tests: 21, guard: Some("crates/skit-cli/tests/port_test_dependency_write_validation_manifest.rs") },
    Module { python: "test_add_lane_contracts.py", tests: 21, guard: Some("crates/skit-cli/tests/port_test_add_lane_contracts_manifest.rs") },
    Module { python: "test_dependency_command_contracts.py", tests: 20, guard: Some("crates/skit-cli/tests/port_test_dependency_command_contracts_manifest.rs") },
    Module { python: "test_params_model.py", tests: 19, guard: Some("crates/skit-cli/tests/port_test_param_model_manifest.rs") },
    Module { python: "test_show.py", tests: 17, guard: Some("crates/skit-cli/tests/port_test_show_manifest.rs") },
    Module { python: "test_add_feedback_contracts.py", tests: 16, guard: Some("crates/skit-cli/tests/port_test_add_feedback_contracts_manifest.rs") },
    Module { python: "test_edit.py", tests: 14, guard: Some("crates/skit-cli/tests/port_test_edit_manifest.rs") },
    Module { python: "test_presets.py", tests: 12, guard: Some("crates/skit-store/tests/port_test_presets_manifest.rs") },
    Module { python: "test_add_review_contracts.py", tests: 12, guard: Some("crates/skit-cli/tests/port_test_add_review_contracts_manifest.rs") },
    Module { python: "test_rename.py", tests: 10, guard: Some("crates/skit-cli/tests/port_test_rename_manifest.rs") },
    Module { python: "test_add_review_validation.py", tests: 10, guard: Some("crates/skit-cli/tests/port_test_add_review_validation_manifest.rs") },
    Module { python: "test_agent_skill.py", tests: 8, guard: Some("crates/skit-cli/tests/port_test_agent_skill_manifest.rs") },
    Module { python: "test_healthcheck.py", tests: 6, guard: Some("crates/skit-cli/tests/port_test_healthcheck_manifest.rs") },
    Module { python: "test_prompt_tui.py", tests: 83, guard: Some("crates/skit-cli/tests/port_test_prompt_tui_manifest.rs") },
    Module { python: "test_path_tui.py", tests: 61, guard: Some("crates/skit-cli/tests/port_test_path_tui_manifest.rs") },
    Module { python: "test_phase1.py", tests: 27, guard: Some("crates/skit-cli/tests/port_test_phase1_manifest.rs") },
    Module { python: "test_tui_responsive.py", tests: 19, guard: Some("crates/skit-cli/tests/port_test_tui_responsive_manifest.rs") },
    Module { python: "test_settings_and_draft_review_atomicity.py", tests: 16, guard: Some("crates/skit-cli/tests/port_test_settings_draft_atomicity_manifest.rs") },
    Module { python: "test_draft_and_reader_tui.py", tests: 16, guard: Some("crates/skit-cli/tests/port_test_draft_and_reader_tui_manifest.rs") },
    Module { python: "test_reset_default_ui.py", tests: 14, guard: Some("crates/skit-cli/tests/port_test_reset_default_ui_manifest.rs") },
    Module { python: "test_tui_edit.py", tests: 6, guard: Some("crates/skit-cli/tests/port_test_tui_edit_manifest.rs") },
    Module { python: "test_tui_nav.py", tests: 5, guard: Some(SMALL) },
    Module { python: "test_ime_input.py", tests: 3, guard: Some("crates/skit-cli/tests/port_test_ime_input_manifest.rs") },
    Module { python: "test_i18n.py", tests: 38, guard: Some("crates/skit-cli/tests/port_test_i18n_manifest.rs") },
    Module { python: "test_prompt_utf8.py", tests: 16, guard: Some("crates/skit-cli/tests/port_test_prompt_utf8_manifest.rs") },
    Module { python: "test_packaging.py", tests: 7, guard: Some("crates/skit-cli/tests/port_test_packaging_manifest.rs") },
    Module { python: "test_benchmarks_tooling.py", tests: 156, guard: None },
    Module { python: "test_mutation_gate.py", tests: 4, guard: Some("crates/skit-cli/tests/port_test_mutation_gate_manifest.rs") },
    Module { python: "test_hermeticity.py", tests: 1, guard: Some(SMALL) },
];

fn has_test_attribute(attributes: &[Attribute]) -> bool { attributes.iter().any(|attribute| attribute.path().is_ident("test")) }
fn guard_has_executable_test(repo: &Path, relative: &str) -> bool {
    let path = repo.join(relative);
    let Ok(source) = fs::read_to_string(&path) else { return false; };
    let Ok(file) = syn::parse_file(&source) else { return false; };
    file.items.iter().any(|item| matches!(item, Item::Fn(function) if has_test_attribute(&function.attrs)))
}
#[test]
fn frozen_behavior_inventory_shape_is_exact() {
    assert_eq!(MODULES.len(), 84, "the frozen behavior-module inventory changed");
    assert_eq!(MODULES.iter().map(|module| module.python).collect::<BTreeSet<_>>().len(), 84, "duplicate module names make behavior accounting dishonest");
    assert!(MODULES.iter().all(|module| module.tests > 0), "zero-count modules do not belong in the behavior inventory");
    assert_eq!(MODULES.iter().map(|module| module.tests).sum::<usize>(), 3_018, "the frozen behavior test-function denominator changed");
}
#[test]
fn every_behavior_module_has_an_audited_executable_completeness_guard() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent().and_then(Path::parent).expect("skit-cli lives under <repo>/crates/skit-cli");
    let mut accounted_modules = 0_usize;
    let mut accounted_tests = 0_usize;
    let mut missing = Vec::new();
    let mut broken_guards = Vec::new();
    for module in MODULES {
        match module.guard {
            Some(guard) if guard_has_executable_test(repo, guard) => { accounted_modules += 1; accounted_tests += module.tests; }
            Some(guard) => broken_guards.push(format!("{} ({} tests) -> {guard} is missing, invalid Rust, or has no executable #[test]", module.python, module.tests)),
            None => missing.push(format!("{} ({})", module.python, module.tests)),
        }
    }
    assert!(broken_guards.is_empty(), "an accounted behavior module lost its executable completeness guard:\n{}", broken_guards.join("\n"));
    assert!(missing.is_empty(), concat!("Python behavior parity is not merge-ready: {accounted_modules}/84 modules and ", "{accounted_tests}/3018 test functions have audited executable completeness guards. ", "Missing accounting:\n{}"), missing.join("\n"));
}
