//! Parser-owned reconcile ports from Python `tests/test_edit.py` at `main@206f9ef`.
//!
//! Python's `edit_specs` mixed source reconciliation with edit-list mutation. Rust exposes the
//! source reconciliation as `ParsedDocument::reconcile`; these tests pin the same stored/current
//! distinctions without changing production code when parity fails.

use skit_domain::parameters::{
    ParamDecl, ParameterBinding, ParameterDelivery, ParameterType,
};
use skit_language::{ParseOutcome, parse_document};

const SCRIPT: &str = concat!(
    "CITY = \"Taipei\"\n",
    "RETRIES = 3\n",
    "who = input(\"Name: \")\n",
    "print(CITY, RETRIES, who)\n",
);

fn stored(name: &str, parameter_type: ParameterType) -> ParamDecl {
    let mut declaration = ParamDecl::new(name);
    declaration.binding = ParameterBinding::Const;
    declaration.delivery = ParameterDelivery::Inject;
    declaration.parameter_type = parameter_type;
    declaration
}

fn document() -> skit_language::ParsedDocument {
    let ParseOutcome::Parsed(document) = parse_document("python", SCRIPT) else {
        panic!("the frozen edit fixture must parse as Python");
    };
    document
}

#[test]
fn test_resync_drops_missing_and_keeps_matching() {
    let city = stored("CITY", ParameterType::Str);
    let gone = stored("GONE", ParameterType::Str);

    let report = document().reconcile(&[city, gone]);

    assert_eq!(
        report
            .ok
            .iter()
            .map(|pair| pair.stored.name.as_str())
            .collect::<Vec<_>>(),
        ["CITY"]
    );
    assert_eq!(
        report
            .missing
            .iter()
            .map(|declaration| declaration.name.as_str())
            .collect::<Vec<_>>(),
        ["GONE"]
    );
    assert!(report.changed.is_empty());
    assert!(report.rebound.is_empty());
}

#[test]
fn test_resync_updates_changed_type_preserving_customization() {
    let mut retries = stored("RETRIES", ParameterType::Str);
    retries.secret = true;
    retries.prompt = "How many? ".to_owned();

    let report = document().reconcile(&[retries]);

    assert_eq!(report.changed.len(), 1);
    let pair = &report.changed[0];
    assert_eq!(pair.stored.name, "RETRIES");
    assert_eq!(pair.stored.parameter_type, ParameterType::Str);
    assert!(pair.stored.secret);
    assert_eq!(pair.stored.prompt, "How many? ");
    assert_eq!(pair.current.declaration.parameter_type, ParameterType::Int);
    assert!(report.ok.is_empty());
    assert!(report.missing.is_empty());
}

#[test]
fn test_edit_specs_is_pure_no_mutation_of_input_list() {
    let original = vec![stored("CITY", ParameterType::Str)];
    let before = original.clone();

    let report = document().reconcile(&original);

    assert_eq!(original, before);
    assert_eq!(report.ok.len(), 1);
    assert_eq!(report.ok[0].stored, before[0]);
}
