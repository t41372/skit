use std::collections::BTreeMap;

use skit_language::{ParseOutcome, inject_values_for_interpreter, parse_document};

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
