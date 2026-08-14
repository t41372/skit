//! Exact-name completeness gate for Python v0.4 `tests/test_add_no_source.py` at `main@206f9ef`.
//!
//! Public behavior is intentionally proven through real CLI/PTTY/store boundaries or Rust's public
//! frontend-neutral add reducer. Six Python-private call-shape/composition seams have no equivalent
//! deterministic public Rust API and are closed explicitly; they are not replaced by weaker tests.

use std::collections::BTreeSet;
use syn::{Attribute, Item};

const EXECUTABLE: &[&str] = &[
    "test_bare_add_no_input_lists_the_lanes",
    "test_bare_add_piped_lists_the_lanes",
    "test_bare_add_interactive_refuses_each_orphan_flag",
    "test_plain_menu_choice2_opens_the_python_editor_lane",
    "test_plain_menu_choice3_opens_the_prompt_editor_lane",
    "test_plain_menu_choice4_command_template_happy_path",
    "test_plain_menu_choice4_empty_template_cancels",
    "test_plain_menu_choice4_empty_name_cancels",
    "test_plain_menu_choice4_stores_the_description",
    "test_plain_menu_choice1_path_continues_into_a_real_add",
    "test_plain_menu_choice1_empty_path_cancels",
    "test_bare_add_tui_form_summary_on_success",
    "test_bare_add_tui_form_cancel_exits_130",
    "test_ask_kind_plain_lists_sorted_interpreted_plus_exe_and_prompt",
    "test_ask_kind_plain_no_exe_when_offer_exe_false",
    "test_ask_kind_plain_shebang_question_variant",
    "test_ask_kind_plain_returns_the_picked_language",
    "test_ask_kind_plain_returns_exe_and_prompt",
    "test_unknown_plain_pick_language_adds_it",
    "test_unknown_plain_pick_exe_adds_it",
    "test_unknown_plain_cancel_exits_130",
    "test_unknown_plain_pick_language_with_runner_hits_prompt_only_refusal",
    "test_unknown_plain_pick_prompt_runs_prompt_onboarding",
    "test_unknown_plain_kept_draft_offers_no_program_option",
    "test_unknown_tui_form_pick_routes_to_the_kind",
    "test_unknown_tui_form_cancel_exits_130",
    "test_unknown_tui_form_shebang_flag_forwarded",
    "test_md_tui_form_passes_suggested_prompt",
    "test_unknown_tui_form_pick_exe_hosts_the_review_panel",
    "test_unknown_tui_form_pick_exe_cancel_exits_130",
    "test_exe_flag_tui_form_hosts_the_panel_and_prefills_flags",
    "test_hosted_interpreted_branch_prints_managed_and_secret_lines",
    "test_hosted_python_branch_prints_managed_and_secret_lines",
    "test_ans_choice2_python_lane_uses_blank_defaults",
    "test_ans_choice3_prompt_lane_uses_blank_defaults",
    "test_ans_tui_summary_receives_deps_params_and_secrets",
    "test_hosted_add_summary_script_reads_decls_and_honors_decl_secret",
    "test_hosted_add_summary_prompt_falls_back_to_meta_and_name_heuristic",
    "test_hosted_add_summary_command_uses_meta_fallback",
    "test_ans_term_dumb_forces_the_plain_menu_even_with_form_tui",
    "test_ans_plain_menu_lines_are_exact",
    "test_ans_choice4_reports_params_and_stores_description",
    "test_ans_choice4_empty_template_cancels_with_exact_message",
    "test_ans_choice4_empty_name_cancels_with_exact_message",
    "test_ans_choice1_empty_path_cancels_with_exact_message",
    "test_ans_choice1_returns_the_typed_path",
    "test_cli_plain_choice4_prompt_labels_and_choices",
    "test_cli_plain_choice1_path_label",
    "test_cli_ask_kind_plain_full_layout",
    "test_cli_ask_kind_plain_shebang_question",
    "test_ans_no_stray_markup_tokens_in_output",
    "test_add_unknown_directory_plain_confirm_yes_adds_program",
    "test_add_unknown_directory_plain_confirm_no_cancels",
    "test_add_unknown_directory_plain_confirm_call_contract",
    "test_add_unknown_directory_tui_hosts_exe_review_with_no_line_confirm",
    "test_command_secret_names_picks_the_secret_holes",
    "test_cmd_flag_secret_hole_gets_never_saved_note",
    "test_plain_menu_choice4_secret_hole_gets_never_saved_note",
    "test_bare_add_tui_command_door_matches_the_cmd_door",
    "test_bare_add_refusal_names_only_lanes_that_honor_the_flag",
    "test_cancelled_add_exact_line_and_exit_code",
    "test_bare_add_tui_command_door_summary_call_contract",
];

