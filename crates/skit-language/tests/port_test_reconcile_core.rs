//! Public-API ports of Python v0.4 `tests/test_reconcile.py`.
//!
//! Reconciliation must never silently rebind a managed input onto a different question. Exact
//! prompt identity wins over bare position; positional fallback stays usable but is surfaced as
//! rebound drift. Newly discovered unmanaged candidates remain informational rather than drift.

use skit_domain::parameters::{ParamDecl, ParameterBinding, ParameterDelivery, ParameterType};
use skit_language::{ParseOutcome, ReconcileReport, parse_document};

const SCRIPT: &str =
    "CITY = \"Taipei\"\nRETRIES = 3\nwho = input(\"Your name: \")\nprint(who, CITY, RETRIES)\n";

fn reconcile(source: &str, stored: &[ParamDecl]) -> ReconcileReport {
    let ParseOutcome::Parsed(document) = parse_document("python", source) else {
        panic!("expected Python source to parse");
    };
    document.reconcile(stored)
}

fn constant(name: &str, parameter_type: ParameterType) -> ParamDecl {
    let mut declaration = ParamDecl::new(name);
    declaration.binding = ParameterBinding::Const;
    declaration.delivery = ParameterDelivery::Inject;
    declaration.parameter_type = parameter_type;
    declaration
}

fn input(name: &str, order: i64, prompt: &str) -> ParamDecl {
    let mut declaration = ParamDecl::new(name);
    declaration.binding = ParameterBinding::Input;
    declaration.delivery = ParameterDelivery::Inject;
    declaration.order = order;
    declaration.prompt = prompt.to_owned();
    declaration
}

#[test]
fn test_all_ok_has_no_drift_and_no_new_candidates() {
    let stored = [
        constant("CITY", ParameterType::Str),
        constant("RETRIES", ParameterType::Int),
        input("input-1", 0, "Your name: "),
    ];
    let report = reconcile(SCRIPT, &stored);

    assert!(!report.has_drift());
    assert_eq!(report.ok.len(), 3);
    assert!(report.new.is_empty());
    assert_eq!(report.usable().len(), 3);
}

#[test]
fn test_const_missing_by_name_is_drift_and_not_usable() {
    let report = reconcile(SCRIPT, &[constant("GONE", ParameterType::Str)]);

    assert!(report.has_drift());
    assert_eq!(
        report
            .missing
            .iter()
            .map(|item| item.name.as_str())
            .collect::<Vec<_>>(),
        ["GONE"]
    );
    assert!(report.usable().is_empty());
}

#[test]
fn test_const_rename_is_missing_plus_new_but_new_itself_is_not_extra_drift() {
    let text = SCRIPT.replace("CITY", "TOWN");
    let report = reconcile(&text, &[constant("CITY", ParameterType::Str)]);

    assert_eq!(
        report
            .missing
            .iter()
            .map(|item| item.name.as_str())
            .collect::<Vec<_>>(),
        ["CITY"]
    );
    assert!(
        report
            .new
            .iter()
            .any(|candidate| candidate.declaration.name == "TOWN")
    );
}

#[test]
fn test_const_type_changed_is_still_usable_but_warned() {
    let text = SCRIPT.replace("RETRIES = 3", "RETRIES = \"3\"");
    let report = reconcile(&text, &[constant("RETRIES", ParameterType::Int)]);

    assert!(report.has_drift());
    assert_eq!(report.changed.len(), 1);
    assert_eq!(report.changed[0].stored.name, "RETRIES");
    assert_eq!(
        report.changed[0].current.declaration.parameter_type,
        ParameterType::Str
    );
    assert_eq!(report.usable()[0].name, "RETRIES");
}

#[test]
fn test_input_matched_by_semantic_order_not_source_line_number() {
    let text = format!("import os\nprint(os.name)\n{SCRIPT}");
    let report = reconcile(&text, &[input("input-1", 0, "Your name: ")]);

    assert!(!report.has_drift());
    assert_eq!(report.ok.len(), 1);
}

#[test]
fn test_new_input_call_is_new_only_not_drift() {
    let text = format!("{SCRIPT}more = input(\"More: \")\nprint(more)\n");
    let report = reconcile(&text, &[input("input-1", 0, "Your name: ")]);

    assert!(!report.has_drift());
    assert!(report.new.iter().any(|candidate| {
        candidate.declaration.binding == ParameterBinding::Input && candidate.declaration.order == 1
    }));
}

