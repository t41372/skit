//! Mechanical port of the Python oracle module `tests/test_source_default_semantics.py`
//! (`origin/main@206f9ef`): "Source-default tracking: the script is the truth, not the
//! `[tool.skit]` cache." Each `#[test]` keeps its Python `def test_*` name so it traces back
//! to its origin, and each Python "WHY" comment is preserved above it.
//!
//! Concept mapping used throughout:
//! - Python `flows.plan_for_entry(entry)` -> `skit_form::form_plan(kind, text, &settings)`
//!   (the Rust form layer takes kind/text/settings, not a filesystem `Entry`; no chdir, no I/O).
//! - Python `flows.FormField` (per-field render model) -> `skit_form::PreparedField`, which keeps
//!   the effective `ParamDecl` instead of a string-rendered `default`. So Python `field.default ==
//!   "bonjour"` maps to `field.declaration.default == Some(ParameterValue::String("bonjour"))` and
//!   Python `field.key`/`field.source` map to `field.declaration.name`/`.delivery`.
//! - Python `flows.FormField.delivers_empty` -> `PreparedField::delivers_empty`.
//! - Python `flows.FormField.from_decl(d).input_binding` -> `PreparedField.input_binding`
//!   (the Rust constructor is private, so this is observed through a real `form_plan`).
//! - Python `analysis.reconcile(text, specs, analyze=...)` -> `parse_document(kind, text)` then
//!   `ParsedDocument::reconcile(&specs)` (the analyzer is the document's own parser, not an
//!   injected callable). `Report.ok` -> `ReconcileReport.ok` (a `Vec` of `{stored, current}`
//!   pairs); `Report.current_defaults` -> `ReconcileReport.current_defaults` (typed
//!   `ParameterValue`, not a string-rendered scalar).
//!
//! Buckets:
//! - Bucket 1 (real asserting tests): the plan-level source refresh and the reconcile
//!   `current_defaults` machinery — the module's thesis — plus the `delivers_empty` matrix and the
//!   `input_binding` flag. Eight tests, all reachable from this crate (`skit-form` +->
//!   `skit-language` + `skit-domain`).
//! - Bucket 2 (white-box, no public seam): the two synthetic-`analyze` reconcile-guard tests. The
//!   guard IS present (`skit-language` `reconcile_analysis`), but the Python API's injectable
//!   analyzer has no Rust equivalent, and a real analyzer never emits a defaultless matched const
//!   (the oracle says so). `#[ignore]`, `kind=absent` (the injection seam, not the behavior).
//! - Bucket 3 (cross-crate): `assemble` (split into `skit-application` `run_inputs` token
//!   expansion + `delivery::assemble` routing), `remembered_values`/`save_after_run`
//!   (`skit-application` `form_state` + the `skit-store` state adapter), and `edit_specs --resync`
//!   (no pure library equivalent; the only analog is `skit-cli` private `prepare_source_management`,
//!   which also diverges — see the resync stubs). `#[ignore]` stubs naming the owning tier.

use std::collections::BTreeMap;

use skit_domain::EntrySettings;
use skit_domain::parameters::{
    ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, ParameterValue,
};
use skit_form::{FormSource, PreparedField, form_plan};
use skit_language::{ParseOutcome, parse_document};

// Block default "hello" is a stale manage-time cache; the body now says "bonjour". The
// script is the truth: the run form must prefill "bonjour", and injecting "bonjour" is a
// no-op the delivery path skips.
const REFRESH_SCRIPT: &str = "# /// script\n\
# dependencies = []\n\
#\n\
# [tool.skit]\n\
# schema = 1\n\
#\n\
# [[tool.skit.params]]\n\
# name = \"GREETING\"\n\
# kind = \"const\"\n\
# type = \"str\"\n\
# default = \"hello\"\n\
# ///\n\
GREETING = 'bonjour'\n\
print(GREETING)\n";

