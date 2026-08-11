//! Mechanical guardrails for the Python-to-Rust parity suite.
//!
//! This does not substitute for behavioral review. It only prevents mapped Python contracts from
//! being represented by ignored/unfinished placeholders or by a few mechanically obvious vacuous
//! patterns. Assertion strength still has to be checked against the frozen Python oracle.

use std::{fs, path::Path};

fn visit(directory: &Path, offenders: &mut Vec<String>) {
    for entry in fs::read_dir(directory).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.is_dir() {
            visit(&path, offenders);
            continue;
        }
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !name.starts_with("port_test_")
            || path.extension().and_then(|ext| ext.to_str()) != Some("rs")
        {
            continue;
        }
        let text = fs::read_to_string(&path).unwrap();
        let forbidden = [
            (["#[", "ignore"].concat(), "ignored test"),
            (
                ["#[", "should_", "panic"].concat(),
                "unscoped panic-as-success test",
            ),
            (["todo", "!()"].concat(), "todo placeholder"),
            (
                ["unimplemented", "!()"].concat(),
                "unimplemented placeholder",
            ),
            (["assert!", "(true)"].concat(), "vacuous true assertion"),
            (
                ["assert_eq!", "(true, true)"].concat(),
                "vacuous constant equality",
            ),
            (
                ["assert_eq!", "(false, false)"].concat(),
                "vacuous constant equality",
            ),
            (
                ["assert_ne!", "(true, false)"].concat(),
                "vacuous constant inequality",
            ),
        ];
        for (needle, reason) in &forbidden {
            if text.contains(needle) {
                offenders.push(format!("{} contains {reason}: {needle}", path.display()));
            }
        }
    }
}

#[test]
fn python_parity_tests_cannot_be_ignored_unfinished_or_obviously_vacuous() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("skit-cli lives under <repo>/crates/skit-cli");
    let mut offenders = Vec::new();
    visit(&root.join("crates"), &mut offenders);
    assert!(
        offenders.is_empty(),
        "Python parity coverage must stay executable and non-vacuous:\n{}",
        offenders.join("\n")
    );
}
