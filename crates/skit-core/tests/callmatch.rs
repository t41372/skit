use std::collections::BTreeMap;

use skit_core::match_calls;

fn pairs(items: &[(i64, &str)]) -> Vec<(i64, String)> {
    items
        .iter()
        .map(|(order, prompt)| (*order, (*prompt).to_owned()))
        .collect()
}

#[test]
fn prompt_match_survives_position_shift() {
    assert_eq!(
        match_calls(
            &pairs(&[(0, "Password: ")]),
            &pairs(&[(0, "Username: "), (1, "Password: ")]),
        ),
        BTreeMap::from([(0, (1, false))])
    );
}

#[test]
fn missing_prompt_match_falls_back_to_position_but_flags_ambiguity() {
    assert_eq!(
        match_calls(
            &pairs(&[(0, "Old prompt: ")]),
            &pairs(&[(0, "New prompt: ")]),
        ),
        BTreeMap::from([(0, (0, true))])
    );
}

#[test]
fn promptless_legacy_definition_falls_back_without_new_warning() {
    assert_eq!(
        match_calls(&pairs(&[(0, "")]), &pairs(&[(0, "Anything: ")])),
        BTreeMap::from([(0, (0, false))])
    );
}

#[test]
fn duplicate_prompt_multiset_pairs_stably_by_order() {
    assert_eq!(
        match_calls(
            &pairs(&[(0, "Go? "), (1, "Go? ")]),
            &pairs(&[(0, "Go? "), (1, "Go? ")]),
        ),
        BTreeMap::from([(0, (0, false)), (1, (1, false))])
    );
}

#[test]
fn deleting_one_duplicate_prompt_never_double_binds_the_survivor() {
    let bindings = match_calls(&pairs(&[(0, "Go? "), (1, "Go? ")]), &pairs(&[(0, "Go? ")]));
    assert_eq!(bindings, BTreeMap::from([(0, (0, false))]));
}

#[test]
fn exact_prompt_claim_blocks_another_entrys_positional_fallback() {
    let bindings = match_calls(
        &pairs(&[(0, "Gone: "), (1, "Keep: ")]),
        &pairs(&[(0, "Keep: ")]),
    );
    assert_eq!(bindings, BTreeMap::from([(1, (0, false))]));
}

#[test]
fn duplicate_stored_prompt_with_edited_second_call_flags_loser() {
    let bindings = match_calls(
        &pairs(&[(0, "Go? "), (1, "Go? ")]),
        &pairs(&[(0, "Go? "), (1, "Different: ")]),
    );
    assert_eq!(bindings, BTreeMap::from([(0, (0, false)), (1, (1, true))]));
}