#[test]
fn test_prompt_identity_survives_an_earlier_input_insertion_without_drift() {
    let text = format!("extra = input(\"Extra: \")\n{SCRIPT}");
    let report = reconcile(&text, &[input("input-1", 0, "Your name: ")]);

    assert!(!report.has_drift());
    assert_eq!(report.ok.len(), 1);
    assert!(report.rebound.is_empty());
    assert_eq!(report.ok[0].current.declaration.order, 1);
}

#[test]
fn test_deleting_an_earlier_call_does_not_silently_swap_later_managed_inputs() {
    let text = concat!(
        "first = input(\"First: \")\n",
        "second = input(\"Second: \")\n",
        "third = input(\"Third: \")\n",
        "print(first, second, third)\n",
    );
    let stored = [
        input("input-1", 0, "First: "),
        input("input-2", 1, "Second: "),
        input("input-3", 2, "Third: "),
    ];
    let edited = text.replace("first = input(\"First: \")\n", "");
    let report = reconcile(&edited, &stored);

    assert_eq!(
        report
            .missing
            .iter()
            .map(|item| item.name.as_str())
            .collect::<Vec<_>>(),
        ["input-1"]
    );
    assert_eq!(
        report
            .ok
            .iter()
            .map(|pair| pair.stored.name.as_str())
            .collect::<std::collections::BTreeSet<_>>(),
        std::collections::BTreeSet::from(["input-2", "input-3"])
    );
    assert!(report.rebound.is_empty());
}

#[test]
fn test_prompt_change_falls_back_to_position_and_is_flagged_rebound() {
    let text = "value = input(\"New label: \")\nprint(value)\n";
    let report = reconcile(text, &[input("input-1", 0, "Old label: ")]);

    assert!(report.has_drift());
    assert_eq!(report.rebound.len(), 1);
    assert_eq!(report.rebound[0].stored.name, "input-1");
    assert_eq!(report.rebound[0].current.declaration.prompt, "New label: ");
    assert_eq!(report.usable()[0].name, "input-1");
}

#[test]
fn test_unselected_candidates_are_new_but_never_drift() {
    let report = reconcile(SCRIPT, &[constant("CITY", ParameterType::Str)]);

    assert!(!report.has_drift());
    assert_eq!(
        report
            .new
            .iter()
            .map(|candidate| candidate.declaration.name.as_str())
            .collect::<std::collections::BTreeSet<_>>(),
        std::collections::BTreeSet::from(["RETRIES", "input-1"])
    );
}

#[test]
fn test_duplicate_prompt_surplus_is_missing_not_silently_ok_after_delete() {
    let text = concat!(
        "first = input(\"Go? \")\n",
        "second = input(\"Go? \")\n",
        "print(first, second)\n",
    );
    let stored = [input("input-1", 0, "Go? "), input("input-2", 1, "Go? ")];
    let edited = text.replace("first = input(\"Go? \")\n", "");
    let report = reconcile(&edited, &stored);

    assert!(report.has_drift());
    assert_eq!(
        report
            .missing
            .iter()
            .map(|item| item.name.as_str())
            .collect::<Vec<_>>(),
        ["input-2"]
    );
    assert_eq!(
        report
            .ok
            .iter()
            .map(|pair| pair.stored.name.as_str())
            .collect::<Vec<_>>(),
        ["input-1"]
    );
    assert!(report.rebound.is_empty());
}

#[test]
fn test_duplicate_prompt_surplus_is_rebound_when_its_position_now_answers_a_different_question() {
    let text = concat!(
        "first = input(\"Go? \")\n",
        "second = input(\"Go? \")\n",
        "print(first, second)\n",
    );
    let stored = [input("input-1", 0, "Go? "), input("input-2", 1, "Go? ")];
    let edited = text.replace(
        "second = input(\"Go? \")",
        "second = input(\"Different: \")",
    );
    let report = reconcile(&edited, &stored);

    assert!(report.has_drift());
    assert_eq!(
        report
            .ok
            .iter()
            .map(|pair| pair.stored.name.as_str())
            .collect::<Vec<_>>(),
        ["input-1"]
    );
    assert_eq!(
        report
            .rebound
            .iter()
            .map(|pair| pair.stored.name.as_str())
            .collect::<Vec<_>>(),
        ["input-2"]
    );
    assert!(report.missing.is_empty());
}
