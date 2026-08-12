//! Public-surface ports of the splitter branch cases in Python `tests/test_pep723_split.py`
//! from `main@206f9ef`.
//!
//! Rust keeps the actual PEP 508 comma partitioner private to the add/settings model. These tests
//! deliberately reach it through public product behavior: `SettingsView::dependencies_edit` for
//! the shared splitter and `ReviewState::create_entry` for the interactive Add-review intake. This
//! avoids a test-only product seam while preserving the mutation-grade Python inputs and their exact
//! partitions.

use skit_application::SourcePermissions;
use skit_language::read_uv_metadata;
use skit_ui::{
    DEPENDENCIES_KEY, DependencyFlavor, FieldValue, KnownEntryKind, ReviewDefaults, ReviewState,
    SettingsInputs, SettingsView, SourceSnapshot,
};

fn split_requirements(value: &str) -> Vec<String> {
    let mut view = SettingsView::from_inputs(&SettingsInputs {
        selector: "pep508".to_owned(),
        kind: "python".to_owned(),
        name: "pep508".to_owned(),
        dependency_flavor: Some(DependencyFlavor::Uv),
        // Keep a non-empty baseline so even the Python empty/blank cases are an actual edit and
        // `dependencies_edit()` must return a value rather than the untouched-axis sentinel.
        effective_dependencies: vec!["baseline".to_owned()],
        ..SettingsInputs::default()
    });
    assert!(
        view.field(DEPENDENCIES_KEY).is_some(),
        "the Python dependency field must be present"
    );
    assert!(
        view.set_value(DEPENDENCIES_KEY, FieldValue::text(value)),
        "the dependency edit was not accepted"
    );
    view.dependencies_edit()
        .expect("an edited dependency field must produce an edit")
}

#[test]
fn test_simple_list_splits() {
    assert_eq!(split_requirements("requests, rich"), ["requests", "rich"]);
}

#[test]
fn test_single_item_no_commas() {
    assert_eq!(split_requirements("requests"), ["requests"]);
}

#[test]
fn test_specifier_commas_stay_joined() {
    assert_eq!(split_requirements("requests>=2,<3"), ["requests>=2,<3"]);
}

#[test]
fn test_specifier_lists_split_only_between_requirements() {
    assert_eq!(
        split_requirements("requests>=2,<3, pillow!=9.0,>=8"),
        ["requests>=2,<3", "pillow!=9.0,>=8"]
    );
}

#[test]
fn test_spaces_around_specifier_commas() {
    assert_eq!(
        split_requirements("foo >= 1 , < 2 , bar"),
        ["foo >= 1 , < 2", "bar"]
    );
}

#[test]
fn test_extras_bracket_commas_stay_joined() {
    assert_eq!(
        split_requirements("requests[security,socks]>=2, rich"),
        ["requests[security,socks]>=2", "rich"]
    );
}

#[test]
fn test_parenthesized_specifier_commas_stay_joined() {
    assert_eq!(
        split_requirements("foo (>=1.0,<2.0), bar"),
        ["foo (>=1.0,<2.0)", "bar"]
    );
}

#[test]
fn test_double_quoted_marker_comma_stays_joined() {
    assert_eq!(
        split_requirements("a; sys_platform in \"linux,darwin\", b"),
        ["a; sys_platform in \"linux,darwin\"", "b"]
    );
}

#[test]
fn test_single_quoted_marker_comma_stays_joined() {
    assert_eq!(
        split_requirements("a; extra in 'x,y', b"),
        ["a; extra in 'x,y'", "b"]
    );
}

#[test]
fn test_name_starting_with_digit_splits() {
    assert_eq!(
        split_requirements("rich, 2captcha-python"),
        ["rich", "2captcha-python"]
    );
}

#[test]
fn test_trailing_comma_dropped() {
    assert_eq!(
        split_requirements("requests>=2,<3,"),
        ["requests>=2,<3"]
    );
}

#[test]
fn test_empty_and_blank_input() {
    assert!(split_requirements("").is_empty());
    assert!(split_requirements("   ").is_empty());
}

#[test]
fn test_uppercase_x_in_name_is_ordinary_text() {
    assert_eq!(split_requirements("pkgX, rich"), ["pkgX", "rich"]);
}

#[test]
fn test_nested_brackets_tracked_by_depth_not_flag() {
    assert_eq!(
        split_requirements("a[[x],y], b"),
        ["a[[x],y]", "b"]
    );
}

#[test]
fn test_next_nonspace_end_of_text_is_empty_string() {
    // Python pins the private `_next_nonspace` sentinel directly because it kills mutants that can
    // otherwise look equivalent through one caller. Rust has no public helper with that identity,
    // so pin both public partitions that the sentinel distinguishes: end-of-text after a comma is
    // not a new requirement, while a following alphanumeric name is.
    assert_eq!(split_requirements("a,  "), ["a"]);
    assert_eq!(split_requirements("a, b"), ["a", "b"]);
}

#[test]
fn test_interactive_deps_answer_keeps_specifier_commas() {
    // Python's call site is `_resolve_python_metadata`, the interactive intake before an add. Rust's
    // architectural twin is the Add review model. Drive its public editable field all the way to
    // the atomic CreateEntry request; testing the shared splitter alone would miss a call-site bug.
    let source = SourceSnapshot {
        path: "s.py".into(),
        source_record: "s.py".to_owned(),
        bytes: b"import requests\nprint(requests)\n".to_vec(),
        permissions: SourcePermissions::default(),
        is_regular: true,
        is_directory: false,
        is_draft: false,
    };
    let mut review = ReviewState::from_source(
        source,
        KnownEntryKind::Python,
        ReviewDefaults::default(),
    );
    review.set_dependencies_text("requests>=2,<3, rich");

    let create = review
        .create_entry()
        .expect("the interactive review must accept both valid PEP 508 requirements");
    let payload = create
        .payload
        .expect("a copied Python add must carry the source payload");
    let stored = std::str::from_utf8(&payload.bytes).expect("Python fixture remains UTF-8");
    let metadata = read_uv_metadata(stored).expect("interactive dependencies must reach PEP 723");
    assert_eq!(metadata.dependencies, ["requests>=2,<3", "rich"]);
}
