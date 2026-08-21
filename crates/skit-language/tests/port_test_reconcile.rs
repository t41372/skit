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
//! `edit_source_declarations` owns the parser-backed, I/O-free source edit pass. Human warning and
//! drift rendering remain frontend concerns.

use std::collections::BTreeSet;

use skit_domain::parameters::{
    NamedEdit, ParamDecl, ParameterBinding, ParameterType, SourceEditRequest, SourceEditResult,
    SourceEditWarning,
};
use skit_language::{
    LanguageError, ParseOutcome, ReconcileReport, edit_source_declarations, parse_document,
};

/// Python `reconcile.reconcile(text, specs)`: parse, reconcile against the current source, or return
/// the conservative all-missing report when the source has a syntax error.
fn reconcile(source: &str, stored: &[ParamDecl]) -> ReconcileReport {
    match parse_document("python", source) {
        ParseOutcome::Parsed(document) => document.reconcile(stored),
        _ => ReconcileReport::from_syntax_error(stored),
    }
}

fn edit(source: &str, stored: &[ParamDecl], request: SourceEditRequest) -> SourceEditResult {
    edit_source_declarations("python", source, stored, &request).unwrap()
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
        spec(
            "RETRIES",
            ParameterBinding::Const,
            ParameterType::Int,
            -1,
            "",
        ),
        spec(
            "input-1",
            ParameterBinding::Input,
            ParameterType::Str,
            0,
            "",
        ),
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
        &[spec(
            "RETRIES",
            ParameterBinding::Const,
            ParameterType::Int,
            -1,
            "",
        )],
    );
    assert!(report.has_drift());
    assert_eq!(
        report
            .changed
            .iter()
            .map(|pair| (
                pair.stored.name.clone(),
                pair.current.declaration.parameter_type
            ))
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
        &[spec(
            "input-1",
            ParameterBinding::Input,
            ParameterType::Str,
            0,
            "",
        )],
    );
    assert!(!report.has_drift());
}

#[test]
fn test_input_removed_is_missing() {
    let text = SCRIPT.replace("who = input(\"Your name: \")", "who = \"nobody\"");
    let report = reconcile(
        &text,
        &[spec(
            "input-1",
            ParameterBinding::Input,
            ParameterType::Str,
            0,
            "",
        )],
    );
    assert_eq!(missing_names(&report), ["input-1"]);
}

#[test]
fn test_new_input_call_reported_as_new_only() {
    let text = format!("{SCRIPT}more = input(\"More: \")\nprint(more)\n");
    let report = reconcile(
        &text,
        &[spec(
            "input-1",
            ParameterBinding::Input,
            ParameterType::Str,
            0,
            "",
        )],
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
        &[spec(
            "input-1",
            ParameterBinding::Input,
            ParameterType::Str,
            0,
            "Your name: ",
        )],
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
        spec(
            "input-1",
            ParameterBinding::Input,
            ParameterType::Str,
            0,
            "First: ",
        ),
        spec(
            "input-2",
            ParameterBinding::Input,
            ParameterType::Str,
            1,
            "Second: ",
        ),
        spec(
            "input-3",
            ParameterBinding::Input,
            ParameterType::Str,
            2,
            "Third: ",
        ),
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
        &[spec(
            "input-1",
            ParameterBinding::Input,
            ParameterType::Str,
            0,
            "Old label: ",
        )],
    );
    assert!(report.has_drift());
    assert_eq!(rebound_names(&report), ["input-1"]);
    assert_eq!(usable_names(&report), ["input-1"]); // still injectable, just warned
}

#[test]
#[ignore = "UNMAPPED: reconcile.drift_lines has no skit-language equivalent (human-string rendering lives in skit-cli/skit-ui)"]
fn test_drift_lines_mention_rebind() {
    // A syntax-error resync combined with drift-line rendering. drift_lines is a CLI/UI concern.
    let _ = reconcile;
}

