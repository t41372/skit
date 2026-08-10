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
//! - Python `compile(out, "<test>", "exec")` -> `source_is_valid("python", &out)`.
//! - Python `_run_injected(source, stdin)` -> `run_injected(source, stdin)`: spawn the real
//!   `python3` interpreter (same discipline as `port_test_launcher_fix.rs` spawning `/bin/sh`),
//!   feed stdin, assert exit 0, return stdout. This verifies the run-time queue/echo/masking
//!   behavior the injected preamble carries, exactly as the oracle does.
//! - Python `spec(...)` fixture -> `spec(name)` (const/str, delivery Inject) plus field edits and
//!   `input_spec(name, order)` for input bindings. Const and Input both imply
//!   `ParameterDelivery::Inject`, which is what `plan_python_injection` selects on.
//!
//! Buckets:
//! - API EXISTS (33 real asserting tests): every `shim.inject` behavior — the whole file except
//!   the five below.
//! - CROSS-CRATE (5 `#[ignore]` stubs):
//!   - `test_physical_lines_matches_splitlines_on_ordinary_text`: white-box test of the
//!     Python-private helper `rewrite._physical_lines`. Rust locates edits through tree-sitter's
//!     native byte offsets, so there is no `splitlines()`-reconciliation helper to observe. The
//!     guarantee it protects is covered here by the three `..._survives_...` ports.
//!   - the four `write_injected` tests: owning tier is skit-cli's private `stage_injected_source`
//!     (`crates/skit-cli/src/run/command.rs:667`), unreachable from a skit-language integration
//!     test; two are monkeypatch fault-injection tests with no non-mock equivalent. NOTE: the
//!     Rust writer also DIVERGES from oracle 3b (see the `lands_outside_entry_dir` stub).

use std::collections::BTreeMap;
use std::io::Write as _;
use std::process::{Command, Stdio};

use skit_domain::parameters::{ParamDecl, ParameterBinding, ParameterDelivery, ParameterType};
use skit_language::{LanguageError, inject_values, source_is_valid};

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

