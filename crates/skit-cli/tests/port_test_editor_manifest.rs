//! Executable completeness guard for Python `tests/test_editor.py` at `main@206f9ef`.
//!
//! The frozen module has 50 `def test_` contracts. Forty-nine have an equal-or-stronger executable
//! Rust oracle. One test is a Python/Windows private-token-shape assertion that cannot be observed
//! after Rust's process boundary; it is required to remain absent rather than being papered over by
//! a weaker test.

use std::{collections::BTreeSet, fs, path::Path};

use syn::{Attribute, Item};

struct Mapping {
    python: &'static str,
    path: &'static str,
    rust: &'static str,
}

const RESOLUTION: &str = "crates/skit-cli/tests/port_test_editor_resolution.rs";
const PROCESS: &str = "crates/skit-cli/tests/port_test_editor_process_contract.rs";
const POST_EDIT: &str = "crates/skit-cli/tests/port_test_editor_post_edit.rs";
const CONFIG: &str = "crates/skit-store/tests/port_test_editor_config.rs";
const CLI: &str = "crates/skit-cli/tests/port_test_editor_cli.rs";
const UNKNOWN: &str = "crates/skit-cli/tests/port_test_editor_unknown.rs";
const ADD_DRAFT: &str = "crates/skit-cli/tests/port_test_editor_add_draft.rs";
const ONBOARDING: &str = "crates/skit-cli/tests/port_test_editor_onboarding.rs";
const POST_EDIT_DRAFT: &str = "crates/skit-cli/tests/port_test_editor_post_edit_draft.rs";
const PARAMS: &str = "crates/skit-cli/tests/port_test_editor_params.rs";

const BLOCKED_PRIVATE_WINDOWS_TOKEN_SHAPE: &str =
    "test_resolve_editor_windows_empty_quoted_token_strips_to_empty";

