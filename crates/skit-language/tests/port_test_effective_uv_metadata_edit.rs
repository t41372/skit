//! Exact `None` / empty / value edit-grammar ports from Python
//! `tests/test_effective_uv_metadata.py` at `main@206f9ef`.
//!
//! `plan_uv_metadata_edit` is the Rust chokepoint that preserves the same distinction: `None` means
//! untouched; `Some(Vec::new())` clears dependencies; `Some(String::new())` (or `-`) clears the
//! Python constraint. Assertions cover effective truth, stored truth, and rewritten source bytes.

use skit_language::{UvMetadata, plan_uv_metadata_edit, read_uv_metadata};

fn block(dependencies: &[&str], requires_python: &str) -> Vec<u8> {
    let mut lines = vec!["# /// script".to_owned()];
    if !dependencies.is_empty() {
        let deps = dependencies
            .iter()
            .map(|value| format!("\"{value}\""))
            .collect::<Vec<_>>()
            .join(", ");
        lines.push(format!("# dependencies = [{deps}]"));
    } else {
        lines.push("# dependencies = []".to_owned());
    }
    if !requires_python.is_empty() {
        lines.push(format!("# requires-python = \"{requires_python}\""));
    }
    lines.push("# ///".to_owned());
    lines.push("print(1)".to_owned());
    lines.push(String::new());
    lines.join("\n").into_bytes()
}

fn metadata(dependencies: &[&str], requires_python: &str) -> UvMetadata {
    UvMetadata {
        dependencies: dependencies
            .iter()
            .map(|value| (*value).to_owned())
            .collect(),
        requires_python: requires_python.to_owned(),
    }
}

#[test]
fn test_update_dependencies_none_none_is_a_full_no_op() {
    let source = block(&["requests"], ">=3.11");
    let plan = plan_uv_metadata_edit(Some(&source), &UvMetadata::default(), None, None).unwrap();

    assert_eq!(plan.effective, metadata(&["requests"], ">=3.11"));
    assert_eq!(plan.stored, UvMetadata::default());
    assert_eq!(
        plan.rewritten_source, None,
        "untouched axes must not rewrite source bytes"
    );
}

#[test]
fn test_update_dependencies_none_python_lands_pin_and_preserves_block_deps() {
    let source = block(&["requests"], "");
    let plan = plan_uv_metadata_edit(
        Some(&source),
        &UvMetadata::default(),
        None,
        Some(">=3.12".to_owned()),
    )
    .unwrap();

    assert_eq!(plan.effective, metadata(&["requests"], ">=3.12"));
    assert_eq!(plan.stored, metadata(&[], ">=3.12"));
    let rewritten = plan
        .rewritten_source
        .expect("the pin must be written into the block");
    let parsed = read_uv_metadata(std::str::from_utf8(&rewritten).unwrap()).unwrap();
    assert_eq!(parsed, metadata(&["requests"], ">=3.12"));
}

#[test]
fn test_update_dependencies_clear_deps_preserves_the_pin() {
    let source = block(&["requests"], ">=3.11");
    let plan = plan_uv_metadata_edit(
        Some(&source),
        &UvMetadata::default(),
        Some(Vec::new()),
        None,
    )
    .unwrap();

    assert_eq!(plan.effective, metadata(&[], ">=3.11"));
    assert_eq!(plan.stored, UvMetadata::default());
    let rewritten = plan
        .rewritten_source
        .expect("an explicit dependency clear must rewrite the authoritative block");
    let parsed = read_uv_metadata(std::str::from_utf8(&rewritten).unwrap()).unwrap();
    assert_eq!(parsed, metadata(&[], ">=3.11"));
}

#[test]
fn test_update_dependencies_python_only_edit_syncs_block_from_meta_deps() {
    let source = block(&[], "");
    let stored = metadata(&["requests"], "");
    let plan =
        plan_uv_metadata_edit(Some(&source), &stored, None, Some(">=3.13".to_owned())).unwrap();

    assert_eq!(plan.effective, metadata(&["requests"], ">=3.13"));
    assert_eq!(plan.stored, metadata(&["requests"], ">=3.13"));
    let rewritten = plan
        .rewritten_source
        .expect("stored deps plus a new pin must synchronize the source block");
    let parsed = read_uv_metadata(std::str::from_utf8(&rewritten).unwrap()).unwrap();
    assert_eq!(parsed, metadata(&["requests"], ">=3.13"));
}

#[test]
fn test_update_dependencies_missing_stored_copy_still_writes_meta() {
    let plan = plan_uv_metadata_edit(
        None,
        &UvMetadata::default(),
        Some(vec!["rich".to_owned()]),
        None,
    )
    .unwrap();

    assert_eq!(plan.effective, metadata(&["rich"], ""));
    assert_eq!(plan.stored, metadata(&["rich"], ""));
    assert_eq!(plan.rewritten_source, None);
}
