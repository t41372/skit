//! Public-surface mutation-hardening ports from Python v0.4 `tests/test_shell_inject_mut.py`.
//!
//! Private gate/tempfile helper contracts are intentionally not recreated. Cases that make a
//! runtime claim execute the rewritten source under a real POSIX shell. A behavioral mismatch is a
//! parity finding and stays red on this test-only branch.

use std::{collections::BTreeMap, fs, process::Command};

use skit_domain::parameters::{
    ParamDecl, ParameterBinding, ParameterDelivery, ParameterType,
};
use skit_language::{
    LanguageError, ParseOutcome, ShellInputError, inject_values, inject_values_for_interpreter,
    parse_document,
};
use tempfile::TempDir;

fn input_declarations(source: &str) -> Vec<ParamDecl> {
    let ParseOutcome::Parsed(document) = parse_document("shell", source) else {
        panic!("expected shell source to parse");
    };
    document
        .analysis()
        .candidates
        .into_iter()
        .filter(|candidate| candidate.declaration.binding == ParameterBinding::Input)
        .map(|candidate| candidate.declaration)
        .collect()
}

fn values(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn const_decl(name: &str, parameter_type: ParameterType) -> ParamDecl {
    let mut declaration = ParamDecl::new(name);
    declaration.binding = ParameterBinding::Const;
    declaration.delivery = ParameterDelivery::Inject;
    declaration.parameter_type = parameter_type;
    declaration
}

#[cfg(unix)]
fn run_shell(program: &str, source: &str) -> std::process::Output {
    let root = TempDir::new().unwrap();
    let path = root.path().join("injected.sh");
    fs::write(&path, source).unwrap();
    Command::new(program)
        .arg(&path)
        .current_dir(root.path())
        .output()
        .unwrap()
}

#[test]
fn test_gap_after_the_first_filled_variable_is_still_refused() {
    let source = "#!/usr/bin/env bash\nread -p \"p: \" A B C\n";
    let declarations = input_declarations(source);
    assert_eq!(
        declarations.iter().map(|decl| decl.name.as_str()).collect::<Vec<_>>(),
        ["input-1", "input-2", "input-3"]
    );

    let error = inject_values(
        "shell",
        source,
        &declarations,
        &values(&[("input-1", "x"), ("input-3", "z")]),
    )
    .unwrap_err();

    assert_eq!(
        error,
        LanguageError::ShellInput(ShellInputError::Gap {
            empty: "input-2".to_owned(),
            filled: "input-3".to_owned(),
        })
    );
}

#[test]
fn test_two_empty_values_are_a_short_line_not_a_gap() {
    let source = "#!/usr/bin/env bash\nread -p \"p: \" A B\nprintf '%s|%s\\n' \"$A\" \"$B\"\n";
    let declarations = input_declarations(source);

    let rewritten = inject_values(
        "shell",
        source,
        &declarations,
        &values(&[("input-1", ""), ("input-2", "")]),
    )
    .unwrap();

    assert!(matches!(parse_document("shell", &rewritten), ParseOutcome::Parsed(_)));
}

#[cfg(unix)]
#[test]
fn test_command_read_spelling_is_rewritten_whole() {
    let source = "#!/bin/sh\ncommand read -p \"Name: \" who\necho \"hi $who\"\n";
    let declarations = input_declarations(source);
    let rewritten = inject_values_for_interpreter(
        "shell",
        source,
        &declarations,
        &values(&[("input-1", "Ada")]),
        Some("sh"),
    )
    .unwrap();

    assert!(!rewritten.contains("command read -p \"Name: \" who"), "{rewritten}");
    assert!(rewritten.contains("_skit_read 0 "), "{rewritten}");

    let output = run_shell("sh", &rewritten);
    assert_eq!(output.status.code(), Some(0), "stderr={}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "Name: Ada\nhi Ada\n");
}

#[test]
fn test_const_literal_bad_int_error_carries_every_field() {
    let declaration = const_decl("WIDTH", ParameterType::Int);
    let error = inject_values(
        "shell",
        "WIDTH=800\n",
        &[declaration],
        &values(&[("WIDTH", "nope")]),
    )
    .unwrap_err();

    assert_eq!(
        error,
        LanguageError::InvalidValue {
            name: "WIDTH".to_owned(),
            value: "nope".to_owned(),
            parameter_type: ParameterType::Int,
        }
    );
}

#[test]
fn test_const_literal_bad_float_error_carries_every_field() {
    let declaration = const_decl("RATE", ParameterType::Float);
    let error = inject_values(
        "shell",
        "RATE=1.5\n",
        &[declaration],
        &values(&[("RATE", "nope")]),
    )
    .unwrap_err();

    assert_eq!(
        error,
        LanguageError::InvalidValue {
            name: "RATE".to_owned(),
            value: "nope".to_owned(),
            parameter_type: ParameterType::Float,
        }
    );
}

#[test]
fn test_const_literal_non_finite_error_carries_every_field() {
    let declaration = const_decl("RATE", ParameterType::Float);
    let error = inject_values(
        "shell",
        "RATE=1.5\n",
        &[declaration],
        &values(&[("RATE", "inf")]),
    )
    .unwrap_err();

    assert_eq!(
        error,
        LanguageError::InvalidValue {
            name: "RATE".to_owned(),
            value: "inf".to_owned(),
            parameter_type: ParameterType::Float,
        }
    );
}

#[test]
fn test_a_subscript_named_target_is_drift_never_rewritten() {
    let declaration = const_decl("ARR[0]", ParameterType::Int);
    let error = inject_values(
        "shell",
        "#!/usr/bin/env bash\nARR[0]=1\n",
        &[declaration],
        &values(&[("ARR[0]", "5")]),
    )
    .unwrap_err();

    assert_eq!(
        error,
        LanguageError::BindingNotFound {
            name: "ARR[0]".to_owned(),
        }
    );
}

#[cfg(unix)]
#[test]
fn test_preamble_is_a_pure_insertion_not_a_duplicating_splice() {
    let source = "#!/usr/bin/env bash\nread -p \"Name: \" who\necho \"hi $who\"\n";
    let declarations = input_declarations(source);
    let rewritten = inject_values(
        "shell",
        source,
        &declarations,
        &values(&[("input-1", "Ada")]),
    )
    .unwrap();

    assert_eq!(rewritten.matches("#!/usr/bin/env bash").count(), 1, "{rewritten}");
    assert_eq!(rewritten.matches("echo \"hi $who\"").count(), 1, "{rewritten}");
    let output = run_shell("bash", &rewritten);
    assert_eq!(output.status.code(), Some(0), "stderr={}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "Name: Ada\nhi Ada\n");
}

#[test]
fn test_preamble_carries_the_shim_marker_comment() {
    let source = "#!/usr/bin/env bash\nread -p \"Name: \" who\n";
    let declarations = input_declarations(source);
    let rewritten = inject_values(
        "shell",
        source,
        &declarations,
        &values(&[("input-1", "Ada")]),
    )
    .unwrap();

    assert!(rewritten.contains("# skit:shim"), "{rewritten}");
}

#[cfg(unix)]
#[test]
fn test_analyzer_detected_secret_is_masked_even_without_a_secret_spec() {
    let source = "#!/usr/bin/env bash\nread -s -p \"Password: \" PW\necho \"len=${#PW}\"\n";
    let mut declaration = ParamDecl::new("input-1");
    declaration.binding = ParameterBinding::Input;
    declaration.delivery = ParameterDelivery::Inject;
    declaration.order = 0;
    declaration.prompt = "Password: ".to_owned();
    declaration.secret = false;

    let rewritten = inject_values(
        "shell",
        source,
        &[declaration],
        &values(&[("input-1", "hunter2")]),
    )
    .unwrap();
    let output = run_shell("bash", &rewritten);
    assert_eq!(output.status.code(), Some(0), "stderr={}", String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Password: ***"), "{stdout}\nsource:\n{rewritten}");
    assert!(!stdout.contains("hunter2"), "{stdout}");
    assert!(stdout.contains("len=7"), "{stdout}");
}

#[cfg(unix)]
#[test]
fn test_spec_marked_secret_masks_a_plain_read_via_its_order() {
    let source = "#!/usr/bin/env bash\nread -p \"K: \" K\necho \"got=$K\"\n";
    let mut declaration = ParamDecl::new("input-1");
    declaration.binding = ParameterBinding::Input;
    declaration.delivery = ParameterDelivery::Inject;
    declaration.order = 0;
    declaration.prompt = "K: ".to_owned();
    declaration.secret = true;

    let rewritten = inject_values(
        "shell",
        source,
        &[declaration],
        &values(&[("input-1", "topsecret")]),
    )
    .unwrap();
    let output = run_shell("bash", &rewritten);
    assert_eq!(output.status.code(), Some(0), "stderr={}", String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("K: ***"), "{stdout}\nsource:\n{rewritten}");
    assert!(!stdout.contains("K: topsecret"), "{stdout}");
    assert!(stdout.contains("got=topsecret"), "{stdout}");
}
