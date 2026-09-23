//! Mechanical port of the Python oracle module `tests/test_callmatch.py`
//! (`origin/main@206f9ef`). Each `#[test]` keeps its Python `def test_*` name and the Python
//! "WHY" comment is preserved verbatim above it.
//!
//! Python drives `skit.callmatch.match_calls(stored, current) -> {stored_order: (current_order,
//! ambiguous)}` directly. In Rust that function is `skit_language::semantic::match_calls`, which is
//! `pub(super)` and unreachable from an integration test, so every test here maps `match_calls`
//! onto the public `ParsedDocument::reconcile` (its production caller) exactly as the analyzer port
//! did: stored `(order, prompt)` tuples become input `ParamDecl`s (`stored_input`), the `current`
//! list becomes generated Python source (`current_source`), and the reconcile report is folded back
//! into the Python dict (`match_bindings`): `ok` -> `(order, false)`, `rebound` -> `(order, true)`,
//! an unresolved stored key is absent. This whole-module mapping is the flagged semantic judgment.

use std::collections::BTreeMap;

use skit_domain::parameters::{ParamDecl, ParameterBinding, ParameterDelivery};
use skit_language::{ParseOutcome, ParsedDocument, parse_document};

fn parsed(source: &str) -> ParsedDocument {
    match parse_document("python", source) {
        ParseOutcome::Parsed(document) => document,
        other => panic!("expected valid Python, got {other:?}"),
    }
}

/// Build a stored input declaration equivalent to a Python `(position, prompt)` tuple.
fn stored_input(order: i64, prompt: &str) -> ParamDecl {
    let mut declaration = ParamDecl::new(format!("input-{}", order + 1));
    declaration.binding = ParameterBinding::Input;
    declaration.delivery = ParameterDelivery::Inject;
    declaration.prompt = prompt.to_owned();
    declaration.order = order;
    declaration
}

/// Build a "current" Python source whose input candidates reproduce a list of `(position, prompt)`
/// tuples: position is source order, an empty prompt becomes a bare `input()`.
fn current_source(prompts: &[&str]) -> String {
    prompts
        .iter()
        .enumerate()
        .map(|(index, prompt)| {
            if prompt.is_empty() {
                format!("v{index} = input()\n")
            } else {
                format!("v{index} = input({prompt:?})\n")
            }
        })
        .collect()
}

/// Reconstruct the Python `match_calls` dict `{stored_order: (current_order, ambiguous)}` from the
/// public reconcile report.
fn match_bindings(stored: &[ParamDecl], current: &str) -> BTreeMap<i64, (i64, bool)> {
    let report = parsed(current).reconcile(stored);
    let mut bindings = BTreeMap::new();
    for pair in &report.ok {
        bindings.insert(pair.stored.order, (pair.current.declaration.order, false));
    }
    for pair in &report.rebound {
        bindings.insert(pair.stored.order, (pair.current.declaration.order, true));
    }
    bindings
}

#[test]
fn test_equal_count_duplicate_prompts_bind_in_positional_order() {
    // A retry pattern: two managed input("Go? ") calls, both still present. Stored and current have
    // the SAME number of call sites for the identical prompt, so the multiset pass pairs them in
    // sorted-position order rather than flagging the shape as a rebind on every run. Neither is
    // ambiguous — the pairing is exact.
    let stored = [stored_input(0, "Go? "), stored_input(1, "Go? ")];
    assert_eq!(
        match_bindings(&stored, &current_source(&["Go? ", "Go? "])),
        BTreeMap::from([(0, (0, false)), (1, (1, false))])
    );
}

#[test]
fn test_duplicate_prompt_gone_from_source_falls_back_to_position_ambiguous() {
    // Two stored specs share the identical literal prompt (a retry pair), but that prompt now
    // appears NOWHERE in the current source (both calls were renamed/edited). The multiset pass must
    // not fire (no current call sites carry the prompt), so both entries fall back to bare position
    // and are flagged ambiguous — the prompt vanished, which is exactly the silent-rebind risk the
    // caller must turn into a warning. (Also proves the multiset pass reads an EMPTY candidate list,
    // never a None default, when a duplicated prompt is absent from the current source.)
    let stored = [stored_input(0, "Go? "), stored_input(1, "Go? ")];
    assert_eq!(
        match_bindings(&stored, &current_source(&["Other: ", "Another: "])),
        BTreeMap::from([(0, (0, true)), (1, (1, true))])
    );
}

