//! Mechanical port of the Python oracle module `tests/test_path_type.py`
//! (`origin/main@206f9ef`): "The `path` parameter type (docs/design/path.md, P1a): str
//! semantics on every value surface, both serialization homes carry it, reconcile treats a
//! declared path over a source-derived str as a refinement (never drift), resync preserves it,
//! and unknown types keep degrading to str." Each `#[test]` keeps its Python `def test_*` name
//! and its Python "WHY" comment so it traces back to its origin.
//!
//! Concept mapping used throughout:
//! - Python `params.as_param_type(v)` / `params.ALLOWED_TYPES`: the Rust closed type set IS the
//!   `ParameterType` enum; membership is observable as parse-at-the-serialization-boundary. A
//!   spelling parses to its variant when known (`"path"` -> `Path`) or degrades to the `Str`
//!   fallback when unknown (`ParameterType::parse` is private, so the two `from_*_map` decoders
//!   are the public windows onto it).
//! - Python `ParamDecl.from_block_dict(d)` / `.to_block_dict()` -> `ParamDecl::from_block_map(&m)`
//!   / `.to_block_map()` (skit-domain, `BTreeMap<String, serde_json::Value>`).
//! - Python `ParamDecl.from_meta_dict(d)` / `.to_meta_dict()` -> `from_meta_map` / `to_meta_map`.
//! - Python `.type` -> `.parameter_type` (a typed `ParameterType`, not a string).
//! - Python `params.coerce_default(v, t)` -> `skit_domain::parameters::coerce_default(v, t)`
//!   (returns a typed `ParameterValue`; path keeps the raw string, wrapped `String(..)`).
//! - Python `reconcile.reconcile(text, specs)` -> the `reconcile` helper below (parse, then
//!   `ParsedDocument::reconcile`, or the conservative all-missing report on a syntax error).
//! - Report fields: Python `.changed`/`.usable` are `(spec, candidate)`-shaped; Rust `.changed`
//!   is `Vec<ReconcilePair>` (`pair.stored`, `pair.current.declaration`), `.usable()` and
//!   `.has_drift()` are methods.
//!
//! ONE-WAY SUPERSET NOTE (not a divergence): the Python refinement rule is path-over-str ONLY
//! (`analysis._type_matches`); the Rust rule (semantic.rs:2413-2421) accepts any pair drawn from
//! {Str, Path} in either direction. This extra acceptance is UNOBSERVABLE through the public
//! reconcile surface: a const candidate is typed from its literal alone (`parameter_type`,
//! semantic.rs:792 — String->Str, Integer->Int, ...; the Path inferences at 1628/2034 are
//! argparse/click reflection, never a reconcile candidate), so no candidate can carry Path and
//! the str-over-path direction never arises. Both reconcile tests below assert only the
//! path-over-str and path-over-int cases, on which Rust and Python agree exactly.
//!
//! Buckets:
//! - REAL asserting `#[test]` (API exists in skit-domain / skit-language): the type axis and the
//!   two reconcile tests — 7 total.
//! - CROSS-CRATE (`#[ignore]`, compiling stub, WHY + owning symbol): the remaining 7 tests drive
//!   tiers skit-language cannot reach without a forbidden Cargo.toml edit — the CLI declared-schema
//!   edit pipeline (`edit_declared`, private in `skit-cli/src/cli.rs`), the resync use case
//!   (`edit_specs`, skit-cli tier, same as the `port_test_reconcile.rs` precedent), the run-form
//!   projection (`skit_form::PreparedField::from_declaration`, private; no collapsed "kind" exists
//!   — `parameter_type` stays typed) plus its degraded free-text collapse (skit-tui), the
//!   pre-submit value check (`skit_application::value_preparation::validate_form_value`, public but
//!   skit-language cannot depend on skit-application), and the dim type hint
//!   (`skit-tui` private `run_type_label`, session.rs:2167).

use std::collections::BTreeMap;

use serde_json::Value;
use skit_domain::parameters::{
    ParamDecl, ParameterBinding, ParameterType, ParameterValue, coerce_default,
};
use skit_language::{ParseOutcome, ReconcileReport, parse_document};

/// Python `_decl(name="SRC", *, prompt="", secret=False)`: a const, path-typed declaration.
/// (Python leaves `delivery` at its "flag" default — a binding/delivery mismatch the block
/// round trip ignores, since `to_block_dict` never serializes delivery. Reconcile matches by
/// binding + name, so the default delivery is likewise irrelevant.)
fn decl(name: &str) -> ParamDecl {
    let mut declaration = ParamDecl::new(name);
    declaration.binding = ParameterBinding::Const;
    declaration.parameter_type = ParameterType::Path;
    declaration
}

