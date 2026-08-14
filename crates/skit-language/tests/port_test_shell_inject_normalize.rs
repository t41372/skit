//! Exact public-surface ports from Python v0.4 `tests/test_shell_inject.py` normalization contracts.

use std::{fs, process::Command};

use skit_domain::parameters::{ParameterBinding, ParameterDelivery, ParameterValue};
use skit_language::{ParseOutcome, normalize_shell_default, parse_document};
use tempfile::TempDir;

#[test]
fn test_normalize_rewrites_only_that_assignments_bytes() {
    let source = "#!/usr/bin/env bash\nWIDTH=800\nHEIGHT=600\necho \"$WIDTH $HEIGHT\"\n";
    let rewritten = normalize_shell_default(source, "WIDTH").unwrap();
    assert_eq!(
        rewritten,
        "#!/usr/bin/env bash\nWIDTH=\"${WIDTH:-800}\"\nHEIGHT=600\necho \"$WIDTH $HEIGHT\"\n"
    );
}

#[test]
fn test_normalize_makes_the_param_an_envdefault() {
    let source = "#!/usr/bin/env bash\nWIDTH=800\n";
    let rewritten = normalize_shell_default(source, "WIDTH").unwrap();
    let ParseOutcome::Parsed(document) = parse_document("shell", &rewritten) else {
        panic!("normalized shell must remain parseable");
    };
    let width = document
        .analysis()
        .candidates
        .into_iter()
        .find(|candidate| candidate.declaration.name == "WIDTH")
        .expect("normalized WIDTH must remain a candidate")
        .declaration;
    assert_eq!(width.binding, ParameterBinding::EnvDefault);
    assert_eq!(width.delivery, ParameterDelivery::Env);
    assert_eq!(width.env_var(), "WIDTH");
    assert_eq!(width.default, Some(ParameterValue::Integer(800)));
}

#[cfg(unix)]
#[test]
fn test_normalized_script_still_runs_standalone() {
    if !Command::new("bash").args(["-c", "exit 0"]).status().is_ok_and(|s| s.success()) {
        return;
    }
    let root = TempDir::new().unwrap();
    let path = root.path().join("s.sh");
    let rewritten = normalize_shell_default(
        "#!/usr/bin/env bash\nGREETING=hello\necho \"$GREETING\"\n",
        "GREETING",
    )
    .unwrap();
    fs::write(&path, rewritten).unwrap();

    let standalone = Command::new("bash")
        .arg(&path)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output()
        .unwrap();
    assert_eq!(standalone.status.code(), Some(0));
    assert_eq!(String::from_utf8(standalone.stdout).unwrap(), "hello\n");

    let inherited = Command::new("bash")
        .arg(&path)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("GREETING", "hi")
        .output()
        .unwrap();
    assert_eq!(inherited.status.code(), Some(0));
    assert_eq!(String::from_utf8(inherited.stdout).unwrap(), "hi\n");
}
