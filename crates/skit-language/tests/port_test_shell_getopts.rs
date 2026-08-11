//! Mechanical port of the Python oracle module `tests/test_shell_getopts.py`
//! (`origin/main@206f9ef`): the shell `getopts` reader — turn a literal
//! `while getopts "ab:c:" opt` into single-dash flag fields, degrade a dynamic
//! optstring / a broken script / a getopts-less script honestly. Each `#[test]`
//! keeps its Python `def test_*` name, and each Python "WHY" comment is preserved
//! above it.
//!
//! Concept mapping (the shell peer of `port_test_argspec.rs`, which binds these):
//! - Python `cli_reader.read_cli(src)` -> `parse_document("shell", src)` +
//!   `ParsedDocument::cli_surface()` (the shell adapter).
//! - Python `spec is None` -> `CliSurface::Absent` (no getopts, or getopts with no
//!   optstring argument) AND `ParseOutcome::SyntaxError` (the script does not parse).
//!   Both are the faithful mapping of the Python `read_cli` returning `None` — one
//!   on `root.has_error`, one on an absent surface. The `read_cli` helper below
//!   collapses them exactly as the Python `None` does.
//! - Python `spec.ok is False` + `spec.reason == "dynamic"` ->
//!   `CliSurface::Dynamic(surface)` with `DegradationReason::DynamicDeclaration`.
//!   The `Dynamic` variant carries no fields, so the oracle's `spec.fields == []`
//!   is inherent in the variant.
//! - Python `spec.fields` -> `surface.fields[i].declaration` (a `ParamDecl`); list
//!   position carries declaration order, so name-order assertions use the `Vec`.
//! - Python field `type` strings -> `ParameterType`; Python `default is False` ->
//!   `Some(ParameterValue::Bool(false))`; empty `flag` / `action` -> `.is_empty()`.
//!
//! Buckets:
//! - Bucket 1 (the optstring matrix): the first ten tests; real asserting `#[test]`s
//!   over `parse_document` + `cli_surface`.
//! - Bucket 3 (plan / assemble integration): the last two Python tests drive
//!   `flows.plan_for_entry` + `flows.assemble` on a stored shell entry — the run-form
//!   plan and flag assembly, owned by skit-application (`delivery::assemble`),
//!   skit-store (`add_script`), and skit-ui (`run.rs` carries the run-form `source`
//!   and `degraded_reason`). None is reachable from a skit-language integration test
//!   without a forbidden dependency edge, so both are `#[ignore]` cross-crate stubs.

use skit_domain::parameters::{ParamDecl, ParameterType, ParameterValue};
use skit_language::{CliSurface, DegradationReason, ParseOutcome, parse_document};

/// Python `cli_reader.read_cli(src)`: `None` when the script does not parse, has no
/// `getopts` call, or has a `getopts` with no optstring argument; otherwise the
/// detected surface (static, or the honestly-degraded dynamic form).
fn read_cli(source: &str) -> Option<CliSurface> {
    match parse_document("shell", source) {
        ParseOutcome::Parsed(document) => match document.cli_surface() {
            CliSurface::Absent => None,
            surface => Some(surface),
        },
        ParseOutcome::SyntaxError(_) | ParseOutcome::ParserUnavailable(_) => None,
    }
}

/// Python `fields(src)`: assert the surface is present and static, then return its
/// field declarations in stored (declaration) order.
fn fields(source: &str) -> Vec<ParamDecl> {
    match read_cli(source) {
        Some(CliSurface::Static(surface)) => surface
            .fields
            .into_iter()
            .map(|field| field.declaration)
            .collect(),
        other => panic!("expected a static CLI surface, got {other:?}"),
    }
}

/// Python `list(fs)`: the field names in stored order.
fn names(fields: &[ParamDecl]) -> Vec<&str> {
    fields.iter().map(|field| field.name.as_str()).collect()
}

/// Python `fs["<letter>"]`: the one field with this name.
fn by_name<'a>(fields: &'a [ParamDecl], name: &str) -> &'a ParamDecl {
    fields
        .iter()
        .find(|field| field.name == name)
        .expect("field present")
}

#[test]
fn test_value_and_bool_flags() {
    let fs = fields("while getopts \"n:v\" opt; do :; done\n");
    let n = by_name(&fs, "n");
    assert_eq!(n.parameter_type, ParameterType::Str);
    assert_eq!(n.flag, "-n");
    assert!(n.action.is_empty());
    let v = by_name(&fs, "v");
    assert_eq!(v.parameter_type, ParameterType::Bool);
    assert_eq!(v.flag, "-v");
    assert_eq!(v.action, "store_true");
    assert_eq!(v.default, Some(ParameterValue::Bool(false)));
}