#[test]
fn test_promptless_entry_cannot_recover_a_site_the_multiset_pass_claimed() {
    // A retry pair input("Go? ") stored at orders 1 and 2, plus a THIRD managed input with no
    // recorded prompt (prompt="") at order 0. In the current source one earlier call was deleted, so
    // the retry pair now sits at current orders 0 and 1; the multiset pass binds the pair there and
    // CLAIMS both those current orders. The prompt-less entry at order 0 has only its bare position
    // to go on — and position 0 is now owned by the retry pair, so it must come back MISSING (absent
    // from the result), never silently recover a call site another definition already claimed.
    let stored = [
        stored_input(1, "Go? "),
        stored_input(2, "Go? "),
        stored_input(0, ""),
    ];
    assert_eq!(
        match_bindings(&stored, &current_source(&["Go? ", "Go? "])),
        BTreeMap::from([(1, (0, false)), (2, (1, false))])
    );
}

#[test]
fn test_unique_prompt_after_a_multiset_match_still_resolves() {
    // Three stored inputs: a retry pair input("A") at orders 0,1 (resolved by the multiset pass) and
    // a distinct input("B") at order 2. In the current source a call was inserted before "B",
    // shifting it from position 2 to 3 while the "A" pair stays at 0,1. The per-entry uniqueness pass
    // must keep scanning PAST the already-resolved multiset entries to bind "B" to its shifted site
    // by prompt — if it stopped at the first resolved entry, "B" would be reported missing even
    // though its prompt uniquely identifies its new position.
    //
    // JUDGMENT CALL: Python passes the sparse current list `[(0,"A"),(1,"A"),(3,"B")]` straight to
    // `match_calls`, but reconcile derives current orders from contiguous source positions. To
    // reproduce "a call inserted before B" (the test's own scenario), an extra `input("Inserted: ")`
    // sits at order 2 so "B" genuinely lands at order 3; that filler is an unmanaged `new` candidate
    // and does not appear in the reconstructed dict.
    let stored = [
        stored_input(0, "A"),
        stored_input(1, "A"),
        stored_input(2, "B"),
    ];
    assert_eq!(
        match_bindings(&stored, &current_source(&["A", "A", "Inserted: ", "B"])),
        BTreeMap::from([(0, (0, false)), (1, (1, false)), (2, (3, false))])
    );
}

#[test]
fn test_single_shared_prompt_resolves_by_uniqueness_not_multiset() {
    // Exactly ONE stored entry carries a given prompt: the multiset pass (which only handles 2+
    // stored sites for one prompt) must leave it alone, and the per-entry uniqueness pass resolves it
    // by following the prompt to its shifted position. A new input() was inserted before the managed
    // one, so its bare position moved 0 -> 1, but the prompt still uniquely identifies it: bound to
    // current order 1, not ambiguous.
    let stored = [stored_input(0, "Password: ")];
    assert_eq!(
        match_bindings(&stored, &current_source(&["Username: ", "Password: "])),
        BTreeMap::from([(0, (1, false))])
    );
}

#[test]
fn test_no_recorded_prompt_falls_back_to_position_silently() {
    // A legacy/dynamic-prompt entry (prompt="") has no stronger signal than position; resolving by
    // position is NOT a new risk introduced by prompt-matching, so it must bind silently
    // (ambiguous=False), preserving pre-prompt behaviour.
    assert_eq!(
        match_bindings(&[stored_input(0, "")], &current_source(&["Anything: "])),
        BTreeMap::from([(0, (0, false))])
    );
}

#[test]
fn test_renamed_prompt_with_a_call_still_at_position_is_flagged_ambiguous() {
    // The stored prompt no longer appears anywhere (renamed), but a call still exists at the stored
    // position: fall back to position AND flag it, so the caller surfaces a rebind warning rather
    // than silently trusting a value onto a different question.
    assert_eq!(
        match_bindings(&[stored_input(0, "Old: ")], &current_source(&["New: "])),
        BTreeMap::from([(0, (0, true))])
    );
}

#[test]
fn test_missing_when_neither_prompt_nor_position_resolves() {
    // The prompt matches nothing and the stored bare position no longer exists either: the entry is
    // genuinely gone and must be absent from the result (the caller reports it missing).
    assert_eq!(
        match_bindings(&[stored_input(2, "Gone: ")], &current_source(&["Other: "])),
        BTreeMap::new()
    );
}

#[test]
fn test_current_site_with_dynamic_prompt_is_ignored_for_prompt_matching() {
    // A current call site with no literal prompt (prompt="", e.g. input(greeting)) carries no text to
    // key on, so it is excluded from the prompt index: the stored "Name: " entry must resolve to the
    // literal-prompt site at order 1, never the dynamic one at order 0.
    let stored = [stored_input(0, "Name: ")];
    assert_eq!(
        match_bindings(&stored, &current_source(&["", "Name: "])),
        BTreeMap::from([(0, (1, false))])
    );
}