// Shell envdefault whose block default (9999) went stale; the body reads ${PORT:-8080}.
const SHELL_ENVDEFAULT_SCRIPT: &str = "#!/usr/bin/env bash\n\
# /// script\n\
# [tool.skit]\n\
# schema = 1\n\
#\n\
# [[tool.skit.params]]\n\
# name = \"PORT\"\n\
# kind = \"envdefault\"\n\
# type = \"int\"\n\
# default = 9999\n\
# ///\n\
echo \"${PORT:-8080}\"\n";

/// Python `ParamDecl(name=…, binding="const", type=…)` — delivery is irrelevant to reconcile,
/// which matches a const by binding and name.
fn const_decl(name: &str, parameter_type: ParameterType) -> ParamDecl {
    let mut declaration = ParamDecl::new(name);
    declaration.binding = ParameterBinding::Const;
    declaration.delivery = ParameterDelivery::Inject;
    declaration.parameter_type = parameter_type;
    declaration
}

/// Python `_envdefault(name)`: `ParamDecl(name=…, binding="envdefault", delivery="env", type="str")`.
fn envdefault_decl(name: &str) -> ParamDecl {
    let mut declaration = ParamDecl::new(name);
    declaration.binding = ParameterBinding::EnvDefault;
    declaration.delivery = ParameterDelivery::Env;
    declaration.parameter_type = ParameterType::Str;
    declaration
}

// --------------------------------------------------------------------------
// 1) plan_for_entry: the SOURCE's current default beats a stale block cache
// --------------------------------------------------------------------------

#[test]
fn test_plan_refreshes_a_stale_block_default_from_the_python_body() {
    // Block says default = "hello"; the body assigns "bonjour". The form field must carry
    // the body's value, not the cache.
    let plan = form_plan("python", REFRESH_SCRIPT, &EntrySettings::default());
    assert_eq!(plan.source, FormSource::Inject);
    assert_eq!(plan.fields.len(), 1);
    let field = &plan.fields[0];
    assert_eq!(field.declaration.name, "GREETING");
    // the script wins over the stale "hello"
    assert_eq!(
        field.declaration.default,
        Some(ParameterValue::String("bonjour".to_owned()))
    );
    assert!(field.declaration.default.is_some());
}

#[test]
fn test_plan_refreshes_a_stale_shell_envdefault_from_the_body() {
    // Same rule through the shell analyzer: block default 9999 is stale, ${PORT:-8080} is
    // the truth. The env-delivered field prefills "8080".
    let plan = form_plan("shell", SHELL_ENVDEFAULT_SCRIPT, &EntrySettings::default());
    assert_eq!(plan.source, FormSource::Inject);
    assert_eq!(plan.fields.len(), 1);
    let field = &plan.fields[0];
    assert_eq!(field.declaration.name, "PORT");
    assert_eq!(field.declaration.delivery, ParameterDelivery::Env);
    // refreshed from ${PORT:-8080}, not 9999
    assert_eq!(
        field.declaration.default,
        Some(ParameterValue::Integer(8080))
    );
    assert!(field.declaration.default.is_some());
}

// --------------------------------------------------------------------------
// 2) reconcile: current_defaults for ok const/envdefault, not for type-changed
// --------------------------------------------------------------------------

#[test]
fn test_reconcile_records_current_default_for_an_ok_const() {
    let ParseOutcome::Parsed(document) =
        parse_document("python", "CITY = \"Taipei\"\nprint(CITY)\n")
    else {
        panic!("python source must parse");
    };
    let report = document.reconcile(&[const_decl("CITY", ParameterType::Str)]);
    let ok_names = report
        .ok
        .iter()
        .map(|pair| pair.stored.name.clone())
        .collect::<Vec<_>>();
    assert_eq!(ok_names, ["CITY"]);
    let mut expected = BTreeMap::new();
    expected.insert(
        "CITY".to_owned(),
        ParameterValue::String("Taipei".to_owned()),
    );
    assert_eq!(report.current_defaults, expected);
}

