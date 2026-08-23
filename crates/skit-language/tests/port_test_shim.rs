//! Mechanical port of the Python oracle module `tests/test_shim.py`
//! (`origin/main@206f9ef`): "Behavioural contract for shim injection: AST location, text
//! substitution, all other bytes unchanged." Each `#[test]` keeps its Python `def test_*`
//! name so it traces back to its origin, and each Python "WHY" comment is preserved above it.
//!
//! Concept mapping used throughout:
//! - Python `shim.inject(text, specs, values)` -> `inject_values("python", text, &specs, &values)`
//!   (skit-language). The Rust entry point additionally re-parses its own output, an
//!   ADDITION over the oracle; every faithful fixture here still emits valid Python, so that
//!   extra gate never fires.
//! - Python `shim.ShimError` (missing/drifted target) -> `LanguageError::BindingNotFound`;
//!   `shim.ShimError` from `ast.parse` failure -> `LanguageError::InvalidSource`.
//! - Python `shim.ShimValueError` (bad type coercion; `.value` / `.type_name` / `.param_name`)
//!   -> `LanguageError::InvalidValue { value, parameter_type, name }`.
//! - `pytest.raises(shim.ShimError)` -> `result.is_err()` (the base class also catches the
//!   subclass, and every `LanguageError` variant is an error). Variants are matched only where
//!   the oracle itself distinguishes the two failure modes.
//! - The fifteen contracts that call Python `compile` or run the emitted source live in
//!   `skit-cli/tests/port_test_shim_runtime.rs`. That composition target uses the production
//!   runtime probe and a real Python child on every supported platform.
//! - Python `spec(...)` fixture -> `spec(name)` (const/str, delivery Inject) plus field edits and
//!   `input_spec(name, order)` for input bindings. Const and Input both imply
//!   `ParameterDelivery::Inject`, which is what `plan_python_injection` selects on.
//!
//! Eighteen portable semantic owners stay here. Fifteen real-runtime owners live in the CLI
//! composition target, and three staged-copy owners live at the CLI staging/composition seam. The
//! final manifest records two Python-only helper/ownership closures.

use skit_domain::parameters::{ParamDecl, ParameterBinding, ParameterDelivery, ParameterType};
use skit_language::{LanguageError, inject_values};
use std::collections::BTreeMap;

// --- The oracle's module-level SCRIPT fixture (byte-identical) ---
const SCRIPT: &str = r#""""Docstring stays."""
# /// script
# dependencies = ["requests"]
# ///
CITY = "Taipei"  # trailing comment stays
RETRIES = 3

def main():
    who = input("Your name: ")
    print(who, CITY, RETRIES)

if __name__ == "__main__":
    DEBUG = True
    main()
"#;

/// Oracle `spec(name, binding="const", type="str", order=-1, secret=False, prompt="")`: the
/// const/str default. Individual tests edit the remaining fields inline.
fn spec(name: &str) -> ParamDecl {
    let mut declaration = ParamDecl::new(name);
    declaration.binding = ParameterBinding::Const;
    declaration.delivery = ParameterDelivery::Inject;
    declaration.parameter_type = ParameterType::Str;
    declaration
}

/// The `binding="input"` variant of the oracle `spec` fixture.
fn input_spec(name: &str, order: i64) -> ParamDecl {
    let mut declaration = spec(name);
    declaration.binding = ParameterBinding::Input;
    declaration.order = order;
    declaration
}

