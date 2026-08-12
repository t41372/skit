//! Exact completeness guard for Python `tests/test_add_lane_contracts.py` at `main@206f9ef`.

use std::collections::BTreeSet;

use syn::{Attribute, Item};

const MAIN: &str = include_str!("port_test_add_lane_contracts.rs");
const EDITOR: &str = include_str!("port_test_add_lane_editor.rs");

const PYTHON_TESTS: &[&str] = &[
    "test_selector_collisions_are_refused_one_voice",
    "test_stdin_versioned_python_shebang_lands_as_python",
    "test_editor_lane_versioned_python_shebang_onboards_as_python",
    "test_stdin_prompt_bogus_runner_refused_before_any_draft",
    "test_prompt_editor_bogus_runner_refused_before_the_editor",
    "test_edit_no_input_is_refused_with_the_pipe_spelling",
    "test_prompt_editor_no_input_in_a_terminal_is_refused",
    "test_prompt_no_input_piped_still_adds",
    "test_edit_description_flag_wins_over_python_docstring",
    "test_edit_description_flag_on_non_python_draft_is_stored",
    "test_edit_post_editor_refusal_keeps_draft_and_announces_short",
    "test_path_add_of_a_drafts_home_file_unlinks_it_on_copy",
    "test_path_add_of_a_drafts_home_file_refuses_reference",
    "test_path_add_of_a_normal_file_never_unlinks_the_original",
    "test_shell_getopts_add_prints_the_read_notice",
    "test_shell_dynamic_getopts_add_prints_the_passthrough_notice",
    "test_js_parseargs_add_prints_the_read_notice",
    "test_params_python_argparse_read_view_is_plain",
    "test_params_python_constants_only_still_offers_manage",
    "test_manage_flip_note_names_the_reader_form_then_stays_quiet",
    "test_manage_flip_json_stdout_is_exactly_one_document",
];

fn is_test(attributes: &[Attribute]) -> bool {
    attributes
        .iter()
        .any(|attribute| attribute.path().is_ident("test"))
}

fn tests(source: &str) -> Vec<String> {
    syn::parse_file(source)
        .expect("add-lane parity target must parse as Rust")
        .items
        .into_iter()
        .filter_map(|item| match item {
            Item::Fn(function) if is_test(&function.attrs) => Some(function.sig.ident.to_string()),
            _ => None,
        })
        .collect()
}

#[test]
fn all_twenty_one_python_add_lane_contracts_are_executable_once() {
    let actual = tests(MAIN)
        .into_iter()
        .chain(tests(EDITOR))
        .collect::<Vec<_>>();
    let actual_set = actual.iter().cloned().collect::<BTreeSet<_>>();
    let expected = PYTHON_TESTS
        .iter()
        .map(|name| (*name).to_owned())
        .collect::<BTreeSet<_>>();

    assert_eq!(PYTHON_TESTS.len(), 21);
    assert_eq!(actual.len(), 21, "unexpected extra or missing add-lane tests");
    assert_eq!(actual_set.len(), actual.len(), "duplicate names hide a missing contract");
    assert_eq!(actual_set, expected);
}
