//! Exact frontend-neutral add-review ports from Python v0.4 `tests/test_js_deps.py`.
//!
//! Python drove these through Textual. Rust captures the source once and exposes the review state
//! directly; the executable owners below assert both the visible dependency field and the atomic
//! `CreateEntry` request that the frontend submits.

use std::{fs, path::PathBuf};

use skit_application::SourcePermissions;
use skit_ui::{DependencySurface, KnownEntryKind, ReviewDefaults, ReviewState, SourceSnapshot};
use tempfile::TempDir;

fn snapshot(path: PathBuf, bytes: &[u8]) -> SourceSnapshot {
    SourceSnapshot {
        source_record: path.display().to_string(),
        path,
        bytes: bytes.to_vec(),
        permissions: SourcePermissions::default(),
        is_regular: true,
        is_directory: false,
        is_draft: false,
    }
}

#[test]
fn test_tui_direct_add_records_scanned_js_dependencies() {
    let review = ReviewState::from_source(
        snapshot(
            PathBuf::from("/work/t.mjs"),
            b"import chalk from \"chalk\";\nimport fs from \"node:fs\";\n",
        ),
        KnownEntryKind::JavaScript,
        ReviewDefaults::default(),
    );
    assert_eq!(review.dependency_surface(), &DependencySurface::Npm);
    assert_eq!(review.dependencies_text(), "chalk");
    let create = review.create_entry().unwrap();
    assert_eq!(create.settings.dependencies, ["chalk"]);
}

#[test]
fn test_tui_direct_add_js_without_imports_records_none() {
    let review = ReviewState::from_source(
        snapshot(PathBuf::from("/work/t.mjs"), b"console.log(1);\n"),
        KnownEntryKind::JavaScript,
        ReviewDefaults::default(),
    );
    assert_eq!(review.dependency_surface(), &DependencySurface::Npm);
    assert_eq!(review.dependencies_text(), "");
    let create = review.create_entry().unwrap();
    assert!(create.settings.dependencies.is_empty());
}

#[test]
fn test_tui_direct_add_survives_the_source_vanishing_after_the_copy() {
    let root = TempDir::new().unwrap();
    let source = root.path().join("t.mjs");
    fs::write(&source, "import chalk from \"chalk\";\n").unwrap();
    let captured = fs::read(&source).unwrap();
    let review = ReviewState::from_source(
        snapshot(source.clone(), &captured),
        KnownEntryKind::JavaScript,
        ReviewDefaults::default(),
    );
    fs::remove_file(&source).unwrap();

    assert_eq!(review.dependencies_text(), "chalk");
    let create = review
        .create_entry()
        .expect("the review must use its captured snapshot, not re-read the vanished source");
    assert_eq!(create.settings.dependencies, ["chalk"]);
    assert_eq!(
        create.payload.expect("copy mode keeps the captured bytes").bytes,
        captured
    );
}

#[test]
fn test_interactive_accept_of_a_scoped_suggestion_round_trips() {
    let review = ReviewState::from_source(
        snapshot(
            PathBuf::from("/work/t.mjs"),
            concat!(
                "import chalk from \"chalk\";\n",
                "import { S3Client } from \"@aws-sdk/client-s3\";\n",
            )
            .as_bytes(),
        ),
        KnownEntryKind::JavaScript,
        ReviewDefaults::default(),
    );
    assert_eq!(review.dependencies_text(), "chalk, @aws-sdk/client-s3");
    let create = review.create_entry().unwrap();
    assert_eq!(
        create.settings.dependencies,
        ["chalk", "@aws-sdk/client-s3"]
    );
}