const MAPPINGS: &[Mapping] = &[
    Mapping {
        python: "test_resolve_editor_config_wins_over_env",
        path: RESOLUTION,
        rust: "test_resolve_editor_config_wins_over_env",
    },
    Mapping {
        python: "test_resolve_editor_visual_over_editor",
        path: RESOLUTION,
        rust: "test_resolve_editor_visual_over_editor",
    },
    Mapping {
        python: "test_resolve_editor_editor_env_when_no_visual",
        path: RESOLUTION,
        rust: "test_resolve_editor_editor_env_when_no_visual",
    },
    Mapping {
        python: "test_resolve_editor_platform_default_unix",
        path: RESOLUTION,
        rust: "test_resolve_editor_platform_default_unix",
    },
    Mapping {
        python: "test_resolve_editor_platform_default_windows",
        path: RESOLUTION,
        rust: "test_resolve_editor_platform_default_windows",
    },
    Mapping {
        python: "test_resolve_editor_quoted_value_uses_posix_split_off_windows",
        path: RESOLUTION,
        rust: "test_resolve_editor_quoted_value_uses_posix_split_off_windows",
    },
    Mapping {
        python: "test_resolve_editor_quoted_value_non_posix_on_windows",
        path: RESOLUTION,
        rust: "test_resolve_editor_quoted_value_non_posix_on_windows",
    },
    Mapping {
        python: "test_resolve_editor_quoted_spaced_path_on_windows",
        path: RESOLUTION,
        rust: "test_resolve_editor_quoted_spaced_path_on_windows",
    },
    Mapping {
        python: "test_resolve_editor_unquoted_windows_path_untouched",
        path: RESOLUTION,
        rust: "test_resolve_editor_unquoted_windows_path_untouched",
    },
    Mapping {
        python: "test_resolve_editor_whitespace_visual_falls_through_to_editor",
        path: RESOLUTION,
        rust: "test_resolve_editor_whitespace_visual_falls_through_to_editor",
    },
    Mapping {
        python: "test_resolve_editor_whitespace_config_falls_through_to_visual",
        path: RESOLUTION,
        rust: "test_resolve_editor_whitespace_config_falls_through_to_visual",
    },
    Mapping {
        python: "test_resolve_editor_all_whitespace_candidates_use_platform_default",
        path: RESOLUTION,
        rust: "test_resolve_editor_all_whitespace_candidates_use_platform_default",
    },
    Mapping {
        python: "test_resolve_editor_unbalanced_quotes_falls_back_to_raw",
        path: RESOLUTION,
        rust: "test_resolve_editor_unbalanced_quotes_falls_back_to_raw",
    },
    Mapping {
        python: "test_open_in_editor_appends_path_and_returns_code",
        path: PROCESS,
        rust: "test_open_in_editor_appends_path_and_returns_code",
    },
    Mapping {
        python: "test_open_in_editor_returns_nonzero_without_raising",
        path: PROCESS,
        rust: "test_open_in_editor_returns_nonzero_without_raising",
    },
    Mapping {
        python: "test_open_in_editor_launch_failure_message_exact",
        path: PROCESS,
        rust: "test_open_in_editor_launch_failure_message_exact",
    },
    Mapping {
        python: "test_open_entry_prompt_removed_by_editor_is_a_clean_edited_source_error",
        path: POST_EDIT,
        rust: "test_open_entry_prompt_removed_by_editor_is_a_clean_edited_source_error",
    },
    Mapping {
        python: "test_config_editor_roundtrip_and_clear",
        path: CONFIG,
        rust: "test_config_editor_roundtrip_and_clear",
    },
    Mapping {
        python: "test_save_editor_preserves_other_keys",
        path: CONFIG,
        rust: "test_save_editor_preserves_other_keys",
    },
    Mapping {
        python: "test_load_editor_non_string_value_is_blank",
        path: CONFIG,
        rust: "test_load_editor_non_string_value_is_blank",
    },
    Mapping {
        python: "test_save_editor_clear_when_absent_does_not_raise",
        path: CONFIG,
        rust: "test_save_editor_clear_when_absent_does_not_raise",
    },
    Mapping {
        python: "test_edit_opens_copy_source",
        path: CLI,
        rust: "test_edit_opens_copy_source",
    },
    Mapping {
        python: "test_edit_opens_reference_original",
        path: CLI,
        rust: "test_edit_opens_reference_original",
    },
    Mapping {
        python: "test_edit_reference_source_gone",
        path: CLI,
        rust: "test_edit_reference_source_gone",
    },
    Mapping {
        python: "test_edit_reports_editor_launch_failure",
        path: CLI,
        rust: "test_edit_reports_editor_launch_failure",
    },
    Mapping {
        python: "test_edit_unknown_confirmed_creates",
        path: UNKNOWN,
        rust: "test_edit_unknown_confirmed_creates",
    },
    Mapping {
        python: "test_edit_unknown_declined_creates_nothing",
        path: UNKNOWN,
        rust: "test_edit_unknown_declined_creates_nothing",
    },
    Mapping {
        python: "test_edit_unknown_non_interactive_errors",
        path: UNKNOWN,
        rust: "test_edit_unknown_non_interactive_errors",
    },
    Mapping {
        python: "test_add_edit_creates_in_editor",
        path: ADD_DRAFT,
        rust: "test_add_edit_creates_in_editor",
    },
    Mapping {
        python: "test_add_edit_bash_shebang_draft_becomes_a_shell_entry",
        path: ADD_DRAFT,
        rust: "test_add_edit_bash_shebang_draft_becomes_a_shell_entry",
    },
    Mapping {
        python: "test_add_edit_js_shebang_draft_scans_npm_deps",
        path: ADD_DRAFT,
        rust: "test_add_edit_js_shebang_draft_scans_npm_deps",
    },
    Mapping {
        python: "test_add_edit_zsh_draft_records_interpreter_and_dry_run_names_zsh",
        path: ADD_DRAFT,
        rust: "test_add_edit_zsh_draft_records_interpreter_and_dry_run_names_zsh",
    },
    Mapping {
        python: "test_add_edit_shell_draft_onboards_picked_constants",
        path: ONBOARDING,
        rust: "test_add_edit_shell_draft_onboards_picked_constants",
    },
    Mapping {
        python: "test_add_edit_dep_flag_on_non_python_draft_is_refused",
        path: ADD_DRAFT,
        rust: "test_add_edit_dep_flag_on_non_python_draft_is_refused",
    },
    Mapping {
        python: "test_add_edit_python_name_taken_refuses_before_the_editor",
        path: ADD_DRAFT,
        rust: "test_add_edit_python_name_taken_refuses_before_the_editor",
    },
    Mapping {
        python: "test_add_edit_python_post_edit_failure_keeps_the_draft",
        path: POST_EDIT_DRAFT,
        rust: "test_add_edit_python_post_edit_failure_keeps_the_draft",
    },
    Mapping {
        python: "test_add_edit_rejects_path",
        path: ADD_DRAFT,
        rust: "test_add_edit_rejects_path",
    },
    Mapping {
        python: "test_add_edit_non_interactive_errors",
        path: ADD_DRAFT,
        rust: "test_add_edit_non_interactive_errors",
    },
    Mapping {
        python: "test_add_edit_empty_content_adds_nothing",
        path: ADD_DRAFT,
        rust: "test_add_edit_empty_content_adds_nothing",
    },
    Mapping {
        python: "test_add_edit_unregistered_shebang_refused_keeps_draft",
        path: ADD_DRAFT,
        rust: "test_add_edit_unregistered_shebang_refused_keeps_draft",
    },
    Mapping {
        python: "test_add_edit_untouched_starter_unlinks_the_draft",
        path: ADD_DRAFT,
        rust: "test_add_edit_untouched_starter_unlinks_the_draft",
    },
    Mapping {
        python: "test_add_prompt_editor_untouched_starter_unlinks_the_draft",
        path: ADD_DRAFT,
        rust: "test_add_prompt_editor_untouched_starter_unlinks_the_draft",
    },
    Mapping {
        python: "test_add_edit_prompts_for_name_when_omitted",
        path: ADD_DRAFT,
        rust: "test_add_edit_prompts_for_name_when_omitted",
    },
    Mapping {
        python: "test_add_edit_blank_name_errors",
        path: ADD_DRAFT,
        rust: "test_add_edit_blank_name_errors",
    },
    Mapping {
        python: "test_add_edit_editor_error_exits_one",
        path: ADD_DRAFT,
        rust: "test_add_edit_editor_error_exits_one",
    },
    Mapping {
        python: "test_add_edit_name_conflict_exits_one",
        path: ADD_DRAFT,
        rust: "test_add_edit_name_conflict_exits_one",
    },
    Mapping {
        python: "test_add_edit_writes_and_reports_managed_and_secret",
        path: ONBOARDING,
        rust: "test_add_edit_writes_and_reports_managed_and_secret",
    },
    Mapping {
        python: "test_params_edit_command_entry_refused",
        path: PARAMS,
        rust: "test_params_edit_command_entry_refused",
    },
    Mapping {
        python: "test_params_edit_missing_copy_refused",
        path: PARAMS,
        rust: "test_params_edit_missing_copy_refused",
    },
];

