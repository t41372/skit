//! Mechanical port of the Python oracle module `tests/test_reconcile.py`
//! (`origin/main@206f9ef`). Each `#[test]` keeps its Python `def test_*` name and the Python
//! "WHY" comment is preserved verbatim above it.
//!
//! Concept mapping:
//! - Python `reconcile.reconcile(text, specs)` -> the `reconcile` helper below, which parses then
//!   calls `ParsedDocument::reconcile`, and falls back to `ReconcileReport::from_syntax_error` when
//!   the source does not parse (the production composition across the parse boundary; flagged).
//! - Python `spec(...)` -> the `spec` helper (Python keyword defaults spelled out positionally).
//! - Report fields: Python `.ok`/`.rebind`/`.changed` are `(spec, candidate)`-shaped; Rust `.ok`/
//!   `.rebound`/`.changed` are `Vec<ReconcilePair>` (`pair.stored`, `pair.current.declaration`).
//!   `.missing` is `Vec<ParamDecl>`, `.new` is `Vec<SemanticCandidate>`, `.usable()` is a method,
//!   `.has_drift()` is a method, `.syntax_error` is a field.
//!
//! Higher-layer Python contracts for drift rendering and source-parameter editing/resync live in
//! `crates/skit-cli/tests/port_test_reconcile_edit.rs`, where the same Python test names execute
//! against the real CLI/storage boundary. This language target contains only reconcile-owned logic.

use std::collections::BTreeSet;

use skit_domain::parameters::{ParamDecl, ParameterBinding, ParameterType};
use skit_language::{ParseOutcome, ReconcileReport, parse_document};

/// Python `reconcile.reconcile(text, specs)`: parse, reconcile against the current source, or return
/// the conservative all-missing report when the source has a syntax error.
fn reconcile(source: &str, stored: &[ParamDecl]) -> ReconcileReport {
    match parse_document("python", source) {
        ParseOutcome::Parsed(document) => document.reconcile(stored),
        _ => ReconcileReport::from_syntax_error(stored),
    }
}

/// Python `spec(name, *, binding="const", type="str", order=-1, prompt="")`.
fn spec(
    name: &str,
    binding: ParameterBinding,
    parameter_type: ParameterType,
    order: i64,
    prompt: &str,
) -> ParamDecl {
    let mut declaration = ParamDecl::new(name);
    declaration.binding = binding;
    declaration.parameter_type = parameter_type;
    declaration.order = order;
    declaration.prompt = prompt.to_owned();
    declaration
}

/// A default const spec: Python `spec("NAME")`.
fn const_spec(name: &str) -> ParamDecl {
    spec(name, ParameterBinding::Const, ParameterType::Str, -1, "")
}

const SCRIPT: &str =
    "CITY = \"Taipei\"\nRETRIES = 3\nwho = input(\"Your name: \")\nprint(who, CITY, RETRIES)\n";

fn usable_names(report: &ReconcileReport) -> Vec<String> {
    report
        .usable()
        .into_iter()
        .map(|declaration| declaration.name.clone())
        .collect()
}

fn missing_names(report: &ReconcileReport) -> Vec<String> {
    report
        .missing
        .iter()
        .map(|declaration| declaration.name.clone())
        .collect()
}

fn ok_names(report: &ReconcileReport) -> Vec<String> {
    report
        .ok
        .iter()
        .map(|pair| pair.stored.name.clone())
        .collect()
}

fn rebound_names(report: &ReconcileReport) -> Vec<String> {
    report
        .rebound
        .iter()
        .map(|pair| pair.stored.name.clone())
        .collect()
}

fn new_names(report: &ReconcileReport) -> BTreeSet<String> {
    report
        .new
        .iter()
        .map(|candidate| candidate.declaration.name.clone())
        .collect()
}

#[test]
fn test_all_ok_no_drift() {
    let specs = vec![
        const_spec("CITY"),
        spec("RETRIES", ParameterBinding::Const, ParameterType::Int, -1, ""),
        spec("input-1", ParameterBinding::Input, ParameterType::Str, 0, ""),
    ];
    let report = reconcile(SCRIPT, &specs);
    assert!(!report.has_drift());
    assert_eq!(
        report.usable().into_iter().cloned().collect::<Vec<_>>(),
        specs
    );
    assert!(report.new.is_empty());
}

#[test]
fn test_const_missing_by_name() {
    let report = reconcile(SCRIPT, &[const_spec("GONE")]);
    assert!(report.has_drift());
    assert_eq!(missing_names(&report), ["GONE"]);
    assert!(usable_names(&report).is_empty());
}

