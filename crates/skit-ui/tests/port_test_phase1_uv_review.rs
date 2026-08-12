//! Rust architecture port of Python `test_inject_block_roundtrip` from `tests/test_phase1.py`.
//! The add-review boundary is the real place Rust decides whether to inject a new PEP 723 block or
//! respect an existing source-owned one; the test never recreates that gate locally.

use skit_application::SourcePermissions;
use skit_ui::{KnownEntryKind, ReviewDefaults, ReviewState, SourceSnapshot};

fn source(path: &str, bytes: &[u8]) -> SourceSnapshot {
    SourceSnapshot {
        path: path.into(),
        source_record: path.to_owned(),
        bytes: bytes.to_vec(),
        permissions: SourcePermissions::default(),
        is_regular: true,
        is_directory: false,
        is_draft: false,
    }
}

#[test]
fn test_inject_block_roundtrip() {
    let original = b"#!/usr/bin/env python3\nimport requests\n";
    let review = ReviewState::from_source(
        source("tool.py", original),
        KnownEntryKind::Python,
        ReviewDefaults {
            dependencies: vec!["requests".to_owned()],
            requires_python: Some(">=3.10".to_owned()),
            ..ReviewDefaults::default()
        },
    );
    let created = review.create_entry().unwrap();
    let payload = created
        .payload
        .expect("copy-mode Python review stores a payload");
    let text = String::from_utf8(payload.bytes).unwrap();

    assert!(
        text.starts_with("#!/usr/bin/env python3\n# /// script\n"),
        "the block was not inserted after the shebang: {text}"
    );
    let metadata = skit_language::read_uv_metadata(&text).unwrap();
    assert_eq!(metadata.dependencies, ["requests"]);
    assert_eq!(metadata.requires_python, ">=3.10");

    let existing = concat!(
        "#!/usr/bin/env python3\n",
        "# /// script\n",
        "# dependencies = [\"requests\"]\n",
        "# requires-python = \">=3.10\"\n",
        "# ///\n",
        "import requests\n",
    );
    let protected = ReviewState::from_source(
        source("existing.py", existing.as_bytes()),
        KnownEntryKind::Python,
        ReviewDefaults {
            dependencies: vec!["other".to_owned()],
            requires_python: Some(">=3.12".to_owned()),
            ..ReviewDefaults::default()
        },
    );
    let created = protected.create_entry().unwrap();
    let payload = created
        .payload
        .expect("copy-mode Python review stores a payload");

    assert_eq!(
        payload.bytes,
        existing.as_bytes(),
        "source-owned PEP 723 metadata was overwritten by add-time defaults"
    );
}
