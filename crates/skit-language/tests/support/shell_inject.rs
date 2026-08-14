use std::collections::BTreeMap;

use skit_domain::parameters::ParamDecl;
use skit_language::{LanguageError, ParseOutcome, inject_values_for_interpreter, parse_document};

pub fn declarations(source: &str) -> Vec<ParamDecl> {
    let ParseOutcome::Parsed(document) = parse_document("shell", source) else {
        panic!("shell injection fixture must parse: {source:?}");
    };
    document
        .analysis()
        .candidates
        .into_iter()
        .map(|candidate| candidate.declaration)
        .collect()
}

pub fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub fn inject(source: &str, pairs: &[(&str, &str)]) -> Result<String, LanguageError> {
    let declarations = declarations(source);
    inject_with(source, &declarations, pairs, Some("bash"))
}

pub fn inject_with(
    source: &str,
    declarations: &[ParamDecl],
    pairs: &[(&str, &str)],
    interpreter: Option<&str>,
) -> Result<String, LanguageError> {
    inject_values_for_interpreter("shell", source, declarations, &map(pairs), interpreter)
}

#[cfg(unix)]
pub fn run_bash(source: &str, stdin: &str) -> std::process::Output {
    use std::{fs, io::Write as _, process::{Command, Stdio}};
    use tempfile::TempDir;

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
        .expect("Python oracle requires bash on POSIX");
    if !stdin.is_empty() {
        child.stdin.as_mut().unwrap().write_all(stdin.as_bytes()).unwrap();
    }
    child.wait_with_output().unwrap()
}

#[cfg(unix)]
pub fn assert_bash_stdout(source: &str, stdin: &str, expected: &str) {
    let output = run_bash(source, stdin);
    assert!(
        output.status.success(),
        "bash failed: {}\nsource:\n{source}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8(output.stdout).unwrap(), expected);
}
