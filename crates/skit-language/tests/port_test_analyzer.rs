//! Mechanical port of the Python oracle module `tests/test_analyzer.py`
//! (`origin/main@206f9ef`). Each `#[test]` keeps its Python `def test_*` name so it
//! traces back to its origin, and the Python "WHY" comment is preserved verbatim above it.
//!
//! Concept mapping used throughout:
//! - Python `analyzer.analyze(src).candidates[i]` -> `parsed(src).analysis().candidates[i].declaration`.
//! - Python `type` strings -> `ParameterType`; Python `default` -> `Option<ParameterValue>`.
//! - Python `binding == "input"/"const"` -> `ParameterBinding::Input/Const`.
//! - Python `c.lineno` -> `candidate.span.start_line`.
//! - Python `result.syntax_error is True` -> `parse_document` returns `ParseOutcome::SyntaxError`.
//! - Python `result.uses_cli_framework` -> derived: `!analysis.frameworks.is_empty()`.
//! - Python `callmatch.match_calls(stored, current)` -> reconstructed from `doc.reconcile(stored)`
//!   (`match_calls` itself is `pub(super)` and not reachable from an integration test). See the
//!   `match_bindings` helper; every callmatch test below carries this semantic-judgment mapping.
//! - Python `shim.inject(src, specs, values)` -> `doc.plan_injection(&decls, &values).apply(src)`.

use std::collections::{BTreeMap, BTreeSet};

use skit_domain::parameters::{
    ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, ParameterValue,
};
use skit_language::{ParseOutcome, ParsedDocument, parse_document, source_is_valid};

fn parsed(source: &str) -> ParsedDocument {
    match parse_document("python", source) {
        ParseOutcome::Parsed(document) => document,
        other => panic!("expected valid Python, got {other:?}"),
    }
}

/// All analyzer candidate declarations in source order.
fn candidates(source: &str) -> Vec<ParamDecl> {
    parsed(source)
        .analysis()
        .candidates
        .into_iter()
        .map(|candidate| candidate.declaration)
        .collect()
}

/// Python `_input_names`: names of input-bound candidates in source order.
fn input_names(source: &str) -> Vec<String> {
    parsed(source)
        .analysis()
        .candidates
        .into_iter()
        .filter(|candidate| candidate.declaration.binding == ParameterBinding::Input)
        .map(|candidate| candidate.declaration.name)
        .collect()
}

/// Candidate declarations keyed by name (Python `{c.name: c for c in candidates}`).
fn by_name(source: &str) -> BTreeMap<String, ParamDecl> {
    parsed(source)
        .analysis()
        .candidates
        .into_iter()
        .map(|candidate| (candidate.declaration.name.clone(), candidate.declaration))
        .collect()
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
/// public reconcile report. A silent match lands in `ok` (ambiguous=false); an ambiguous positional
/// fallback lands in `rebound` (ambiguous=true); an unresolved stored key is absent (Python drops it).
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
fn test_module_level_consts() {
    let src = concat!(
        "CITY = 'Taipei'\n",
        "RETRIES = 3\n",
        "THRESHOLD = -0.5\n",
        "VERBOSE = True\n",
        "_INTERNAL = 'skip me'\n",
        "derived = RETRIES * 2\n", // non-literal, not a candidate
    );
    let names = by_name(src);
    assert_eq!(
        names.keys().cloned().collect::<BTreeSet<_>>(),
        BTreeSet::from([
            "CITY".to_owned(),
            "RETRIES".to_owned(),
            "THRESHOLD".to_owned(),
            "VERBOSE".to_owned(),
        ])
    );
    assert_eq!(names["CITY"].parameter_type, ParameterType::Str);
    assert_eq!(
        names["CITY"].default,
        Some(ParameterValue::String("Taipei".to_owned()))
    );
    assert_eq!(names["RETRIES"].parameter_type, ParameterType::Int);
    assert_eq!(names["RETRIES"].default, Some(ParameterValue::Integer(3)));
    assert_eq!(names["THRESHOLD"].parameter_type, ParameterType::Float);
    assert_eq!(names["THRESHOLD"].default, Some(ParameterValue::Float(-0.5)));
    assert_eq!(names["VERBOSE"].parameter_type, ParameterType::Bool);
    assert_eq!(names["VERBOSE"].default, Some(ParameterValue::Bool(true)));
}

#[test]
fn test_ann_assign_and_bool_not_int() {
    let src = "count: int = 10\nflag: bool = False\n";
    let types = by_name(src)
        .into_iter()
        .map(|(name, declaration)| (name, declaration.parameter_type))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        types,
        BTreeMap::from([
            ("count".to_owned(), ParameterType::Int),
            ("flag".to_owned(), ParameterType::Bool),
        ])
    );
}