#[test]
fn test_leading_colon_silent_mode_is_skipped() {
    let fs = fields("while getopts \":ab:c:\" opt; do :; done\n");
    assert_eq!(names(&fs), ["a", "b", "c"]);
    assert_eq!(by_name(&fs, "a").parameter_type, ParameterType::Bool);
    assert_eq!(by_name(&fs, "b").parameter_type, ParameterType::Str);
}

#[test]
fn test_non_alphanumeric_characters_are_skipped() {
    let fs = fields("while getopts \"a-b\" opt; do :; done\n");
    // the stray '-' is ignored, both letters are bool flags
    assert_eq!(names(&fs), ["a", "b"]);
}

#[test]
fn test_repeated_letter_keeps_first() {
    let fs = fields("while getopts \"vv\" opt; do :; done\n");
    assert_eq!(names(&fs), ["v"]);
}

#[test]
fn test_dynamic_optstring_degrades_to_dynamic() {
    // A dynamic optstring is DETECTED but unmodelable: the reader degrades honestly to
    // ok=False 'dynamic' (the python/JS distinction), not None — None would claim the
    // script has no CLI at all, and the run form must instead say it couldn't model
    // this one.
    let spec = read_cli("getopts \"$OPTS\" opt\n").expect("dynamic surface is not None");
    let CliSurface::Dynamic(surface) = spec else {
        panic!("expected a dynamic CLI surface, got {spec:?}");
    };
    // Python `spec.reason == "dynamic"`.
    assert_eq!(surface.reason, DegradationReason::DynamicDeclaration);
    // Python `spec.fields == []` is inherent: the `Dynamic` variant carries no fields.
}

#[test]
fn test_getopts_without_optstring_is_none() {
    assert!(read_cli("getopts\n").is_none());
}

#[test]
fn test_no_getopts_is_none() {
    assert!(read_cli("echo hello\n").is_none());
}

#[test]
fn test_unparseable_script_is_none() {
    // tree-sitter reports has_error -> no readable surface
    assert!(read_cli("if\n").is_none());
}

#[test]
fn test_secret_letter_is_not_special() {
    // A single option letter never matches the secret-name heuristic (KEY/TOKEN/…).
    let fs = fields("while getopts \"k:\" opt; do :; done\n");
    assert!(!by_name(&fs, "k").secret);
}

// ---------------------------------------------------------------- plan / assemble

#[test]
#[ignore = "CROSS-CRATE (bucket 3): plan/assemble integration. The run-form plan + \
flag assembly is owned by skit-application (delivery::assemble, \
crates/skit-application/src/delivery.rs:75), skit-store (add_script), and skit-ui \
(run.rs run-form source/degraded_reason); none is reachable from a skit-language \
integration test without a forbidden dependency edit. Oracle \
tests/test_shell_getopts.py:78: a stored shell entry with `while getopts \"n:v\" opt` \
plans as plan.source == \"argparse\" with field keys [\"n\", \"v\"], and \
flows.assemble(plan, {n: \"Ada\", v: \"true\"}) == [\"-n\", \"Ada\", \"-v\"]."]
fn test_plan_reads_getopts_and_assembles_flags() {
    // Oracle body (unreachable here): SKIT_DATA/STATE/CONFIG_DIR -> tmp; write tool.sh
    // with '#!/usr/bin/env bash\nwhile getopts "n:v" opt; do :; done\n';
    // entry = store.add_script(src, kind="shell", name="gt");
    // plan = flows.plan_for_entry(entry); assert plan.source == "argparse";
    // assert [f.key for f in plan.fields] == ["n", "v"];
    // asm = flows.assemble(plan, {"n": "Ada", "v": "true"}, [], cwd=tmp);
    // assert asm.args == ["-n", "Ada", "-v"].
}

#[test]
#[ignore = "CROSS-CRATE (bucket 3): plan/assemble integration. Owned by skit-application \
(flows.plan_for_entry), skit-store (add_script), and skit-ui (run.rs carries the \
degraded_reason the run form shows); unreachable from a skit-language integration test. \
Oracle tests/test_shell_getopts.py:92: a stored shell entry whose getopts optstring is \
dynamic surfaces plan.source == \"argparse\", plan.degraded_reason == \"dynamic\", and \
plan.fields == []."]
fn test_plan_dynamic_getopts_degrades_with_reason() {
    // A shell entry whose getopts optstring is dynamic surfaces the degraded-form notice
    // on the run form (source='argparse', degraded_reason='dynamic') instead of silently
    // claiming there is no CLI — the same honest degradation python/JS give.
    // Oracle body (unreachable here): write dyn.sh with
    // '#!/usr/bin/env bash\nOPTS="n:v"\nwhile getopts "$OPTS" opt; do :; done\n';
    // entry = store.add_script(src, kind="shell", name="dyn");
    // plan = flows.plan_for_entry(entry); assert plan.source == "argparse";
    // assert plan.degraded_reason == "dynamic"; assert plan.fields == [].
}