#[test]
fn test_const_renamed_is_missing_plus_new() {
    // User renamed CITY to TOWN: old definition is missing, new name appears in new (informational,
    // not considered drift).
    let text = SCRIPT.replace("CITY", "TOWN");
    let report = reconcile(&text, &[const_spec("CITY")]);
    assert_eq!(missing_names(&report), ["CITY"]);
    assert!(new_names(&report).contains("TOWN"));
}

#[test]
fn test_const_type_changed_still_usable() {
    let text = SCRIPT.replace("RETRIES = 3", "RETRIES = \"3\"");
    let report = reconcile(
        &text,
        &[spec("RETRIES", ParameterBinding::Const, ParameterType::Int, -1, "")],
    );
    assert!(report.has_drift());
    assert_eq!(
        report
            .changed
            .iter()
            .map(|pair| (pair.stored.name.clone(), pair.current.declaration.parameter_type))
            .collect::<Vec<_>>(),
        [("RETRIES".to_owned(), ParameterType::Str)]
    );
    assert_eq!(usable_names(&report), ["RETRIES"]); // still injectable, but warned
}

#[test]
fn test_input_matched_by_order_not_position_in_file() {
    // Code was inserted before the script; the input() line number changed but it's still call 0 —
    // not considered drift.
    let text = format!("import os\nprint(os.name)\n{SCRIPT}");
    let report = reconcile(
        &text,
        &[spec("input-1", ParameterBinding::Input, ParameterType::Str, 0, "")],
    );
    assert!(!report.has_drift());
}

#[test]
fn test_input_removed_is_missing() {
    let text = SCRIPT.replace("who = input(\"Your name: \")", "who = \"nobody\"");
    let report = reconcile(
        &text,
        &[spec("input-1", ParameterBinding::Input, ParameterType::Str, 0, "")],
    );
    assert_eq!(missing_names(&report), ["input-1"]);
}

#[test]
fn test_new_input_call_reported_as_new_only() {
    let text = format!("{SCRIPT}more = input(\"More: \")\nprint(more)\n");
    let report = reconcile(
        &text,
        &[spec("input-1", ParameterBinding::Input, ParameterType::Str, 0, "")],
    );
    assert!(!report.has_drift()); // existing definitions are still present; new is not drift
    assert_eq!(
        report
            .new
            .iter()
            .filter(|candidate| candidate.declaration.binding == ParameterBinding::Input)
            .map(|candidate| candidate.declaration.order)
            .collect::<Vec<_>>(),
        [1]
    );
}

// ---------- 3a: input matching prefers the stored prompt over bare position ----------

#[test]
fn test_input_prompt_match_survives_an_earlier_insertion_no_drift() {
    // A new input() call was inserted BEFORE the managed one, shifting its bare position from 0 to
    // 1. Pre-3a this was invisible (still "ok" by sheer luck of matching *some* candidate at
    // position 0) but could silently rebind a different value onto the wrong question. With the
    // prompt recorded, the match follows the prompt to its new position and is NOT flagged.
    let text = format!("extra = input(\"Extra: \")\n{SCRIPT}");
    let report = reconcile(
        &text,
        &[spec("input-1", ParameterBinding::Input, ParameterType::Str, 0, "Your name: ")],
    );
    assert!(!report.has_drift());
    assert_eq!(ok_names(&report), ["input-1"]);
    assert!(report.rebound.is_empty());
}

#[test]
fn test_input_deleted_earlier_call_flags_rebind_instead_of_silent_ok() {
    // Reproduces the reconcile/shim gap directly: three input() calls, one stored per question.
    // Deleting the FIRST input() call shifts input-2 and input-3's bare position down by one each.
    // Pre-3a, position-only matching would silently report every one of them "ok" (some candidate
    // still exists at every stored position) even though position 0 now holds a DIFFERENT question
    // than the one input-1 used to describe -- exactly the silent-rebind risk 3a must catch.
    let text = concat!(
        "first = input(\"First: \")\n",
        "second = input(\"Second: \")\n",
        "third = input(\"Third: \")\n",
        "print(first, second, third)\n",
    );
    let specs = [
        spec("input-1", ParameterBinding::Input, ParameterType::Str, 0, "First: "),
        spec("input-2", ParameterBinding::Input, ParameterType::Str, 1, "Second: "),
        spec("input-3", ParameterBinding::Input, ParameterType::Str, 2, "Third: "),
    ];
    let edited = text.replace("first = input(\"First: \")\n", ""); // delete the first input() call
    let report = reconcile(&edited, &specs);
    // input-1 (First:) is genuinely gone: its own prompt matches nothing, and the position it used
    // to occupy (0) is now legitimately owned by input-2's own exact prompt match -- so it must be
    // reported missing, not silently handed input-2's call site.
    assert_eq!(missing_names(&report), ["input-1"]);
    // input-2 (Second:) and input-3 (Third:) still resolve correctly by prompt, at their new
    // positions (0 and 1) -- not flagged, because the prompt uniquely identifies each of them
    // despite the shift. This is the concrete proof the fix does its job: no silent swap is even
    // possible here, since the match never falls back to position at all when the prompt still
    // uniquely resolves.
    assert_eq!(
        ok_names(&report).into_iter().collect::<BTreeSet<_>>(),
        BTreeSet::from(["input-2".to_owned(), "input-3".to_owned()])
    );
    assert!(report.rebound.is_empty());
    assert_eq!(
        usable_names(&report).into_iter().collect::<BTreeSet<_>>(),
        BTreeSet::from(["input-2".to_owned(), "input-3".to_owned()])
    );
}