#[test]
fn test_reconcile_records_current_default_for_an_ok_envdefault() {
    // Shell envdefault: the source's fallback (int 8080) is the recorded current default.
    let ParseOutcome::Parsed(document) = parse_document("shell", "echo \"${PORT:-8080}\"\n") else {
        panic!("shell source must parse");
    };
    let report = document.reconcile(&[envdefault_decl("PORT")]);
    let ok_names = report
        .ok
        .iter()
        .map(|pair| pair.stored.name.clone())
        .collect::<Vec<_>>();
    assert_eq!(ok_names, ["PORT"]);
    let mut expected = BTreeMap::new();
    expected.insert("PORT".to_owned(), ParameterValue::Integer(8080));
    assert_eq!(report.current_defaults, expected);
}

#[test]
fn test_reconcile_omits_current_default_for_a_type_changed_const() {
    // Block says int, the source now holds a string literal: this is drift (report.changed),
    // so the stale prefill is kept until the user resyncs — NOT tracked in current_defaults.
    let ParseOutcome::Parsed(document) =
        parse_document("python", "RETRIES = \"three\"\nprint(RETRIES)\n")
    else {
        panic!("python source must parse");
    };
    let report = document.reconcile(&[const_decl("RETRIES", ParameterType::Int)]);
    let changed_names = report
        .changed
        .iter()
        .map(|pair| pair.stored.name.clone())
        .collect::<Vec<_>>();
    assert_eq!(changed_names, ["RETRIES"]);
    // a type-changed spec is excluded
    assert!(report.current_defaults.is_empty());
}

// --------------------------------------------------------------------------
// 3) edit_specs --resync writes the refreshed default back into the record
// --------------------------------------------------------------------------

#[test]
#[ignore = "CROSS-CRATE (skit-cli): the pure `analysis.edit_specs(text, specs, resync=True)` has \
no library equivalent; the only analog is private `prepare_source_management` (crates/skit-cli/\
src/cli.rs:3559). Its behavior overlaps here (CITY ok -> default refreshed to \"Taipei\"; RETRIES \
type-changed -> takes type+default from candidate), but it is unreachable from a library test."]
fn test_resync_writes_source_default_into_ok_and_type_changed_specs() {
    // One resync exercises both write paths: an ok const's default follows the source, and
    // a type-changed const takes BOTH its type and its default from the candidate.
    //   text  = 'CITY = "Taipei"\nRETRIES = "three"\nprint(CITY, RETRIES)\n'
    //   specs = [const CITY str default "old-city", const RETRIES int default 3]
    //   result = analysis.edit_specs(text, specs, resync=True)
    //   by["CITY"].(type, default)    == ("str", "Taipei")  # ok: default refreshed
    //   by["RETRIES"].(type, default) == ("str", "three")   # changed: type+default
}

#[test]
#[ignore = "CROSS-CRATE (skit-cli) + DIVERGENCE: the resync rebind semantics exist nowhere in \
Rust. `prepare_source_management` (crates/skit-cli/src/cli.rs:3579-3597) keeps the stored prompt \
on a name-matched input (`if !current.prompt.is_empty() { candidate.prompt = current.prompt; }`) \
and emits no warnings, so it would keep input-2's prompt \"Old label: \" and never produce \
\"resync-rebound:input-2\". The oracle re-anchors input-2 by position to \"New label: \"."]
fn test_resync_current_default_and_rebind_and_untouched_input_share_one_pass() {
    // The resync elif chain, exercised end to end in one call:
    //   CITY    -> current_defaults elif (its literal moved) -> default "Taipei"
    //   input-1 -> exact prompt match, falls through untouched -> (order 0, "Name: ")
    //   input-2 -> its prompt no longer resolves, re-anchored by position (rebind) ->
    //              (order 1, "New label: "), and "resync-rebound:input-2" in result.warnings
}

