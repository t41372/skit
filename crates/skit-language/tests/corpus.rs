use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use skit_domain::parameters::{ParamDecl, ParameterDelivery, ParameterType};
use skit_language::{
    detect_candidates, inject_values, managed_params, source_is_valid, write_managed_params,
};

fn corpus_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus")
}

fn files(directory: &Path, extension: &str) -> Vec<PathBuf> {
    let mut paths = fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", directory.display()))
        .map(|entry| entry.expect("corpus directory entry").path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some(extension))
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

fn source(path: &Path) -> String {
    String::from_utf8(
        fs::read(path).unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display())),
    )
    .unwrap_or_else(|error| panic!("{} is not UTF-8: {error}", path.display()))
}

fn sample_values(declarations: &[ParamDecl]) -> BTreeMap<String, String> {
    declarations
        .iter()
        .filter(|declaration| declaration.delivery == ParameterDelivery::Inject)
        .map(|declaration| {
            let value = match declaration.parameter_type {
                ParameterType::Int => "7",
                ParameterType::Float => "1.5",
                ParameterType::Bool => "true",
                ParameterType::Str | ParameterType::Choice | ParameterType::Path => "sample",
            };
            (declaration.name.clone(), value.to_owned())
        })
        .collect()
}

fn assert_comment_block_fidelity(kind: &str, path: &Path, comment: &str) {
    let original = source(path);
    let declarations = detect_candidates(kind, &original);
    let written = write_managed_params(kind, &original, &declarations)
        .unwrap_or_else(|error| panic!("{} metadata write failed: {error}", path.display()));
    assert_eq!(
        managed_params(kind, &written),
        declarations,
        "{}",
        path.display()
    );

    let original_lines = original.split_inclusive(['\n', '\r']).collect::<Vec<_>>();
    for line in written.split_inclusive(['\n', '\r']) {
        if !original_lines.contains(&line) && !line.trim().is_empty() {
            assert!(
                line.trim_start().starts_with(comment),
                "{} added a non-comment line: {line:?}",
                path.display()
            );
        }
    }

    let original_lines = original.lines().collect::<Vec<_>>();
    if original_lines
        .first()
        .is_some_and(|line| line.starts_with("#!"))
    {
        assert_eq!(written.lines().next(), original_lines.first().copied());
    }
    for line in original_lines {
        if !line.trim_start().starts_with(comment) {
            assert!(
                written.contains(line),
                "{} lost source line {line:?}",
                path.display()
            );
        }
    }
}

fn assert_rewrite_contract(kind: &str, path: &Path) {
    let original = source(path);
    let declarations = detect_candidates(kind, &original);
    if !source_is_valid(kind, &original) {
        assert!(
            declarations.is_empty(),
            "{} exposed partial results for invalid source",
            path.display()
        );
        return;
    }
    assert_eq!(
        inject_values(kind, &original, &declarations, &BTreeMap::new())
            .unwrap_or_else(|error| panic!("{} empty rewrite failed: {error}", path.display())),
        original,
        "{}",
        path.display()
    );

    let values = sample_values(&declarations);
    let rewritten = inject_values(kind, &original, &declarations, &values)
        .unwrap_or_else(|error| panic!("{} full rewrite failed: {error}", path.display()));
    assert!(source_is_valid(kind, &rewritten), "{}", path.display());

    let comment = if matches!(kind, "js" | "ts") {
        "//"
    } else {
        "#"
    };
    for line in original.lines() {
        if line.trim_start().starts_with(comment) {
            assert!(
                rewritten.contains(line),
                "{} lost comment line {line:?}",
                path.display()
            );
        }
    }
}

#[test]
fn python_and_shell_corpus_preserves_metadata_and_source_bytes() {
    let root = corpus_root();
    for path in files(&root, "py") {
        assert_comment_block_fidelity("python", &path, "#");
    }
    for path in files(&root.join("shell"), "sh") {
        assert_comment_block_fidelity("shell", &path, "#");
    }
}

#[test]
fn javascript_and_typescript_corpus_preserves_metadata_and_source_bytes() {
    let root = corpus_root();
    for path in files(&root.join("js"), "mjs") {
        assert_comment_block_fidelity("js", &path, "//");
    }
    for path in files(&root.join("ts"), "ts") {
        assert_comment_block_fidelity("ts", &path, "//");
    }
}

#[test]
fn every_rewritten_corpus_file_remains_valid_source() {
    let root = corpus_root();
    for (kind, directory, extension) in [
        ("python", root.clone(), "py"),
        ("shell", root.join("shell"), "sh"),
        ("js", root.join("js"), "mjs"),
        ("ts", root.join("ts"), "ts"),
    ] {
        for path in files(&directory, extension) {
            assert_rewrite_contract(kind, &path);
        }
    }
}
