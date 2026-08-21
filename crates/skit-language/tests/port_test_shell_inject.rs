//! Mechanical port of the Python oracle module `tests/test_shell_inject.py`
//! (`origin/main@206f9ef`). This is the shell injector contract: the three deliveries (const,
//! env, read), the two syntax gates, and the four documented correctness risks
//! (#2 wrong call site, #3 quoting injection, #4 multibyte/CRLF, #7 double binding).
//!
//! Each `#[test]` keeps its Python `def test_*` name so it traces back to its origin, and the
//! Python "WHY" comment is preserved. Same input bytes, same expected output.
//!
//! ## This module spans layers. Every Python test is sorted into one of three buckets.
//!
//! The Python file mixes pure-logic tests with EXECUTION tests that spawn a real bash on the
//! injected copy and assert on the child's stdout. In the Rust architecture the injector produces
//! rewritten BYTES (`skit-language`), while running the injected copy end-to-end is a CLI/runtime
//! concern (`skit run --set`; see `skit-cli/tests/surface_edges.rs`). So:
//!
//! - **Bucket 1 (pure inject byte-logic / typed refusal):** asserted on
//!   `plan_injection(...).apply(source)` output bytes, `source_is_valid("shell", ..)` where the
//!   Python re-parses, and `LanguageError` where the Python raises `InjectError`/`InjectValueError`/
//!   `InjectSplitError`/`InjectGapError`. This is the bulk.
//! - **Bucket 2 (execution claim the injected BYTES fully establish):** ported as byte assertions
//!   with an explicit `PORTED AS BYTE ASSERTION:` note stating the runtime claim the byte form
//!   proves. An all-single-quoted literal is provably inert without running it; a value present at
//!   its target byte-for-byte "lands".
//! - **Bucket 3 (execution claim that genuinely needs a running shell, or a layer above
//!   skit-language):** kept as a compiling `#[ignore]` stub with its WHY comment. `#[ignore]` is
//!   used ONLY for this bucket.
//!
//! ## Error mapping (Python `skit.langs.base` -> `skit_language`)
//! - `InjectError` (target not found / two specs one site: drift) -> `LanguageError::BindingNotFound`.
//! - `InjectValueError` (value not coercible; `.param_name`) -> `LanguageError::InvalidValue{name}`.
//! - `InjectGapError` (`.empty`, `.filled`) -> `LanguageError::ShellInput(ShellInputError::Gap)`.
//! - `InjectSplitError` (`.reason`) -> `LanguageError::ShellInput(ShellInputError::{LineBreak
//!   (line-break), FieldSplit (field-split), EdgeSpace (edge-space)})`.
//! - `InjectSyntaxError` (post-inject gate) -> the re-parse gate inside `inject_values`; only
//!   reachable by monkeypatching the internal escaper, so those tests are Bucket 3.
//!
//! The injector produces bytes only. `InjectResult.path`/`.env`/`.warnings`, the 0600 temp copy,
//! the `bash -n` interpreter gate, and the `$0` warning STRING all live in the CLI/runtime tier,
//! so tests whose sole claim is one of those are Bucket 3.

use std::collections::BTreeMap;

use skit_domain::parameters::{
    ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, ParameterValue,
};
use skit_language::{
    LanguageError, ParseOutcome, ParsedDocument, SemanticCandidate, ShellInputError,
    SourceEditPlan, normalize_shell_default, parse_document, source_is_valid,
};

// ---------------------------------------------------------------- helpers

fn parsed(source: &str) -> ParsedDocument {
    match parse_document("shell", source) {
        ParseOutcome::Parsed(document) => document,
        other => panic!("expected valid shell, got {other:?}"),
    }
}