#[test]
#[ignore = "ABSENT (injection seam, not behavior): the guard IS present — skit-language \
`reconcile_analysis` records a default only under `!declaration.secret && let Some(default)` \
(crates/skit-language/src/semantic.rs:2428-2434). What is absent is the Python API's injectable \
`analyze` callable; `ParsedDocument::reconcile` always runs the real parser, and the oracle notes \
a real analyzer never emits a defaultless const, so no production change is implied."]
fn test_reconcile_ok_const_without_a_default_is_not_recorded() {
    // A matched ok const whose candidate carries no default (default is None) must not be
    // written into current_defaults — the `if cand.default is not None` guard. Real analyzers
    // always give a const a literal, so the oracle drives this through a synthetic analyze:
    //   analyze(_) -> Analysis([Candidate(binding="const", name="X", type="str", default=None)])
    //   report = reconcile("_\n", [const X str], analyze=analyze)
    //   report.ok names == ["X"]; report.current_defaults == {}
}

#[test]
#[ignore = "ABSENT (injection seam, not behavior): envdefault twin of the guard above. The \
recording guard is present in skit-language `reconcile_analysis` (semantic.rs:2428-2434); the \
injectable `analyze` callable that would feed a defaultless env candidate has no Rust equivalent \
(`ParsedDocument::reconcile` runs the real parser)."]
fn test_reconcile_ok_envdefault_without_a_default_is_not_recorded() {
    // The envdefault twin of the guard: an ok env match with a None default records nothing
    // (the value arrives by env either way):
    //   analyze(_) -> Analysis([Candidate(binding="envdefault", name="PORT",
    //                                      env_name="PORT", type="str", default=None)])
    //   report = reconcile("_\n", [envdefault PORT], analyze=analyze)
    //   report.ok names == ["PORT"]; report.current_defaults == {}
}

// --------------------------------------------------------------------------
// 4) assemble: a value that equals the source default is not injected
// --------------------------------------------------------------------------

#[test]
#[ignore = "CROSS-CRATE (skit-application): Python `flows.assemble(plan, values, …, cwd, env, now)` \
was SPLIT — token/glob expansion moved to `run_inputs` (crates/skit-application/src/run_inputs.rs) \
and delivery routing to `delivery::assemble` (crates/skit-application/src/delivery.rs). This test \
needs the value-preparation seam, unreachable from skit-form."]
fn test_assemble_injects_a_value_that_equals_the_source_default() {
    // Whatever the form shows IS what the script gets: a value equal to the source's own
    // literal is still injected, so the run matches the form.
    //   plan = plan_for_entry(REFRESH_SCRIPT)   # GREETING default "bonjour"
    //   equal   = assemble(plan, {"GREETING": "bonjour"}, [], cwd, env={}, now=NOW)
    //   equal.inject_values == {"GREETING": "bonjour"}; equal.display == [("GREETING","bonjour")]
    //   changed = assemble(plan, {"GREETING": "other"}, [], cwd, env={}, now=NOW)
    //   changed.inject_values == {"GREETING": "other"}; changed.display == [("GREETING","other")]
}

#[test]
#[ignore = "CROSS-CRATE (skit-application): token expansion lives in `run_inputs` \
(crates/skit-application/src/run_inputs.rs), unreachable from skit-form; `delivery::assemble` never \
expands `{today}`."]
fn test_assemble_injects_the_expansion_of_an_untouched_token_default() {
    // The token preview the form shows must be what lands: an untouched default carrying
    // {today} delivers the EXPANDED text, never the literal braces.
    //   default = "out_{today}.csv"; body GREETING = 'out_{today}.csv'
    //   asm = assemble(plan, {"GREETING": "out_{today}.csv"}, [], now=datetime(2026,7,9,…))
    //   asm.inject_values == {"GREETING": "out_2026-07-09.csv"}
}

// --------------------------------------------------------------------------
// 5) delivers_empty: a cleared free-text field delivers '' across all sources
// --------------------------------------------------------------------------

