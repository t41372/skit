//! Public-API ports of Python v0.4 JavaScript analyzer mutation-kill contracts.

use skit_form::onboarding_plan;
use skit_language::{DegradationReason, external_dependencies};

fn names(source: &str, kind: &str) -> Vec<String> {
    onboarding_plan(kind, source)
        .candidates
        .into_iter()
        .map(|candidate| candidate.declaration.name)
        .collect()
}

#[test]
fn test_destructuring_literal_is_not_a_candidate_but_later_identifier_const_survives() {
    assert!(names("const {p} = 5;\n", "js").is_empty());
    assert!(names("const [x] = 5;\n", "js").is_empty());
    assert_eq!(names("const {p} = 5;\nconst KEEP = 7;\n", "js"), ["KEEP"]);
}

#[test]
fn test_nonliteral_const_is_skipped_without_abandoning_later_literals() {
    assert_eq!(names("const A = foo();\nconst B = 5;\n", "js"), ["B"]);
    assert_eq!(
        names("const A = foo();\nconst B = 5;\nconst C = 9;\n", "js"),
        ["B", "C"]
    );
}

#[test]
fn test_top_level_reassigned_const_is_demoted_as_accumulator() {
    let plan = onboarding_plan("js", "const C = 1;\nC = 2;\n");
    let [candidate] = plan.candidates.as_slice() else {
        panic!("expected one C candidate: {plan:?}");
    };
    assert_eq!(candidate.declaration.name, "C");
    assert_eq!(candidate.demotion, Some(DegradationReason::Accumulator));
    assert!(!candidate.selected_by_default());
}

#[test]
fn test_augmented_reassigned_const_has_same_accumulator_demotion() {
    let plan = onboarding_plan("js", "const N = 0;\nN += 5;\n");
    let [candidate] = plan.candidates.as_slice() else {
        panic!("expected one N candidate: {plan:?}");
    };
    assert_eq!(candidate.declaration.name, "N");
    assert_eq!(candidate.demotion, Some(DegradationReason::Accumulator));
}

#[test]
fn test_sourceless_export_is_not_an_external_dependency_and_does_not_hide_real_import() {
    assert_eq!(
        external_dependencies("js", "import chalk from 'chalk';\nexport const X = 5;\n",),
        ["chalk"]
    );
    assert!(external_dependencies("js", "export const X = 5;\n").is_empty());
}

#[test]
fn test_typescript_uses_the_same_const_candidate_guards() {
    assert_eq!(
        names("const {p} = 5;\nconst KEEP: number = 7;\n", "ts"),
        ["KEEP"]
    );
    assert_eq!(
        names("const A = foo();\nconst B: number = 5;\n", "ts"),
        ["B"]
    );
}