#[test]
fn test_main_guard_scanned_c4() {
    let src = concat!(
        "import sys\n",
        "TOP = 1\n",
        "if __name__ == \"__main__\":\n",
        "    GUARD_CONST = 'hello'\n",
        "    TOP = 99\n", // same name: module-level wins, no duplicate
        "    print(GUARD_CONST)\n",
    );
    let names = candidates(src)
        .iter()
        .map(|declaration| declaration.name.clone())
        .collect::<Vec<_>>();
    assert_eq!(names.iter().filter(|name| *name == "TOP").count(), 1);
    assert!(names.iter().any(|name| name == "GUARD_CONST"));
}

#[test]
fn test_main_guard_reversed_form() {
    let src = "if \"__main__\" == __name__:\n    X = 5\n";
    assert_eq!(
        candidates(src)
            .iter()
            .map(|declaration| declaration.name.as_str())
            .collect::<Vec<_>>(),
        ["X"]
    );
}

#[test]
fn test_input_calls_ordered_b1() {
    let src = "name = input(\"Name: \")\ndef f():\n    return input(\"Inner: \")\nage = input()\n";
    let inputs = candidates(src)
        .into_iter()
        .filter(|declaration| declaration.binding == ParameterBinding::Input)
        .collect::<Vec<_>>();
    assert_eq!(
        inputs.iter().map(|input| input.order).collect::<Vec<_>>(),
        [0, 1, 2]
    );
    assert_eq!(inputs[0].prompt, "Name: ");
    assert_eq!(inputs[1].prompt, "Inner: ");
    assert_eq!(inputs[2].prompt, "");
    assert_eq!(inputs[0].name, "input-1");
}

#[test]
fn test_secret_heuristics() {
    let src = "API_KEY = \"x\"\ntoken = \"y\"\npw = input(\"Password: \")\nCITY = \"z\"\n";
    let by_name = by_name(src);
    assert!(by_name["API_KEY"].secret);
    assert!(by_name["token"].secret);
    assert!(!by_name["CITY"].secret);
    assert!(by_name["input-1"].secret); // prompt contains "Password"
}

#[test]
fn test_framework_detection() {
    assert_eq!(parsed("import argparse\n").analysis().frameworks, ["argparse"]);
    assert_eq!(
        parsed("from click import command\n").analysis().frameworks,
        ["click"]
    );
    assert_eq!(
        parsed("import typer\nimport click\n").analysis().frameworks,
        ["typer", "click"]
    );
    // Python `uses_cli_framework is False` -> derived from an empty framework list.
    assert!(parsed("import os\n").analysis().frameworks.is_empty());
}

#[test]
fn test_syntax_error_returns_empty() {
    // Python exposes `result.syntax_error` and an empty candidate list. The Rust surface reports
    // invalid syntax as a distinct `ParseOutcome::SyntaxError` variant (no parsed document, so no
    // candidates), which is the faithful mapping of both assertions.
    assert!(matches!(
        parse_document("python", "def broken(:\n"),
        ParseOutcome::SyntaxError(_)
    ));
}

