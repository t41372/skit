//! Exact public-surface ports of Python v0.4 `tests/test_js_analyzer_mut.py`.
//!
//! The broader Rust onboarding suite exercises similar facts, but these five tests retain the
//! frozen Python mutation-suite identities independently so no broad smoke test can substitute for
//! one missing oracle.

use skit_form::onboarding_plan;
use skit_language::{DegradationReason, external_dependencies};

fn names(source: &str) -> Vec<String> {
    onboarding_plan("js", source)
        .candidates
        .into_iter()
        .map(|candidate| candidate.declaration.name)
        .collect()
}

#[test]
fn test_destructuring_with_literal_value_is_not_a_candidate() {
    assert!(
        onboarding_plan("js", "const {p} = 5;\n")
            .candidates
            .is_empty()
    );
    assert!(names("const [x] = 5;\n").is_empty());
    assert_eq!(names("const {p} = 5;\nconst KEEP = 7;\n"), ["KEEP"]);
}

#[test]
fn test_non_literal_const_is_skipped_but_later_literals_still_land() {
    assert_eq!(names("const A = foo();\nconst B = 5;\n"), ["B"]);
    assert_eq!(
        names("const A = foo();\nconst B = 5;\nconst C = 9;\n"),
        ["B", "C"]
    );
}

#[test]
fn test_reassigned_const_carries_the_accumulator_demotion_marker() {
    let plan = onboarding_plan("js", "const C = 1;\nC = 2;\n");
    let [candidate] = plan.candidates.as_slice() else {
        panic!("expected one candidate: {plan:?}");
    };
    assert_eq!(candidate.demotion, Some(DegradationReason::Accumulator));
    assert!(!candidate.selected_by_default());
}

#[test]
fn test_augmented_reassigned_const_demotion_marker() {
    let plan = onboarding_plan("js", "const N = 0;\nN += 5;\n");
    let [candidate] = plan.candidates.as_slice() else {
        panic!("expected one candidate: {plan:?}");
    };
    assert_eq!(candidate.demotion, Some(DegradationReason::Accumulator));
    assert!(!candidate.selected_by_default());
}

#[test]
fn test_external_imports_skips_sourceless_export_statements() {
    assert_eq!(
        external_dependencies("js", "import chalk from 'chalk';\nexport const X = 5;\n"),
        ["chalk"]
    );
    assert!(external_dependencies("js", "export const X = 5;\n").is_empty());
}
