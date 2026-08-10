//! Mechanical port of the Python oracle module `tests/test_js_inject.py`
//! (`origin/main@206f9ef`). This is the JS/TS injector contract: const value delivery, quoting
//! normalization (int/float -> bare number, bool -> lowercase keyword, str -> `json.dumps` literal),
//! same-name multi-occurrence rewrite, drift vs bad-value refusal, and the mandatory offline
//! re-parse gate.
//!
//! Each `#[test]` keeps its Python `def test_*` name so it traces back to its origin, and the
//! Python "WHY" comment is preserved. Same input bytes, same expected output.
//!
//! ## This module spans layers. Every Python test is sorted into one of three buckets.
//!
//! The Python file mixes pure-logic tests with EXECUTION tests that spawn a real node/deno/bun on
//! the injected copy and assert on the child's stdout, plus runtime `node --check` gate tests, a
//! 0600 temp-copy mode test, and `flows.execute`/CLI integration. In the Rust architecture the
//! injector produces rewritten BYTES (`skit-language`), while running the injected copy, resolving a
//! runner, the `node --check` gate, the 0600 temp copy, and `skit run --set` end-to-end are
//! CLI/runtime concerns (`skit run --set`; see `skit-cli`). So:
//!
//! - **Bucket 1 (pure inject byte-logic / typed refusal):** asserted on
//!   `plan_injection(...).apply(source)` output bytes, `source_is_valid("js"|"ts", ..)` where the
//!   Python re-parses (`not analyzer.analyze(text).syntax_error`), and `LanguageError` where the
//!   Python raises `InjectError`/`InjectValueError`. This is the bulk.
//! - **Bucket 2 (execution claim the injected BYTES fully establish):** ported as byte assertions
//!   with an explicit `PORTED AS BYTE ASSERTION:` note stating the runtime claim the byte form
//!   proves. An int/str value present at its const target byte-for-byte, in a copy that re-parses,
//!   is provably delivered without running node.
//! - **Bucket 3 (execution claim that genuinely needs a running runtime, or a layer above
//!   skit-language):** kept as a compiling `#[ignore]` stub with its WHY comment. `#[ignore]` is
//!   used ONLY for this bucket.
//!
//! ## Error mapping (Python `skit.langs.base` -> `skit_language`)
//! - `InjectError` (target not found: drift) -> `LanguageError::BindingNotFound`.
//! - `InjectValueError` (value not coercible; `.param_name`) -> `LanguageError::InvalidValue{name}`.
//! - `InjectSyntaxError` (post-inject gate) -> the re-parse gate inside `inject_values`; only
//!   reachable by monkeypatching the internal escaper, so those tests are Bucket 3.
//!
//! The injector produces bytes only. `InjectResult.path`/`.env`, the temp-copy `.mjs/.cjs/.ts`
//! flavor, the 0600 temp copy, the `node --check` interpreter gate, and `_resolve_runner` all live
//! in the CLI/runtime tier, so tests whose sole claim is one of those are Bucket 3.

use std::collections::BTreeMap;

use skit_domain::parameters::{ParamDecl, ParameterBinding, ParameterDelivery, ParameterType};
use skit_language::{
    LanguageError, ParseOutcome, ParsedDocument, SourceEditPlan, parse_document, source_is_valid,
};

// ---------------------------------------------------------------- helpers

fn parsed(kind: &str, source: &str) -> ParsedDocument {
    match parse_document(kind, source) {
        ParseOutcome::Parsed(document) => document,
        other => panic!("expected valid {kind}, got {other:?}"),
    }
}