// ---------- duplicate top-level const names (corrupted/wrong injected run) ----------

#[test]
fn test_duplicate_top_level_const_is_deduped_to_one_candidate() {
    // A name bound twice at module top level (e.g. from hand-editing) must yield exactly one
    // candidate, not two: two same-named ParamDecls made the shim compute and apply the same
    // replacement span twice (see shim.inject), corrupting the injected source.
    let src = "CITY = 'a'\nCITY = 'b'\nprint(CITY)\n";
    let names = candidates(src)
        .iter()
        .map(|declaration| declaration.name.clone())
        .collect::<Vec<_>>();
    assert_eq!(names.iter().filter(|name| *name == "CITY").count(), 1);
}

#[test]
fn test_duplicate_top_level_const_keeps_last_occurrence_value() {
    // Module top-level execution is sequential, so by the time the script finishes running, CITY
    // holds 'b' (the second assignment), not 'a'. The kept candidate's type/default must reflect
    // that runtime-effective value, or the onboarding form default and the injected type would
    // disagree with what the script actually does when left unmanaged.
    let src = "N = 1\nN = 2\nprint(N)\n";
    let named = candidates(src)
        .into_iter()
        .filter(|declaration| declaration.name == "N")
        .collect::<Vec<_>>();
    assert_eq!(named.len(), 1);
    let cand = &named[0];
    assert_eq!(cand.default, Some(ParameterValue::Integer(2)));
    assert_eq!(cand.parameter_type, ParameterType::Int);
}

#[test]
fn test_duplicate_top_level_const_keeps_first_occurrence_position() {
    // Display/onboarding order should still read top-to-bottom like the source: the de-duplicated
    // candidate keeps the *first* occurrence's slot even though its value comes from the last one.
    let src = "X = 1\nY = 5\nX = 2\n";
    let names = candidates(src)
        .iter()
        .map(|declaration| declaration.name.clone())
        .collect::<Vec<_>>();
    let index_of = |target: &str| names.iter().position(|name| name == target).unwrap();
    assert!(index_of("X") < index_of("Y"));
}

#[test]
fn test_duplicate_top_level_const_mixed_ann_assign() {
    let src = "X: int = 1\nX = 2\n";
    let all = candidates(src);
    assert_eq!(all.iter().filter(|declaration| declaration.name == "X").count(), 1);
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].default, Some(ParameterValue::Integer(2)));
}

#[test]
fn test_duplicate_const_injection_no_longer_corrupts_source() {
    // A valid script with a duplicate top-level const used to become unparseable (str case) or
    // silently run with the wrong value (int case) once
    // injected. With a single deduped candidate/spec, shim replaces every same-named occurrence
    // exactly once and the result stays valid and correct.
    let src = "CITY = 'a'\nCITY = 'b'\nprint(CITY)\n";
    let document = parsed(src);
    let declarations = document
        .analysis()
        .candidates
        .into_iter()
        .map(|candidate| candidate.declaration)
        .collect::<Vec<_>>();
    assert_eq!(declarations.len(), 1);
    let injected = document
        .plan_injection(
            &declarations,
            &BTreeMap::from([("CITY".to_owned(), "Paris".to_owned())]),
        )
        .unwrap()
        .apply(src)
        .unwrap();
    assert_eq!(injected, "CITY = 'Paris'\nCITY = 'Paris'\nprint(CITY)\n");
    assert!(source_is_valid("python", &injected)); // must still be valid Python
}

// ---------- shadowed `input`: a binding disables detection in ITS scope ----------

#[test]
fn test_shadowed_input_via_def_yields_no_input_candidates() {
    // `def input(...)` binds the name `input`: every input() call in the file reaches THAT
    // function, not the builtin prompt — so there is nothing for skit to manage.
    assert_eq!(
        input_names("def input(prompt=''):\n    return 'x'\nname = input('Name: ')\n"),
        Vec::<String>::new()
    );
}

