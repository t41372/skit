//! Rust-additive strengthening for the explicit `bash -n` assertion carried by the frozen payload
//! oracle. The exact `test_const_payload_is_inert` also executes every rewritten payload for real;
//! this separate check keeps the independent syntax gate instead of weakening it into runtime-only
//! evidence.
#![cfg(unix)]

use std::{collections::BTreeMap, fs, process::Command};

use skit_language::{ParseOutcome, inject_values, parse_document};
use tempfile::TempDir;

#[test]
fn rust_additive_const_payload_passes_bash_n_for_every_frozen_payload() {
    if !Command::new("bash")
        .args(["-c", "exit 0"])
        .status()
        .is_ok_and(|status| status.success())
    {
        return;
    }

    let source = "#!/usr/bin/env bash\nTITLE=hello\necho \"[$TITLE]\"\n";
    let ParseOutcome::Parsed(document) = parse_document("shell", source) else {
        panic!("fixture must parse");
    };
    let declarations = document
        .analysis()
        .candidates
        .into_iter()
        .map(|candidate| candidate.declaration)
        .collect::<Vec<_>>();

    for payload in [
        "'; touch pwned; echo '",
        "$(touch pwned)",
        "`touch pwned`",
        "$(id) && touch pwned",
    ] {
        let rewritten = inject_values(
            "shell",
            source,
            &declarations,
            &BTreeMap::from([("TITLE".to_owned(), payload.to_owned())]),
        )
        .unwrap();
        let root = TempDir::new().unwrap();
        let path = root.path().join("payload.sh");
        fs::write(&path, rewritten).unwrap();
        let output = Command::new("bash").args(["-n"]).arg(&path).output().unwrap();
        assert_eq!(
            output.status.code(),
            Some(0),
            "payload={payload:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            !root.path().join("pwned").exists(),
            "syntax validation itself must never execute the payload: {payload:?}"
        );
    }
}
