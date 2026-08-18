//! Frozen silent-undercount defense for `generate_runover`.
//!
//! The Rust function hard-codes `FileStore`, so tests cannot inject the lying repository used by the
//! Python oracle without changing production architecture. Keep the public normal-path assertion,
//! then require a post-create validation gate in the function body. This is deliberately red until
//! the implementation restores the missing defense; it is not an architecture closure.

use std::{fs, path::Path};

use skit_application::EntryRepository as _;
use skit_benchmarks::dataset::{dataset_dirs, generate_runover};
use skit_store::FileStore;
use tempfile::TempDir;

fn function_body<'a>(source: &'a str, signature: &str) -> &'a str {
    let start = source.find(signature).expect("function signature remains present");
    let open = source[start..].find('{').map(|offset| start + offset).unwrap();
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
    panic!("unterminated generate_runover body")
}

#[test]
fn test_runover_refuses_silent_store_undercount() {
    let root = TempDir::new().unwrap();
    let manifest = generate_runover(root.path().join("runover")).unwrap();
    let dirs = dataset_dirs(&manifest.root).unwrap();
    assert_eq!(manifest.n, 3);
    assert_eq!(FileStore::new(dirs.data).scan().unwrap().entries.len(), 3);

    let source = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/dataset.rs"),
    )
    .unwrap();
    let body = function_body(&source, "pub fn generate_runover");
    let last_create = body
        .rfind("create_from_path")
        .expect("runover still creates entries through the store service");
    let post_create = &body[last_create..];
    assert!(
        post_create.contains("validate_generated_count")
            || (post_create.contains("scan()") && post_create.contains("entries.len()")),
        "generate_runover can return a three-entry manifest without verifying the real store count after creation; frozen Python test rejected this silent undercount"
    );
}