#[test]
fn test_shadowed_input_via_assignment_yields_no_input_candidates() {
    // A plain reassignment `input = <something>` is a binding too.
    assert_eq!(
        input_names("input = str\nname = input('Name: ')\n"),
        Vec::<String>::new()
    );
}

#[test]
fn test_shadowed_input_via_from_import_yields_no_input_candidates() {
    // `from mod import input` binds the name to the imported object.
    assert_eq!(
        input_names("from mymod import input\nname = input('Name: ')\n"),
        Vec::<String>::new()
    );
}

#[test]
fn test_shadowed_input_via_plain_import_yields_no_input_candidates() {
    // Even `import input` (a module literally named input) binds the name — a weird but real
    // binding that the file-wide guard must still catch.
    assert_eq!(
        input_names("import input\nname = input('Name: ')\n"),
        Vec::<String>::new()
    );
}

#[test]
fn test_function_parameter_named_input_does_not_shadow_the_module_level_call() {
    // A function PARAMETER named `input` binds it only INSIDE that function — `input` is a
    // common parameter name, and a file-wide rule would strip the managed prompt off every
    // entry whose script happens to contain one, turning its next run into definition drift.
    assert_eq!(
        input_names("def f(input):\n    return input\nname = input('Name: ')\n"),
        ["input-1"]
    );
}

#[test]
fn test_call_inside_the_shadowing_function_is_not_a_candidate() {
    // The other half of the same rule: within the function that binds it, `input(...)` really
    // is the parameter, so that call site must stay unmanaged while the module-level one below
    // is still detected.
    let src = "def f(input):\n    return input('inner')\nname = input('Name: ')\n";
    let lines = parsed(src)
        .analysis()
        .candidates
        .iter()
        .filter(|candidate| candidate.declaration.binding == ParameterBinding::Input)
        .map(|candidate| candidate.span.start_line)
        .collect::<Vec<_>>();
    assert_eq!(lines, [3]);
}

#[test]
fn test_local_assignment_shadows_only_its_own_function() {
    let src = "def f():\n    input = str\n    return input('inner')\nname = input('Name: ')\n";
    let lines = parsed(src)
        .analysis()
        .candidates
        .iter()
        .filter(|candidate| candidate.declaration.binding == ParameterBinding::Input)
        .map(|candidate| candidate.span.start_line)
        .collect::<Vec<_>>();
    assert_eq!(lines, [4]);
}

#[test]
fn test_module_level_binding_still_shadows_calls_nested_in_functions() {
    // A module-scope binding reaches into every function below it (closures see it), so a call
    // inside one is NOT the builtin either.
    assert_eq!(
        input_names("input = str\ndef f():\n    return input('inner')\n"),
        Vec::<String>::new()
    );
}

#[test]
fn test_comprehension_and_lambda_bindings_stay_local() {
    let src = "xs = [input for input in range(3)]\ng = lambda input: input\nname = input('Name: ')\n";
    assert_eq!(input_names(src), ["input-1"]);
}

#[test]
fn test_shadowing_input_does_not_suppress_const_detection() {
    // The guard nukes ONLY input candidates: literal const assignments in the same file are still
    // detected, so it must not short-circuit the whole analysis.
    let src = "def input(p=''):\n    return 'x'\nCITY = 'Taipei'\nname = input('Name: ')\n";
    let all = candidates(src);
    assert_eq!(
        all.iter()
            .filter(|declaration| declaration.binding == ParameterBinding::Input)
            .map(|declaration| declaration.name.as_str())
            .collect::<Vec<_>>(),
        Vec::<&str>::new()
    );
    assert_eq!(
        all.iter()
            .filter(|declaration| declaration.binding == ParameterBinding::Const)
            .map(|declaration| declaration.name.as_str())
            .collect::<Vec<_>>(),
        ["CITY"]
    );
}

#[test]
fn test_unshadowed_input_is_still_detected() {
    // Control: with no binding of `input`, the builtin call is a candidate as before — the guard
    // must not fire unconditionally.
    assert_eq!(input_names("name = input('Name: ')\n"), ["input-1"]);
}

