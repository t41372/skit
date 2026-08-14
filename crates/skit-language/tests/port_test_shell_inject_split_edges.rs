//! Exact late-regression ports from Python v0.4 `tests/test_shell_inject.py`.

use std::{collections::BTreeMap, fs, io::Write as _, process::{Command, Stdio}};

use skit_language::{LanguageError, ParseOutcome, ShellInputError, inject_values, parse_document};
use tempfile::TempDir;

fn declarations(source: &str) -> Vec<skit_domain::parameters::ParamDecl> {
    let ParseOutcome::Parsed(document) = parse_document("shell", source) else {
        panic!("expected valid shell source");
    };
    document.analysis().candidates.into_iter().map(|c| c.declaration).collect()
}

fn inject(source: &str, pairs: &[(&str, &str)]) -> Result<String, LanguageError> {
    let values = pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect::<BTreeMap<_, _>>();
    inject_values("shell", source, &declarations(source), &values)
}

#[cfg(unix)]
fn run_bash(source: &str) -> std::process::Output {
    let root = TempDir::new().unwrap();
    let path = root.path().join("injected.sh");
    fs::write(&path, source).unwrap();
    let mut child = Command::new("bash")
        .arg(&path)
        .current_dir(root.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    if let Some(mut stdin) = child.stdin.take() { stdin.write_all(b"").unwrap(); }
    child.wait_with_output().unwrap()
}

#[test]
fn test_split_guard_refuses_only_what_the_shell_would_actually_mangle() {
    let source = "#!/usr/bin/env bash\nread -p \"a b: \" FIRST LAST\n";

    for accepted in ["a\u{00a0}b", "a\rb"] {
        assert!(
            inject(source, &[("input-1", accepted), ("input-2", "x")]).is_ok(),
            "default IFS must not treat {accepted:?} as a splitter"
        );
    }

    for (bad, expected) in [
        (
            "a b",
            ShellInputError::FieldSplit { name: "input-1".to_owned() },
        ),
        (
            "a\tb",
            ShellInputError::FieldSplit { name: "input-1".to_owned() },
        ),
        (
            "a\nb",
            ShellInputError::LineBreak { name: "input-1".to_owned() },
        ),
    ] {
        assert_eq!(
            inject(source, &[("input-1", bad), ("input-2", "x")]).unwrap_err(),
            LanguageError::ShellInput(expected),
            "bad value {bad:?} must be refused for the same reason as the shell oracle"
        );
    }
}

#[test]
fn test_empty_value_in_a_non_last_read_variable_is_a_gap() {
    let source = "#!/usr/bin/env bash\nread -p \"p: \" A B\n";
    assert_eq!(
        inject(source, &[("input-1", ""), ("input-2", "b")]).unwrap_err(),
        LanguageError::ShellInput(ShellInputError::Gap {
            empty: "input-1".to_owned(),
            filled: "input-2".to_owned(),
        })
    );
}

#[cfg(unix)]
#[test]
fn test_empty_value_in_the_last_read_variable_is_fine() {
    if !Command::new("bash").args(["-c", "exit 0"]).status().is_ok_and(|s| s.success()) {
        return;
    }
    let source = concat!(
        "#!/usr/bin/env bash\n",
        "read -p \"p: \" A B\n",
        "printf \"[%s][%s]\" \"$A\" \"$B\"\n",
    );
    let rewritten = inject(source, &[("input-1", "a"), ("input-2", "")]).unwrap();
    let output = run_bash(&rewritten);
    assert_eq!(output.status.code(), Some(0), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "p: a\n[a][]");
}
