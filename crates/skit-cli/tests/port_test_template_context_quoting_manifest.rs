//! Completeness gate for Python v0.4 `tests/test_template_context_quoting.py` at `main@206f9ef`.
//!
//! Rust exposes whole-template rendering, launch planning, and the resulting child command, but its
//! quote-state tracker is intentionally private. We execute every whole-template frozen contract and
//! close only the Python-private helper-state/value contracts; no helper is recreated in the test.

use std::collections::BTreeSet;
use syn::{Attribute, Item};

const EXECUTABLE: &[&str] = &[
    "test_double_quoted_placeholder_neutralizes_command_substitution",
    "test_single_quoted_placeholder_stays_literal_with_apostrophe_and_substitution",
    "test_unquoted_placeholder_embedded_in_a_word",
    "test_unquoted_placeholder_hostile_value_cannot_escape_the_word",
    "test_unfilled_placeholder_travels_through_unchanged",
    "test_brace_escapes_collapse_inside_quotes_without_disturbing_state",
    "test_substituted_value_containing_double_braces_is_not_rescanned",
    "test_extra_args_are_appended_shell_quoted_after_the_template",
    "test_quote_state_affects_only_later_placeholders",
    "test_describe_command_uses_the_same_context_aware_quoting",
    "test_dangling_backslash_before_a_placeholder_cannot_eat_the_value_escape",
    "test_dangling_backslash_in_unquoted_position_is_neutralized_too",
    "test_even_backslash_run_before_a_placeholder_adds_no_neutralizer",
    "test_dangling_backslash_before_brace_escape_and_unfilled_placeholder_is_absorbed",
    "test_render_win32_uses_list2cmdline_not_posix_quoting",
    "test_render_win32_repl_handles_brace_escapes_and_unfilled_placeholders",
    "test_value_survives_a_nested_command_substitution_verbatim",
    "test_double_quotes_nested_in_backticks_are_refused_not_guessed",
];

const ARCHITECTURE_CLOSED: &[&str] = &[
    "test_state_open_and_close_single_quote",
    "test_state_open_and_close_double_quote",
    "test_state_backslash_skips_next_char_in_unquoted_so_quote_stays_shut",
    "test_state_backslash_skips_closing_quote_inside_double",
    "test_state_backslash_is_literal_inside_single_quotes",
    "test_state_the_other_quote_kind_is_literal",
    "test_state_carries_across_successive_chunks",
    "test_state_dangling_backslash_pends_across_the_boundary",
    "test_state_resumes_a_pending_backslash_by_consuming_the_first_char",
    "test_value_single_context_escapes_embedded_apostrophe",
    "test_value_single_context_plain_value_is_verbatim",
    "test_value_double_context_escapes_backslash",
    "test_value_double_context_escapes_double_quote",
    "test_value_double_context_escapes_dollar",
    "test_value_double_context_escapes_backtick",
    "test_value_double_context_neutralizes_command_substitution",
    "test_value_double_context_backslash_doubling_precedes_dollar_escape",
    "test_value_double_context_backslash_before_double_quote_order",
    "test_value_double_context_backtick_after_dollar",
    "test_value_unquoted_context_defers_to_shlex_quote",
    "test_state_pushes_and_pops_a_command_substitution_frame",
    "test_state_treats_backticks_as_a_frame_that_one_character_opens_and_closes",
    "test_state_pops_exactly_one_frame_off_a_deep_stack",
    "test_state_opening_a_frame_never_discards_the_ones_below_it",
    "test_state_only_a_dollar_followed_by_paren_opens_a_substitution",
    "test_value_inside_a_substitution_takes_the_unquoted_branch",
];

fn has_test_attribute(attributes: &[Attribute]) -> bool {
    attributes
        .iter()
        .any(|attribute| attribute.path().is_ident("test"))
}

fn parity_names(source: &str) -> BTreeSet<String> {
    syn::parse_file(source)
        .expect("ported Rust quoting test source must parse")
        .items
        .into_iter()
        .filter_map(|item| match item {
            Item::Fn(function)
                if has_test_attribute(&function.attrs)
                    && function.sig.ident.to_string().starts_with("test_") =>
            {
                Some(function.sig.ident.to_string())
            }
            _ => None,
        })
        .collect()
}

#[test]
fn test_template_context_quoting_frozen_names_are_exactly_accounted() {
    assert_eq!(EXECUTABLE.len(), 18);
    assert_eq!(ARCHITECTURE_CLOSED.len(), 26);
    let expected = EXECUTABLE.iter().copied().collect::<BTreeSet<_>>();
    let closed = ARCHITECTURE_CLOSED.iter().copied().collect::<BTreeSet<_>>();
    assert_eq!(expected.len(), EXECUTABLE.len(), "duplicate executable oracle name");
    assert_eq!(closed.len(), ARCHITECTURE_CLOSED.len(), "duplicate closed oracle name");
    assert!(expected.is_disjoint(&closed));
    assert_eq!(expected.len() + closed.len(), 44, "frozen Python denominator changed");

    let actual = parity_names(include_str!(
        "../../skit-runtime/tests/port_test_command_template_quoting.rs"
    ));
    let actual = actual.iter().map(String::as_str).collect::<BTreeSet<_>>();
    assert_eq!(actual, expected, "template-quoting parity names drifted from the frozen Python oracle");
    assert!(actual.is_disjoint(&closed), "a Python-private quote helper was falsely presented as executable parity");
}
