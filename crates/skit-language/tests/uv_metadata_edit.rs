use skit_language::{
    UvMetadata, UvMetadataEditError, effective_uv_metadata_bytes, plan_uv_metadata_edit,
};

#[test]
fn effective_metadata_fills_each_blank_stored_axis_from_the_source_block() {
    let source = br#"# /// script
# dependencies = ["block-dep"]
# requires-python = ">=3.10"
# ///
print(1)
"#;
    let stored = UvMetadata {
        dependencies: vec!["meta-dep".to_owned()],
        requires_python: String::new(),
    };

    assert_eq!(
        effective_uv_metadata_bytes(Some(source), &stored),
        UvMetadata {
            dependencies: vec!["meta-dep".to_owned()],
            requires_python: ">=3.10".to_owned(),
        }
    );
}

#[test]
fn editing_one_axis_preserves_the_other_block_only_axis() {
    let source = br#"# /// script
# dependencies = ["requests"]
# ///
print(1)
"#;

    let plan = plan_uv_metadata_edit(
        Some(source),
        &UvMetadata::default(),
        None,
        Some(">=3.12".to_owned()),
    )
    .unwrap();

    assert_eq!(plan.stored.dependencies, Vec::<String>::new());
    assert_eq!(plan.stored.requires_python, ">=3.12");
    assert_eq!(plan.effective.dependencies, ["requests"]);
    assert_eq!(plan.effective.requires_python, ">=3.12");
    let written = String::from_utf8(plan.rewritten_source.unwrap()).unwrap();
    assert!(written.contains("requests"));
    assert!(written.contains(">=3.12"));
}

#[test]
fn explicit_clear_is_different_from_an_untouched_axis() {
    let source = br#"# /// script
# dependencies = ["requests"]
# requires-python = ">=3.11"
# ///
print(1)
"#;

    let plan = plan_uv_metadata_edit(Some(source), &UvMetadata::default(), Some(Vec::new()), None)
        .unwrap();

    assert!(plan.effective.dependencies.is_empty());
    assert_eq!(plan.effective.requires_python, ">=3.11");
    let written = String::from_utf8(plan.rewritten_source.unwrap()).unwrap();
    assert!(!written.contains("requests"));
    assert!(written.contains(">=3.11"));
}

#[test]
fn an_invalid_utf8_copy_without_a_block_keeps_its_bytes_and_uses_metadata() {
    let source = b"# coding: latin-1\nTEXT = 'caf\xe9'\n";

    let plan = plan_uv_metadata_edit(
        Some(source),
        &UvMetadata::default(),
        Some(vec!["httpx".to_owned()]),
        None,
    )
    .unwrap();

    assert_eq!(plan.stored.dependencies, ["httpx"]);
    assert_eq!(plan.effective.dependencies, ["httpx"]);
    assert_eq!(plan.rewritten_source, None);
}

#[test]
fn an_invalid_utf8_authoritative_block_refuses_only_an_actual_axis_edit() {
    let source = b"# /// script\n# dependencies = [\"requests\"]\n# ///\nTEXT = 'caf\xe9'\n";

    let untouched =
        plan_uv_metadata_edit(Some(source), &UvMetadata::default(), None, None).unwrap();
    assert_eq!(untouched.effective.dependencies, ["requests"]);
    assert_eq!(untouched.rewritten_source, None);

    assert_eq!(
        plan_uv_metadata_edit(Some(source), &UvMetadata::default(), Some(Vec::new()), None,),
        Err(UvMetadataEditError::NonUtf8OwnBlock)
    );
}

#[test]
fn metadata_edits_preserve_crlf_and_unrelated_source_bytes() {
    let source = b"#!/usr/bin/env python3\r\nprint('caf\xff')\r\n";
    let plan = plan_uv_metadata_edit(
        Some(source),
        &UvMetadata::default(),
        Some(vec!["rich".to_owned()]),
        Some(">=3.12".to_owned()),
    )
    .unwrap();

    // Invalid UTF-8 without an authoritative block is delivered through metadata.
    assert_eq!(plan.rewritten_source, None);
    assert_eq!(plan.stored.dependencies, ["rich"]);
    assert_eq!(plan.stored.requires_python, ">=3.12");
}

#[test]
fn a_noop_never_rewrites_a_valid_source_block() {
    let source = br#"# /// script
# dependencies = ["requests"]
# ///
print(1)
"#;
    let plan = plan_uv_metadata_edit(Some(source), &UvMetadata::default(), None, None).unwrap();

    assert_eq!(plan.rewritten_source, None);
}