/// Python `reconcile.reconcile(text, specs)`: parse, reconcile against the current source, or
/// return the conservative all-missing report when the source has a syntax error.
fn reconcile(source: &str, stored: &[ParamDecl]) -> ReconcileReport {
    match parse_document("python", source) {
        ParseOutcome::Parsed(document) => document.reconcile(stored),
        _ => ReconcileReport::from_syntax_error(stored),
    }
}

// ---------- the type axis ----------

#[test]
fn test_path_is_an_allowed_type() {
    // Python: `as_param_type("path") == "path"` and `"path" in ALLOWED_TYPES`. The Rust closed
    // set is the `ParameterType` enum: "path" is a member, spelled "path", and both serialization
    // homes parse it to `Path` (not the `Str` fallback that an unknown spelling degrades to).
    assert_eq!(ParameterType::Path.as_str(), "path");
    let block = BTreeMap::from([
        ("name".to_owned(), Value::String("X".to_owned())),
        ("type".to_owned(), Value::String("path".to_owned())),
    ]);
    assert_eq!(
        ParamDecl::from_block_map(&block).parameter_type,
        ParameterType::Path
    );
    let meta = BTreeMap::from([
        ("name".to_owned(), Value::String("X".to_owned())),
        ("type".to_owned(), Value::String("path".to_owned())),
    ]);
    assert_eq!(
        ParamDecl::from_meta_map(&meta).parameter_type,
        ParameterType::Path
    );
}

#[test]
fn test_unknown_type_still_degrades_to_str() {
    // The graceful-degrade mechanism an older skit's read of type="path" relies on:
    // anything outside the closed set coerces to str, in both serialization homes.
    let block = BTreeMap::from([
        ("name".to_owned(), Value::String("X".to_owned())),
        ("kind".to_owned(), Value::String("const".to_owned())),
        ("type".to_owned(), Value::String("pathlike".to_owned())),
    ]);
    assert_eq!(
        ParamDecl::from_block_map(&block).parameter_type,
        ParameterType::Str
    );
    let meta = BTreeMap::from([
        ("name".to_owned(), Value::String("X".to_owned())),
        ("type".to_owned(), Value::String("pathlike".to_owned())),
    ]);
    assert_eq!(
        ParamDecl::from_meta_map(&meta).parameter_type,
        ParameterType::Str
    );
}

#[test]
fn test_block_round_trip_carries_path() {
    let d = decl("SRC");
    assert_eq!(
        ParamDecl::from_block_map(&d.to_block_map()).parameter_type,
        ParameterType::Path
    );
}

#[test]
fn test_meta_round_trip_carries_path() {
    // Python: ParamDecl(name="src", delivery="flag", type="path"); binding stays "none".
    let mut d = ParamDecl::new("src");
    d.parameter_type = ParameterType::Path;
    assert_eq!(
        ParamDecl::from_meta_map(&d.to_meta_map()).parameter_type,
        ParameterType::Path
    );
}

#[test]
fn test_coerce_default_path_keeps_raw_string() {
    // path carries str semantics: no coercion, no existence check.
    assert_eq!(
        coerce_default("./no such file.csv", ParameterType::Path),
        Ok(ParameterValue::String("./no such file.csv".to_owned()))
    );
}

#[test]
#[ignore = "CROSS-CRATE (skit-cli): the declared-schema edit pipeline `params.edit_declared` is \
private in crates/skit-cli/src/cli.rs and observable only via `skit params --type src=path`; \
skit-language cannot depend on skit-cli. Ported by test_params_edit.py -> skit-cli."]
fn test_edit_declared_accepts_path_type() {
    // A `--type src=path` edit on a declared flag row is accepted: the row's type becomes "path"
    // and no warning is emitted (path is a member of the closed type set).
    //   decls = [ParamDecl(name="src", delivery="flag")]
    //   res = params.edit_declared(decls, types={"src": "path"})
    //   assert res.decls[0].type == "path" and res.warnings == []
}

// ---------- reconcile: refinement, not drift ----------

const SCRIPT: &str = "SRC = \"./data.csv\"\nRETRIES = 3\nprint(SRC, RETRIES)\n";

