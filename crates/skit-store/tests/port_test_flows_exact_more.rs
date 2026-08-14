//! Exact cross-pipeline flows contracts that require the real filesystem glob adapter.

use std::{collections::BTreeMap, path::Path};

use skit_application::{
    form_feedback::{GlobCountPort, glob_count_request},
    glob_expansion::GlobExpander,
    run_inputs::assemble_run_inputs,
    tokens::TokenContext,
};
use skit_domain::parameters::{ParamDecl, ParameterDelivery};
use skit_store::FileGlobExpander;
use tempfile::TempDir;

fn context(root: &TempDir) -> TokenContext {
    TokenContext {
        cwd: root.path().display().to_string(),
        home: None,
        env: BTreeMap::new(),
        today: "2026-07-09".to_owned(),
        now: "14-30-05".to_owned(),
    }
}

fn multiple_inputs() -> ParamDecl {
    let mut inputs = ParamDecl::new("inputs");
    inputs.delivery = ParameterDelivery::Flag;
    inputs.required = true;
    inputs.multiple = true;
    inputs.flag.clear();
    inputs
}

#[test]
fn test_assemble_glob_expands_multiple_fields_against_cwd() {
    let root = TempDir::new().unwrap();
    std::fs::create_dir(root.path().join("shots")).unwrap();
    std::fs::write(root.path().join("shots/2.png"), b"").unwrap();
    std::fs::write(root.path().join("shots/1.png"), b"").unwrap();
    let glob = FileGlobExpander::new(root.path());
    let assembly = assemble_run_inputs(
        &[multiple_inputs()],
        &BTreeMap::from([("inputs".to_owned(), "shots/*.png".to_owned())]),
        &[],
        true,
        &context(&root),
        &glob,
    )
    .unwrap();
    assert_eq!(
        assembly.args,
        [
            Path::new("shots").join("1.png").display().to_string(),
            Path::new("shots").join("2.png").display().to_string(),
        ]
    );
}

#[test]
fn test_assemble_glob_without_match_keeps_literal() {
    let root = TempDir::new().unwrap();
    let glob = FileGlobExpander::new(root.path());
    let assembly = assemble_run_inputs(
        &[multiple_inputs()],
        &BTreeMap::from([("inputs".to_owned(), "none/*.xyz".to_owned())]),
        &[],
        true,
        &context(&root),
        &glob,
    )
    .unwrap();
    assert_eq!(assembly.args, ["none/*.xyz"]);
}

