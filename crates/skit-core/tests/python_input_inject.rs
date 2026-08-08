use std::collections::BTreeMap;

use skit_core::{
    Binding, Delivery, ParamDecl, PythonInjectError, analyze_python_managed, inject_python_managed,
};

fn input_spec(name: &str, order: i64, prompt: &str, secret: bool) -> ParamDecl {
    ParamDecl {
        name: name.to_owned(),
        binding: Binding::Input,
        delivery: Delivery::Inject,
        prompt: prompt.to_owned(),
        order,
        secret,
        ..ParamDecl::default()
    }
}

#[test]
fn prompt_match_survives_inserted_earlier_input_and_preamble_follows_future_import()
-> Result<(), Box<dyn std::error::Error>> {
    let source = concat!(
        "\"\"\"module docs\"\"\"\n",
        "from __future__ import annotations\n",
        "new = input('New: ')\n",
        "password = input('Password: ')\n",
    );
    let output = inject_python_managed(
        source,
        &[input_spec("input-1", 0, "Password: ", true)],
        &BTreeMap::from([("input-1".to_owned(), "s3cret".to_owned())]),
    )?;

    let docs = output.find("\"\"\"module docs\"\"\"").ok_or("docs missing")?;
    let future = output.find("from __future__ import annotations").ok_or("future missing")?;
    let preamble = output.find("import sys as _skit_s").ok_or("preamble missing")?;
    let first_call = output.find("new = input('New: ')").ok_or("new input changed")?;
    let managed_call = output
        .find("password = _skit_i[1]('Password: ')")
        .ok_or("managed input not rebound by prompt")?;
    assert!(docs < future && future < preamble && preamble < first_call && first_call < managed_call);
    assert!(output.contains("1: (\"s3cret\", True)"));
    assert!(output.contains("'***' if _skit_q[k][1]"));
    assert!(!analyze_python_managed(&output).syntax_error);
    Ok(())
}

#[test]
fn renamed_prompt_position_fallback_is_refused_as_ambiguous() {
    let result = inject_python_managed(
        "name = input('New name: ')\n",
        &[input_spec("input-1", 0, "Old name: ", false)],
        &BTreeMap::from([("input-1".to_owned(), "Ada".to_owned())]),
    );
    assert_eq!(
        result,
        Err(PythonInjectError::AmbiguousInput("input-1".to_owned()))
    );
}

#[test]
fn duplicate_prompt_multiset_rewrites_each_call_once() -> Result<(), Box<dyn std::error::Error>> {
    let source = "first = input('Go? ')\nsecond = input('Go? ')\n";
    let specs = vec![
        input_spec("input-1", 0, "Go? ", false),
        input_spec("input-2", 1, "Go? ", false),
    ];
    let values = BTreeMap::from([
        ("input-1".to_owned(), "yes".to_owned()),
        ("input-2".to_owned(), "no".to_owned()),
    ]);
    let output = inject_python_managed(source, &specs, &values)?;
    assert!(output.contains("first = _skit_i[0]('Go? ')"));
    assert!(output.contains("second = _skit_i[1]('Go? ')"));
    assert!(output.contains("0: (\"yes\", False)"));
    assert!(output.contains("1: (\"no\", False)"));
    assert_eq!(output.matches("_skit_i[0]('Go? ')").count(), 1);
    assert_eq!(output.matches("_skit_i[1]('Go? ')").count(), 1);
    Ok(())
}

#[test]
fn vanished_prompt_and_position_is_a_missing_target() {
    let result = inject_python_managed(
        "print('no input now')\n",
        &[input_spec("input-2", 1, "Gone: ", false)],
        &BTreeMap::from([("input-2".to_owned(), "x".to_owned())]),
    );
    assert_eq!(
        result,
        Err(PythonInjectError::MissingTarget("input-2".to_owned()))
    );
}