/// Build the `{name: value}` accepted-value map the oracle passes as `values`.
fn values(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

/// Oracle `shim.inject(text, specs, values)`.
fn inject(
    text: &str,
    specs: &[ParamDecl],
    pairs: &[(&str, &str)],
) -> Result<String, LanguageError> {
    inject_values("python", text, specs, &values(pairs))
}

#[test]
fn test_const_str_injection_preserves_everything_else() {
    let out = inject(SCRIPT, &[spec("CITY")], &[("CITY", "Kaohsiung")]).unwrap();
    assert!(out.contains("CITY = 'Kaohsiung'  # trailing comment stays"));
    assert!(out.contains(r#"# dependencies = ["requests"]"#));
    assert!(out.contains("RETRIES = 3"));
}

#[test]
fn test_const_typed_injection() {
    let mut retries = spec("RETRIES");
    retries.parameter_type = ParameterType::Int;
    let out = inject(SCRIPT, &[retries], &[("RETRIES", "7")]).unwrap();
    assert!(out.contains("RETRIES = 7"));
}

#[test]
fn test_main_guard_const() {
    let mut debug = spec("DEBUG");
    debug.parameter_type = ParameterType::Bool;
    let out = inject(SCRIPT, &[debug], &[("DEBUG", "false")]).unwrap();
    assert!(out.contains("DEBUG = False"));
}

#[test]
fn test_input_queue_preamble_is_single_line_after_docstring() {
    let out = inject(SCRIPT, &[input_spec("input-1", 0)], &[("input-1", "Alice")]).unwrap();
    let lines = out.lines().collect::<Vec<_>>();
    // docstring is still line 0; __doc__ semantics preserved
    assert_eq!(lines[0], r#""""Docstring stays.""""#);
    let shim_lines = lines
        .iter()
        .filter(|line| line.ends_with("# skit:shim"))
        .count();
    assert_eq!(shim_lines, 1); // single physical line; line-number shift is always exactly 1
    assert!(out.contains(r#"# dependencies = ["requests"]"#)); // PEP 723 block untouched
}

#[test]
fn test_missing_value_leaves_script_untouched() {
    let out = inject(SCRIPT, &[spec("CITY")], &[]).unwrap();
    assert_eq!(out, SCRIPT);
}

// ---------- shadowed `input`: the analyzer's guard, mirrored in the shim (A2) ----------

#[test]
fn test_shadowed_input_is_not_rewritten_and_surfaces_as_drift() {
    // A script that binds `input` itself (a def) has NO managed call sites, so `_input_calls`
    // returns [] and a stored input spec cannot resolve — it must surface as drift (ShimError)
    // rather than the shim splicing a stdin-fallback wrapper over the script's OWN function call.
    let src = "def input(prompt=''):\n    return 'HARDCODED'\ny = input('Q: ')\nprint(y)\n";
    let mut declaration = input_spec("input-1", 0);
    declaration.prompt = "Q: ".to_owned();
    assert!(inject(src, &[declaration], &[("input-1", "typed")]).is_err());
}

#[test]
fn test_shadowed_input_leaves_the_call_site_text_intact_when_only_a_const_is_delivered() {
    // A const in the same shadowed-input file still injects, and the `input('Q: ')` call site is
    // left byte-for-byte intact (never rewritten to `_skit_i[K]`) because the shim treats the
    // bound name as the script's own function.
    let src = "def input(prompt=''):\n    return 'x'\nCITY = 'Taipei'\ny = input('Q: ')\nprint(y, CITY)\n";
    let out = inject(src, &[spec("CITY")], &[("CITY", "Tainan")]).unwrap();
    assert!(out.contains("CITY = 'Tainan'"));
    assert!(out.contains("y = input('Q: ')")); // untouched
    assert!(!out.contains("_skit_i"));
}

#[test]
fn test_drifted_target_raises() {
    assert!(inject(SCRIPT, &[spec("GONE")], &[("GONE", "x")]).is_err());
}

#[test]
fn test_bad_type_coercion_raises() {
    let mut retries = spec("RETRIES");
    retries.parameter_type = ParameterType::Int;
    assert!(inject(SCRIPT, &[retries], &[("RETRIES", "not-a-number")]).is_err());
}

#[test]
fn test_bad_type_coercion_raises_the_value_subclass_not_plain_shim_error() {
    // A bad value is a distinct failure mode from a missing/drifted target: the target (RETRIES)
    // WAS found; only the supplied value does not fit its declared int type. Callers (the CLI) need
    // to tell the two apart to avoid misdiagnosing a bad input as source drift, so this must raise
    // the ShimValueError subclass specifically, carrying the value/type/param for the caller's
    // message -- not just the generic base ShimError raised for a genuinely missing target.
    let mut retries = spec("RETRIES");
    retries.parameter_type = ParameterType::Int;
    let error = inject(SCRIPT, &[retries], &[("RETRIES", "not-a-number")]).unwrap_err();
    match error {
        // InvalidValue is the ShimValueError analogue: still a LanguageError (existing error
        // handlers hold), but distinguishable from BindingNotFound and carrying the same trio.
        LanguageError::InvalidValue {
            name,
            value,
            parameter_type,
        } => {
            assert_eq!(value, "not-a-number");
            assert_eq!(parameter_type, ParameterType::Int);
            assert_eq!(name, "RETRIES");
        }
        other => panic!("expected InvalidValue, got {other:?}"),
    }
}

#[test]
fn test_drifted_target_raises_plain_shim_error_not_value_subclass() {
    // The converse: a genuinely missing target must NOT be reported as ShimValueError (that would
    // wrongly suggest to a caller that the value, not the target, was the problem).
    let error = inject(SCRIPT, &[spec("GONE")], &[("GONE", "x")]).unwrap_err();
    assert!(matches!(error, LanguageError::BindingNotFound { .. }));
    assert!(!matches!(error, LanguageError::InvalidValue { .. }));
}

// ---------- _coerce_bool: invalid string ----------

#[test]
fn test_coerce_bool_invalid_raises() {
    // Any string not in the recognized set must raise ShimError, not return a falsy value.
    let mut flag = spec("FLAG");
    flag.parameter_type = ParameterType::Bool;
    assert!(inject("FLAG = True\n", &[flag], &[("FLAG", "maybe")]).is_err());
}

// ---------- inject: SyntaxError in source raises ShimError ----------

#[test]
fn test_inject_syntax_error_raises() {
    assert!(inject("def broken(:\n", &[spec("X")], &[("X", "1")]).is_err());
}

// ---------- inject: input order beyond available calls is drift ----------

#[test]
fn test_input_order_beyond_calls_is_drift() {
    // order=5 when there are no input() calls means the definition drifted.
    let src = "print('hello')\n";
    assert!(inject(src, &[input_spec("input-1", 5)], &[("input-1", "x")]).is_err());
}

// ---------- inject: duplicate-prompt specs must never double-bind onto one call site ----------

#[test]
fn test_inject_two_identical_prompts_one_deleted_raises_cleanly_never_corrupts() {
    // Regression: input-1 and input-2 both stored prompt "Go? "; the first of the two input()
    // calls was deleted, leaving one current call site with that prompt. Pre-fix, both specs
    // exact-matched onto that single call site and inject() spliced two replacements over the
    // same `input` callee span, producing corrupt source like `x = _skit_i[0]_i[0]("Go? ")` that
    // fails compile(). The surplus spec must now be reported as drift (ShimError), and the output
    // must never contain a doubled callee.
    let src = "first = input(\"Go? \")\nsecond = input(\"Go? \")\nprint(first, second)\n";
    let edited = src.replace("first = input(\"Go? \")\n", ""); // delete the first call
    let mut input_one = input_spec("input-1", 0);
    input_one.prompt = "Go? ".to_owned();
    let mut input_two = input_spec("input-2", 1);
    input_two.prompt = "Go? ".to_owned();
    assert!(
        inject(
            &edited,
            &[input_one, input_two],
            &[("input-1", "AAA"), ("input-2", "BBB")],
        )
        .is_err()
    );
}

#[test]
fn test_inject_specs_sharing_the_same_order_never_double_bind() {
    // Defense-in-depth at the shim layer itself: two ParamDecl entries that carry the identical
    // `order` (e.g. a hand-edited or otherwise corrupted [tool.skit] block) look up the exact same
    // match_calls binding and would both try to queue a replacement over the same input() callee
    // span. inject() must refuse to emit the second, overlapping replacement -- reporting drift via
    // ShimError instead of corrupting the temp copy.
    let src = "x = input(\"Go? \")\nprint(x)\n";
    let mut input_one = input_spec("input-1", 0);
    input_one.prompt = "Go? ".to_owned();
    let mut input_two = input_spec("input-2", 0);
    input_two.prompt = "Go? ".to_owned();
    assert!(
        inject(
            src,
            &[input_one, input_two],
            &[("input-1", "AAA"), ("input-2", "BBB")],
        )
        .is_err()
    );
}

#[test]
fn test_inject_triple_duplicate_specs_same_order_never_double_bind() {
    let src = "x = input(\"Go? \")\nprint(x)\n";
    let mut input_one = input_spec("input-1", 0);
    input_one.prompt = "Go? ".to_owned();
    let mut input_two = input_spec("input-2", 0);
    input_two.prompt = "Go? ".to_owned();
    let mut input_three = input_spec("input-3", 0);
    input_three.prompt = "Go? ".to_owned();
    assert!(
        inject(
            src,
            &[input_one, input_two, input_three],
            &[("input-1", "AAA"), ("input-2", "BBB"), ("input-3", "CCC")],
        )
        .is_err()
    );
}

// ---------- _insert_preamble: empty body inserts at end ----------

#[test]
fn test_preamble_inserted_at_end_for_no_docstring_no_future() {
    // When the source has no docstring and no __future__ import, the preamble goes right before
    // the first real statement (which is at index 0, so the preamble is inserted at the top).
    // A file with only a bare input() call has no docstring and no __future__ import,
    // so _preamble_line_index returns 0: the preamble is inserted before line 0.
    let src = "x = input('v: ')\nprint(x)\n";
    let out = inject(src, &[input_spec("input-1", 0)], &[("input-1", "hi")]).unwrap();
    assert!(out.contains("# skit:shim"));
    // The preamble must be the very first line (index 0).
    assert!(out.lines().next().unwrap().ends_with("# skit:shim"));
}
