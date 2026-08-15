use std::path::Path;

use skit_application::{add_workdir, canonical_stored_filename, supports_storage_modes};
use skit_domain::{EntryKind, StorageMode};
use skit_language::{infer_kind, placeholder_params};

#[test]
fn test_prompt_spec_shape() {
    let prompt=EntryKind::parse("prompt").unwrap();
    assert_eq!(canonical_stored_filename("prompt"),Some("prompt.md"));
    assert!(supports_storage_modes(&prompt));
    assert_eq!(add_workdir(&prompt,StorageMode::Copy),"invoke");
    assert_eq!(add_workdir(&prompt,StorageMode::Reference),"invoke");
    assert_eq!(infer_kind(Path::new("review.prompt.md"),None,false),Some("prompt"));
    assert_eq!(
        placeholder_params("prompt","Do {{a}}")
            .into_iter().map(|field|field.name).collect::<Vec<_>>(),
        ["a"]
    );
    // Prompt fields are not argv fields of the prompt itself; they are placeholder deliveries.
    assert!(placeholder_params("prompt","plain text").is_empty());
}

#[test]
fn test_command_spec_carries_the_placeholder_trait() {
    assert_eq!(
        placeholder_params("command","convert {size} {out}")
            .into_iter().map(|field|field.name).collect::<Vec<_>>(),
        ["size","out"]
    );
}

#[test]
fn test_infer_kind_compound_suffix() {
    assert_eq!(infer_kind(Path::new("notes/review.prompt.md"),None,false),Some("prompt"));
    assert_eq!(infer_kind(Path::new("REVIEW.PROMPT.MD"),None,false),Some("prompt"));
    assert_eq!(infer_kind(Path::new("x.prompt"),None,false),Some("prompt"));
    assert_eq!(infer_kind(Path::new("notes.md"),None,false),None);
    assert_eq!(infer_kind(Path::new("a.mts"),None,false),Some("ts"));
    assert_eq!(infer_kind(Path::new("b.sh"),None,false),Some("shell"));
}