#[test]
#[ignore = "CROSS-CRATE (skit-application): delivery routing is `delivery::assemble` \
(crates/skit-application/src/delivery.rs); a delivers-empty inject field writes '' and displays \
'' at delivery.rs:98-106, unreachable from skit-form. The `delivers_empty` predicate itself is \
covered by `test_delivers_empty_matrix` in this file."]
fn test_assemble_inject_delivers_empty_string_when_cleared() {
    // A str const with a known default is WYSIWYG: clearing it delivers '' (an empty string
    // is a legitimate value), shown as '' in the transparency display.
    //   asm = assemble(plan, {"GREETING": ""}, …)
    //   asm.inject_values == {"GREETING": ""}; ("GREETING", "''") in asm.display
}

#[test]
#[ignore = "CROSS-CRATE (skit-application): env routing is `delivery::assemble` \
(crates/skit-application/src/delivery.rs:108-117), which exports a delivers-empty env field set to \
\"\"; unreachable from skit-form."]
fn test_assemble_env_delivers_empty_string_when_cleared() {
    // An env-delivered free-text field with a default exports the variable set to "" when
    // cleared (the ${NAME:-default} script still falls back; a ${NAME-default} one gets '').
    //   plan = FormPlan(inject, [FormField(CITY, source="env", str, has_default, default="Taipei")])
    //   asm = assemble(plan, {"CITY": ""}, …); asm.env_values == {"CITY": ""}
}

#[test]
#[ignore = "CROSS-CRATE (skit-application): flag routing is `delivery::assemble` \
(crates/skit-application/src/delivery.rs:186-212); a delivers-empty flag emits `--x ''` instead of \
omitting it; unreachable from skit-form."]
fn test_assemble_flag_delivers_empty_string_when_cleared() {
    // A free-text flag with a default emits `--x ''` when cleared, instead of omitting it.
    //   plan = FormPlan(argparse, [FormField(x, source="flag", flag="--x", str, default="def")])
    //   asm = assemble(plan, {"x": ""}, …); asm.args == ["--x", ""]
}

// --------------------------------------------------------------------------
// 6) delivers_empty is False everywhere WYSIWYG is unsound
// --------------------------------------------------------------------------

/// Python matrix helper: an inject-delivered `FormField` with tunable disqualifiers. Source is
/// always "inject" in the oracle matrix. Only `default.is_some()` matters, so the value is a
/// placeholder.
fn matrix_field(
    parameter_type: ParameterType,
    has_default: bool,
    secret: bool,
    degraded: bool,
    multiple: bool,
    input_binding: bool,
) -> PreparedField {
    let mut declaration = ParamDecl::new("k");
    declaration.delivery = ParameterDelivery::Inject;
    declaration.parameter_type = parameter_type;
    declaration.secret = secret;
    declaration.degraded = degraded;
    declaration.multiple = multiple;
    if has_default {
        declaration.default = Some(ParameterValue::String("x".to_owned()));
    }
    PreparedField {
        declaration,
        input_binding,
        empty_uses_default: false,
    }
}

#[test]
fn test_delivers_empty_matrix() {
    // WYSIWYG applies to exactly one shape: a non-secret, single-value, free-text (str/path)
    // inject/flag/env field with a known default. Every disqualifier keeps '' meaning "unset".

    // The one true delivers-empty shape (both str and path qualify).
    assert!(matrix_field(ParameterType::Str, true, false, false, false, false).delivers_empty());
    assert!(matrix_field(ParameterType::Path, true, false, false, false, false).delivers_empty());
    // Every disqualifier.
    assert!(!matrix_field(ParameterType::Int, true, false, false, false, false).delivers_empty());
    assert!(!matrix_field(ParameterType::Float, true, false, false, false, false).delivers_empty());
    assert!(!matrix_field(ParameterType::Bool, true, false, false, false, false).delivers_empty());
    assert!(
        !matrix_field(ParameterType::Choice, true, false, false, false, false).delivers_empty()
    );
    assert!(!matrix_field(ParameterType::Str, true, true, false, false, false).delivers_empty());
    assert!(!matrix_field(ParameterType::Str, true, false, true, false, false).delivers_empty());
    assert!(!matrix_field(ParameterType::Str, true, false, false, true, false).delivers_empty());
    // no default: nothing to clear back to
    assert!(!matrix_field(ParameterType::Str, false, false, false, false, false).delivers_empty());
    // An input binding never carries a default (empty = let the script ask), so it never
    // delivers empty either.
    assert!(!matrix_field(ParameterType::Str, false, false, false, false, true).delivers_empty());
}

