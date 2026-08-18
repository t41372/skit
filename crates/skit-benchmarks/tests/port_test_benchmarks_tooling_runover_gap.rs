//! Frozen silent-undercount defenses for benchmark dataset generation.
//!
//! The Rust generators hard-code `FileStore`, so an integration test cannot inject the lying
//! repository used by the Python oracle without changing production architecture. These executable
//! owners therefore combine a real public generation/store scan with a source-level post-mutation
//! validation gate. The runover owner is deliberately red until production restores the missing
//! defense and frozen diagnostic; it is not an architecture closure.

use std::{fs, path::Path};

use skit_application::EntryRepository as _;
use skit_benchmarks::dataset::{
    DEFAULT_SEED, DEFAULT_STATE_FRACTION, dataset_dirs, generate, generate_runover,
};
use skit_store::FileStore;
use tempfile::TempDir;

fn function_body<'a>(source: &'a str, signature: &str) -> &'a str {
    let start = source
        .find(signature)
        .unwrap_or_else(|| panic!("missing function signature {signature:?}"));
    let open = source[start..]
        .find('{')
        .map(|offset| start + offset)
        .unwrap();
    let mut depth = 0_usize;
    for (offset, byte) in source.as_bytes()[open..].iter().enumerate() {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return &source[open + 1..open + offset];
                }
            }
            _ => {}
        }
    }
    panic!("unterminated function body for {signature:?}")
}

fn dataset_source() -> String {
    fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/dataset.rs")).unwrap()
}

fn body_after_last<'a>(body: &'a str, marker: &str) -> &'a str {
    let start = body
        .rfind(marker)
        .unwrap_or_else(|| panic!("dataset generator no longer contains {marker:?}"));
    &body[start..]
}

fn source_has_store_count(scope: &str) -> bool {
    scope.contains(".list()") && scope.contains(".entries") && scope.contains(".len()")
}

#[test]
fn test_generate_refuses_silent_store_undercount() {
    let root = TempDir::new().unwrap();
    let manifest = generate(
        &root.path().join("dataset"),
        3,
        DEFAULT_SEED,
        DEFAULT_STATE_FRACTION,
    )
    .unwrap();
    let dirs = dataset_dirs(&manifest.root).unwrap();
    assert_eq!(manifest.n, 3);
    assert_eq!(FileStore::new(dirs.data).scan().unwrap().entries.len(), 3);

    let source = dataset_source();
    let body = function_body(&source, "pub fn generate(");
    let post_mutation = body_after_last(body, ".record_run(");
    assert!(
        source_has_store_count(post_mutation),
        "generate must count the real persisted store after its final write"
    );
    assert!(
        post_mutation.contains("validate_generated_count(found, n)"),
        "generate must reject a post-write count that differs from the requested cardinality"
    );

    let validator = function_body(&source, "fn validate_generated_count");
    assert!(validator.contains("if found == expected"));
    assert!(
        validator.contains("\"generated {found} entries, expected {expected}\""),
        "the frozen loud undercount diagnostic changed"
    );
}

#[test]
fn test_runover_refuses_silent_store_undercount() {
    let root = TempDir::new().unwrap();
    let manifest = generate_runover(root.path().join("runover")).unwrap();
    let dirs = dataset_dirs(&manifest.root).unwrap();
    assert_eq!(manifest.n, 3);
    assert_eq!(FileStore::new(dirs.data).scan().unwrap().entries.len(), 3);

    let source = dataset_source();
    let body = function_body(&source, "pub fn generate_runover");
    let post_mutation = body_after_last(body, ".commit_copy_edit(");

    let helper = source
        .contains("fn validate_runover_count")
        .then(|| function_body(&source, "fn validate_runover_count"));
    let helper_is_called = post_mutation.contains("validate_runover_count");
    let inline_validation = source_has_store_count(post_mutation)
        && (post_mutation.contains("validate_generated_count")
            || post_mutation.contains("found != 3")
            || post_mutation.contains("found == 3"));
    let helper_validation = helper_is_called
        && helper.is_some_and(|body| {
            source_has_store_count(body)
                || (body.contains("found") && body.contains("expected"))
        });

    assert!(
        inline_validation || helper_validation,
        "generate_runover can return a three-entry manifest without counting and validating the real store after its final commit"
    );
    assert!(
        source.contains("\"runover library has {found} entries, expected {expected}\"")
            || source.contains("\"runover library has {found} entries, expected 3\""),
        "the runover undercount refusal must preserve the frozen diagnostic"
    );
}
