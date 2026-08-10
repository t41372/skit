//! Public-API ports of Python v0.4 filesystem glob assembly contracts from `tests/test_flows.py`.

use std::collections::BTreeMap;

use skit_application::{run_inputs::assemble_run_inputs, tokens::TokenContext};
use skit_domain::parameters::{ParamDecl, ParameterDelivery, ParameterType};
use skit_store::FileGlobExpander;
use tempfile::TempDir;

fn context(root: &TempDir, env: &[(&str, &str)]) -> TokenContext {
    TokenContext {
        cwd: root.path().display().to_string(),
        home: Some(root.path().join("home").display().to_string()),
        env: env
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect(),
        today: "2026-07-09".to_owned(),
        now: "14-30-05".to_owned(),
    }
}

fn inputs_decl() -> ParamDecl {
    let mut declaration = ParamDecl::new("inputs");
    declaration.delivery = ParameterDelivery::Flag;
    declaration.parameter_type = ParameterType::Path;
    declaration.multiple = true;
    declaration.required = true;
    declaration.flag.clear();
    declaration
}

#[test]
fn test_relative_glob_expands_multiple_field_against_invocation_cwd_in_sorted_order() {
    let root = TempDir::new().unwrap();
    std::fs::create_dir(root.path().join("shots")).unwrap();
    std::fs::write(root.path().join("shots/2.png"), b"").unwrap();
    std::fs::write(root.path().join("shots/1.png"), b"").unwrap();
    let glob = FileGlobExpander::new(root.path());

    let assembly = assemble_run_inputs(
        &[inputs_decl()],
        &BTreeMap::from([("inputs".to_owned(), "shots/*.png".to_owned())]),
        &[],
        true,
        &context(&root, &[]),
        &glob,
    )
    .unwrap();

    assert_eq!(
        assembly.args,
        [
            format!("shots{}1.png", std::path::MAIN_SEPARATOR),
            format!("shots{}2.png", std::path::MAIN_SEPARATOR),
        ]
    );
}

#[test]
fn test_relative_glob_without_matches_keeps_the_literal() {
    let root = TempDir::new().unwrap();
    let glob = FileGlobExpander::new(root.path());

    let assembly = assemble_run_inputs(
        &[inputs_decl()],
        &BTreeMap::from([("inputs".to_owned(), "none/*.xyz".to_owned())]),
        &[],
        true,
        &context(&root, &[]),
        &glob,
    )
    .unwrap();

    assert_eq!(assembly.args, ["none/*.xyz"]);
}

#[test]
fn test_multiple_field_splits_before_expanding_each_glob_piece() {
    let root = TempDir::new().unwrap();
    for name in ["a1.txt", "a2.txt", "b1.txt"] {
        std::fs::write(root.path().join(name), b"").unwrap();
    }
    let glob = FileGlobExpander::new(root.path());

    let assembly = assemble_run_inputs(
        &[inputs_decl()],
        &BTreeMap::from([("inputs".to_owned(), "a*.txt b*.txt".to_owned())]),
        &[],
        true,
        &context(&root, &[]),
        &glob,
    )
    .unwrap();

    assert_eq!(assembly.args, ["a1.txt", "a2.txt", "b1.txt"]);
}

#[test]
fn test_extra_args_expand_glob_and_named_tokens_through_the_same_pipeline() {
    let root = TempDir::new().unwrap();
    std::fs::write(root.path().join("x2.txt"), b"").unwrap();
    std::fs::write(root.path().join("x1.txt"), b"").unwrap();
    let glob = FileGlobExpander::new(root.path());

    let assembly = assemble_run_inputs(
        &[],
        &BTreeMap::new(),
        &[
            "x*.txt".to_owned(),
            "{today}".to_owned(),
            "{now}".to_owned(),
            "{cwd}".to_owned(),
            "{env:XV}".to_owned(),
        ],
        true,
        &context(&root, &[("XV", "envval")]),
        &glob,
    )
    .unwrap();

    assert_eq!(
        assembly.args,
        [
            "x1.txt".to_owned(),
            "x2.txt".to_owned(),
            "2026-07-09".to_owned(),
            "14-30-05".to_owned(),
            root.path().display().to_string(),
            "envval".to_owned(),
        ]
    );
}

#[test]
fn test_extra_arg_glob_without_match_remains_literal() {
    let root = TempDir::new().unwrap();
    let glob = FileGlobExpander::new(root.path());

    let assembly = assemble_run_inputs(
        &[],
        &BTreeMap::new(),
        &["missing-*.txt".to_owned()],
        true,
        &context(&root, &[]),
        &glob,
    )
    .unwrap();

    assert_eq!(assembly.args, ["missing-*.txt"]);
}

#[test]
fn test_non_glob_piece_is_returned_byte_for_byte() {
    let root = TempDir::new().unwrap();
    let glob = FileGlobExpander::new(root.path());
    assert_eq!(
        skit_application::glob_expansion::GlobExpander::expand_piece(&glob, "literal path.txt"),
        ["literal path.txt"]
    );
}