#[test]
fn test_resync_reanchors_rebound_input_order_and_prompt() {
    // --resync must not just prune/retype: an input whose prompt no longer uniquely resolves should
    // be re-anchored to wherever the fallback landed, so the *next* plain run sees an exact prompt
    // match again instead of re-deriving the same fallback (and the same warning) every time.
    let source = "value = input(\"New label: \")\nprint(value)\n";
    let result = edit(
        source,
        &[spec(
            "input-1",
            ParameterBinding::Input,
            ParameterType::Str,
            0,
            "Old label: ",
        )],
        SourceEditRequest {
            resync: true,
            ..SourceEditRequest::default()
        },
    );
    assert_eq!(
        result.warnings,
        [SourceEditWarning::ResyncRebound {
            name: "input-1".to_owned()
        }]
    );
    assert_eq!(result.declarations[0].order, 0);
    assert_eq!(result.declarations[0].prompt, "New label: ");
    let report = reconcile(source, &result.declarations);
    assert!(!report.has_drift());
    assert!(report.rebound.is_empty());
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
        spec(
            "input-1",
            ParameterBinding::Input,
            ParameterType::Str,
            0,
            "Go? ",
        ),
        spec(
            "input-2",
            ParameterBinding::Input,
            ParameterType::Str,
            1,
            "Go? ",
        ),
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
        spec(
            "input-1",
            ParameterBinding::Input,
            ParameterType::Str,
            0,
            "Go? ",
        ),
        spec(
            "input-2",
            ParameterBinding::Input,
            ParameterType::Str,
            1,
            "Go? ",
        ),
    ];
    let edited = text.replace(
        "second = input(\"Go? \")",
        "second = input(\"Different: \")",
    );
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

#[test]
#[ignore = "UNMAPPED: reconcile.drift_lines has no skit-language equivalent (human-string rendering lives in skit-cli/skit-ui)"]
fn test_drift_lines_mention_old_and_new_type() {
    // drift_lines renders old-vs-new type text for a warning; that string rendering is a CLI/UI
    // concern, not part of skit-language's ReconcileReport surface.
    let _ = reconcile;
}

// ---------- edit_specs: not-managed warning branches ----------

#[test]
fn test_edit_specs_not_managed_in_secret_warning() {
    // Passing a name that isn't managed into secret= must record a warning, not crash.
    let result = edit(
        "CITY = \"Taipei\"\n",
        &[const_spec("CITY")],
        SourceEditRequest {
            secret: vec!["GONE".to_owned()],
            ..SourceEditRequest::default()
        },
    );
    assert_eq!(
        result.warnings,
        [SourceEditWarning::NotManaged {
            name: "GONE".to_owned()
        }]
    );
}

#[test]
fn test_edit_specs_not_managed_in_no_secret_warning() {
    let result = edit(
        "CITY = \"Taipei\"\n",
        &[const_spec("CITY")],
        SourceEditRequest {
            no_secret: vec!["GONE".to_owned()],
            ..SourceEditRequest::default()
        },
    );
    assert_eq!(
        result.warnings,
        [SourceEditWarning::NotManaged {
            name: "GONE".to_owned()
        }]
    );
}

#[test]
fn test_edit_specs_not_managed_in_prompts_warning() {
    let result = edit(
        "CITY = \"Taipei\"\n",
        &[const_spec("CITY")],
        SourceEditRequest {
            prompts: vec![NamedEdit::new("GONE", "Enter city:")],
            ..SourceEditRequest::default()
        },
    );
    assert_eq!(
        result.warnings,
        [SourceEditWarning::NotManaged {
            name: "GONE".to_owned()
        }]
    );
}

// ---------- Resync must not wipe definitions on a transient syntax error ----------

#[test]
fn test_resync_on_unparseable_script_leaves_definitions_untouched() {
    // A copy-mode script left mid-edit with a syntax error must not have its entire
    // managed-parameter set dropped by --resync. reconcile() can't distinguish "really
    // gone" from "can't parse right now", so _apply_resync must consult report.syntax_error itself.
    let mut api_key = const_spec("API_KEY");
    api_key.secret = true;
    let specs = vec![
        api_key,
        spec(
            "RETRIES",
            ParameterBinding::Const,
            ParameterType::Int,
            -1,
            "",
        ),
        spec(
            "input-1",
            ParameterBinding::Input,
            ParameterType::Str,
            0,
            "",
        ),
    ];
    let result = edit(
        "API_KEY = 'x'\nRETRIES = (3\n",
        &specs,
        SourceEditRequest {
            resync: true,
            ..SourceEditRequest::default()
        },
    );
    assert_eq!(result.declarations, specs);
    assert_eq!(result.warnings, [SourceEditWarning::ResyncSkipped]);
}

#[test]
fn test_resync_syntax_error_does_not_also_apply_other_edits_incorrectly() {
    // A syntax-error resync combined with --remove: the resync guard must only skip the resync
    // step; the rest of edit_specs (remove/add/tweaks) still runs normally on the untouched specs.
    let result = edit(
        "def broken(:\n",
        &[const_spec("CITY"), const_spec("Y")],
        SourceEditRequest {
            resync: true,
            remove: vec!["Y".to_owned()],
            ..SourceEditRequest::default()
        },
    );
    assert_eq!(
        result
            .declarations
            .iter()
            .map(|declaration| declaration.name.as_str())
            .collect::<Vec<_>>(),
        ["CITY"]
    );
    assert_eq!(result.warnings, [SourceEditWarning::ResyncSkipped]);
}

