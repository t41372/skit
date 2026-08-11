//! Mechanical guardrails for the Python-to-Rust parity suite.
//!
//! This does not substitute for behavioral tests. It only prevents a mapped Python contract from
//! being represented by an ignored or unfinished Rust placeholder again.

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
            ["#[", "ignore"].concat(),
            ["todo", "!()"].concat(),
            ["unimplemented", "!()"].concat(),
        ];
        for forbidden in &forbidden {
            if text.contains(forbidden) {
                offenders.push(format!("{} contains {forbidden}", path.display()));
            }
        }
    }
}

#[test]
fn python_parity_tests_cannot_be_ignored_or_left_as_unimplemented_stubs() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("skit-cli lives under <repo>/crates/skit-cli");
    let mut offenders = Vec::new();
    visit(&root.join("crates"), &mut offenders);
    assert!(
        offenders.is_empty(),
        "Python parity coverage must stay executable:\n{}",
        offenders.join("\n")
    );
}
