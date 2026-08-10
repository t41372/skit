//! Public-API ports of Python v0.4 shell `read` delivery-safety contracts.
//!
//! Detecting an interactive read is only half the contract: accepted form values must still reach
//! the shell byte-for-byte. Values that a normal `read` would split, trim, or shift are rejected
//! before launch rather than silently changing what the script receives.

use std::collections::BTreeMap;

use skit_form::onboarding_plan;
use skit_language::{LanguageError, ShellInputError, inject_values};

fn declarations(source: &str) -> Vec<skit_domain::parameters::ParamDecl> {
    onboarding_plan("shell", source)
        .candidates
        .into_iter()
        .filter(|candidate| {
            candidate.declaration.binding
                == skit_domain::parameters::ParameterBinding::Input
        })
        .map(|candidate| candidate.declaration)
        .collect()
}

fn values(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn test_empty_earlier_read_value_with_later_filled_value_is_gap_error() {
    let source = "read FIRST LAST\nprintf '%s|%s\\n' \"$FIRST\" \"$LAST\"\n";
    let declarations = declarations(source);
    assert_eq!(declarations.len(), 2);

    let error = inject_values(
        "shell",
        source,
        &declarations,
        &values(&[("input-1", ""), ("input-2", "later")]),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        LanguageError::ShellInput(ShellInputError::Gap { empty, filled })
            if empty == "input-1" && filled == "input-2"
    ));
}

#[test]
fn test_line_break_in_read_value_is_rejected() {
    let source = "read NAME\necho \"$NAME\"\n";
    let declarations = declarations(source);
    let error = inject_values(
        "shell",
        source,
        &declarations,
        &values(&[("input-1", "line1\nline2")]),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        LanguageError::ShellInput(ShellInputError::LineBreak { name }) if name == "input-1"
    ));
}

#[test]
fn test_nonfinal_read_value_cannot_contain_default_ifs_spaces_or_tabs() {
    let source = "read FIRST LAST\necho \"$FIRST:$LAST\"\n";
    let declarations = declarations(source);
    for bad in ["two words", "two\twords"] {
        let error = inject_values(
            "shell",
            source,
            &declarations,
            &values(&[("input-1", bad), ("input-2", "tail")]),
        )
        .unwrap_err();
        assert!(matches!(
            error,
            LanguageError::ShellInput(ShellInputError::FieldSplit { name })
                if name == "input-1"
        ));
    }
}

#[test]
fn test_final_read_value_may_contain_internal_spaces() {
    let source = "read FIRST LAST\necho \"$FIRST:$LAST\"\n";
    let declarations = declarations(source);
    let output = inject_values(
        "shell",
        source,
        &declarations,
        &values(&[("input-1", "head"), ("input-2", "two words inside")]),
    )
    .unwrap();
    assert!(output.contains("two words inside"), "{output}");
}

#[test]
fn test_final_read_value_cannot_have_edge_spaces_or_tabs() {
    let source = "read FIRST LAST\necho \"$FIRST:$LAST\"\n";
    let declarations = declarations(source);
    for bad in [" leading", "trailing ", "\tleading", "trailing\t"] {
        let error = inject_values(
            "shell",
            source,
            &declarations,
            &values(&[("input-1", "head"), ("input-2", bad)]),
        )
        .unwrap_err();
        assert!(matches!(
            error,
            LanguageError::ShellInput(ShellInputError::EdgeSpace { name })
                if name == "input-2"
        ));
    }
}

#[test]
fn test_single_read_value_with_internal_spaces_is_allowed_but_edge_space_is_not() {
    let source = "read NAME\necho \"$NAME\"\n";
    let declarations = declarations(source);
    assert!(
        inject_values(
            "shell",
            source,
            &declarations,
            &values(&[("input-1", "Ada Lovelace")]),
        )
        .is_ok()
    );

    let error = inject_values(
        "shell",
        source,
        &declarations,
        &values(&[("input-1", " Ada")]),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        LanguageError::ShellInput(ShellInputError::EdgeSpace { name }) if name == "input-1"
    ));
}
