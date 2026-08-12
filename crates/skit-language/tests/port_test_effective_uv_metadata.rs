//! Exact branch ports of Python `store.effective_uv_metadata` tests in
//! `tests/test_effective_uv_metadata.py` at `main@206f9ef`.
//!
//! The Rust language layer receives source bytes only when the caller is allowed and able to read
//! the authoritative Python-copy block. `None` therefore represents the same boundary used for a
//! reference entry, a non-Python entry, or a missing stored copy; stored metadata remains the only
//! truth in each of those cases.

use skit_language::{UvMetadata, effective_uv_metadata_bytes};

fn metadata(dependencies: &[&str], requires_python: &str) -> UvMetadata {
    UvMetadata {
        dependencies: dependencies.iter().map(|value| (*value).to_owned()).collect(),
        requires_python: requires_python.to_owned(),
    }
}

#[test]
fn test_effective_meta_carried_skips_the_block() {
    let source = br#"# /// script
# dependencies = ["wrong-block-dep"]
# requires-python = ">=9"
# ///
print(1)
"#;
    let stored = metadata(&["requests"], ">=3.11");
    assert_eq!(
        effective_uv_metadata_bytes(Some(source), &stored),
        metadata(&["requests"], ">=3.11")
    );
}

#[test]
fn test_effective_block_only_reads_both_axes_from_the_block() {
    let source = br#"# /// script
# dependencies = ["requests"]
# requires-python = ">=3.11"
# ///
print(1)
"#;
    assert_eq!(
        effective_uv_metadata_bytes(Some(source), &UvMetadata::default()),
        metadata(&["requests"], ">=3.11")
    );
}

#[test]
fn test_effective_meta_deps_blank_constraint_reads_constraint_from_block() {
    let source = br#"# /// script
# requires-python = ">=3.9"
# ///
print(1)
"#;
    let stored = metadata(&["requests"], "");
    assert_eq!(
        effective_uv_metadata_bytes(Some(source), &stored),
        metadata(&["requests"], ">=3.9")
    );
}

#[test]
fn test_effective_meta_constraint_blank_deps_reads_deps_from_block() {
    let source = br#"# /// script
# dependencies = ["rich"]
# ///
print(1)
"#;
    let stored = metadata(&[], ">=3.10");
    assert_eq!(
        effective_uv_metadata_bytes(Some(source), &stored),
        metadata(&["rich"], ">=3.10")
    );
}

#[test]
fn test_effective_both_blank_returns_empty() {
    assert_eq!(
        effective_uv_metadata_bytes(Some(b"print(1)\n"), &UvMetadata::default()),
        UvMetadata::default()
    );
}

#[test]
fn test_effective_reference_mode_python_reads_meta_only() {
    // Reference-mode callers deliberately do not pass original-source PEP 723 bytes into this
    // helper. Even if that original contains a block, the stored record is the authority.
    let stored = metadata(&[], "");
    assert_eq!(effective_uv_metadata_bytes(None, &stored), stored);
}

#[test]
fn test_effective_js_entry_reads_meta_only() {
    // Non-Python callers likewise have no PEP 723 source channel here.
    let stored = metadata(&["chalk"], "");
    assert_eq!(effective_uv_metadata_bytes(None, &stored), stored);
}

#[test]
fn test_effective_missing_stored_copy_reads_meta_only() {
    // A missing stored copy is represented by absence of source bytes, not by inventing an empty
    // source that could accidentally change precedence.
    let stored = UvMetadata::default();
    assert_eq!(effective_uv_metadata_bytes(None, &stored), stored);
}
