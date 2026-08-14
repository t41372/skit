//! Exact cross-pipeline flows contracts that require the real filesystem glob adapter.

use std::collections::BTreeMap;

use skit_application::{run_inputs::assemble_run_inputs, tokens::TokenContext};
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