fn has_test_attribute(attributes: &[Attribute]) -> bool {
    attributes
        .iter()
        .any(|attribute| attribute.path().is_ident("test"))
}

#[test]
fn editor_python_contracts_have_real_executable_rust_oracles() {
    assert_eq!(
        MAPPINGS.len(),
        49,
        "editor executable mapping count drifted"
    );
    assert_eq!(
        MAPPINGS
            .iter()
            .map(|mapping| mapping.python)
            .collect::<BTreeSet<_>>()
            .len(),
        49,
        "duplicate Python mappings make the editor completeness count dishonest"
    );
    assert_eq!(
        49 + 1,
        50,
        "frozen Python editor module has 50 test functions"
    );

    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("skit-cli lives under <repo>/crates/skit-cli");
    let mut failures = Vec::new();
    let mut inspected_sources = BTreeSet::new();
    for mapping in MAPPINGS {
        let path = repo.join(mapping.path);
        let source = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("could not read {}: {error}", path.display()));
        inspected_sources.insert(mapping.path);
        let file = syn::parse_file(&source)
            .unwrap_or_else(|error| panic!("{} is not valid Rust: {error}", path.display()));
        let matched = file.items.iter().find_map(|item| match item {
            Item::Fn(function) if function.sig.ident == mapping.rust => {
                Some(has_test_attribute(&function.attrs))
            }
            _ => None,
        });
        match matched {
            Some(true) => {}
            Some(false) => failures.push(format!(
                "{} -> {}::{} exists but is not #[test]",
                mapping.python, mapping.path, mapping.rust
            )),
            None => failures.push(format!(
                "{} -> {}::{} is missing",
                mapping.python, mapping.path, mapping.rust
            )),
        }
    }
    assert!(
        failures.is_empty(),
        "editor parity manifest contains fake/non-executable mappings:\n{}",
        failures.join("\n")
    );

    // Do not let the one private Python/Windows token-shape gap get "closed" with a fake function.
    // A future real seam may add it, but then this manifest must be intentionally re-adjudicated.
    for relative in inspected_sources {
        let source = fs::read_to_string(repo.join(relative)).unwrap();
        let file = syn::parse_file(&source).unwrap();
        let names = file.items.iter().filter_map(|item| match item {
            Item::Fn(function) if has_test_attribute(&function.attrs) => {
                Some(function.sig.ident.to_string())
            }
            _ => None,
        });
        assert!(
            !names
                .into_iter()
                .any(|name| name == BLOCKED_PRIVATE_WINDOWS_TOKEN_SHAPE),
            "{BLOCKED_PRIVATE_WINDOWS_TOKEN_SHAPE} needs a genuine observable Windows seam; do not fake it"
        );
    }
}
