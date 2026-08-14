//! Exact add-review suggestion ports from Python v0.4 `tests/test_js_deps.py`.

use std::path::PathBuf;

use skit_application::SourcePermissions;
use skit_ui::{
    DependencySurface, KnownEntryKind, ReviewDefaults, ReviewState, SourceSnapshot,
};

fn review(kind: KnownEntryKind, source_name: &str, source: &str) -> ReviewState {
    ReviewState::from_source(
        SourceSnapshot {
            path: PathBuf::from(source_name),
            source_record: source_name.to_owned(),
            bytes: source.as_bytes().to_vec(),
            permissions: SourcePermissions::default(),
            is_regular: true,
            is_directory: false,
            is_draft: false,
        },
        kind,
        ReviewDefaults::default(),
    )
}

#[test]
fn test_js_and_ts_specs_declare_the_npm_flavor() {
    for (kind, filename) in [
        (KnownEntryKind::JavaScript, "/work/t.mjs"),
        (KnownEntryKind::TypeScript, "/work/t.ts"),
    ] {
        let review = review(kind, filename, "console.log(1);\n");
        assert_eq!(
            review.dependency_surface(),
            &DependencySurface::Npm,
            "kind={kind:?}"
        );
        assert!(review.requires_python().is_empty(), "kind={kind:?}");
    }
}

#[test]
fn test_resolve_npm_dependencies_interactive_accepts_the_suggestion() {
    let review = review(
        KnownEntryKind::JavaScript,
        "/work/t.mjs",
        "import chalk from \"chalk\";\n",
    );
    assert_eq!(review.dependencies_text(), "chalk");
    assert_eq!(review.create_entry().unwrap().settings.dependencies, ["chalk"]);
}

#[test]
fn test_resolve_npm_dependencies_interactive_dash_declines() {
    let mut review = review(
        KnownEntryKind::JavaScript,
        "/work/t.mjs",
        "import chalk from \"chalk\";\n",
    );
    review.set_dependencies_text(" - ");
    assert_eq!(
        review.create_entry().unwrap().settings.dependencies,
        Vec::<String>::new(),
        "the frozen interactive '-' answer means decline all suggested npm dependencies"
    );
}

#[test]
fn test_resolve_npm_dependencies_interactive_edit_splits_requirements() {
    let mut review = review(
        KnownEntryKind::JavaScript,
        "/work/t.mjs",
        "import chalk from \"chalk\";\n",
    );
    review.set_dependencies_text("chalk@^5, zod");
    assert_eq!(
        review.create_entry().unwrap().settings.dependencies,
        ["chalk@^5", "zod"]
    );
}