#[test]
fn test_reconcile_path_over_str_const_is_refinement() {
    // A string constant is exactly how a path lives in source: a declared `path` over a derived
    // `str` const is a refinement, not drift.
    let report = reconcile(SCRIPT, &[decl("SRC")]);
    assert!(!report.has_drift());
    assert!(report.changed.is_empty());
    let usable: Vec<&str> = report
        .usable()
        .into_iter()
        .map(|declaration| declaration.name.as_str())
        .collect();
    assert_eq!(usable, ["SRC"]);
}

#[test]
fn test_reconcile_path_over_int_const_is_drift() {
    let report = reconcile(SCRIPT, &[decl("RETRIES")]);
    assert!(report.has_drift());
    assert_eq!(report.changed.len(), 1);
    assert_eq!(report.changed[0].stored.name, "RETRIES");
    assert_eq!(
        report.changed[0].current.declaration.parameter_type,
        ParameterType::Int
    );
}

#[test]
#[ignore = "CROSS-CRATE (skit-cli tier): the resync use case `reconcile.edit_specs` lives in \
skit-cli/skit-ui/skit-form, not skit-language — same disposition port_test_reconcile.rs records \
for edit_specs. skit-language cannot reach it without a forbidden Cargo.toml edit."]
fn test_resync_preserves_declared_path() {
    // The path refinement survives --resync: a path declared over a str const keeps type "path"
    // and its prompt, and is NOT dropped ("resync-dropped:SRC" absent).
    //   res = reconcile.edit_specs(SCRIPT, [_decl(secret=False, prompt="Which file? ")], resync=True)
    //   assert res.specs[0].type == "path" and res.specs[0].prompt == "Which file? "
    //   assert "resync-dropped:SRC" not in res.warnings
}

#[test]
#[ignore = "CROSS-CRATE (skit-cli tier): the resync use case `reconcile.edit_specs` lives in \
skit-cli/skit-ui/skit-form, not skit-language — same disposition port_test_reconcile.rs records \
for edit_specs."]
fn test_resync_still_corrects_real_type_drift() {
    // The refinement rule is path-over-str ONLY: a path declared over an int constant is real
    // drift and resync re-anchors it to the source truth (type becomes "int").
    //   res = reconcile.edit_specs(SCRIPT, [_decl(name="RETRIES")], resync=True)
    //   assert res.specs[0].type == "int"
}

// ---------- form projection and validation ----------

#[test]
#[ignore = "CROSS-CRATE (skit-form): the run-form projection `flows.FormField.from_decl` maps to \
`skit_form::PreparedField::from_declaration`, which is private and keeps `parameter_type` TYPED \
(no collapsed string \"kind\" exists in the Rust form tier). skit-language cannot depend on \
skit-form."]
fn test_formfield_carries_path_for_every_delivery() {
    // Projecting a path-typed decl through inject/flag/env/placeholder delivery keeps kind "path"
    // on every surface.
    //   kinds == {"inject": "path", "flag": "path", "env": "path", "placeholder": "path"}
}

#[test]
#[ignore = "CROSS-CRATE (skit-tui): the degraded -> free-text collapse (`kind = \"str\" if \
degraded else d.type`) lives in the frontend field renderer, not skit-language. skit-form keeps \
`parameter_type` typed and `degraded` as a flag; the collapse is a skit-tui concern."]
fn test_degraded_flag_field_still_renders_free_text() {
    // A degraded path-typed flag renders as free text: kind == "str".
    //   d = ParamDecl(name="src", delivery="flag", type="path", degraded=True)
    //   assert flows.FormField.from_decl(d).kind == "str"
}

#[test]
#[ignore = "CROSS-CRATE (skit-application): the pre-submit check `flows.validate_value` maps to \
`skit_application::value_preparation::validate_form_value` (public), but skit-language cannot \
depend on skit-application. A path field is free text, so a non-existent path validates."]
fn test_validate_value_path_is_free_text() {
    // path is free text: no coercion, no existence check, so any string passes.
    //   f = flows.FormField(key="src", label="src", kind="path")
    //   assert flows.validate_value(f, "./definitely/not/created/yet.csv") is None
}

#[test]
#[ignore = "CROSS-CRATE (skit-tui): the dim type hint `tui_form._type_label` is a private \
render helper (`run_type_label`, crates/skit-tui/src/session.rs:2167); skit-language cannot reach \
it. The English label for the path type is \"path\"."]
fn test_type_label_path() {
    // The dim type hint for a path field reads "path".
    //   assert tui_form._type_label("path") == "path"
}
