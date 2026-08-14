//! Exact-name completeness gate for Python v0.4 `tests/test_config_cmd.py` at `main@206f9ef`.
//!
//! Sixty-one non-interactive contracts execute through the real `skit config` CLI. Fourteen frozen
//! wizard/first-run functions are dependency-injection tests whose only deterministic Rust seam is
//! private to `cli.rs`: the public bare invocation hard-wires `SystemNetworkProbe` and terminal
//! prompts. A live-network PTY would be weaker and flaky, so those names are closed explicitly and
//! a separate strength gate requires Rust's private deterministic tests to remain present.

use std::collections::BTreeSet;
use syn::{Attribute, Item};

const EXECUTABLE: &[&str] = &[
    "test_bare_config_lists_all_keys",
    "test_bare_config_json",
    "test_config_json_emits_raw_values_never_a_localized_sentinel",
    "test_read_one_key_json_emits_single_pair",
    "test_set_one_key_json_emits_final_pair",
    "test_unknown_key_exits_2",
    "test_set_lang_writes_language_key",
    "test_read_lang_shows_override",
    "test_lang_auto_clears",
    "test_unknown_lang_exits_2",
    "test_set_mirror_pypi_preset",
    "test_pypi_axis_does_not_drag_other_axes",
    "test_set_mirror_npm_alone",
    "test_set_mirror_github_expands_both_urls",
    "test_set_mirror_github_custom_base_expands",
    "test_set_mirror_github_off_clears_both_urls",
    "test_paused_github_write_prints_notice_and_clear_does_not",
    "test_set_mirror_github_rejects_http_base",
    "test_set_mirror_axis_custom_url",
    "test_set_mirror_axis_off_keeps_the_others",
    "test_set_last_axis_off_disables",
    "test_unknown_axis_value_exits_2",
    "test_npm_axis_rejects_pypi_vendor_name",
    "test_mirror_master_off_preserves_urls_and_on_restores",
    "test_mirror_master_on_with_nothing_saved_exits_2",
    "test_mirror_master_rejects_vendor_names_with_axis_pointer",
    "test_paused_axis_write_preserves_other_axes_and_stays_paused",
    "test_paused_axis_clear_leaves_other_axes_and_prints_no_notice",
    "test_paused_config_is_fully_visible_in_config_list",
    "test_read_mirror_axis_shows_custom_url",
    "test_mirror_github_read_value_round_trips",
    "test_mirror_github_rejects_display_strings",
    "test_mirror_axis_rejects_whitespace_url",
    "test_config_json_single_key_is_raw_master_token",
    "test_config_json_single_key_lang_is_raw_override_tag",
    "test_config_json_lang_unset_is_empty_string",
    "test_lang_override_non_string_reads_as_empty",
    "test_config_json_mirror_github_raw_token",
    "test_config_json_mirror_github_underivable_pair_is_literal_custom",
    "test_set_editor",
    "test_clear_editor_with_empty_value",
    "test_read_editor_default_line",
    "test_form_defaults_to_tui",
    "test_set_form_plain_and_back",
    "test_unknown_form_style_exits_2",
    "test_read_after_run_default",
    "test_set_after_run_stay_and_back",
    "test_unknown_after_run_exits_2",
    "test_after_run_garbage_in_config_file_normalizes_to_exit",
    "test_mirror_write_preserves_language",
    "test_lang_clear_preserves_mirror",
    "test_form_write_preserves_mirror_and_language",
    "test_read_bash_path_default_line",
    "test_set_bash_path_to_existing_file",
    "test_set_bash_path_to_missing_file_is_usage_error",
    "test_clear_bash_path_with_empty_value",
    "test_bare_config_lists_dotted_keys",
    "test_read_js_runner_default_line",
    "test_set_js_runner",
    "test_set_js_runner_unknown_is_usage_error",
    "test_clear_js_runner_with_empty_value",
];

const ARCHITECTURE_CLOSED: &[&str] = &[
    "test_mirror_wizard_asks_one_question_per_axis",
    "test_mirror_wizard_defaults_are_the_recommended_presets",
    "test_mirror_wizard_axes_answer_independently",
    "test_mirror_wizard_default_ignores_saved_choice",
    "test_mirror_wizard_default_ignores_non_preset_saved_url",
    "test_mirror_wizard_custom",
    "test_mirror_wizard_custom_rejects_non_https_github_base",
    "test_mirror_wizard_custom_axis_bad_url_reprompts",
    "test_first_run_offers_and_configures_when_blocked",
    "test_first_run_declined_still_marks_done",
    "test_first_run_not_blocked_marks_done",
    "test_first_run_skipped_when_not_interactive",
    "test_first_run_skipped_when_already_configured",
    "test_first_run_still_offered_after_language_only_config",
];

fn has_test(attributes: &[Attribute]) -> bool {
    attributes.iter().any(|attribute| attribute.path().is_ident("test"))
}

fn names(source: &str) -> Vec<String> {
    syn::parse_file(source)
        .expect("config command port source must parse")
        .items
        .into_iter()
        .filter_map(|item| match item {
            Item::Fn(function)
                if has_test(&function.attrs) && function.sig.ident.to_string().starts_with("test_") =>
            {
                Some(function.sig.ident.to_string())
            }
            _ => None,
        })
        .collect()
}

#[test]
fn test_config_cmd_frozen_names_are_exactly_accounted() {
    assert_eq!(EXECUTABLE.len(), 61);
    assert_eq!(ARCHITECTURE_CLOSED.len(), 14);
    let expected = EXECUTABLE.iter().copied().collect::<BTreeSet<_>>();
    let closed = ARCHITECTURE_CLOSED.iter().copied().collect::<BTreeSet<_>>();
    assert_eq!(expected.len(), 61, "duplicate executable config names");
    assert_eq!(closed.len(), 14, "duplicate closed config names");
    assert!(expected.is_disjoint(&closed));
    assert_eq!(expected.len() + closed.len(), 75);

    let mut actual_names = Vec::new();
    for source in [
        include_str!("port_test_config_cmd_core.rs"),
        include_str!("port_test_config_cmd_mirror.rs"),
        include_str!("port_test_config_cmd_state.rs"),
        include_str!("port_test_config_cmd_runtime_choices.rs"),
    ] {
        actual_names.extend(names(source));
    }
    assert_eq!(
        actual_names.len(),
        61,
        "canonical config command sources contain duplicate or extra frozen-looking test_* functions"
    );
    let actual = actual_names.iter().map(String::as_str).collect::<BTreeSet<_>>();
    assert_eq!(actual, expected, "config command executable parity is incomplete or mislabeled");
    assert!(actual.is_disjoint(&closed));
}
