use std::collections::BTreeMap;

use skit_language::{ParseOutcome, inject_values_for_interpreter, parse_document};

fn read_fixture() -> (&'static str, Vec<skit_domain::parameters::ParamDecl>, BTreeMap<String, String>) {
    let source = "#!/usr/bin/env bash\nread -p \"Name: \" NAME\necho \"$NAME\"\n";
    let ParseOutcome::Parsed(document) = parse_document("shell", source) else {
        panic!("fixture must parse");
    };
    let declarations = document
        .analysis()
        .candidates
        .into_iter()
        .map(|candidate| candidate.declaration)
        .collect::<Vec<_>>();
    let values = BTreeMap::from([("input-1".to_owned(), "Ada".to_owned())]);
    (source, declarations, values)
}

#[test]
fn test_fallthrough_keyword_is_dialect_selected() {
    let (source, declarations, values) = read_fixture();
    for (interpreter, expected, forbidden) in [
        ("bash", "builtin read", "command read"),
        ("zsh", "builtin read", "command read"),
        ("/bin/zsh", "builtin read", "command read"),
        ("bash.exe", "builtin read", "command read"),
        ("sh", "command read", "builtin read"),
        ("dash", "command read", "builtin read"),
        ("ksh", "command read", "builtin read"),
        ("", "command read", "builtin read"),
    ] {
        let rewritten = inject_values_for_interpreter(
            "shell",
            source,
            &declarations,
            &values,
            Some(interpreter),
        )
        .unwrap();
        assert!(
            rewritten.contains(expected),
            "interpreter={interpreter:?} must select {expected:?}:\n{rewritten}"
        );
        assert!(
            !rewritten.contains(forbidden),
            "interpreter={interpreter:?} selected the wrong fallthrough keyword:\n{rewritten}"
        );
    }
}

#[test]
fn test_interpreter_gate_is_skipped_when_the_shell_is_not_installed() {
    let source = "#!/usr/bin/env bash\nWIDTH=800\n";
    let ParseOutcome::Parsed(document) = parse_document("shell", source) else {
        panic!("fixture must parse");
    };
    let declarations = document
        .analysis()
        .candidates
        .into_iter()
        .map(|candidate| candidate.declaration)
        .collect::<Vec<_>>();
    let values = BTreeMap::from([("WIDTH".to_owned(), "1200".to_owned())]);

    let rewritten = inject_values_for_interpreter(
        "shell",
        source,
        &declarations,
        &values,
        Some("skit-no-such-shell"),
    )
    .unwrap();
    assert!(rewritten.contains("WIDTH=1200"), "{rewritten}");
}