#[test]
fn test_assemble_extra_args_expand_tokens_and_globs() {
    let root = TempDir::new().unwrap();
    std::fs::write(root.path().join("x1.txt"), b"").unwrap();
    std::fs::write(root.path().join("x2.txt"), b"").unwrap();
    let glob = FileGlobExpander::new(root.path());
    let mut token_context = context(&root);
    token_context.env.insert("XV".to_owned(), "envval".to_owned());
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
        &token_context,
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
fn test_glob_feedback_counts() {
    let root = TempDir::new().unwrap();
    std::fs::write(root.path().join("a.png"), b"").unwrap();
    std::fs::write(root.path().join("b.png"), b"").unwrap();
    let glob = FileGlobExpander::new(root.path());
    let cwd = root.path().display().to_string();

    let request = glob_count_request("*.png", &cwd).expect("glob-shaped input must request feedback");
    assert_eq!(glob.count_matches(&request), 2);

    let request = glob_count_request("*.png extra.txt", &cwd).expect("mixed input still contains a glob");
    assert_eq!(glob.count_matches(&request), 3);

    let request = glob_count_request("*.png ?.png", &cwd).expect("two glob pieces must both be counted");
    assert_eq!(glob.count_matches(&request), 4);

    assert_eq!(glob_count_request("plain.txt", &cwd), None);
}

#[test]
fn test_expand_glob_piece_globs_only_when_glob_chars_present() {
    let root = TempDir::new().unwrap();
    std::fs::write(root.path().join("ax"), b"").unwrap();
    std::fs::write(root.path().join("bx"), b"").unwrap();
    let glob = FileGlobExpander::new(root.path());
    assert_eq!(glob.expand_piece("[ab]?*"), ["ax", "bx"]);
    assert_eq!(glob.expand_piece("ax"), ["ax"]);
}

#[test]
fn test_expand_glob_piece_supports_recursive_doublestar() {
    let root = TempDir::new().unwrap();
    std::fs::create_dir_all(root.path().join("deep/deeper")).unwrap();
    std::fs::write(root.path().join("deep/deeper/x.txt"), b"").unwrap();
    let glob = FileGlobExpander::new(root.path());
    assert_eq!(
        glob.expand_piece("**/x.txt"),
        [Path::new("deep").join("deeper").join("x.txt").display().to_string()]
    );
}

#[test]
fn test_assemble_display_order_and_masking() {
    let root = TempDir::new().unwrap();
    let glob = FileGlobExpander::new(root.path());

    let mut output = ParamDecl::new("OUTPUT");
    output.delivery = ParameterDelivery::Inject;
    let mut width = ParamDecl::new("WIDTH");
    width.delivery = ParameterDelivery::Inject;
    let mut key = ParamDecl::new("API_KEY");
    key.delivery = ParameterDelivery::Inject;
    key.secret = true;

    let assembly = assemble_run_inputs(
        &[output, width, key],
        &BTreeMap::from([
            ("OUTPUT".to_owned(), "long_{today}.jpg".to_owned()),
            ("WIDTH".to_owned(), "900".to_owned()),
            ("API_KEY".to_owned(), "sekret".to_owned()),
        ]),
        &[],
        true,
        &context(&root),
        &glob,
    )
    .unwrap();

    assert_eq!(
        assembly.display,
        [
            ("OUTPUT".to_owned(), "long_2026-07-09.jpg".to_owned()),
            ("WIDTH".to_owned(), "900".to_owned()),
            ("API_KEY".to_owned(), "•••".to_owned()),
        ]
    );
    assert_eq!(
        assembly.inject_values,
        BTreeMap::from([
            ("OUTPUT".to_owned(), "long_2026-07-09.jpg".to_owned()),
            ("WIDTH".to_owned(), "900".to_owned()),
            ("API_KEY".to_owned(), "sekret".to_owned()),
        ])
    );
    assert_eq!(assembly.masked_args, assembly.args);
}

#[test]
fn test_assemble_none_plan_only_carries_extras() {
    let root = TempDir::new().unwrap();
    let glob = FileGlobExpander::new(root.path());
    let assembly = assemble_run_inputs(
        &[],
        &BTreeMap::new(),
        &["-v".to_owned()],
        false,
        &context(&root),
        &glob,
    )
    .unwrap();
    assert_eq!(assembly.args, ["-v"]);
    assert_eq!(assembly.masked_args, ["-v"]);
    assert!(assembly.inject_values.is_empty());
    assert!(assembly.command_values.is_empty());
    assert!(assembly.display.is_empty());
}

#[test]
fn test_masked_args_still_glob_expand_multiple_fields() {
    let root = TempDir::new().unwrap();
    std::fs::write(root.path().join("a.png"), b"").unwrap();
    let glob = FileGlobExpander::new(root.path());

    let mut inputs = ParamDecl::new("inputs");
    inputs.delivery = ParameterDelivery::Flag;
    inputs.multiple = true;
    inputs.flag.clear();
    let mut key = ParamDecl::new("api_key");
    key.delivery = ParameterDelivery::Flag;
    key.flag = "--api-key".to_owned();
    key.secret = true;

    let assembly = assemble_run_inputs(
        &[inputs, key],
        &BTreeMap::from([
            ("inputs".to_owned(), "*.png".to_owned()),
            ("api_key".to_owned(), "sk-1".to_owned()),
        ]),
        &[],
        true,
        &context(&root),
        &glob,
    )
    .unwrap();

    assert_eq!(assembly.args, ["a.png", "--api-key", "sk-1"]);
    assert_eq!(assembly.masked_args, ["a.png", "--api-key", "•••"]);
}