#[test]
#[ignore = "UNMAPPED: reconcile.render_warning has no skit-language equivalent (warning rendering lives in skit-cli/skit-ui)"]
fn test_render_warning_resync_skipped() {
    let _ = reconcile;
}

// ---------- edit_specs must not crash on duplicate-named specs ----------

#[test]
fn test_edit_specs_remove_with_duplicate_names_does_not_crash() {
    // A duplicate-named const used to make `order.remove(name)` leave a dangling name in `order`
    // after `del by_name[name]`, raising KeyError on the final list-comp.
    let result = edit(
        "X = 1\nX = 2\nY = 5\n",
        &[const_spec("X"), const_spec("X"), const_spec("Y")],
        SourceEditRequest {
            remove: vec!["X".to_owned()],
            ..SourceEditRequest::default()
        },
    );
    assert_eq!(
        result
            .declarations
            .iter()
            .map(|declaration| declaration.name.as_str())
            .collect::<Vec<_>>(),
        ["Y"]
    );
    assert!(result.warnings.is_empty());
}

#[test]
fn test_edit_specs_resync_drop_with_duplicate_names_does_not_crash() {
    // Same dangling-name crash, reached via --resync instead of --remove.
    let result = edit(
        "Y = 5\n",
        &[const_spec("X"), const_spec("X"), const_spec("Y")],
        SourceEditRequest {
            resync: true,
            ..SourceEditRequest::default()
        },
    );
    assert_eq!(
        result
            .declarations
            .iter()
            .map(|declaration| declaration.name.as_str())
            .collect::<Vec<_>>(),
        ["Y"]
    );
    assert_eq!(
        result.warnings,
        [SourceEditWarning::ResyncDropped {
            name: "X".to_owned()
        }]
    );
}

#[test]
fn test_edit_specs_dedups_duplicate_names_even_when_untouched() {
    // Duplicate names must never survive edit_specs, even when no operation targets them directly.
    let result = edit(
        "X = 1\nX = 2\nY = 5\n",
        &[const_spec("X"), const_spec("X"), const_spec("Y")],
        SourceEditRequest {
            secret: vec!["Y".to_owned()],
            ..SourceEditRequest::default()
        },
    );
    assert_eq!(
        result
            .declarations
            .iter()
            .map(|declaration| declaration.name.as_str())
            .collect::<Vec<_>>(),
        ["X", "Y"]
    );
}

#[test]
fn test_no_secret_also_clears_the_env_source() {
    // Clearing secret must also clear env_source (an env source only means anything on a secret).
    let mut declaration = const_spec("API");
    declaration.secret = true;
    declaration.env_source = "MY_KEY".to_owned();
    let result = edit(
        "API = \"x\"\nprint(API)\n",
        &[declaration],
        SourceEditRequest {
            no_secret: vec!["API".to_owned()],
            ..SourceEditRequest::default()
        },
    );
    assert!(!result.declarations[0].secret);
    assert!(result.declarations[0].env_source.is_empty());
}

#[test]
fn source_edit_collects_remove_prompt_and_environment_warnings_without_losing_valid_siblings() {
    let mut secret = const_spec("API");
    secret.secret = true;
    let public = const_spec("CITY");
    let result = edit(
        "API = \"x\"\nCITY = \"Taipei\"\n",
        &[secret, public],
        SourceEditRequest {
            remove: vec!["GONE".to_owned()],
            prompts: vec![
                NamedEdit::new("API", "Old: "),
                NamedEdit::new("API", "New: "),
            ],
            env_sources: vec![
                NamedEdit::new("API", " API_TOKEN "),
                NamedEdit::new("CITY", "CITY_TOKEN"),
                NamedEdit::new("GHOST", "GHOST_TOKEN"),
            ],
            ..SourceEditRequest::default()
        },
    );
    assert_eq!(result.declarations[0].prompt, "New: ");
    assert_eq!(result.declarations[0].env_source, "API_TOKEN");
    assert_eq!(
        result.warnings,
        [
            SourceEditWarning::NotManaged {
                name: "GONE".to_owned()
            },
            SourceEditWarning::EnvSourceNotSecret {
                name: "CITY".to_owned()
            },
            SourceEditWarning::EnvSourceNotManaged {
                name: "GHOST".to_owned()
            }
        ]
    );
}

#[test]
fn source_edit_refuses_a_kind_without_a_parser() {
    let error = edit_source_declarations(
        "future-kind",
        "opaque",
        &[const_spec("CITY")],
        &SourceEditRequest::default(),
    )
    .unwrap_err();
    assert_eq!(
        error,
        LanguageError::UnsupportedKind {
            kind: "future-kind".to_owned()
        }
    );
}