// ---------- callmatch.match_calls: prompt-keyed input matching ----------

#[test]
fn test_match_inputs_prompt_survives_position_shift() {
    // A source edit inserted a new input() call before the stored one, shifting its bare position
    // from 0 to 1 -- but the prompt text is unchanged, so it must still resolve correctly, and not
    // be flagged (ambiguous=False): this is exactly the "no silent rebind" case working as intended.
    let stored = [stored_input(0, "Password: ")];
    let bindings = match_bindings(&stored, &current_source(&["Username: ", "Password: "]));
    assert_eq!(bindings, BTreeMap::from([(0, (1, false))]));
}

#[test]
fn test_match_inputs_falls_back_to_position_when_no_prompt_recorded() {
    // Legacy/dynamic-prompt entries (prompt="") have no stronger signal than position, and that's
    // not a newly introduced risk, so it resolves silently (ambiguous=False), matching the
    // previous positional behavior.
    let stored = [stored_input(0, "")];
    let bindings = match_bindings(&stored, &current_source(&["Anything: "]));
    assert_eq!(bindings, BTreeMap::from([(0, (0, false))]));
}

#[test]
fn test_match_inputs_flags_ambiguous_when_prompt_renamed_but_position_still_exists() {
    // The stored prompt no longer appears anywhere in the current source (renamed), but a call
    // still exists at the stored position: fall back to position, but flag it -- the caller must
    // surface a warning rather than silently trusting it.
    let stored = [stored_input(0, "Old prompt: ")];
    let bindings = match_bindings(&stored, &current_source(&["New prompt: "]));
    assert_eq!(bindings, BTreeMap::from([(0, (0, true))]));
}

#[test]
fn test_match_inputs_flags_ambiguous_when_two_call_sites_share_a_prompt() {
    // Two distinct call sites with the identical literal prompt text can't be told apart by prompt
    // alone; falling back to position is still flagged as ambiguous rather than silently trusted.
    let stored = [stored_input(0, "Value: ")];
    let bindings = match_bindings(&stored, &current_source(&["Value: ", "Value: "]));
    assert_eq!(bindings, BTreeMap::from([(0, (0, true))]));
}

#[test]
fn test_match_inputs_missing_when_neither_prompt_nor_position_resolves() {
    let stored = [stored_input(2, "Gone: ")];
    let bindings = match_bindings(&stored, &current_source(&["Other: "]));
    assert_eq!(bindings, BTreeMap::new());
}

// ---------- callmatch.match_calls: duplicate STORED prompts must never map two-to-one (regression) ----------

#[test]
fn test_match_inputs_duplicate_stored_prompts_never_double_bind_on_delete() {
    // Two stored specs shared the identical literal prompt (a retry pattern: two input("Go? ")
    // calls, both managed). The user deletes one of the two calls, leaving a single current call
    // site with that prompt. The first-listed stored entry wins the exact match; the second must
    // NOT also resolve to that same current order (that would corrupt the injected copy) -- its
    // bare position (1) no longer exists either, so it must come back missing entirely.
    let stored = [stored_input(0, "Go? "), stored_input(1, "Go? ")];
    let bindings = match_bindings(&stored, &current_source(&["Go? "]));
    assert_eq!(bindings, BTreeMap::from([(0, (0, false))]));
    // Explicit invariant: no two stored keys ever resolve to the same current order.
    let resolved = bindings
        .values()
        .map(|(current_order, _)| *current_order)
        .collect::<Vec<_>>();
    assert_eq!(resolved.len(), resolved.iter().collect::<BTreeSet<_>>().len());
}