const ARCHITECTURE_CLOSED: &[&str] = &[
    // Python calls the private TUI host callback directly and then checks the Rich host's exact
    // cancellation print. Rust exposes the workflow Cancel effect and the exact CLI cancellation
    // line independently (both are executable above), but no public single seam composes them.
    "test_ans_tui_cancel_prints_exact_message_and_exits_130",
    // These four tests inspect Prompt.ask/Confirm/store helper positional and keyword arguments,
    // including Python Rich console object identity. Exact user-visible prompts, choices, defaults,
    // path trimming, command persistence, and cancellation are all executed through real PTYs above;
    // recreating Python's call signatures in Rust would be a fake helper implementation.
    "test_ask_kind_plain_prompt_call_contract",
    "test_ans_which_one_prompt_call_contract",
    "test_ans_choice4_call_contracts",
    "test_ans_path_prompt_call_contract",
    // Direct mutation test of Python's private `_wants_tui_form`: Rust's selector is likewise private
    // composition code. Its two public false consequences (form=plain; TERM=dumb) are executed above,
    // while actual TUI workflow state is covered through the public reducer. A source-code mirror of
    // the private boolean helper would not be behavioral parity.
    "test_wants_tui_form_matrix",
];

fn has_test(attributes: &[Attribute]) -> bool {
    attributes.iter().any(|attribute| attribute.path().is_ident("test"))
}

fn names(source: &str) -> Vec<String> {
    syn::parse_file(source)
        .expect("add-no-source port source must parse")
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
fn test_add_no_source_frozen_names_are_exactly_accounted() {
    assert_eq!(EXECUTABLE.len(), 62);
    assert_eq!(ARCHITECTURE_CLOSED.len(), 6);
    let expected = EXECUTABLE.iter().copied().collect::<BTreeSet<_>>();
    let closed = ARCHITECTURE_CLOSED.iter().copied().collect::<BTreeSet<_>>();
    assert_eq!(expected.len(), 62, "duplicate executable add-no-source names");
    assert_eq!(closed.len(), 6, "duplicate closed add-no-source names");
    assert!(expected.is_disjoint(&closed));
    assert_eq!(expected.len() + closed.len(), 68);

    let mut actual_names = Vec::new();
    for source in [
        include_str!("port_test_add_no_source_plain.rs"),
        include_str!("port_test_add_no_source_more_public.rs"),
        include_str!("port_test_add_no_source_helper_consequences.rs"),
        include_str!("port_test_add_no_source_plain_edges.rs"),
        include_str!("port_test_add_no_source_summary.rs"),
        include_str!("port_test_add_no_source_command_door.rs"),
        include_str!("../../skit-ui/tests/port_test_add_no_source_state.rs"),
        include_str!("../../skit-ui/tests/port_test_add_no_source_lifecycle.rs"),
        include_str!("../../skit-ui/tests/port_test_add_no_source_editor_routes.rs"),
    ] {
        actual_names.extend(names(source));
    }
    assert_eq!(
        actual_names.len(),
        62,
        "canonical add-no-source sources contain duplicate or extra frozen-looking test_* functions"
    );
    let actual = actual_names.iter().map(String::as_str).collect::<BTreeSet<_>>();
    assert_eq!(actual, expected, "add-no-source executable parity is incomplete or mislabeled");
    assert!(actual.is_disjoint(&closed));
}
