//! Public-API ports of Python v0.4 `tests/test_template_context_quoting.py`.
//!
//! Command-template values are shell data, never shell syntax. The renderer must escape each value
//! for the quote context of its placeholder and perform substitution in one pass so injected braces
//! or command-substitution syntax never become active template/shell syntax.

#![cfg(not(windows))]

use std::collections::BTreeMap;

use skit_runtime::render_command_template;

fn values(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn test_double_quoted_placeholder_neutralizes_command_substitution() {
    assert_eq!(
        render_command_template(
            "printf \"%s\\n\" \"{value}\"",
            &values(&[("value", "$(printf unexpected)")]),
        )
        .unwrap(),
        "printf \"%s\\n\" \"\\$(printf unexpected)\""
    );
}

#[test]
fn test_single_quoted_placeholder_keeps_apostrophe_and_command_substitution_literal() {
    assert_eq!(
        render_command_template("echo '{v}'", &values(&[("v", "a'b $(id)")]),).unwrap(),
        "echo 'a'\\''b $(id)'"
    );
}

#[test]
fn test_unquoted_placeholder_embedded_in_word_preserves_surrounding_literal_text() {
    assert_eq!(
        render_command_template("echo scale={width}:-1", &values(&[("width", "640")]),).unwrap(),
        "echo scale=640:-1"
    );
}

#[test]
fn test_unquoted_hostile_value_is_quoted_as_one_word() {
    assert_eq!(
        render_command_template("echo scale={width}:-1", &values(&[("width", "640 $(id)")]),)
            .unwrap(),
        "echo scale='640 $(id)':-1"
    );
}

#[test]
fn test_unknown_unfilled_placeholder_travels_through_unchanged() {
    assert_eq!(
        render_command_template("echo {leftover}", &BTreeMap::new()).unwrap(),
        "echo {leftover}"
    );
}

#[test]
fn test_brace_escapes_collapse_inside_quotes_without_resetting_quote_state() {
    assert_eq!(
        render_command_template("echo \"{{x}} {v}\"", &values(&[("v", "$X")]),).unwrap(),
        "echo \"{x} \\$X\""
    );
}

#[test]
fn test_substituted_value_containing_double_braces_is_not_rescanned() {
    assert_eq!(
        render_command_template("echo \"{v}\"", &values(&[("v", "{{x}}")]),).unwrap(),
        "echo \"{{x}}\""
    );
}

#[test]
fn test_quote_state_affects_only_the_placeholder_currently_inside_it() {
    assert_eq!(
        render_command_template("echo \"{a}\" {b}", &values(&[("a", "$A"), ("b", "$B")]),).unwrap(),
        "echo \"\\$A\" '$B'"
    );
}

#[test]
fn test_uppercase_placeholder_is_substituted() {
    assert_eq!(
        render_command_template("echo {NAME}", &values(&[("NAME", "x")])).unwrap(),
        "echo x"
    );
}

#[test]
fn test_replacement_text_is_not_reparsed_as_another_placeholder() {
    assert_eq!(
        render_command_template("echo {a} {b}", &values(&[("a", "{b}"), ("b", "real")]),).unwrap(),
        "echo '{b}' real"
    );
}

#[test]
fn test_double_quoted_value_escapes_backslash_quote_dollar_and_backtick() {
    assert_eq!(
        render_command_template("printf \"%s\" \"{v}\"", &values(&[("v", "\\\"$x`y`")]),).unwrap(),
        "printf \"%s\" \"\\\\\\\"\\$x\\`y\\`\""
    );
}