#[test]
fn test_match_inputs_duplicate_stored_prompts_edit_one_flags_rebind_for_loser() {
    // Same duplicate-prompt setup, but this time the call at position 1 still exists -- its prompt
    // was just edited to something else. The losing stored entry can't get an exact match (its
    // prompt's one candidate was already claimed by the winner), so it falls back to bare position
    // 1, which now holds a *different* question -- that must be flagged ambiguous (rebind), never
    // silently trusted and never double-bound onto position 0.
    let stored = [stored_input(0, "Go? "), stored_input(1, "Go? ")];
    let bindings = match_bindings(&stored, &current_source(&["Go? ", "Different: "]));
    assert_eq!(bindings, BTreeMap::from([(0, (0, false)), (1, (1, true))]));
    let resolved = bindings
        .values()
        .map(|(current_order, _)| *current_order)
        .collect::<Vec<_>>();
    assert_eq!(resolved.len(), resolved.iter().collect::<BTreeSet<_>>().len());
}

#[test]
fn test_match_inputs_triple_duplicate_stored_prompts_only_one_winner() {
    // Three stored specs share one prompt; only one current call site remains. Exactly one stored
    // entry may claim it; the other two must come back missing (their bare positions 1 and 2 don't
    // exist in the current source either) -- never sharing the winner's current order.
    let stored = [
        stored_input(0, "Go? "),
        stored_input(1, "Go? "),
        stored_input(2, "Go? "),
    ];
    let bindings = match_bindings(&stored, &current_source(&["Go? "]));
    assert_eq!(bindings, BTreeMap::from([(0, (0, false))]));
    let resolved = bindings
        .values()
        .map(|(current_order, _)| *current_order)
        .collect::<Vec<_>>();
    assert_eq!(resolved.len(), resolved.iter().collect::<BTreeSet<_>>().len());
}

#[test]
fn test_match_capture_named_input_shadows_only_its_own_scope() {
    // The pattern-capture binding forms count too — a `case {**input}` rest-capture and a
    // `case [*input]` star-capture bind the name where they appear, like any assignment.
    let mapping = "def f(d):\n    match d:\n        case {**input}:\n            return input\nname = input('Name: ')\n";
    let star = "def g(d):\n    match d:\n        case [*input]:\n            return input\nname = input('Name: ')\n";
    // Each pattern alone, so neither arm can be masked by the other short-circuiting first.
    assert_eq!(input_names(mapping), ["input-1"]);
    assert_eq!(input_names(star), ["input-1"]);
}

#[test]
fn test_except_handler_named_input_shadows_only_its_own_scope() {
    let src = "def f():\n    try:\n        pass\n    except ValueError as input:\n        return input\nname = input('Name: ')\n";
    assert_eq!(input_names(src), ["input-1"]);
}

#[test]
fn test_dotted_import_binds_only_its_top_level_name() {
    // `import input.sub` binds `input`, so the module-level call is that module, not the
    // builtin — the split(".")[0] the binding scan does.
    assert_eq!(
        input_names("import input.sub\nname = input('Name: ')\n"),
        Vec::<String>::new()
    );
    // ...and a dotted import of anything else leaves the builtin alone.
    assert_eq!(
        input_names("import os.path\nname = input('Name: ')\n"),
        ["input-1"]
    );
}

#[test]
fn test_star_import_is_treated_as_possibly_binding_input() {
    // A star import can bind any public name, `input` included; nothing static can rule it
    // out, so the scope it appears in stops offering input candidates.
    assert_eq!(
        input_names("from mymod import *\nname = input('Name: ')\n"),
        Vec::<String>::new()
    );
}

#[test]
fn test_one_shadowing_scope_does_not_stop_the_scan_of_the_others() {
    // The shadow check skips THAT scope, not the rest of the walk: a script with a helper
    // that binds `input` and another that calls it must still surface the second one.
    let src = concat!(
        "def a(input):\n    return input\n",
        "def b():\n    return input('B: ')\n",
        "def c(input):\n    return input\n",
        "def d():\n    return input('D: ')\n",
    );
    assert_eq!(input_names(src), ["input-1", "input-2"]);
}