fn to_map(values: &[(&str, &str)]) -> BTreeMap<String, String> {
    values
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

/// Python `specs_of(src, lang=L)` = the analyzer candidates' declarations.
fn specs_of_lang(kind: &str, source: &str) -> Vec<ParamDecl> {
    parsed(kind, source)
        .analysis()
        .candidates
        .into_iter()
        .map(|candidate| candidate.declaration)
        .collect()
}

/// Python `specs_of(src)` (default `lang="js"`).
fn specs_of(source: &str) -> Vec<ParamDecl> {
    specs_of_lang("js", source)
}

/// Python `inject_src(src, values, lang=L)`: plan with the analyzer's own specs, then apply.
/// Returns the rewritten bytes, or the `LanguageError` the planner raised.
fn inject_lang(kind: &str, source: &str, values: &[(&str, &str)]) -> Result<String, LanguageError> {
    inject_with_specs_lang(kind, source, values, &specs_of_lang(kind, source))
}

/// Python `inject_src(src, values)` (default `lang="js"`).
fn inject(source: &str, values: &[(&str, &str)]) -> Result<String, LanguageError> {
    inject_lang("js", source, values)
}

/// Python `inject_src(src, values, specs=...)`: the caller supplies the stored declarations.
fn inject_with_specs(
    source: &str,
    values: &[(&str, &str)],
    specs: &[ParamDecl],
) -> Result<String, LanguageError> {
    inject_with_specs_lang("js", source, values, specs)
}

fn inject_with_specs_lang(
    kind: &str,
    source: &str,
    values: &[(&str, &str)],
    specs: &[ParamDecl],
) -> Result<String, LanguageError> {
    plan_with_specs_lang(kind, source, values, specs)?.apply(source)
}

/// Python `inject_src(src, values)` stopping at the plan (default `lang="js"`).
fn plan(source: &str, values: &[(&str, &str)]) -> Result<SourceEditPlan, LanguageError> {
    plan_with_specs_lang("js", source, values, &specs_of(source))
}

fn plan_with_specs(
    source: &str,
    values: &[(&str, &str)],
    specs: &[ParamDecl],
) -> Result<SourceEditPlan, LanguageError> {
    plan_with_specs_lang("js", source, values, specs)
}

fn plan_with_specs_lang(
    kind: &str,
    source: &str,
    values: &[(&str, &str)],
    specs: &[ParamDecl],
) -> Result<SourceEditPlan, LanguageError> {
    parsed(kind, source).plan_injection(specs, &to_map(values))
}

/// Python `ParamDecl(name=name, binding="const", delivery="inject", type=type)`.
fn inject_const_spec(name: &str, parameter_type: ParameterType) -> ParamDecl {
    let mut declaration = ParamDecl::new(name);
    declaration.binding = ParameterBinding::Const;
    declaration.delivery = ParameterDelivery::Inject;
    declaration.parameter_type = parameter_type;
    declaration
}

// ---------------------------------------------------------------- const quoting

#[test]
fn test_int_injects_a_bare_number() {
    let out = inject("const W = 800;\n", &[("W", "1200")]).unwrap();
    assert!(out.contains("const W = 1200;"));
}

#[test]
fn test_float_injects_a_bare_number() {
    let out = inject("const R = 0.5;\n", &[("R", "2.75")]).unwrap();
    assert!(out.contains("const R = 2.75;"));
}

#[test]
fn test_string_injects_a_json_dumps_literal() {
    let out = inject("const C = \"x\";\n", &[("C", "New York")]).unwrap();
    assert!(out.contains("const C = \"New York\";"));
}

#[test]
fn test_string_json_escapes_quote_backslash_newline() {
    // quote, backslash, newline all escaped, and the copy re-parses.
    let out = inject("const M = \"x\";\n", &[("M", "a\"b\\c\nd")]).unwrap();
    assert!(out.contains(r#"const M = "a\"b\\c\nd";"#));
    assert!(source_is_valid("js", &out));
}

#[test]
fn test_cjk_and_emoji_escape_to_valid_js() {
    // json.dumps escapes non-ASCII (ensure_ascii): `高` -> `高`, and the copy re-parses.
    let out = inject("const C = \"x\";\n", &[("C", "高雄 🚀")]).unwrap();
    assert!(out.contains("\\u9ad8"), "expected `\\u9ad8` in\n{out}");
    assert!(source_is_valid("js", &out));
}

#[test]
fn test_bool_injects_true_or_false_lowercase() {
    let on = inject("const B = false;\n", &[("B", "yes")]).unwrap();
    assert!(on.contains("const B = true;"));
    let off = inject("const B = true;\n", &[("B", "0")]).unwrap();
    assert!(off.contains("const B = false;"));
}

#[test]
fn test_rewrites_every_same_name_occurrence() {
    let src = "var M = 1;\nvar M = 2;\nconsole.log(M);\n";
    let out = inject(src, &[("M", "9")]).unwrap();
    assert_eq!(out.matches("= 9;").count(), 2);
}

#[test]
fn test_same_name_nonliteral_declaration_is_not_a_target() {
    // A later `var M = compute()` (non-literal) is not a rewrite target — only literal ones are.
    let src = "var M = 1;\nvar M = compute();\n";
    let out = inject(src, &[("M", "9")]).unwrap();
    assert!(out.contains("var M = 9;"));
    assert!(out.contains("var M = compute();")); // untouched
}

#[test]
fn test_ts_temp_copy_has_ts_suffix() {
    // PORTED AS BYTE ASSERTION: the injector-observable half is the ts const rewrite. The Python
    // `result.path.suffix == ".ts"` is the temp-copy filename (Tier 3/4, store/CLI); skit-language
    // only produces the bytes.
    let out = inject_lang("ts", "const N: number = 5;\n", &[("N", "7")]).unwrap();
    assert!(out.contains("const N: number = 7;"));
}

#[test]
#[ignore = "UNMAPPED: the injected temp copy re-encodes the origin's .mjs/.cjs/.mts/.cts module flavor in its filename suffix (parametrized over 7 origins) -> Tier 3/4 (store/CLI temp-copy naming). skit-language produces only the rewritten bytes and never names the copy."]
fn test_injected_copy_carries_the_origins_module_flavor() {
    // Python asserts result.path.suffix matches the origin's module flavor.
}

// ---------------------------------------------------------------- drift / bad value

#[test]
fn test_missing_target_is_drift_not_value_error() {
    // The analyzer never offers GONE, so a stored definition naming it is drift, not a value error.
    // (Python `not temp_files(tmp_path)` — no copy written — is the Tier 3/4 consequence.)
    let spec = inject_const_spec("GONE", ParameterType::Str);
    match plan_with_specs("const W = 800;\n", &[("GONE", "x")], &[spec]) {
        Err(LanguageError::BindingNotFound { name }) => assert_eq!(name, "GONE"),
        other => panic!("expected BindingNotFound for GONE (not InvalidValue), got {other:?}"),
    }
}

#[test]
fn test_bad_int_value_raises_value_error() {
    // Python asserts `exc_info.value.param_name == "W"`. (No temp files is Tier 3/4.)
    match plan("const W = 800;\n", &[("W", "not-a-number")]) {
        Err(LanguageError::InvalidValue { name, .. }) => assert_eq!(name, "W"),
        other => panic!("expected InvalidValue for W, got {other:?}"),
    }
}

#[test]
fn test_bad_float_and_non_finite_are_refused() {
    for bad in ["abc", "inf", "-inf", "nan"] {
        match plan("const R = 0.5;\n", &[("R", bad)]) {
            Err(LanguageError::InvalidValue { name, .. }) => assert_eq!(name, "R"),
            other => panic!("expected InvalidValue for R={bad:?}, got {other:?}"),
        }
    }
}

#[test]
fn test_bad_bool_value_raises_value_error() {
    match plan("const B = true;\n", &[("B", "maybe")]) {
        Err(LanguageError::InvalidValue { name, .. }) => assert_eq!(name, "B"),
        other => panic!("expected InvalidValue for B, got {other:?}"),
    }
}

#[test]
fn test_no_values_writes_nothing() {
    // Python: `result.path is None` — no temp copy, no rewrite, the original runs. In skit-language
    // this is "the plan has no edits", so `apply` returns the source byte-for-byte.
    // (`result.env == {}` and "no temp files" are the assembly / Tier 3/4 tiers.)
    let src = "const W = 800;\n";
    assert!(plan(src, &[]).unwrap().edits().is_empty());
    assert_eq!(inject(src, &[]).unwrap(), src);
}

#[test]
fn test_value_for_unmanaged_name_is_ignored() {
    // A value whose key isn't a managed spec never produces a span.
    let src = "const W = 800;\n";
    assert!(
        plan_with_specs(src, &[("OTHER", "x")], &[])
            .unwrap()
            .edits()
            .is_empty()
    );
    assert_eq!(inject_with_specs(src, &[("OTHER", "x")], &[]).unwrap(), src);
}

// ---------------------------------------------------------------- gate 1 (offline)

#[test]
#[ignore = "UNMAPPED: the offline gate fires only for a BROKEN escaper (Python monkeypatches inject.escape_string to drop the closing quote) and expects InjectSyntaxError. The gate exists — inject_values re-parses the output and returns InvalidSource — but skit-language exposes no way to corrupt the internal escaper, so the failure path is unreachable from a public test."]
fn test_offline_gate_refuses_a_corrupted_injection() {
    // Python monkeypatches `inject.escape_string` to emit an unterminated string.
}

// ---------------------------------------------------------------- gate 2 (node --check)

#[test]
#[ignore = "UNMAPPED: `inject._resolve_runner` picks the first installed node/deno/bun via shutil.which -> Tier 3/4 (skit-runtime/CLI runner resolution). skit-language does not resolve or spawn a runtime."]
fn test_resolve_runner_finds_first_installed() {}

#[test]
#[ignore = "UNMAPPED: `inject._resolve_runner` returns (None, None) when nothing is installed -> Tier 3/4 (skit-runtime/CLI). skit-language does not resolve a runtime."]
fn test_resolve_runner_none_when_nothing_installed() {}

#[test]
#[ignore = "UNMAPPED: `inject._resolve_runner` normalizes a pinned path/.exe interpreter to a bare runner name -> Tier 3/4 (skit-runtime/CLI). skit-language does not resolve a runtime."]
fn test_resolve_runner_respects_pinned_interpreter_and_normalizes() {}

#[test]
#[ignore = "UNMAPPED: `inject._gate_node` skips `node --check` for a `.ts` suffix (node cannot check TS) -> Tier 3/4 (skit-runtime interpreter gate). skit-language performs no interpreter spawn."]
fn test_gate_node_skips_ts_suffix() {}

#[test]
#[ignore = "UNMAPPED: regression that runs the REAL `node --check` on an ESM `.mjs`-origin copy before any package.json exists -> Tier 3/4 (skit-runtime interpreter gate + temp-copy flavor). skit-language never spawns node."]
fn test_mjs_origin_esm_copy_survives_gate2_before_any_package_json() {}

#[test]
#[ignore = "UNMAPPED: `inject._gate_node` returns without spawning when the runner is not node -> Tier 3/4 (skit-runtime). skit-language performs no interpreter spawn."]
fn test_gate_node_skips_when_runner_is_not_node() {}

#[test]
#[ignore = "UNMAPPED: `inject._gate_node` returns when no runner is installed (program is None) -> Tier 3/4 (skit-runtime). skit-language performs no interpreter spawn."]
fn test_gate_node_skips_when_no_runner_installed() {}

#[test]
#[ignore = "UNMAPPED: `inject._gate_node` passes on `node --check` returncode 0 -> Tier 3/4 (skit-runtime interpreter gate). skit-language performs no interpreter spawn."]
fn test_gate_node_passes_on_returncode_zero() {}

#[test]
#[ignore = "UNMAPPED: `inject._gate_node` raises InjectSyntaxError on a nonzero `node --check` -> Tier 3/4 (skit-runtime interpreter gate). skit-language performs no interpreter spawn."]
fn test_gate_node_raises_on_nonzero() {}

#[test]
#[ignore = "UNMAPPED: `inject._gate_node` raises InjectSyntaxError on nonzero with empty stderr -> Tier 3/4 (skit-runtime interpreter gate). skit-language performs no interpreter spawn."]
fn test_gate_node_raises_on_nonzero_with_empty_stderr() {}

#[test]
#[ignore = "UNMAPPED: `inject._gate_node` swallows a subprocess spawn OSError (gate 1 already vouched) -> Tier 3/4 (skit-runtime). skit-language performs no interpreter spawn."]
fn test_gate_node_survives_a_spawn_failure() {}

#[test]
#[ignore = "UNMAPPED: a `node --check` rejection must remove the written temp copy from disk -> Tier 3/4 (skit-runtime interpreter gate + temp-copy cleanup). skit-language writes no temp copy."]
fn test_gate2_failure_removes_the_temp_copy() {}

// ---------------------------------------------------------------- secret handling

#[test]
#[ignore = "MUST-VERIFY (secret handling): the injected copy's 0600 mode is a store/runtime property (the temp copy is written by the CLI tier) -> Tier 3/4. The const str single-quoting/JSON-literal form is covered by test_string_injects_a_json_dumps_literal."]
fn test_injected_copy_is_0600() {
    // Python asserts result.path.stat() mode & 0o777 == 0o600.
}

// ---------------------------------------------------------------- execution (runner-gated)

#[test]
fn test_injected_const_reaches_the_child() {
    // PORTED AS BYTE ASSERTION: Python runs the copy and asserts stdout `w=1200`. The int value
    // lands bare at its const target (`const WIDTH = 1200;`) in a copy that re-parses; running only
    // confirms node concatenates it.
    let src = "const WIDTH = 800;\nconsole.log(\"w=\" + WIDTH);\n";
    let out = inject(src, &[("WIDTH", "1200")]).unwrap();
    assert!(out.contains("const WIDTH = 1200;"));
    assert!(source_is_valid("js", &out));
}

// Runtime execution test: Python runs the injected copy under node and asserts stdout `台北 🚀`.
// The oracle emits the value as an ensure_ascii JSON literal (`台北 🚀`); that
// escaped-bytes contract is now covered by test_cjk_and_emoji_escape_to_valid_js. The remaining
// claim -- node decodes the escaped literal and prints the value -- needs a running runtime.
#[test]
#[ignore = "UNMAPPED: runtime execution (run_js on the injected copy asserts node stdout) -> Tier 3/4 (skit run); the escaped-literal byte contract is covered by test_cjk_and_emoji_escape_to_valid_js"]
fn test_injected_string_reaches_the_child() {}

#[test]
#[ignore = "UNMAPPED: `skit params --manage` + `skit run --set --no-input` end to end (CliRunner + store.add_script), asserts the script's own stdout `w=1200` on the real fd -> Tier 4 (skit-cli). Byte injection covered by test_injected_const_reaches_the_child."]
fn test_run_injects_and_executes_end_to_end() {}

// ---------------------------------------------------------------- flows.execute integration

#[test]
#[ignore = "UNMAPPED: `skit params --manage` + `flows.plan_for_entry` reports source == \"inject\" for a js entry (CliRunner + store.add_script) -> Tier 4 (skit-cli/flows). The inject plan itself is covered by the bucket-1 const tests."]
fn test_execute_runs_a_js_entry_offline_plan() {}

#[test]
#[ignore = "UNMAPPED: flows.execute maps a drifted js const to FAIL_DRIFT with the `--resync` hint -> Tier 4 (skit-cli/flows). The injector's drift refusal is covered by test_missing_target_is_drift_not_value_error."]
fn test_execute_maps_a_drifted_js_definition_to_drift() {}

#[test]
#[ignore = "UNMAPPED: `skit run --set WIDTH=abc --no-input` returns the FAIL_BAD_VALUE exit code before launch -> Tier 4 (skit-cli). The typed refusal is covered by test_bad_int_value_raises_value_error."]
fn test_execute_refuses_a_bad_value_before_launch() {}

#[test]
#[ignore = "UNMAPPED: a syntax-gate failure must map to FAIL_DRIFT WITHOUT a `--resync` hint and never launch -> Tier 4 (skit-cli/flows); Python also monkeypatches inject.escape_string to force the corruption and spies on launcher.run_entry."]
fn test_execute_syntax_gate_failure_never_launches() {}
