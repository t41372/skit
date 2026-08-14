use std::path::PathBuf;

use skit_application::SourcePermissions;
use skit_domain::StorageMode;
use skit_ui::{KnownEntryKind, ReviewDefaults, ReviewState, SourceSnapshot};

#[test]
fn test_add_interactive_tui_form_opens_review_panel() {
    let source = SourceSnapshot {
        path: PathBuf::from("/work/job.py"),
        source_record: "/work/job.py".to_owned(),
        bytes: b"print(1)\n".to_vec(),
        permissions: SourcePermissions::default(),
        is_regular: true,
        is_directory: false,
        is_draft: false,
    };
    let review = ReviewState::from_source(
        source,
        KnownEntryKind::Python,
        ReviewDefaults {
            name: Some("hint".to_owned()),
            reference: true,
            ..ReviewDefaults::default()
        },
    );

    assert_eq!(review.name(), "hint", "the CLI --name flag must arrive as a panel prefill");
    assert_eq!(review.storage(), StorageMode::Reference, "--ref must select reference storage in the panel");
    let create = review.create_entry().expect("the hosted review must produce the same atomic create request");
    assert_eq!(create.name, "hint");
    assert_eq!(create.mode, StorageMode::Reference);
    assert!(create.payload.is_none(), "reference review must not smuggle a copied payload into the commit");
}