#[test]
fn test_input_rebind_flagged_when_prompt_can_no_longer_disambiguate() {
    // When the prompt genuinely can't resolve the call site any more (renamed prompt, but a call
    // still exists at the old bare position), the match must fall back to position AND be flagged
    // as `rebind` -- still usable (no silent drop), but visibly warned (no silent trust either).
    let text = "value = input(\"New label: \")\nprint(value)\n";
    let report = reconcile(
        text,
        &[spec("input-1", ParameterBinding::Input, ParameterType::Str, 0, "Old label: ")],
    );
    assert!(report.has_drift());
    assert_eq!(rebound_names(&report), ["input-1"]);
    assert_eq!(usable_names(&report), ["input-1"]); // still injectable, just warned
}

#[test]
fn test_unselected_candidates_are_new_but_not_drift() {
    // Onboarding only selected CITY; RETRIES and input are "new" but must never nag the user on
    // every run.
    let report = reconcile(SCRIPT, &[const_spec("CITY")]);
    assert!(!report.has_drift());
    assert_eq!(
        new_names(&report),
        BTreeSet::from(["RETRIES".to_owned(), "input-1".to_owned()])
    );
}

#[test]
fn test_input_duplicate_prompt_surplus_is_missing_not_ok_on_delete() {
    // Regression: two stored input specs share the identical literal prompt (a retry pattern, e.g.
    // two `input("Go? ")` calls, both managed). The user deletes one of the two calls, leaving a
    // single current call site with that prompt. Pre-fix, match_calls's exact pass let BOTH
    // stored orders exact-match onto that one surviving call site (ambiguous=False), so reconcile
    // reported both "ok" with no drift warning at all -- and shim would go on to splice two
    // replacements over the same input() callee, corrupting the injected copy. The surplus spec
    // must instead come back "missing" (drift), never silently "ok".
    let text = "first = input(\"Go? \")\nsecond = input(\"Go? \")\nprint(first, second)\n";
    let specs = [
        spec("input-1", ParameterBinding::Input, ParameterType::Str, 0, "Go? "),
        spec("input-2", ParameterBinding::Input, ParameterType::Str, 1, "Go? "),
    ];
    let edited = text.replace("first = input(\"Go? \")\n", ""); // delete the first call
    let report = reconcile(&edited, &specs);
    assert!(report.has_drift());
    assert_eq!(missing_names(&report), ["input-2"]);
    assert_eq!(ok_names(&report), ["input-1"]);
    assert!(report.rebound.is_empty());
    assert_eq!(usable_names(&report), ["input-1"]);
}

#[test]
fn test_input_duplicate_prompt_surplus_is_rebind_not_ok_when_position_edited() {
    // Same duplicate-prompt setup, but the call at the loser's bare position (1) still exists --
    // its prompt was just edited to something else. The loser can't win an exact match (its
    // candidate was already claimed), so it falls back to position 1, which now answers a
    // *different* question: that must surface as `rebind` (still usable, but warned), never a
    // silent "ok" and never the winner's call site.
    let text = "first = input(\"Go? \")\nsecond = input(\"Go? \")\nprint(first, second)\n";
    let specs = [
        spec("input-1", ParameterBinding::Input, ParameterType::Str, 0, "Go? "),
        spec("input-2", ParameterBinding::Input, ParameterType::Str, 1, "Go? "),
    ];
    let edited = text.replace("second = input(\"Go? \")", "second = input(\"Different: \")");
    let report = reconcile(&edited, &specs);
    assert!(report.has_drift());
    assert_eq!(ok_names(&report), ["input-1"]);
    assert_eq!(rebound_names(&report), ["input-2"]);
    assert!(report.missing.is_empty());
    assert_eq!(
        usable_names(&report).into_iter().collect::<BTreeSet<_>>(),
        BTreeSet::from(["input-1".to_owned(), "input-2".to_owned()])
    );
}

#[test]
fn test_syntax_error_marks_all_missing() {
    let report = reconcile("def broken(:\n", &[const_spec("CITY")]);
    assert!(report.syntax_error);
    assert_eq!(missing_names(&report), ["CITY"]);
    assert!(usable_names(&report).is_empty());
}