// --------------------------------------------------------------------------
// 7) preset (persistable) keeps the default; last-used (remembered) drops it
// --------------------------------------------------------------------------

#[test]
#[ignore = "CROSS-CRATE (skit-application): `remembered_values` lives in \
crates/skit-application/src/form_state.rs:305 (declarations + submitted map), unreachable from \
skit-form."]
fn test_last_used_drops_values_equal_to_their_default() {
    // Last-used tracks the source: an untouched default is acceptance, not intent -> dropped.
    //   plan fields: GREETING str default "bonjour", WIDTH int default "800"
    //   values = {"GREETING": "bonjour", "WIDTH": "800"}  # both equal to their defaults
    //   remembered_values(plan, values) == {}
}

#[test]
#[ignore = "CROSS-CRATE (skit-application): `remembered_values` \
(crates/skit-application/src/form_state.rs:305, delivers-empty gate at :324) is unreachable from \
skit-form."]
fn test_last_used_keeps_a_cleared_empty_only_where_it_was_delivered() {
    // GREETING (str, has_default) delivered '' so it must replay as ''; WIDTH (int) did not
    // — there "" only ever meant "unset", and storing it would shadow a later default.
    //   values = {"GREETING": "", "WIDTH": ""}
    //   remembered_values(plan, values) == {"GREETING": ""}
}

#[test]
#[ignore = "CROSS-CRATE (skit-application + skit-store): Python `flows.save_after_run` -> \
`FormStateService::save_last` (crates/skit-application/src/form_state.rs:150, already routing \
through `remembered_values` at :158) plus the skit-store state adapter for `argstate.load_state`. \
Neither the service nor the store is reachable from skit-form."]
fn test_save_after_run_persists_via_the_remembered_rule() {
    // save_after_run stores last-used through remembered_values: a value equal to the default
    // is dropped, a changed one is kept.
    //   save_after_run("rem", plan, {"GREETING": "bonjour", "WIDTH": "900"}, [], 0, at=…)
    //   load_state("rem")["values"] == {"WIDTH": "900"}  # GREETING (== default) dropped
}

// --------------------------------------------------------------------------
// 8) FormField.input_binding tracks the ParamDecl binding
// --------------------------------------------------------------------------

// A managed python script with one input param and one const param. The oracle drives the flag
// through `FormField.from_decl`; the Rust constructor `PreparedField::from_declaration` is
// private, so this observes the same flag through a real `form_plan` whose reconcile keeps both.
const INPUT_AND_CONST_SCRIPT: &str = "# /// script\n\
# dependencies = []\n\
#\n\
# [tool.skit]\n\
# schema = 1\n\
#\n\
# [[tool.skit.params]]\n\
# name = \"input-1\"\n\
# kind = \"input\"\n\
# order = 0\n\
# prompt = \"Name: \"\n\
#\n\
# [[tool.skit.params]]\n\
# name = \"X\"\n\
# kind = \"const\"\n\
# type = \"str\"\n\
# default = \"v\"\n\
# ///\n\
X = 'v'\n\
who = input(\"Name: \")\n\
print(X, who)\n";

#[test]
fn test_input_binding_flag_reflects_the_decl_binding() {
    let plan = form_plan("python", INPUT_AND_CONST_SCRIPT, &EntrySettings::default());
    let by_name = plan
        .fields
        .iter()
        .map(|field| (field.declaration.name.clone(), field))
        .collect::<BTreeMap<_, _>>();
    assert!(
        by_name
            .get("input-1")
            .expect("input-1 field must survive reconcile")
            .input_binding
    );
    assert!(
        !by_name
            .get("X")
            .expect("X field must survive reconcile")
            .input_binding
    );
}