/// Oracle `_run_injected(source, stdin="")`: run the injected output in a subprocess and return
/// stdout (behaviour verification). Assert exit 0, surfacing stderr on failure.
fn run_injected(source: &str, stdin: &str) -> String {
    let mut child = Command::new("python3")
        .arg("-c")
        .arg(source)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn python3");
    child
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(stdin.as_bytes())
        .expect("write stdin");
    let output = child.wait_with_output().expect("wait for python3");
    assert!(
        output.status.success(),
        "python3 exited nonzero: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("utf-8 stdout")
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
fn test_input_queue_by_order() {
    let out = inject(SCRIPT, &[input_spec("input-1", 0)], &[("input-1", "Alice")]).unwrap();
    // 3a: the managed call site itself is rewritten (input(...) -> _skit_i[K](...)) and a
    // single-line preamble defines the one-shot per-call-site overrides.
    assert!(out.contains(r#"who = _skit_i[0]("Your name: ")"#));
    assert!(out.contains("# skit:shim"));
    let stdout = run_injected(&out, "");
    assert!(stdout.contains("Alice Taipei 3"));
    // The prompt + injected value is echoed to mimic a terminal.
    assert!(stdout.contains("Your name: Alice"));
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
fn test_input_queue_exhaustion_falls_back_to_stdin() {
    let src = "a = input('a: ')\nb = input('b: ')\nprint(a, b)\n";
    let out = inject(src, &[input_spec("input-1", 0)], &[("input-1", "one")]).unwrap();
    let stdout = run_injected(&out, "two\n");
    // Call 0 consumes the queue; call 1 falls back to native stdin pass-through.
    assert!(stdout.contains("one two"));
}

#[test]
fn test_input_queue_in_loop_consumes_by_call_order() {
    let src = "vals = [input('v: ') for _ in range(3)]\nprint('|'.join(vals))\n";
    // The analyzer sees one input() call site (order 0), but it is invoked three times:
    // call 0 consumes the queue value; subsequent calls fall back to stdin. This is the key
    // advantage of the queue approach over in-place rewriting.
    let out = inject(src, &[input_spec("input-1", 0)], &[("input-1", "first")]).unwrap();
    let stdout = run_injected(&out, "second\nthird\n");
    assert!(stdout.contains("first|second|third"));
}

#[test]
fn test_input_queue_secret_masks_echo() {
    let src = "token = input('token: ')\nprint('len', len(token))\n";
    let mut secret = input_spec("input-1", 0);
    secret.secret = true;
    let out = inject(src, &[secret], &[("input-1", "hunter2")]).unwrap();
    let stdout = run_injected(&out, "");
    assert!(!stdout.contains("hunter2")); // Secret values must never be echoed
    assert!(stdout.contains("token: ***"));
    assert!(stdout.contains("len 7")); // But the script itself receives the real value
}

#[test]
fn test_input_queue_with_future_import() {
    let src = "\"\"\"doc\"\"\"\nfrom __future__ import annotations\nx = input()\nprint(x)\n";
    let out = inject(src, &[input_spec("input-1", 0)], &[("input-1", "ok")]).unwrap();
    let lines = out.lines().collect::<Vec<_>>();
    assert_eq!(lines[1], "from __future__ import annotations"); // preamble must go after __future__
    assert!(lines[2].ends_with("# skit:shim"));
    assert!(run_injected(&out, "").contains("ok"));
}

#[test]
fn test_input_queue_combined_with_const_injection() {
    let out = inject(
        SCRIPT,
        &[spec("CITY"), input_spec("input-1", 0)],
        &[("CITY", "Tainan"), ("input-1", "Bob")],
    )
    .unwrap();
    assert!(out.contains("CITY = 'Tainan'"));
    assert!(run_injected(&out, "").contains("Bob Tainan 3"));
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
    // returns [] and a stored input spec can't resolve — it must surface as drift (ShimError)
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
fn test_unshadowed_input_is_rewritten_to_the_wrapper() {
    // Control: the SAME input spec against an unshadowed script DOES rewrite the call site, so the
    // shadow guard is not firing unconditionally / returning [] always.
    let mut declaration = input_spec("input-1", 0);
    declaration.prompt = "Q: ".to_owned();
    let out = inject(
        "y = input('Q: ')\nprint(y)\n",
        &[declaration],
        &[("input-1", "typed")],
    )
    .unwrap();
    assert!(out.contains("y = _skit_i[0]('Q: ')"));
    assert!(run_injected(&out, "").contains("typed"));
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
    // WAS found; only the supplied value doesn't fit its declared int type. Callers (the CLI) need
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

#[test]
fn test_multiline_value_span() {
    // Parenthesised literal: the AST span covers the literal only; the parens are preserved
    // (semantically equivalent). Rust replaces the whole parenthesised span instead — the oracle
    // asserts only on the literal text, so both dispositions satisfy it.
    let src = "MSG = (\n    \"hello\"\n)\nprint(MSG)\n";
    let out = inject(src, &[spec("MSG")], &[("MSG", "bye")]).unwrap();
    assert!(out.contains("'bye'"));
    assert!(!out.contains("\"hello\""));
    assert!(source_is_valid("python", &out)); // Injected output must still be valid Python
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
fn test_inject_duplicate_prompt_winner_only_still_injects_and_compiles() {
    // The healthy end of the same scenario: once reconcile has dropped the surplus spec (as it now
    // correctly does, see test_reconcile), injecting only the surviving spec must still work
    // normally and produce compilable output with a single, non-doubled callee.
    let src = "first = input(\"Go? \")\nsecond = input(\"Go? \")\nprint(first, second)\n";
    let edited = src.replace("first = input(\"Go? \")\n", "");
    let mut input_one = input_spec("input-1", 0);
    input_one.prompt = "Go? ".to_owned();
    let out = inject(&edited, &[input_one], &[("input-1", "AAA")]).unwrap();
    assert!(!out.contains("_skit_i[0]_i[0]"));
    assert!(source_is_valid("python", &out));
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

// ---------- _apply: multi-line (cross-line) span replacement ----------

#[test]
fn test_multiline_span_replacement() {
    // Parenthesised literal spanning two lines must be replaced cleanly.
    let src = "X = (\n    \"old\"\n    \"also old\"\n)\nprint(X)\n";
    let out = inject(src, &[spec("X")], &[("X", "new")]).unwrap();
    assert!(out.contains("'new'"));
    assert!(source_is_valid("python", &out));
}

// ---------- _physical_lines: AST-line-boundary characters str.splitlines() over-splits on ----------

#[test]
#[ignore = "UNMAPPED (bucket 2): white-box Python-private helper rewrite._physical_lines; Rust uses tree-sitter native byte offsets, no equivalent to observe"]
fn test_physical_lines_matches_splitlines_on_ordinary_text() {
    // Oracle: for text in ("", "a", "a\nb", "a\nb\n", "a\r\nb\rc\n"):
    //   assert rewrite._physical_lines(text) == text.splitlines(keepends=True)
    // Rust has no `_physical_lines` (line/col -> byte reconciliation is not exposed); tree-sitter
    // yields byte offsets directly. The guarantee this protects (AST-line vs splitlines desync on
    // form feed / U+2028) is verified through the three `..._survives_...` ports below.
}

#[test]
fn test_const_injection_survives_form_feed_between_targets() {
    // A form-feed page break (e.g. an Emacs section marker) sits on its own physical line as far
    // as str.splitlines() is concerned, but the tokenizer/AST do NOT count it as a line break — so
    // indexing lines[lineno - 1] from splitlines() output lands on the wrong physical line entirely.
    // Reproduces the corruption: PORT's replacement used to land one physical line early (the form
    // feed's own splitlines() "line"), producing `\x0c\n9090PORT = 8080` and a SyntaxError.
    let src = "HOST = \"localhost\"\n\u{0c}\nPORT = 8080\nprint(HOST, PORT)\n";
    let mut port = spec("PORT");
    port.parameter_type = ParameterType::Int;
    let out = inject(src, &[port], &[("PORT", "9090")]).unwrap();
    assert_eq!(
        out,
        "HOST = \"localhost\"\n\u{0c}\nPORT = 9090\nprint(HOST, PORT)\n"
    );
    assert!(source_is_valid("python", &out)); // used to raise SyntaxError before the fix
}

#[test]
fn test_const_injection_survives_u2028_inside_earlier_string_literal() {
    // U+2028 (LINE SEPARATOR) is an ordinary character inside a Python string literal -- it does
    // not end the string, and the tokenizer does not treat it as a line break. str.splitlines(),
    // however, always treats it as one, so a line count computed that way silently disagrees with
    // the AST from the very next statement onward.
    let src = "MSG = \"hi\u{2028}there\"\nPORT = 8080\nprint(MSG, PORT)\n";
    let mut port = spec("PORT");
    port.parameter_type = ParameterType::Int;
    let out = inject(src, &[port], &[("PORT", "9090")]).unwrap();
    assert_eq!(
        out,
        "MSG = \"hi\u{2028}there\"\nPORT = 9090\nprint(MSG, PORT)\n"
    );
    assert!(source_is_valid("python", &out));
}

#[test]
fn test_preamble_insertion_survives_form_feed_inside_docstring() {
    // A form feed embedded inside the module docstring makes str.splitlines() split the
    // docstring into two entries, so the _insert_preamble index (computed from the true,
    // 1-entry-per-docstring AST line count) lands one entry early -- squarely inside the
    // docstring's text. The result still compiles (it's still valid Python), but input() is never
    // actually overridden, and the queued value is silently dropped with no error at all -- the
    // worst kind of failure this fix exists to prevent.
    let src = "\"\"\"line one\u{0c}line two\"\"\"\nname = input(\"who: \")\nprint(name)\n";
    let out = inject(src, &[input_spec("input-1", 0)], &[("input-1", "Bob")]).unwrap();
    // The docstring must be left intact as a single statement (the preamble must NOT have landed
    // inside it): it must still be the true first line, unsplit, before any skit-injected text.
    assert!(out.starts_with("\"\"\"line one\u{0c}line two\"\"\"\n"));
    assert!(out.contains("# skit:shim"));
    assert!(source_is_valid("python", &out));
    assert!(run_injected(&out, "").contains("Bob")); // the value actually reaches the script
}

// ---------- Input values are bound to their prompt/call site, not runtime call order ----------

#[test]
fn test_input_value_follows_prompt_despite_runtime_call_order_diverging_from_source_order() {
    // A function's input() is defined ABOVE a top-level input() in source order, but only
    // invoked (at runtime) AFTER it runs. The old design queued/consumed values by a single global
    // runtime counter keyed to *source* order, so the top-level call (which actually runs first)
    // stole the function's queued value and vice versa -- silently swapping a secret into the wrong
    // variable. Binding by call site (not a shared counter) must keep each value with its own
    // question regardless of execution order.
    let src = "def get_password():\n    return input(\"Password: \")\n\nusername = input(\"Username: \")\npassword = get_password()\nprint(username, password)\n";
    let mut password = input_spec("input-1", 0); // "Password: ", defined first
    password.secret = true;
    let username = input_spec("input-2", 1); // "Username: ", defined second, RUNS first
    let out = inject(
        src,
        &[password, username],
        &[("input-1", "SUPERSECRET"), ("input-2", "alice")],
    )
    .unwrap();
    let stdout = run_injected(&out, "");
    assert!(stdout.contains("alice SUPERSECRET")); // username=alice, password=SUPERSECRET — not swapped
    assert!(
        !stdout
            .replace("alice SUPERSECRET", "")
            .contains("SUPERSECRET")
    ); // only echoed as ***
    assert!(stdout.contains("Password: ***"));
    assert!(stdout.contains("Username: alice"));
}

#[test]
fn test_input_value_follows_prompt_after_an_earlier_input_is_deleted() {
    // Reproduces the reconcile/shim "positional key is unstable under source edits" gap: a stored
    // definition for the SECOND question ("Password: ", order=1, secret) must keep landing on the
    // Password prompt even after the user deletes the FIRST input() call from the source, which
    // shifts every remaining call's bare position down by one. Prompt-based matching (3a) resolves
    // this without needing the caller to re-add anything.
    let original_order = 1; // as recorded when both input() calls existed
    let edited_src = "password = input(\"Password: \")\nprint(\"got\", password)\n"; // first input() deleted
    let mut stored = input_spec("input-2", original_order);
    stored.secret = true;
    stored.prompt = "Password: ".to_owned();
    let out = inject(edited_src, &[stored], &[("input-2", "hunter2")]).unwrap();
    let stdout = run_injected(&out, "");
    assert!(stdout.contains("got hunter2"));
    assert!(stdout.contains("Password: ***")); // still masked as a secret, still the right prompt
}

// ---------- write_injected: exception during write cleans up the temp file ----------

#[test]
#[ignore = "CROSS-CRATE (skit-cli tier): oracle rewrite.write_injected monkeypatches os.fdopen to force an OSError and asserts no orphan .injected-* file remains. The Rust analogue is skit-cli's private stage_injected_source (crates/skit-cli/src/run/command.rs:667); it is unreachable from a skit-language integration test and this fault-injection has no non-mock equivalent."]
fn test_write_injected_cleanup_on_error() {
    // Oracle: with a failing os.fdopen, rewrite.write_injected(tmp_path, "print(1)\n", suffix=".py")
    // raises OSError("disk full") and leaves no .injected-*.py file behind.
}

// ---------- write_injected: fd leak when os.chmod raises before fdopen (nit fix) ----------

#[test]
#[ignore = "CROSS-CRATE (skit-cli tier): oracle monkeypatches os.chmod to raise before fdopen and asserts the fd was already closed (double close -> EBADF). The Rust analogue is skit-cli's private stage_injected_source (crates/skit-cli/src/run/command.rs:667), unreachable from here, and this fd-leak fault-injection has no non-mock equivalent."]
fn test_write_injected_closes_fd_when_chmod_raises() {
    // Oracle: os.chmod raises before fdopen; write_injected must still close the fd (a second
    // os.close on the captured fd must fail with EBADF) and leave no .injected-*.py behind.
}

// ---------- write_injected: 3b — the temp file no longer lives in the persistent store ----------

#[test]
#[ignore = "CROSS-CRATE (skit-cli tier) + DIVERGENCE: oracle rewrite.write_injected lands the plaintext-secret-bearing copy in the OS temp dir, NOT entry_dir (crates ref: skit-oracle rewrite.py:176-180). Rust's stage_injected_source writes entry_dir.join('.run-<id>') unconditionally (crates/skit-cli/src/run/command.rs:686-693), mitigated only by sweep_staged_sources. Unreachable from skit-language; owning tier is skit-cli."]
fn test_write_injected_lands_outside_entry_dir() {
    // Oracle: path = write_injected(tmp_path, "print(1)\n", suffix=".py"); path.parent != tmp_path;
    // (tmp_path / path.name) does not exist; path.name starts with ".injected-"; content preserved.
    // Rust diverges here: the injected copy lands inside entry_dir. See notes for the skit-cli
    // decision this needs.
}

#[test]
#[ignore = "CROSS-CRATE (skit-cli tier): oracle asserts write_injected falls back to entry_dir when the OS temp dir is unavailable (monkeypatched mkstemp). Rust's stage_injected_source already writes entry_dir directly (crates/skit-cli/src/run/command.rs:686-693); there is no OS-temp-first path to fall back FROM, and it is unreachable from skit-language regardless."]
fn test_write_injected_falls_back_to_entry_dir_if_os_temp_unavailable() {
    // Oracle: with the primary (OS-temp) mkstemp attempt forced to OSError, write_injected returns
    // a path whose parent == entry_dir.
}