fn to_map(values: &[(&str, &str)]) -> BTreeMap<String, String> {
    values
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

/// Python `specs_of(src)` = the analyzer candidates' declarations.
fn specs_of(source: &str) -> Vec<ParamDecl> {
    parsed(source)
        .analysis()
        .candidates
        .into_iter()
        .map(|candidate| candidate.declaration)
        .collect()
}

/// Python `inject_src(src, values)`: plan with the default `interpreter="bash"`, then apply.
/// Returns the rewritten bytes, or the `LanguageError` the planner raised.
fn inject(source: &str, values: &[(&str, &str)]) -> Result<String, LanguageError> {
    inject_with_specs(source, values, &specs_of(source))
}

/// Python `inject_src(src, values, specs=...)`: the caller supplies the stored declarations.
fn inject_with_specs(
    source: &str,
    values: &[(&str, &str)],
    specs: &[ParamDecl],
) -> Result<String, LanguageError> {
    plan_with_specs(source, values, specs)?.apply(source)
}

/// Python `inject_src(src, values, interpreter=...)`.
fn inject_with_interpreter(
    source: &str,
    values: &[(&str, &str)],
    interpreter: &str,
) -> Result<String, LanguageError> {
    parsed(source)
        .plan_injection_for_interpreter(&specs_of(source), &to_map(values), Some(interpreter))?
        .apply(source)
}

fn plan(source: &str, values: &[(&str, &str)]) -> Result<SourceEditPlan, LanguageError> {
    plan_with_specs(source, values, &specs_of(source))
}

fn plan_with_specs(
    source: &str,
    values: &[(&str, &str)],
    specs: &[ParamDecl],
) -> Result<SourceEditPlan, LanguageError> {
    parsed(source).plan_injection_for_interpreter(specs, &to_map(values), Some("bash"))
}

/// The POSIX single-quote escaper contract shared by every str value the injector emits.
/// `'` is closed, an escaped `\'` is added, then the quote reopens — the exact form that makes an
/// injected value impossible to break out of.
fn sq(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// The `read`-delivery escaper: a raw (`read -r`) line is fed verbatim; a cooked line doubles every
/// backslash so `read`'s own unescaping restores the exact form value.
fn fed(value: &str, raw: bool) -> String {
    if raw {
        value.to_owned()
    } else {
        value.replace('\\', "\\\\")
    }
}

fn by_name(source: &str) -> BTreeMap<String, SemanticCandidate> {
    parsed(source)
        .analysis()
        .candidates
        .into_iter()
        .map(|candidate| (candidate.declaration.name.clone(), candidate))
        .collect()
}

fn input_spec(name: &str, order: i64, prompt: &str) -> ParamDecl {
    let mut declaration = ParamDecl::new(name);
    declaration.binding = ParameterBinding::Input;
    declaration.delivery = ParameterDelivery::Inject;
    declaration.order = order;
    declaration.prompt = prompt.to_owned();
    declaration
}

fn const_spec(name: &str, parameter_type: ParameterType) -> ParamDecl {
    let mut declaration = ParamDecl::new(name);
    declaration.binding = ParameterBinding::Const;
    declaration.delivery = ParameterDelivery::Inject;
    declaration.parameter_type = parameter_type;
    declaration
}

const PAYLOADS: [&str; 4] = [
    "'; touch pwned; echo '",
    "$(touch pwned)",
    "`touch pwned`",
    "$(id) && touch pwned",
];

// ---------------------------------------------------------------- const delivery

#[test]
fn test_const_injection_runs_with_the_new_value() {
    // PORTED AS BYTE ASSERTION: Python runs the copy and asserts stdout `w=1200`. The int value
    // lands bare at its target (`WIDTH=1200`); running only confirms bash echoes it.
    let src = "#!/usr/bin/env bash\nWIDTH=800\necho \"w=$WIDTH\"\n";
    let out = inject(src, &[("WIDTH", "1200")]).unwrap();
    assert_eq!(out, "#!/usr/bin/env bash\nWIDTH=1200\necho \"w=$WIDTH\"\n");
    assert!(source_is_valid("shell", &out));
}

#[test]
fn test_const_str_is_single_quoted_and_int_is_bare() {
    let src = "#!/usr/bin/env bash\nWIDTH=800\nCITY=Taipei\n";
    let out = inject(src, &[("WIDTH", "1200"), ("CITY", "New York")]).unwrap();
    assert!(out.contains("WIDTH=1200")); // int coerces -> bare word, no quotes
    assert!(out.contains("CITY='New York'")); // str -> POSIX single-quoted, always
}

#[test]
fn test_const_rewrites_every_same_name_occurrence() {
    // PORTED AS BYTE ASSERTION: the byte count establishes every same-named occurrence is
    // rewritten; the Python run (`turbo\n`) then just reads `$MODE`.
    let src = "#!/usr/bin/env bash\nMODE=fast\nMODE=slow\necho \"$MODE\"\n";
    let out = inject(src, &[("MODE", "turbo")]).unwrap();
    assert_eq!(out.matches("MODE='turbo'").count(), 2);
    assert!(source_is_valid("shell", &out));
}

#[test]
fn test_const_quoting_is_normalized_not_preserved() {
    // The source quoting (raw string, double-quoted, bare word) is irrelevant: every str value
    // comes out single-quoted, which is what makes injection impossible.
    let src = "#!/usr/bin/env bash\nA=bare\nB='raw'\nC=\"double\"\n";
    let out = inject(src, &[("A", "x y"), ("B", "x y"), ("C", "x y")]).unwrap();
    assert_eq!(out.matches("='x y'").count(), 3);
}

#[test]
fn test_bad_int_value_raises_the_value_error_not_drift() {
    let src = "#!/usr/bin/env bash\nWIDTH=800\n";
    match plan(src, &[("WIDTH", "not-a-number")]) {
        Err(LanguageError::InvalidValue { name, .. }) => assert_eq!(name, "WIDTH"),
        other => panic!("expected InvalidValue for WIDTH, got {other:?}"),
    }
}

#[test]
fn test_bad_float_and_non_finite_values_are_refused() {
    let src = "#!/usr/bin/env bash\nRATE=0.5\n";
    for bad in ["abc", "inf", "-inf", "nan"] {
        match plan(src, &[("RATE", bad)]) {
            Err(LanguageError::InvalidValue { name, .. }) => assert_eq!(name, "RATE"),
            other => panic!("expected InvalidValue for RATE={bad:?}, got {other:?}"),
        }
    }
}

#[test]
fn test_float_const_injects_a_bare_number() {
    // PORTED AS BYTE ASSERTION: Python runs the copy and asserts stdout `r=2.75`. The float value
    // lands as a bare number at its target.
    let src = "#!/usr/bin/env bash\nRATE=0.5\necho \"r=$RATE\"\n";
    let out = inject(src, &[("RATE", "2.75")]).unwrap();
    assert_eq!(out, "#!/usr/bin/env bash\nRATE=2.75\necho \"r=$RATE\"\n");
    assert!(source_is_valid("shell", &out));
}

#[test]
fn test_missing_const_target_is_drift() {
    // The analyzer never offers GONE, so a stored definition naming it is drift, not a value error.
    let src = "#!/usr/bin/env bash\nWIDTH=800\n";
    let spec = const_spec("GONE", ParameterType::Str);
    match plan_with_specs(src, &[("GONE", "x")], &[spec]) {
        Err(LanguageError::BindingNotFound { name }) => assert_eq!(name, "GONE"),
        other => panic!("expected BindingNotFound for GONE (not InvalidValue), got {other:?}"),
    }
}

#[test]
fn test_readonly_const_is_never_a_target() {
    // The analyzer never offers a readonly const, so a stored definition naming one is drift —
    // rewriting it would produce a script that dies with "readonly variable" at run time.
    let src = "#!/usr/bin/env bash\nreadonly MAX=100\n";
    let spec = const_spec("MAX", ParameterType::Int);
    match plan_with_specs(src, &[("MAX", "5")], &[spec]) {
        Err(LanguageError::BindingNotFound { name }) => assert_eq!(name, "MAX"),
        other => panic!("expected BindingNotFound for readonly MAX, got {other:?}"),
    }
}

#[test]
fn test_const_targets_skip_array_and_valueless_assignments() {
    // `ARR[0]=…` (a subscript target) and `EMPTY=` (no value) are not const candidates, so they are
    // not rewrite targets either — the two sides must agree, or a run would rewrite what the form
    // never offered. PORTED AS BYTE ASSERTION for the untouched-neighbours claim (Python run: `12001[]`).
    let src =
        "#!/usr/bin/env bash\nARR[0]=1\nEMPTY=\nWIDTH=800\necho \"$WIDTH${ARR[0]}[$EMPTY]\"\n";
    let out = inject(src, &[("WIDTH", "1200")]).unwrap();
    assert!(out.contains("ARR[0]=1"));
    assert!(out.contains("EMPTY=\n"));
    assert!(out.contains("WIDTH=1200"));
}

#[test]
fn test_no_values_writes_nothing_at_all() {
    // Python: `result.path is None` — no temp copy, no rewrite, the original runs. In skit-language
    // this is "the plan has no edits", so `apply` returns the source byte-for-byte.
    // (`result.env == {}` is the assembly tier.)
    let src = "#!/usr/bin/env bash\nWIDTH=800\nread -p 'Name: ' who\n";
    assert!(plan(src, &[]).unwrap().edits().is_empty());
    assert_eq!(inject(src, &[]).unwrap(), src);
}

// ---------------------------------------------------------------- env delivery

#[test]
fn test_env_delivery_writes_no_temp_file() {
    // THE point of env delivery: zero rewrite, no temp copy. Env-delivered params are excluded from
    // the injector's selection, so the plan has no edits. (`result.env`/`warnings` are Tier 3/4.)
    let src = "#!/usr/bin/env bash\necho \"${GREETING:-hello}\"\n";
    assert!(
        plan(src, &[("GREETING", "hi there")])
            .unwrap()
            .edits()
            .is_empty()
    );
    assert_eq!(inject(src, &[("GREETING", "hi there")]).unwrap(), src);
}

#[test]
#[ignore = "UNMAPPED: runtime env delivery reaching the child (spawns bash with GREETING in env) -> Tier 3/4 (skit run / skit-runtime); does not call the injector at all"]
fn test_env_delivery_actually_reaches_the_script() {
    // Python spawns bash with env={GREETING: "hi there"} and asserts stdout. No injection; pure
    // child-process environment delivery, which is the runtime tier.
}

#[test]
fn test_mixed_env_and_const_delivery() {
    // The envdefault never touches the source, but the const still needs its temp copy: only WIDTH
    // is rewritten; `${MODE:-auto}` stays. (`result.env == {MODE: manual}` is the assembly tier.)
    let src = "#!/usr/bin/env bash\nWIDTH=800\necho \"${MODE:-auto} $WIDTH\"\n";
    let out = inject(src, &[("WIDTH", "1200"), ("MODE", "manual")]).unwrap();
    assert!(out.contains("WIDTH=1200"));
    assert!(out.contains("${MODE:-auto}")); // untouched
}

// ---------------------------------------------------------------- read delivery

#[test]
#[ignore = "UNMAPPED: runtime prompt echo + value delivery through the _skit_read shim (spawns bash, asserts stdout) -> Tier 3/4 (skit run --set). The byte rewrite is covered by test_read_rewrite_keeps_every_flag_and_varname."]
fn test_read_interception_echoes_prompt_and_value() {
    // Python runs the copy and asserts `Name: Ada\nhi Ada\n` — the shim's runtime echo and heredoc
    // delivery, which needs a shell.
}

#[test]
fn test_read_rewrite_keeps_every_flag_and_varname() {
    let src = "#!/usr/bin/env bash\nread -r -p \"Name: \" who\n";
    let out = inject(src, &[("input-1", "Ada")]).unwrap();
    assert!(out.contains("_skit_read 0 'Ada' 0 'Name: ' -r -p \"Name: \" who"));
}

#[test]
fn test_function_read_defined_above_invoked_after_keeps_its_value() {
    // Risk #2, the one that makes call-site binding non-negotiable: the function's read is FIRST in
    // source order but runs LAST. A runtime counter would swap the two values (and hand the secret
    // to the wrong question); binding to the call site cannot.
    //
    // PORTED AS BYTE ASSERTION: the byte form binds each value to its own SOURCE call site —
    // SUPERSECRET into the function's read (command 0, source-first) with secret flag 1, alice into
    // the top-level read (command 1) with flag 0 — regardless of runtime invocation order. The
    // Python runtime `name=alice pw=SUPERSECRET` follows from that binding; the masked echo is
    // covered by the (Tier 3/4) secret-masking tests.
    let src = concat!(
        "#!/usr/bin/env bash\n",
        "ask_secret() {\n",
        "  read -s -p \"Password: \" PW\n",
        "}\n",
        "read -p \"Name: \" NAME\n",
        "ask_secret\n",
        "echo \"name=$NAME pw=$PW\"\n",
    );
    let out = inject(src, &[("input-1", "SUPERSECRET"), ("input-2", "alice")]).unwrap();
    assert!(out.contains("_skit_read 0 'SUPERSECRET' 1 'Password: ' -s -p \"Password: \" PW"));
    assert!(out.contains("_skit_read 1 'alice' 0 'Name: ' -p \"Name: \" NAME"));
    assert!(source_is_valid("shell", &out));
}

#[test]
fn test_two_specs_claiming_one_read_site_is_drift() {
    // Risk #7: two definitions carrying the same order (a hand-edited block) would otherwise splice
    // two replacements over one command-name span, corrupting the copy into unparsable text.
    let src = "#!/usr/bin/env bash\nread -p \"Go? \" a\n";
    let specs = [
        input_spec("input-1", 0, "Go? "),
        input_spec("input-2", 0, "Go? "),
    ];
    match plan_with_specs(src, &[("input-1", "AAA"), ("input-2", "BBB")], &specs) {
        Err(LanguageError::BindingNotFound { name }) => assert_eq!(name, "input-2"),
        other => panic!("expected BindingNotFound for input-2, got {other:?}"),
    }
}

#[test]
fn test_vanished_read_site_is_drift() {
    let src = "#!/usr/bin/env bash\nread -p \"Go? \" a\n";
    let spec = input_spec("input-3", 2, "Gone? ");
    match plan_with_specs(src, &[("input-3", "x")], &[spec]) {
        Err(LanguageError::BindingNotFound { name }) => assert_eq!(name, "input-3"),
        other => panic!("expected BindingNotFound for input-3, got {other:?}"),
    }
}

#[test]
fn test_value_follows_its_prompt_not_its_position() {
    // A new read inserted ABOVE an existing one shifts every position; the stored value must still
    // land on its own question (shared callmatch — the same rule reconcile uses).
    //
    // PORTED AS BYTE ASSERTION: the stored value binds to the "Password: " read (command 1), not to
    // position 0; the newly-inserted "Name: " read is left as a plain `read` (so it reads real
    // stdin at runtime — the `name=[typed]` half). Python runtime `pw=hunter2 name=[typed]` follows.
    let stored = [{
        let mut declaration = input_spec("input-1", 0, "Password: ");
        declaration.secret = true;
        declaration
    }];
    let edited = concat!(
        "#!/usr/bin/env bash\n",
        "read -p \"Name: \" NAME\n", // a new read, inserted first
        "read -s -p \"Password: \" PW\n",
        "echo \"pw=$PW name=[$NAME]\"\n",
    );
    let out = inject_with_specs(edited, &[("input-1", "hunter2")], &stored).unwrap();
    assert!(out.contains("_skit_read 1 'hunter2' 1 'Password: ' -s -p \"Password: \" PW"));
    assert!(out.contains("read -p \"Name: \" NAME")); // the value followed its prompt, not position 0
    assert!(source_is_valid("shell", &out));
}

#[test]
fn test_multi_variable_read_joins_its_values_on_one_line() {
    // PORTED AS BYTE ASSERTION: the two values join into one fed line `'Ada Lovelace'`; the shell's
    // default-$IFS split back into `[Ada][Lovelace]` is what the Python run confirms.
    let src =
        "#!/usr/bin/env bash\nread -p \"First and last: \" FIRST LAST\necho \"[$FIRST][$LAST]\"\n";
    let out = inject(src, &[("input-1", "Ada"), ("input-2", "Lovelace")]).unwrap();
    assert!(out.contains(
        "_skit_read 0 'Ada Lovelace' 0 'First and last: ' -p \"First and last: \" FIRST LAST"
    ));
}

#[test]
fn test_multi_variable_read_accepts_a_short_prefix() {
    // Only the first variable filled: exactly what a short typed line does (the rest read empty).
    // PORTED AS BYTE ASSERTION: the fed line is just `'Ada'`; the runtime `[Ada][]` follows.
    let src =
        "#!/usr/bin/env bash\nread -p \"First and last: \" FIRST LAST\necho \"[$FIRST][$LAST]\"\n";
    let out = inject(src, &[("input-1", "Ada")]).unwrap();
    assert!(
        out.contains("_skit_read 0 'Ada' 0 'First and last: ' -p \"First and last: \" FIRST LAST")
    );
}

#[test]
fn test_multi_variable_read_refuses_a_positional_gap() {
    // input-1 empty + input-2 filled: one `read` line cannot express that — the shell would hand
    // "Lovelace" to FIRST. Refused loudly instead of binding the value to the wrong variable.
    let src = "#!/usr/bin/env bash\nread -p \"First and last: \" FIRST LAST\n";
    match plan(src, &[("input-2", "Lovelace")]) {
        Err(LanguageError::ShellInput(ShellInputError::Gap { empty, filled })) => {
            assert_eq!((empty.as_str(), filled.as_str()), ("input-1", "input-2"));
        }
        other => panic!("expected Gap(input-1, input-2), got {other:?}"),
    }
}

#[test]
fn test_multi_variable_read_refuses_whitespace_in_a_non_last_field() {
    // "John Paul" in FIRST would spill across the IFS boundary when the joined line is re-split
    // ("John" → FIRST, "Paul" → LAST). Refused instead of silently delivering the wrong value.
    let src = "#!/usr/bin/env bash\nread -p \"First and last: \" FIRST LAST\n";
    match plan(src, &[("input-1", "John Paul"), ("input-2", "Doe")]) {
        Err(LanguageError::ShellInput(ShellInputError::FieldSplit { name })) => {
            assert_eq!(name, "input-1");
        }
        other => panic!("expected FieldSplit(input-1), got {other:?}"),
    }
}

#[test]
fn test_read_refuses_a_newline_in_any_field_including_a_single_variable() {
    // A newline ENDS the read's line, so no variable can hold it — not even the only one. Accepting
    // it delivered "a" while skit's own echo showed "a\nb": the value and the echo disagreed.
    let single = "#!/usr/bin/env bash\nread -p \"Name: \" NAME\n";
    match plan(single, &[("input-1", "a\nb")]) {
        Err(LanguageError::ShellInput(ShellInputError::LineBreak { name })) => {
            assert_eq!(name, "input-1"); // reason == "line-break"
        }
        other => panic!("expected LineBreak(input-1), got {other:?}"),
    }
    // and in the LAST variable of a multi-var read, which is exempt from field-splitting only
    let multi = "#!/usr/bin/env bash\nread -p \"A B: \" A B\n";
    match plan(multi, &[("input-1", "x"), ("input-2", "a\nb")]) {
        Err(LanguageError::ShellInput(ShellInputError::LineBreak { name })) => {
            assert_eq!(name, "input-2"); // reason == "line-break"
        }
        other => panic!("expected LineBreak(input-2), got {other:?}"),
    }
}

#[test]
fn test_read_refuses_edge_whitespace_that_the_shell_would_strip() {
    // `read` strips leading/trailing IFS whitespace off the line, so " lead" would arrive as "lead"
    // — a silent modification. Interior spaces in the last variable are fine (the line's remainder).
    let src = "#!/usr/bin/env bash\nread -p \"Name: \" NAME\n";
    for edge in [" lead", "trail ", "\ttab-lead"] {
        match plan(src, &[("input-1", edge)]) {
            Err(LanguageError::ShellInput(ShellInputError::EdgeSpace { name })) => {
                assert_eq!(name, "input-1"); // reason == "edge-space"
            }
            other => panic!("expected EdgeSpace for {edge:?}, got {other:?}"),
        }
    }
    assert!(plan(src, &[("input-1", "de Lovelace")]).is_ok()); // interior: accepted
}

#[test]
fn test_read_accepts_a_carriage_return_which_the_shell_delivers_intact() {
    // CR is neither a default-$IFS splitter nor a line terminator: every supported shell hands the
    // value over byte-intact, so refusing it would be a false positive.
    //
    // PORTED AS BYTE ASSERTION: the CR passes the escaper unmodified into the single-quoted fed
    // value `'a\rb'` (real CR byte), so the shell delivers `<a\rb>`; the Python run confirms it.
    let src = "#!/usr/bin/env bash\nread -p \"V: \" V\nprintf \"<%s>\" \"$V\"\n";
    let out = inject(src, &[("input-1", "a\rb")]).unwrap();
    let call = format!("_skit_read 0 {} 0 'V: ' -p \"V: \" V", sq("a\rb"));
    assert!(out.contains(&call), "expected {call:?} in\n{out}");
    assert!(source_is_valid("shell", &out));
}

#[test]
fn test_multi_variable_read_refuses_whitespace_when_a_trailing_var_is_unmanaged() {
    // The exemption is keyed on the read's last VARIABLE, not the last supplied value: with only
    // input-1 managed, the shell still binds LAST from the same line, so "John Paul" would have
    // silently delivered FIRST="John", LAST="Paul". Refused.
    let src = "#!/usr/bin/env bash\nread -p \"First and last: \" FIRST LAST\n";
    match plan(src, &[("input-1", "John Paul")]) {
        Err(LanguageError::ShellInput(ShellInputError::FieldSplit { name })) => {
            assert_eq!(name, "input-1");
        }
        other => panic!("expected FieldSplit(input-1), got {other:?}"),
    }
}

#[test]
fn test_multi_variable_read_refuses_a_newline_in_a_non_last_field() {
    // A newline in an earlier value truncates the whole line and silently discards EVERY later
    // field. It must be refused rather than silently run. (Python asserts InjectSplitError; the
    // newline is caught first, so a LineBreak split error.)
    let src = "#!/usr/bin/env bash\nread -p \"First and last: \" FIRST LAST\n";
    assert!(matches!(
        plan(src, &[("input-1", "a\nb"), ("input-2", "KEEP")]),
        Err(LanguageError::ShellInput(
            ShellInputError::LineBreak { .. }
                | ShellInputError::FieldSplit { .. }
                | ShellInputError::EdgeSpace { .. }
        ))
    ));
}

#[test]
fn test_multi_variable_read_allows_whitespace_in_the_last_field() {
    // The LAST variable absorbs the remainder of the line, so it may safely hold spaces — exactly
    // what a typed multi-word tail does.
    //
    // PORTED AS BYTE ASSERTION: the fed line is `'Ada de Lovelace'`; the shell splits it as
    // FIRST="Ada", LAST="de Lovelace", which the Python run (`[Ada][de Lovelace]`) confirms.
    let src =
        "#!/usr/bin/env bash\nread -p \"First and last: \" FIRST LAST\necho \"[$FIRST][$LAST]\"\n";
    let out = inject(src, &[("input-1", "Ada"), ("input-2", "de Lovelace")]).unwrap();
    assert!(out.contains(
        "_skit_read 0 'Ada de Lovelace' 0 'First and last: ' -p \"First and last: \" FIRST LAST"
    ));
}

#[test]
#[ignore = "UNMAPPED: `skit params --manage` + `skit run --set --no-input` reporting a whitespace split as FAIL_BAD_VALUE exit code -> Tier 4 (skit-cli). The split refusal itself is covered by test_multi_variable_read_refuses_whitespace_in_a_non_last_field."]
fn test_execute_reports_a_whitespace_split_as_a_bad_value() {
    // Python drives cli.app run --set and asserts the FAIL_BAD_VALUE exit code + "input-1" output.
}

#[test]
fn test_builtin_read_spelling_is_rewritten_whole() {
    // `builtin read x` must become `_skit_read …  x`, not `builtin _skit_read …` (which would try
    // to run the wrapper function as a shell builtin and fail).
    // PORTED AS BYTE ASSERTION for the whole-rewrite; the Python run just confirms it executes.
    let src = "#!/usr/bin/env bash\nbuiltin read -p \"Name: \" who\necho \"hi $who\"\n";
    let out = inject(src, &[("input-1", "Ada")]).unwrap();
    assert!(!out.contains("builtin _skit_read"));
    assert!(out.contains("_skit_read 0 'Ada' 0 'Name: ' -p \"Name: \" who"));
    assert!(source_is_valid("shell", &out));
}

#[test]
fn test_unmanaged_read_still_reads_real_stdin() {
    // PORTED AS BYTE ASSERTION: only the managed read (One:) is intercepted; the unmanaged read
    // (Two:) is left as a plain `read`, which is WHY it reads real stdin — the Python run
    // (stdin "typed") ends `[injected][typed]`.
    let src = "#!/usr/bin/env bash\nread -p \"One: \" a\nread -p \"Two: \" b\necho \"[$a][$b]\"\n";
    let out = inject(src, &[("input-1", "injected")]).unwrap();
    assert!(out.contains("_skit_read 0 'injected' 0 'One: ' -p \"One: \" a"));
    assert!(out.contains("read -p \"Two: \" b")); // second read left untouched
}

// ---------------------------------------------------------------- risk #3: quoting injection

#[test]
fn test_const_payload_is_inert() {
    // MUST-VERIFY (risk #3). PORTED AS BYTE ASSERTION: an all-single-quoted literal is provably
    // inert WITHOUT running it. Each payload comes out as `TITLE={sq(payload)}` — a POSIX
    // single-quoted word with no unescaped expansion — and the whole copy re-parses (the offline
    // `source_is_valid` gate, which is the `bash -n` analog Python asserts). Python then runs it and
    // proves no `pwned` file appears and the variable holds the payload as literal text.
    for payload in PAYLOADS {
        let src = "#!/usr/bin/env bash\nTITLE=hello\necho \"[$TITLE]\"\n";
        let out = inject(src, &[("TITLE", payload)]).unwrap();
        let expected = format!(
            "#!/usr/bin/env bash\nTITLE={}\necho \"[$TITLE]\"\n",
            sq(payload)
        );
        assert_eq!(out, expected, "payload {payload:?}");
        assert!(
            source_is_valid("shell", &out),
            "payload {payload:?} must reparse (bash -n analog)"
        );
    }
}

#[test]
fn test_read_payload_is_inert() {
    // MUST-VERIFY (risk #3). PORTED AS BYTE ASSERTION: the read value is passed to `_skit_read` as
    // an inert single-quoted argument `{sq(payload)}` and the copy re-parses, so nothing in the
    // payload is a shell operator at parse time; the shim then feeds it literally via heredoc.
    // Python runs it and proves no `pwned` file and `[payload]` echoed verbatim.
    for payload in PAYLOADS {
        let src = "#!/usr/bin/env bash\nread -p \"Name: \" who\necho \"[$who]\"\n";
        let out = inject(src, &[("input-1", payload)]).unwrap();
        let call = format!(
            "_skit_read 0 {} 0 'Name: ' -p \"Name: \" who",
            sq(&fed(payload, false))
        );
        assert!(
            out.contains(&call),
            "payload {payload:?}: expected inert arg {call:?} in\n{out}"
        );
        assert!(
            source_is_valid("shell", &out),
            "payload {payload:?} must reparse"
        );
    }
}

#[test]
fn test_quote_in_a_read_prompt_survives() {
    // The PROMPT is re-emitted as an argument, so it goes through the same escaper: an apostrophe in
    // the script's own prompt text must not break out of the single-quoted argument.
    // PORTED AS BYTE ASSERTION: the prompt is emitted single-quoted with `'` escaped. The Python
    // runtime owner confirms that the exact prompt text echoes intact.
    let src = "#!/usr/bin/env bash\nread -p \"It's here: \" who\necho \"[$who]\"\n";
    let out = inject(src, &[("input-1", "x")]).unwrap();
    let call = format!(
        "_skit_read 0 'x' 0 {} -p \"It's here: \" who",
        sq("It's here: ")
    );
    assert!(out.contains(&call), "expected {call:?} in\n{out}");
    assert!(source_is_valid("shell", &out));
}

// ---------------------------------------------------------------- risk #4: multibyte / CRLF

#[test]
fn test_cjk_emoji_const_and_prompt_round_trip() {
    // Multibyte const value and CJK/emoji prompt survive the rewrite byte-for-byte and the copy
    // re-parses (Python: `not analyzer.analyze(text).syntax_error`). PORTED AS BYTE ASSERTION for
    // the round trip; the Python run confirms the echoed stdout.
    let src =
        "#!/usr/bin/env bash\nCITY=台北\nread -p \"请输入名字 🙂: \" NAME\necho \"$CITY|$NAME\"\n";
    let out = inject(src, &[("CITY", "高雄 🚀"), ("input-1", "愛達")]).unwrap();
    assert!(out.contains("CITY='高雄 🚀'"));
    assert!(out.contains("_skit_read 0 '愛達' 0 '请输入名字 🙂: ' -p \"请输入名字 🙂: \" NAME"));
    assert!(source_is_valid("shell", &out));
}

#[test]
fn test_crlf_script_injects_and_runs() {
    // The line ending survived; only the value's bytes changed. PORTED AS BYTE ASSERTION for the
    // preserved CRLF + reparse; the Python run confirms exit 0 and stdout.
    let src = "#!/usr/bin/env bash\r\nWIDTH=800\r\nHEIGHT=600\r\necho \"$WIDTH\"\r\n";
    let out = inject(src, &[("WIDTH", "1200")]).unwrap();
    assert!(out.contains("WIDTH=1200\r\n")); // only the value's bytes changed
    assert!(source_is_valid("shell", &out));
}

#[test]
fn test_no_trailing_newline_script_injects() {
    let src = "#!/usr/bin/env bash\nVERSION=1.2.0";
    let out = inject(src, &[("VERSION", "2.0.0")]).unwrap();
    assert!(out.ends_with("VERSION='2.0.0'"));
}

#[test]
fn test_no_shebang_puts_the_preamble_at_the_very_top() {
    // PORTED AS BYTE ASSERTION for the preamble placement; the Python run confirms it executes.
    let src = "read -p \"Name: \" who\necho \"hi $who\"\n";
    let out = inject(src, &[("input-1", "Ada")]).unwrap();
    assert!(out.starts_with("_skit_read() {"));
    assert!(source_is_valid("shell", &out));
}

#[test]
fn test_preamble_lands_after_the_shebang() {
    let src = "#!/usr/bin/env bash\nread -p \"Name: \" who\n";
    let out = inject(src, &[("input-1", "Ada")]).unwrap();
    let lines = out.lines().collect::<Vec<_>>();
    assert_eq!(lines[0], "#!/usr/bin/env bash");
    assert_eq!(lines[1], "_skit_read() {");
}

// ---------------------------------------------------------------- documented dialect variance

#[test]
fn test_backslash_values_arrive_byte_identical_raw_or_not() {
    // The delivery contract: what is in the form is what the script gets. A bare `read` UNESCAPES
    // backslashes in the line it consumes, so skit doubles them; a `read -r` takes the line
    // literally, so it must NOT. Both land byte-identical.
    //
    // PORTED AS BYTE ASSERTION: the raw/cooked doubling is the byte mechanism that makes the shell's
    // `read` deliver each value byte-identical. The value that made this load-bearing: a trailing
    // backslash in a NON-LAST variable escaped skit's own join separator, merging two fields —
    // "C:\\" + "Doe" must stay `[C:\][Doe]`, not `[C: Doe][]`. The Python runs each and asserts
    // the delivered stdout; here the fed lines carry the correct escaping.
    let value = "a\\b";

    let raw = "#!/usr/bin/env bash\nread -r -p \"P: \" a\necho \"[$a]\"\n";
    let out = inject(raw, &[("input-1", value)]).unwrap();
    assert!(out.contains(&format!(
        "_skit_read 0 {} 0 'P: ' -r -p \"P: \" a",
        sq(&fed(value, true))
    )));
    assert!(source_is_valid("shell", &out));

    let cooked = "#!/usr/bin/env bash\nread -p \"P: \" a\necho \"[$a]\"\n";
    let out = inject(cooked, &[("input-1", value)]).unwrap();
    // doubled for the non-raw read, so read's unescaping restores it
    assert!(out.contains(&format!(
        "_skit_read 0 {} 0 'P: ' -p \"P: \" a",
        sq(&fed(value, false))
    )));
    assert!(source_is_valid("shell", &out));

    // the field-merging case: a trailing backslash in a non-last variable
    let two = "#!/usr/bin/env bash\nread -p \"P: \" A B\necho \"[$A][$B]\"\n";
    let out = inject(two, &[("input-1", "C:\\"), ("input-2", "Doe")]).unwrap();
    let line = format!("{} {}", fed("C:\\", false), fed("Doe", false));
    assert!(out.contains(&format!(
        "_skit_read 0 {} 0 'P: ' -p \"P: \" A B",
        sq(&line)
    ))); // NOT `C: Doe`
    assert!(source_is_valid("shell", &out));

    // A const value is not read()'s business at all, so it is byte-exact in every dialect:
    let const_src = "#!/usr/bin/env bash\nP=x\necho \"[$P]\"\n";
    let out = inject(const_src, &[("P", value)]).unwrap();
    assert!(out.contains(&format!("P={}", sq(value))));
}

#[test]
fn test_reframing_and_custom_ifs_reads_are_never_offered() {
    // A read that reframes its input (-n/-N/-d) or redefines IFS cannot receive a value through the
    // one line skit feeds it — `read -n 3 X` would truncate "abcdefgh" to "abc", and `IFS=: read A B`
    // would hand the whole space-joined line to A. Neither is offered as a candidate at all, the same
    // honest degradation `read -a` already gets.
    for src in [
        "read -n 3 X\n",
        "read -N 5 X\n",
        "read -d : X\n",
        "IFS=: read A B\n",
        "IFS= read -r LINE\n",
        "read -a ARR\n",
    ] {
        assert!(parsed(src).analysis().candidates.is_empty(), "{src}");
    }
    // …while an ordinary read still is
    let names = parsed("read -p \"p: \" A B\n")
        .analysis()
        .candidates
        .into_iter()
        .map(|candidate| candidate.declaration.name)
        .collect::<Vec<_>>();
    assert_eq!(names, ["input-1", "input-2"]);
}

#[test]
fn test_fallthrough_keyword_is_dialect_selected() {
    // `command read` is a silent no-op in zsh; `builtin` does not exist in dash. There is no single
    // keyword that works everywhere — so the dialect picks it. Python calls the private
    // `inject._fallthrough_keyword`; here the same choice is observed through the emitted preamble.
    let keyword = |interpreter: &str| -> String {
        let src = "read X\n";
        let out = inject_with_interpreter(src, &[("input-1", "v")], interpreter).unwrap();
        if out.contains("builtin read \"$@\"") {
            "builtin".to_owned()
        } else if out.contains("command read \"$@\"") {
            "command".to_owned()
        } else {
            panic!("no fall-through keyword in preamble:\n{out}")
        }
    };
    assert_eq!(keyword("bash"), "builtin");
    assert_eq!(keyword("zsh"), "builtin");
    assert_eq!(keyword("/bin/zsh"), "builtin");
    assert_eq!(keyword("bash.exe"), "builtin");
    assert_eq!(keyword("sh"), "command");
    assert_eq!(keyword("dash"), "command");
    assert_eq!(keyword("ksh"), "command");
    assert_eq!(keyword(""), "command"); // no interpreter, no shebang -> `sh` default -> command
}

// ---------------------------------------------------------------- the two syntax gates

#[test]
#[ignore = "UNMAPPED: the offline gate fires only for a BROKEN escaper (Python monkeypatches inject.quote to drop the closing quote). The gate exists — inject_values re-parses the output and returns InvalidSource — but skit-language exposes no way to corrupt the internal escaper, so the failure path is unreachable from a public test."]
fn test_offline_gate_refuses_a_corrupted_injection() {
    // Python monkeypatches `inject.quote` to emit an unterminated quote and expects InjectSyntaxError.
}

#[test]
#[ignore = "UNMAPPED: gate 2 is the interpreter's own `bash -n` re-check, which spawns bash -> Tier 3/4 (skit-runtime); Python also monkeypatches `_gate_reparse` and `quote`. skit-language performs no interpreter spawn."]
fn test_interpreter_gate_refuses_what_the_offline_gate_missed() {
    // Python pretends the offline re-parse passed and proves `bash -n` still stops the launch.
}

#[test]
#[ignore = "UNMAPPED: the `bash -n` interpreter gate (and skipping it when the shell is not installed) is a runtime spawn concern -> Tier 3/4. skit-language's plan_injection_for_interpreter uses the interpreter only to pick the preamble keyword; it never runs the shell, so there is no gate to skip."]
fn test_interpreter_gate_is_skipped_when_the_shell_is_not_installed() {
    // Python injects with interpreter="skit-no-such-shell" and asserts result.path is not None.
}

#[test]
#[ignore = "UNMAPPED: Python monkeypatches subprocess.run to raise and asserts the run continues (gate 1 already passed). The subprocess spawn is the runtime tier -> Tier 3/4."]
fn test_interpreter_gate_survives_a_spawn_failure() {
    // Python: the `bash -n` spawn raising OSError must not fail the injection.
}

#[test]
#[ignore = "UNMAPPED: Python monkeypatches subprocess.run to return returncode 1 with empty stderr and asserts InjectSyntaxError without crashing. Runtime spawn -> Tier 3/4."]
fn test_interpreter_gate_reports_an_empty_stderr_without_crashing() {
    // Python drives the `bash -n` gate's empty-stderr branch.
}

// ---------------------------------------------------------------- $0 warning

#[test]
#[ignore = "UNMAPPED: the `$0` warning STRING (the `NAME=\"${NAME:-value}\"` advice, \"on a stored copy\") is rendered in the CLI/UI tier -> Tier 4. skit-language's observable half is analysis().uses_self_location, covered by port_test_shell_analyzer.rs (test_uses_self_location_*)."]
fn test_self_location_warns_when_a_temp_copy_is_written() {
    // Python asserts result.warnings carries the $0 advice string.
}

#[test]
fn test_self_location_does_not_warn_for_env_delivery() {
    // No copy is written, so `$0` is not affected at all: env delivery produces no source edit.
    // (The warnings-empty assertion is the Tier 3/4 consequence of this no-rewrite fact.)
    let src = "#!/usr/bin/env bash\nHERE=$(dirname \"$0\")\necho \"${MODE:-auto} $HERE\"\n";
    assert!(plan(src, &[("MODE", "manual")]).unwrap().edits().is_empty());
    assert_eq!(inject(src, &[("MODE", "manual")]).unwrap(), src);
}

#[test]
fn test_no_self_location_no_warning() {
    // The const IS rewritten (Python: `result.path is not None`); with no `$0`, no warning is due.
    // (The warnings-empty assertion is Tier 3/4.)
    let src = "#!/usr/bin/env bash\nWIDTH=800\n";
    let out = inject(src, &[("WIDTH", "1200")]).unwrap();
    assert!(out.contains("WIDTH=1200"));
}

// ---------------------------------------------------------------- normalization

#[test]
fn test_normalize_rewrites_only_that_assignments_bytes() {
    let src = "#!/usr/bin/env bash\nWIDTH=800\nHEIGHT=600\necho \"$WIDTH $HEIGHT\"\n";
    assert_eq!(
        normalize_shell_default(src, "WIDTH").unwrap(),
        "#!/usr/bin/env bash\nWIDTH=\"${WIDTH:-800}\"\nHEIGHT=600\necho \"$WIDTH $HEIGHT\"\n"
    );
}

#[test]
fn test_normalize_makes_the_param_an_envdefault() {
    let src = "#!/usr/bin/env bash\nWIDTH=800\n";
    let out = normalize_shell_default(src, "WIDTH").unwrap();
    let cands = by_name(&out);
    assert_eq!(
        cands["WIDTH"].declaration.binding,
        ParameterBinding::EnvDefault
    );
    assert_eq!(cands["WIDTH"].declaration.env_var(), "WIDTH");
    // the literal is still the script's standalone default
    assert_eq!(
        cands["WIDTH"].declaration.default,
        Some(ParameterValue::Integer(800))
    );
}

#[test]
#[ignore = "UNMAPPED: writes the normalized script and RUNS it standalone (default applies), then with GREETING inherited from env (env wins) -> Tier 3/4 (skit run / shell spawn). The normalized bytes are covered by test_normalize_rewrites_only_that_assignments_bytes + test_normalize_makes_the_param_an_envdefault."]
fn test_normalized_script_still_runs_standalone() {
    // Python spawns bash on the normalized copy twice and asserts the default vs inherited value.
}

#[test]
fn test_normalize_refuses_and_leaves_the_source_untouched() {
    // A refusal never half-rewrites. In skit-language, `normalize_shell_default` returns an Err for
    // every refusal, so the caller keeps the byte-identical source. (Python distinguishes the
    // refusal CODE — unsafe-literal / readonly / multiple-assignments / already-env / not-a-const —
    // but that string rendering is Tier 3/4; skit-language collapses all refusals to one error.)
    let cases = [
        ("A='literal $VAR'\n", "unsafe-literal"), // a $ would start expanding once re-homed
        ("A='say \"hi\"'\n", "unsafe-literal"),   // a quote would close the wrapper's quote
        ("A='back\\slash'\n", "unsafe-literal"),
        ("A='tick `x`'\n", "unsafe-literal"),
        ("A='brace }'\n", "unsafe-literal"), // would close the ${...} early
        ("readonly A=1\n", "readonly"),
        ("declare -r A=1\n", "readonly"),
        ("A=1\nA=2\n", "multiple-assignments"),
        ("A=\"${A:-1}\"\n", "already-env"), // it already IS the idiom
        ("B=1\n", "not-a-const"),
        ("A=$(date)\n", "not-a-const"),
        ("A=\"pre${OTHER}post\"\n", "not-a-const"), // no literal RHS at all
        ("A=\n", "not-a-const"),
        ("A+=1\n", "not-a-const"),
    ];
    for (src, code) in cases {
        assert!(
            normalize_shell_default(src, "A").is_err(),
            "{code}: expected refusal for {src:?}"
        );
    }
}

#[test]
fn test_normalize_ignores_array_and_valueless_assignments() {
    // Same agreement as the injector: a subscript target is not a const, so it cannot be normalized
    // (`--normalize ARR` reports it rather than rewriting an array element). Python normalizes the
    // batch ["WIDTH", "ARR"]; here each name is a single call (batch aggregation is Tier 3/4).
    let src = "#!/usr/bin/env bash\nARR[0]=1\nWIDTH=800\n";
    assert_eq!(
        normalize_shell_default(src, "WIDTH").unwrap(),
        "#!/usr/bin/env bash\nARR[0]=1\nWIDTH=\"${WIDTH:-800}\"\n"
    );
    assert!(normalize_shell_default(src, "ARR").is_err()); // not-a-const:ARR
}

#[test]
fn test_normalize_on_an_unparseable_script_changes_nothing() {
    // Python: refused ["syntax-error:A"], text unchanged. In skit-language a parse failure is an
    // InvalidSource error, so nothing is rewritten.
    let src = "#!/usr/bin/env zsh\nif [[ -n $X ]] {\n  print hi\n}\nA=1\n";
    assert!(matches!(
        normalize_shell_default(src, "A"),
        Err(LanguageError::InvalidSource { .. })
    ));
}

#[test]
fn test_normalize_mixed_batch_reports_each_name() {
    // Python normalizes ["WIDTH", "MAX", "NOPE"] in one batch: WIDTH normalized, MAX refused
    // (readonly), NOPE refused (not-a-const). Ported as three single-name calls (batch is Tier 3/4).
    let src = "#!/usr/bin/env bash\nWIDTH=800\nreadonly MAX=100\n";
    let out = normalize_shell_default(src, "WIDTH").unwrap();
    assert!(out.contains("WIDTH=\"${WIDTH:-800}\""));
    assert!(out.contains("readonly MAX=100")); // untouched
    assert!(normalize_shell_default(src, "MAX").is_err()); // readonly:MAX
    assert!(normalize_shell_default(src, "NOPE").is_err()); // not-a-const:NOPE
}

// ---------------------------------------------------------------- flows.execute integration

#[test]
#[ignore = "UNMAPPED: `skit params --manage` + `skit run --set` end to end (CliRunner + store.add_script), asserts the script's own stdout on the real fd -> Tier 4 (skit-cli). Byte injection covered by test_const_injection_runs_with_the_new_value."]
fn test_execute_runs_a_shell_entry_with_injected_values() {}

#[test]
#[ignore = "UNMAPPED: flows.execute + launcher spy proving env delivery passes no script_override (no injected copy) -> Tier 4 (skit-cli/flows). The no-rewrite fact is covered by test_env_delivery_writes_no_temp_file."]
fn test_execute_env_delivery_writes_no_temp_copy() {}

#[test]
#[ignore = "UNMAPPED: `skit run --set WIDTH=abc --no-input` returns the FAIL_BAD_VALUE exit code before launch -> Tier 4 (skit-cli). The typed refusal is covered by test_bad_int_value_raises_the_value_error_not_drift."]
fn test_run_refuses_a_bad_value_before_it_ever_launches() {}

#[test]
#[ignore = "UNMAPPED: flows.execute maps a drifted const to FAIL_DRIFT with the `--resync` hint -> Tier 4 (skit-cli/flows). The injector's drift refusal is covered by test_missing_const_target_is_drift."]
fn test_execute_maps_a_drifted_shell_definition_to_drift() {}

#[test]
#[ignore = "UNMAPPED: `skit run --set input-2=Lovelace` maps a positional gap to the FAIL_BAD_VALUE exit code -> Tier 4 (skit-cli). The gap refusal is covered by test_multi_variable_read_refuses_a_positional_gap."]
fn test_execute_reports_a_positional_gap_as_a_bad_value() {}

#[test]
#[ignore = "UNMAPPED: flows.execute surfaces the $0 warning through its `emit` callback -> Tier 4 (skit-cli/flows/UI). uses_self_location is the skit-language half (port_test_shell_analyzer.rs)."]
fn test_execute_surfaces_the_self_location_warning() {}

#[test]
#[ignore = "UNMAPPED: a syntax-gate failure must map to FAIL_DRIFT WITHOUT a `--resync` hint and never launch -> Tier 4 (skit-cli/flows); Python also monkeypatches inject.quote to force the corruption."]
fn test_execute_syntax_gate_failure_never_launches() {}

#[test]
#[ignore = "UNMAPPED: an entry whose kind grew an analyzer but no injector must degrade, not crash (store.add_script fish + flows.execute) -> Tier 4 (skit-cli/flows). skit-language returns UnsupportedKind for kinds without an injector, but the graceful-degradation path is in flows."]
fn test_execute_without_an_injector_does_not_crash() {}

// ---------------------------------------------------------------- CLI: --normalize

#[test]
#[ignore = "UNMAPPED: `skit run --dry-run` transparency line shows the ORIGINAL script path (no temp copy) -> Tier 4 (skit-cli)."]
fn test_cli_dry_run_shows_the_command() {}

#[test]
#[ignore = "UNMAPPED: `skit params --normalize` writes the envdefault back to the stored copy + updates the [tool.skit] block + `skit show --json` -> Tier 4 (skit-cli). The normalize bytes are covered by test_normalize_makes_the_param_an_envdefault."]
fn test_cli_normalize_turns_a_const_into_an_env_param() {}

#[test]
#[ignore = "UNMAPPED: `skit run` after `--normalize` delivers through the environment (env prefix + ORIGINAL path in the transparency line) -> Tier 4 (skit-cli)."]
fn test_cli_normalized_param_runs_through_the_environment() {}

#[test]
#[ignore = "UNMAPPED: `skit params --normalize MAX` reports the readonly refusal + leaves the file untouched -> Tier 4 (skit-cli). The refusal is covered by test_normalize_refuses_and_leaves_the_source_untouched."]
fn test_cli_normalize_reports_refusals() {}

#[test]
#[ignore = "UNMAPPED: `skit params --normalize` on a non-shell (python) kind exits 1 -> Tier 4 (skit-cli). skit-language's plan_shell_normalization returns UnsupportedKind for non-shell, but the CLI gate is above it."]
fn test_cli_normalize_refuses_a_non_shell_kind() {}

#[test]
#[ignore = "UNMAPPED: `skit params --normalize` refuses reference mode (no stored copy to edit) exits 1 -> Tier 4 (skit-cli/store)."]
fn test_cli_normalize_refuses_reference_mode() {}

#[test]
#[ignore = "UNMAPPED: `skit params --normalize` with the stored copy deleted exits 1 with `no stored copy` -> Tier 4 (skit-cli/store)."]
fn test_cli_normalize_without_a_stored_copy() {}

#[test]
#[ignore = "UNMAPPED: cli._render_normalize_warning renders every refusal code -> Tier 4 (skit-cli). The refusal codes themselves are the CLI's string rendering of what skit-language returns as one error."]
fn test_cli_normalize_warning_renderer_covers_every_code() {}

// ---------------------------------------------------------------- remaining pure-logic

#[test]
fn test_split_guard_refuses_only_what_the_shell_would_actually_mangle() {
    // The refusal set is measured against real shells, not Python's str.isspace():
    //   U+00A0 - whitespace to Python, but not a default-$IFS splitter: the shell keeps it whole.
    //   CR     - neither a splitter nor a line terminator: delivered byte-intact.
    // Both must be ACCEPTED in a non-last field. Only space/tab (which split the line across the
    // fields) and newline (which ends the line) are refused there.
    let src = "#!/usr/bin/env bash\nread -p \"a b: \" FIRST LAST\n";
    for accepted in ["a\u{00a0}b", "a\rb"] {
        assert!(
            plan(src, &[("input-1", accepted), ("input-2", "x")]).is_ok(),
            "{accepted:?} should be accepted"
        );
    }
    for splitter in [" ", "\t", "\n"] {
        let value = format!("a{splitter}b");
        assert!(
            matches!(
                plan(src, &[("input-1", &value), ("input-2", "x")]),
                Err(LanguageError::ShellInput(_))
            ),
            "{splitter:?} should be refused"
        );
    }
}

#[test]
#[ignore = "UNMAPPED: `skit params <name>` prints the self-locating-const advice string (`--normalize NAME` … `on the stored copy`, the `NAME=\"${NAME:-value}\"` idiom) -> Tier 4 (skit-cli). uses_self_location + the injectable consts are the skit-language half."]
fn test_params_warns_when_a_self_locating_script_has_injectable_consts() {}

#[test]
#[ignore = "UNMAPPED: `skit params <name>` omits the `locates itself` advice when the script never self-locates -> Tier 4 (skit-cli)."]
fn test_params_does_not_warn_when_the_script_never_self_locates() {}

#[test]
fn test_normalize_refuses_shell_metacharacters() {
    // `;` `|` `&` `(` `)` `<` `>` are inert in the single-quoted source (the analyzer offers the
    // const) but break tree-sitter inside "${NAME:-…}" — and because analyze() degrades the WHOLE
    // file on a parse error, letting one through would silently drop EVERY parameter on the entry
    // while reporting success. They must be refused, so the const keeps delivering by temp copy.
    for meta in [";", "|", "&", "(", ")", "<", ">"] {
        assert!(
            normalize_shell_default(&format!("MSG='a{meta}b'\n"), "MSG").is_err(),
            "meta {meta:?} must be refused (unsafe-literal)"
        );
    }
    assert_eq!(
        normalize_shell_default("MSG='plain'\n", "MSG").unwrap(),
        "MSG=\"${MSG:-plain}\"\n"
    );
}

#[test]
fn test_empty_value_in_a_non_last_read_variable_is_a_gap() {
    // A managed empty string in a non-last position collapses on the join (the next field shifts
    // into it), the same wrong binding as an unmanaged gap. The injector is self-correct: it treats
    // an empty non-last value as a gap.
    let src = "#!/usr/bin/env bash\nread -p \"p: \" A B\n";
    match plan(src, &[("input-1", ""), ("input-2", "x")]) {
        Err(LanguageError::ShellInput(ShellInputError::Gap { empty, filled })) => {
            assert_eq!((empty.as_str(), filled.as_str()), ("input-1", "input-2"));
        }
        other => panic!("expected Gap(input-1, input-2), got {other:?}"),
    }
}

#[test]
fn test_empty_value_in_the_last_read_variable_is_fine() {
    // A trailing empty is just a short line, which read handles (B reads empty).
    // PORTED AS BYTE ASSERTION: accepted, with the fed line `'x '`; the Python run confirms `[x][]`.
    let src = "#!/usr/bin/env bash\nread -p \"p: \" A B\nprintf \"[%s][%s]\" \"$A\" \"$B\"\n";
    let out = inject(src, &[("input-1", "x"), ("input-2", "")]).unwrap();
    assert!(out.contains("_skit_read 0 'x ' 0 'p: ' -p \"p: \" A B"));
}
