//! Mechanical port of the Python oracle module `tests/test_shell_analyzer.py`
//! (`origin/main@206f9ef`). Each `#[test]` keeps its Python `def test_*` name so it traces back to
//! its origin, and the Python "WHY" comment is preserved verbatim above it.
//!
//! Concept mapping used throughout:
//! - Python `shell.analyze(src)` -> `parsed(src).analysis()` (a `SemanticAnalysis`).
//! - Python `shell.analyze(src).candidates[i]` -> `parsed(src).analysis().candidates[i]`
//!   (a `SemanticCandidate`); `c.name/type/default/binding/prompt/order/secret/env_name` read from
//!   `c.declaration`, while Python `c.demoted`/`c.demotion` map to the candidate-level
//!   `c.demotion: Option<DegradationReason>` (present => demoted).
//! - Python binding strings -> `ParameterBinding::{Const,Input,EnvDefault}`.
//! - Python `c.env_name` -> `c.declaration.env_var()` (env_target or, when blank, the name).
//! - Python `result.syntax_error is True` -> `parse_document` returns `ParseOutcome::SyntaxError`.
//! - Python `result.uses_argv`/`uses_self_location` -> the same-named `SemanticAnalysis` fields.
//! - Python `shell.reconcile(text, specs)` -> the `reconcile` helper below (parse then
//!   `ParsedDocument::reconcile`, with the conservative all-missing report on a syntax error).
//!
//! Bucket 2 (white-box Python parser internals with no public-observable candidate equivalent) and
//! bucket 3 (`skit params` CLI / `flows` / registry-import integration that lives outside
//! `skit-language`) are kept as compiling `#[ignore]` stubs with their WHY comment.

use std::collections::{BTreeMap, BTreeSet};

use skit_domain::parameters::{
    ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, ParameterValue,
};
use skit_language::{
    ParseOutcome, ParsedDocument, ReconcileReport, SemanticAnalysis, SemanticCandidate,
    inject_values_for_interpreter, parse_document,
};

fn parsed(source: &str) -> ParsedDocument {
    match parse_document("shell", source) {
        ParseOutcome::Parsed(document) => document,
        other => panic!("expected valid shell, got {other:?}"),
    }
}

/// Python `shell.analyze(src)`.
fn analyze(source: &str) -> SemanticAnalysis {
    parsed(source).analysis()
}

/// Python `cands(src)` = `shell.analyze(src).candidates`.
fn cands(source: &str) -> Vec<SemanticCandidate> {
    analyze(source).candidates
}

/// Python `by_name(src)` = `{c.name: c for c in cands(src)}`.
fn by_name(source: &str) -> BTreeMap<String, SemanticCandidate> {
    cands(source)
        .into_iter()
        .map(|candidate| (candidate.declaration.name.clone(), candidate))
        .collect()
}

/// Candidate names in source order.
fn names(source: &str) -> Vec<String> {
    cands(source)
        .into_iter()
        .map(|candidate| candidate.declaration.name)
        .collect()
}

/// Python `_reads(src)` = `[c for c in cands(src) if c.binding == "input"]`.
fn reads(source: &str) -> Vec<SemanticCandidate> {
    cands(source)
        .into_iter()
        .filter(|candidate| candidate.declaration.binding == ParameterBinding::Input)
        .collect()
}

/// Python `_demoted(src)` = `{c.name for c in cands(src) if c.demoted}`.
fn demoted(source: &str) -> BTreeSet<String> {
    cands(source)
        .into_iter()
        .filter(|candidate| candidate.demotion.is_some())
        .map(|candidate| candidate.declaration.name)
        .collect()
}

fn string_set<const N: usize>(values: [&str; N]) -> BTreeSet<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

// ---------------------------------------------------------------- const

#[test]
fn test_const_word_number_raw_double_quoted() {
    let b = by_name("A=plain\nB=42\nC='raw text'\nD=\"double q\"\n");
    assert_eq!(
        (
            b["A"].declaration.parameter_type,
            &b["A"].declaration.default
        ),
        (
            ParameterType::Str,
            &Some(ParameterValue::String("plain".to_owned()))
        )
    );
    assert_eq!(
        (
            b["B"].declaration.parameter_type,
            &b["B"].declaration.default
        ),
        (ParameterType::Int, &Some(ParameterValue::Integer(42)))
    );
    assert_eq!(
        (
            b["C"].declaration.parameter_type,
            &b["C"].declaration.default
        ),
        (
            ParameterType::Str,
            &Some(ParameterValue::String("raw text".to_owned()))
        )
    );
    assert_eq!(
        (
            b["D"].declaration.parameter_type,
            &b["D"].declaration.default
        ),
        (
            ParameterType::Str,
            &Some(ParameterValue::String("double q".to_owned()))
        )
    );
}

#[test]
fn test_const_excludes_empty_array_concat_expansion_cmdsub() {
    let src =
        "EMPTY=\nQUOTED_EMPTY=''\nARR=(1 2 3)\nCONCAT=a$B\nSUBBED=$(date)\nEXPANDED=${OTHER}\n";
    assert!(cands(src).is_empty()); // none is a plain literal
}

#[test]
fn test_const_leading_underscore_skipped() {
    assert_eq!(names("_HIDDEN=1\nSHOWN=2\n"), ["SHOWN"]);
}

#[test]
fn test_const_last_write_wins_keeps_first_slot() {
    let b = by_name("X=1\nY=5\nX=2\n");
    assert_eq!(b["X"].declaration.default, Some(ParameterValue::Integer(2))); // last value wins
    let names = names("X=1\nY=5\nX=2\n");
    let index_of = |target: &str| names.iter().position(|name| name == target).unwrap();
    assert!(index_of("X") < index_of("Y")); // first slot kept
}

#[test]
fn test_const_plus_equals_is_not_a_literal_const() {
    // A bare += with no prior literal assignment yields no const candidate (it's an accumulator).
    assert!(names("N+=1\n").is_empty());
}

#[test]
fn test_declaration_export_declare_typeset_included_local_excluded() {
    let src = concat!(
        "export A=1\n",
        "declare -i B=2\n",
        "typeset C=three\n",
        "local D=4\n", // function scope — never a top-level const
    );
    assert_eq!(
        cands(src)
            .into_iter()
            .map(|candidate| candidate.declaration.name)
            .collect::<BTreeSet<_>>(),
        string_set(["A", "B", "C"])
    );
}

#[test]
fn test_readonly_and_declare_r_excluded() {
    let src = "readonly LOCKED=1\ndeclare -r FROZEN=2\ntypeset -rx ALSO=3\nOPEN=4\n";
    assert_eq!(names(src), ["OPEN"]);
}

// ---------------------------------------------------------------- envdefault

#[test]
fn test_envdefault_all_four_operators() {
    let b = by_name(": \"${A:-x}\"\n: \"${B:=y}\"\n: \"${C-z}\"\n: \"${D=w}\"\n");
    assert_eq!(
        b.keys().cloned().collect::<BTreeSet<_>>(),
        string_set(["A", "B", "C", "D"])
    );
    assert!(
        b.values()
            .all(|candidate| candidate.declaration.binding == ParameterBinding::EnvDefault)
    );
    assert_eq!(
        b["A"].declaration.default,
        Some(ParameterValue::String("x".to_owned()))
    );
}

#[test]
fn test_envdefault_non_default_operators_ignored() {
    // ${VAR:?err}, ${#VAR}, ${VAR#pat}, ${VAR/a/b} are not defaults.
    assert!(cands(": \"${VAR:?missing}\"\necho \"${#LIST}\"\n").is_empty());
}

#[test]
fn test_envdefault_type_inference_on_default() {
    let b = by_name(": \"${PORT:-8080}\"\n: \"${RATIO:-1.5}\"\n: \"${NAME:-guest}\"\n");
    assert_eq!(
        (
            b["PORT"].declaration.parameter_type,
            &b["PORT"].declaration.default
        ),
        (ParameterType::Int, &Some(ParameterValue::Integer(8080)))
    );
    assert_eq!(
        (
            b["RATIO"].declaration.parameter_type,
            &b["RATIO"].declaration.default
        ),
        (ParameterType::Float, &Some(ParameterValue::Float(1.5)))
    );
    assert_eq!(b["NAME"].declaration.parameter_type, ParameterType::Str);
}

#[test]
fn test_envdefault_empty_default() {
    let all = cands(": \"${OPT:-}\"\n");
    assert_eq!(all.len(), 1);
    assert_eq!(
        (
            all[0].declaration.parameter_type,
            &all[0].declaration.default
        ),
        (
            ParameterType::Str,
            &Some(ParameterValue::String(String::new()))
        )
    );
}

#[test]
fn test_envdefault_subscript_skipped() {
    assert!(cands("echo \"${ARR[0]:-x}\"\n").is_empty());
}

#[test]
fn test_envdefault_dedupes_by_name_first_default_wins() {
    let all = cands("echo \"${MODE:-first}\"\necho \"${MODE:-second}\"\n");
    assert_eq!(all.len(), 1);
    assert_eq!(
        all[0].declaration.default,
        Some(ParameterValue::String("first".to_owned()))
    );
}

#[test]
fn test_envdefault_carries_env_name() {
    // Python `c.env_name == "TOKEN_URL" == c.name`; the env var is `env_var()` (blank env_target
    // falls back to the declaration name).
    let all = cands(": \"${TOKEN_URL:-http://x}\"\n");
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].declaration.env_var(), "TOKEN_URL");
    assert_eq!(all[0].declaration.name, "TOKEN_URL");
}

#[test]
fn test_self_idiom_is_envdefault_not_suppressed() {
    let b = by_name("PORT=\"${PORT:-8080}\"\nNAME=${NAME:-guest}\n");
    assert_eq!(b["PORT"].declaration.binding, ParameterBinding::EnvDefault);
    assert_eq!(b["NAME"].declaration.binding, ParameterBinding::EnvDefault);
}

// ---------------------------------------------------------------- suppression (risk #1)

#[test]
fn test_suppression_bare_literal_assignment_wins() {
    let src = "PORT=8080\necho \"${PORT:-9090}\"\n";
    let b = by_name(src);
    assert_eq!(b["PORT"].declaration.binding, ParameterBinding::Const);
    assert!(
        !cands(src)
            .iter()
            .any(|candidate| candidate.declaration.name == "PORT"
                && candidate.declaration.binding == ParameterBinding::EnvDefault)
    );
}

#[test]
fn test_suppression_cmdsub_assignment_shadows_envdefault() {
    // A non-literal clobbering assignment isn't a const candidate, but still suppresses the env.
    assert!(cands("HOST=$(hostname)\necho \"${HOST:-local}\"\n").is_empty());
}

#[test]
fn test_suppression_only_targets_the_shadowed_name() {
    let b = by_name("PORT=8080\necho \"${PORT:-9090}\"\necho \"${MODE:-auto}\"\n");
    assert_eq!(b["MODE"].declaration.binding, ParameterBinding::EnvDefault);
}

// ---------------------------------------------------------------- read

#[test]
fn test_read_prompt_and_order_keys() {
    let rs = reads("read -p \"Name: \" NAME\nread -p \"Age: \" AGE\n");
    assert_eq!(
        rs.iter()
            .map(|candidate| (
                candidate.declaration.name.clone(),
                candidate.declaration.order,
                candidate.declaration.prompt.clone()
            ))
            .collect::<Vec<_>>(),
        [
            ("input-1".to_owned(), 0, "Name: ".to_owned()),
            ("input-2".to_owned(), 1, "Age: ".to_owned()),
        ]
    );
}

#[test]
fn test_read_secret_certainty_via_dash_s() {
    let rs = reads("read -s -p \"Enter value: \" V\n");
    assert_eq!(rs.len(), 1);
    assert!(rs[0].declaration.secret); // -s is certainty, not a name heuristic
}

#[test]
fn test_read_clustered_sp() {
    let rs = reads("read -sp \"PIN: \" PIN\n");
    assert_eq!(rs.len(), 1);
    assert_eq!(
        (rs[0].declaration.secret, rs[0].declaration.prompt.clone()),
        (true, "PIN: ".to_owned())
    );
}

#[test]
fn test_read_clustered_rp_not_secret() {
    let rs = reads("read -rp \"Confirm: \" C\n");
    assert_eq!(rs.len(), 1);
    assert_eq!(
        (rs[0].declaration.secret, rs[0].declaration.prompt.clone()),
        (false, "Confirm: ".to_owned())
    );
}

#[test]
fn test_read_multiple_varnames_share_prompt() {
    let rs = reads("read -p \"Two: \" FIRST LAST\n");
    assert_eq!(
        rs.iter()
            .map(|candidate| candidate.declaration.name.clone())
            .collect::<Vec<_>>(),
        ["input-1", "input-2"]
    );
    assert!(
        rs.iter()
            .all(|candidate| candidate.declaration.prompt == "Two: ")
    );
}

#[test]
fn test_read_dynamic_prompt_collapses_to_empty() {
    let rs = reads("read -p \"$MSG\" V\n");
    assert_eq!(rs.len(), 1);
    assert_eq!(rs[0].declaration.prompt, "");
}

#[test]
fn test_read_prompt_from_bare_word() {
    let rs = reads("read -p Enter: V\n");
    assert_eq!(rs.len(), 1);
    assert_eq!(rs[0].declaration.prompt, "Enter:");
}

#[test]
fn test_read_attached_prompt() {
    let rs = reads("read -pHello V\n");
    assert_eq!(rs.len(), 1);
    assert_eq!(rs[0].declaration.prompt, "Hello");
}

#[test]
fn test_read_value_flags_skip_their_argument() {
    // -t 5 and -u 0 consume their value; only V is a varname. (-n/-N/-d also consume theirs, but
    // they REFRAME the input, so such a read is excluded outright — see the reframing test below.)
    let rs = reads("read -t 5 -u 0 V\n");
    assert_eq!(rs.len(), 1);
    assert_eq!(rs[0].declaration.name, "input-1");
}

#[test]
fn test_read_attached_value_flag_not_consumed() {
    // -t5 attaches its value; W is still the varname.
    let rs = reads("read -t5 W\n");
    assert_eq!(rs.len(), 1);
    assert_eq!(rs[0].declaration.name, "input-1");
}

#[test]
fn test_reframing_reads_are_excluded_from_candidacy() {
    // -n/-N/-d make the read stop early or on another delimiter, so the single line skit feeds it is
    // not the value the script would end up with (`read -n 3 X` on "abcdefgh" yields "abc"). Such a
    // read cannot be delivered faithfully, so it is never offered — as `read -a` already isn't.
    for src in [
        "read -n 3 X\n",
        "read -N 5 X\n",
        "read -d : X\n",
        "read -n3 X\n",
    ] {
        assert!(reads(src).is_empty(), "{src}");
    }
}

#[test]
fn test_custom_ifs_reads_are_excluded_from_candidacy() {
    // skit joins a multi-var read's values with a SPACE and relies on default $IFS to split them
    // back. A custom IFS breaks that in both directions: `IFS=: read A B` would hand the whole line
    // to A, and `IFS= read -r LINE` does no splitting or edge-stripping at all (so a value skit
    // would refuse as unsafe actually arrives intact). Neither is offered.
    assert!(reads("IFS=: read A B\n").is_empty());
    assert!(reads("IFS= read -r LINE\n").is_empty());
    assert_eq!(reads("read -p \"p: \" A B\n").len(), 2); // an ordinary read still is
}

#[test]
fn test_read_end_of_options_marker() {
    // After --, a dash-led token is a varname, not a flag.
    let rs = reads("read -- -weird\n");
    assert_eq!(
        rs.iter()
            .map(|candidate| candidate.declaration.prompt.clone())
            .collect::<Vec<_>>(),
        [""]
    );
}

#[test]
fn test_read_single_dash_is_a_varname() {
    assert_eq!(reads("read -\n").len(), 1);
}

#[test]
fn test_read_non_word_argument_skipped() {
    // A string arg between flags/varnames isn't a plain word varname.
    let rs = reads("read \"$dyn\" REAL\n");
    assert_eq!(
        rs.iter()
            .map(|candidate| candidate.declaration.name.clone())
            .collect::<Vec<_>>(),
        ["input-1"]
    );
}

#[test]
fn test_read_dash_p_at_end_no_argument() {
    assert!(reads("read -p\n").is_empty()); // no varname, no prompt source; exercises the branch
}

#[test]
fn test_builtin_and_command_read_recognized() {
    assert_eq!(reads("builtin read X\n").len(), 1);
    assert_eq!(reads("command read Y\n").len(), 1);
}

#[test]
fn test_non_read_command_ignored() {
    assert!(reads("echo hello\nprintf '%s' x\n").is_empty());
}

#[test]
fn test_builtin_without_read_is_not_a_read() {
    assert!(reads("builtin pwd\ncommand ls\n").is_empty());
}

#[test]
fn test_bare_builtin_is_not_a_read() {
    assert!(reads("builtin\n").is_empty());
}

#[test]
fn test_read_secret_by_varname_and_prompt() {
    assert!(reads("read PASSWORD\n")[0].declaration.secret); // is_secret_name(varname)
    assert!(reads("read -p \"API key: \" K\n")[0].declaration.secret); // is_secret_name(prompt)
}

// ---------------------------------------------------------------- data-read exclusion (risk #5)

#[test]
fn test_data_read_pipe_right_operand_excluded() {
    assert!(reads("cat f | while read -r line; do echo $line; done\n").is_empty());
}

#[test]
fn test_data_read_pipe_three_stage_excluded() {
    assert!(reads("a | b | read Z\n").is_empty());
}

#[test]
fn test_read_first_pipe_operand_is_interactive() {
    assert_eq!(reads("read X | cat\n").len(), 1); // head of a pipe reads the terminal
}

#[test]
fn test_data_read_loop_fed_by_file_redirect_excluded() {
    assert!(reads("while read -r x; do echo $x; done < f\n").is_empty());
}

#[test]
fn test_data_read_own_stdin_redirect_excluded() {
    assert!(reads("read -r x < input.txt\n").is_empty());
}

#[test]
fn test_data_read_herestring_excluded() {
    assert!(reads("read -r x <<< \"$data\"\n").is_empty());
}

#[test]
fn test_data_read_heredoc_loop_excluded() {
    assert!(reads("while read -r x; do :; done <<EOF\na\nEOF\n").is_empty());
}

#[test]
fn test_read_with_output_redirect_is_still_interactive() {
    // `> out` is stdout, not stdin — the read still prompts.
    assert_eq!(reads("read -r x > out.log\n").len(), 1);
}

// ---------------------------------------------------------------- demotions

#[test]
fn test_demote_plus_equals() {
    assert_eq!(demoted("N=0\nN+=1\n"), string_set(["N"]));
}

#[test]
fn test_demote_arithmetic_self_reference() {
    assert_eq!(
        demoted("TOTAL=100\nTOTAL=$((TOTAL - 1))\n"),
        string_set(["TOTAL"])
    );
}

#[test]
fn test_demote_postfix_increment() {
    assert_eq!(demoted("N=0\n((N++))\n"), string_set(["N"]));
}

#[test]
fn test_demote_arithmetic_compound_assignment() {
    assert_eq!(demoted("N=0\n((N += 5))\n"), string_set(["N"]));
}

#[test]
fn test_demote_let_target() {
    assert_eq!(demoted("M=1\nlet M=M+1\n"), string_set(["M"]));
}

#[test]
fn test_demote_loop_body_reassignment() {
    assert_eq!(
        demoted("SUM=0\nfor i in 1 2; do SUM=$((SUM + i)); done\n"),
        string_set(["SUM"])
    );
}

#[test]
fn test_non_mutated_const_not_demoted() {
    let all = cands("STABLE=7\n");
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].demotion, None); // Python `(c.demoted, c.demotion) == (False, "")`
}

#[test]
fn test_arithmetic_read_only_does_not_demote() {
    // `(( n > 5 ))` reads n; it must not be mistaken for a mutation.
    assert_eq!(demoted("N=3\n(( N > 5 )) && echo big\n"), BTreeSet::new());
}

#[test]
fn test_subscript_assignment_is_not_a_const_or_mutation() {
    // arr[0]=5 has a subscript name (not a plain variable_name): never a const, never a suppressor.
    assert!(cands("arr[0]=5\n").is_empty());
}

#[test]
fn test_subscript_loop_reassignment_ignored() {
    assert!(cands("arr[0]=1\nfor i in 1 2; do arr[i]=$i; done\n").is_empty());
}

#[test]
fn test_arithmetic_subscript_mutation_has_no_named_target() {
    assert!(cands("(( arr[0] += 1 ))\n").is_empty());
}

#[test]
fn test_let_with_non_identifier_argument() {
    // `let COUNT=1 999` — COUNT is a target, the bare number 999 contributes nothing.
    let all = cands("COUNT=0\nlet COUNT=1 999\n");
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].declaration.name, "COUNT");
    assert!(all[0].demotion.is_some());
}

#[test]
fn test_postfix_on_subscript_marks_the_base_name() {
    // ((arr[0]++)) mutates an element, but demotes the base name arr (which is a scalar const here).
    assert_eq!(demoted("arr=1\n((arr[0]++))\n"), string_set(["arr"]));
}

// ---------------------------------------------------------------- hints

#[test]
fn test_uses_self_location_dollar_zero() {
    assert!(analyze("D=$(dirname \"$0\")\n").uses_self_location);
}

#[test]
fn test_uses_self_location_bash_source_and_subscript() {
    assert!(analyze("echo \"$BASH_SOURCE ${BASH_SOURCE[0]}\"\n").uses_self_location);
}

#[test]
fn test_no_self_location() {
    assert!(!analyze("X=1\n").uses_self_location);
}

#[test]
fn test_uses_argv_positional() {
    assert!(analyze("echo \"$1 $2\"\n").uses_argv);
}

#[test]
fn test_uses_argv_special_at_hash_star() {
    assert!(analyze("echo \"$@ $# $*\"\n").uses_argv);
}

#[test]
fn test_uses_argv_getopts_and_shift() {
    assert!(analyze("getopts \"ab\" o\n").uses_argv);
    assert!(analyze("shift\n").uses_argv);
}

#[test]
fn test_dollar_zero_is_not_argv() {
    assert!(!analyze("echo \"$0\"\n").uses_argv);
}

#[test]
fn test_other_special_variables_are_not_argv() {
    // $? $$ $! are special variables, but not positional-argument markers.
    assert!(!analyze("echo $? $$ $!\n").uses_argv);
}

#[test]
fn test_no_argv() {
    assert!(!analyze("X=1\n").uses_argv);
}

// ---------------------------------------------------------------- type inference edges

#[test]
fn test_type_leading_zeros_read_as_int() {
    let all = cands("Z=007\n");
    assert_eq!(all.len(), 1);
    assert_eq!(
        (
            all[0].declaration.parameter_type,
            &all[0].declaration.default
        ),
        (ParameterType::Int, &Some(ParameterValue::Integer(7))) // leading zeros not preserved (documented)
    );
}

#[test]
fn test_type_negative_int() {
    let all = cands("N=-3\n");
    assert_eq!(all.len(), 1);
    assert_eq!(
        (
            all[0].declaration.parameter_type,
            &all[0].declaration.default
        ),
        (ParameterType::Int, &Some(ParameterValue::Integer(-3)))
    );
}

#[test]
fn test_type_negative_float() {
    let all = cands("F=-2.5\n");
    assert_eq!(all.len(), 1);
    assert_eq!(
        (
            all[0].declaration.parameter_type,
            &all[0].declaration.default
        ),
        (ParameterType::Float, &Some(ParameterValue::Float(-2.5)))
    );
}

#[test]
fn test_type_dotted_version_is_str() {
    let all = cands("V=1.5.2\n");
    assert_eq!(all.len(), 1);
    assert_eq!(
        (
            all[0].declaration.parameter_type,
            &all[0].declaration.default
        ),
        (
            ParameterType::Str,
            &Some(ParameterValue::String("1.5.2".to_owned()))
        )
    );
}

#[test]
fn test_type_never_bool() {
    let b = by_name("FLAG=true\nOTHER=false\n");
    assert_eq!(
        (
            b["FLAG"].declaration.parameter_type,
            &b["FLAG"].declaration.default
        ),
        (
            ParameterType::Str,
            &Some(ParameterValue::String("true".to_owned()))
        )
    );
    assert_eq!(b["OTHER"].declaration.parameter_type, ParameterType::Str);
}

// ---------------------------------------------------------------- degradation

#[test]
fn test_has_error_returns_empty_syntax_error() {
    // Python exposes `result.syntax_error is True` and an empty candidate list. The Rust surface
    // reports invalid syntax as a distinct `ParseOutcome::SyntaxError` variant (no parsed document,
    // hence no candidates), which is the faithful mapping of both assertions.
    assert!(matches!(
        parse_document("shell", "if [[ -n $x ]] { echo hi }\nCONFIG=1\n"),
        ParseOutcome::SyntaxError(_)
    ));
}

#[test]
fn test_empty_script() {
    // Python: `result.candidates == []` and `result.syntax_error is False`.
    assert!(cands("").is_empty());
    assert!(matches!(
        parse_document("shell", ""),
        ParseOutcome::Parsed(_)
    ));
}

// ---------------------------------------------------------------- reconcile parity + envdefault matrix

/// Python `shell.reconcile(text, specs)`: parse then reconcile, or the conservative all-missing
/// report when the source has a syntax error.
fn reconcile(source: &str, stored: &[ParamDecl]) -> ReconcileReport {
    match parse_document("shell", source) {
        ParseOutcome::Parsed(document) => document.reconcile(stored),
        _ => ReconcileReport::from_syntax_error(stored),
    }
}

/// Python `_spec(name)` = `ParamDecl(name=name, binding="envdefault", delivery="env", type="str")`.
fn env_spec(name: &str) -> ParamDecl {
    let mut declaration = ParamDecl::new(name);
    declaration.binding = ParameterBinding::EnvDefault;
    declaration.delivery = ParameterDelivery::Env;
    declaration.parameter_type = ParameterType::Str;
    declaration
}

fn ok_names(report: &ReconcileReport) -> Vec<String> {
    report
        .ok
        .iter()
        .map(|pair| pair.stored.name.clone())
        .collect()
}

fn missing_names(report: &ReconcileReport) -> Vec<String> {
    report
        .missing
        .iter()
        .map(|declaration| declaration.name.clone())
        .collect()
}

#[test]
fn test_reconcile_const_and_input_parity() {
    // Shell reconcile handles const (by name) and input (by prompt/order) just like Python.
    let text = "CITY=Taipei\nread -p \"Name: \" NAME\n";
    let mut city = ParamDecl::new("CITY");
    city.binding = ParameterBinding::Const;
    city.parameter_type = ParameterType::Str;
    let mut input = ParamDecl::new("input-1");
    input.binding = ParameterBinding::Input;
    input.parameter_type = ParameterType::Str;
    input.order = 0;
    input.prompt = "Name: ".to_owned();
    let report = reconcile(text, &[city, input]);
    assert!(!report.has_drift());
    assert_eq!(
        ok_names(&report).into_iter().collect::<BTreeSet<_>>(),
        string_set(["CITY", "input-1"])
    );
}

#[test]
fn test_reconcile_envdefault_ok() {
    let report = reconcile("echo \"${PORT:-8080}\"\n", &[env_spec("PORT")]);
    assert!(!report.has_drift());
    assert_eq!(ok_names(&report), ["PORT"]);
}

#[test]
fn test_reconcile_envdefault_default_change_is_still_ok() {
    // The default text changed (8080 -> 9090); env delivery still works, so no drift.
    let report = reconcile("echo \"${PORT:-9090}\"\n", &[env_spec("PORT")]);
    assert!(!report.has_drift());
    assert_eq!(ok_names(&report), ["PORT"]);
}

#[test]
fn test_reconcile_envdefault_gone_is_missing() {
    let report = reconcile("echo hello\n", &[env_spec("PORT")]);
    assert!(report.has_drift());
    assert_eq!(missing_names(&report), ["PORT"]);
}

#[test]
fn test_reconcile_envdefault_bare_assignment_shadow_is_missing() {
    // A plain assignment now clobbers PORT — the env value would be silently ignored.
    let report = reconcile("PORT=8080\necho \"${PORT:-9090}\"\n", &[env_spec("PORT")]);
    assert!(report.has_drift());
    assert_eq!(missing_names(&report), ["PORT"]);
}

#[test]
#[ignore = "UNMAPPED: analysis.drift_lines has no skit-language equivalent (loud human-string drift rendering lives in skit-cli/skit-ui) -> Tier 4"]
fn test_envdefault_loud_drift_line() {
    // Python asserts drift_lines(report, "deploy") contains the loud env line
    // ("no longer read from the environment"/"PORT") and NOT the generic "injection target no
    // longer exists" one. drift_lines is CLI/UI string rendering, not part of ReconcileReport.
    let _ = reconcile;
}

#[test]
fn test_envdefault_unmanaged_is_new_not_drift() {
    let report = reconcile("echo \"${LOG_LEVEL:-info}\"\n", &[]);
    assert!(!report.has_drift());
    assert_eq!(
        report
            .new
            .iter()
            .map(|candidate| candidate.declaration.name.clone())
            .collect::<Vec<_>>(),
        ["LOG_LEVEL"]
    );
}

// ---------------------------------------------------------------- registry import guard

#[test]
#[ignore = "UNMAPPED: Python registry dynamic-import degradation (monkeypatch sys.modules so `from .shell import analyzer` raises) has no Rust equivalent; analyzers are statically linked into ParsedDocument::analysis"]
fn test_import_guard_degrades_analyzer_to_none() {
    // Python asserts registry.spec_for("shell").analyzer is None while params_io stays present after
    // the analyzer import is broken. Rust has no per-language optional analyzer import to degrade.
    let _ = reconcile;
}

#[test]
#[ignore = "UNMAPPED: skit.flows.plan_for_entry + store.add_script + registry import degradation -> Tier 4 skit-cli/flows (not reachable from skit-language)"]
fn test_plan_degrades_to_none_when_analyzer_missing() {
    // Python: with the analyzer import broken, flows.plan_for_entry(entry).source == "none" so the
    // entry stays launchable with no inject plan. flows/store live above skit-language.
    let _ = reconcile;
}

// ---------------------------------------------------------------- `skit params` shell integration

#[test]
#[ignore = "UNMAPPED: `skit params <name> --manage` CLI integration (CliRunner + store.add_script) -> Tier 4 skit-cli"]
fn test_params_manage_writes_block_into_shell_copy() {
    // Python drives cli.app to write a `[tool.skit]` block after the shebang of a shell copy.
    let _ = reconcile;
}

#[test]
#[ignore = "UNMAPPED: `skit params <name>` CLI show output (CliRunner + store.add_script) -> Tier 4 skit-cli"]
fn test_params_show_lists_shell_const_and_unmanaged() {
    // Python asserts the CLI show view lists both the const (CITY) and the envdefault (MODE).
    let _ = reconcile;
}

#[test]
#[ignore = "UNMAPPED: `skit params <name>` getopts-gating of --manage in the CLI show view (CliRunner) -> Tier 4 skit-cli"]
fn test_params_show_getopts_shell_stops_advertising_manage() {
    // A getopts shell drives its OWN CLI (uses_cli_framework): the read view must NOT advertise
    // --manage. The uses_cli_framework signal itself IS in skit-language (frameworks non-empty for
    // getopts), but the CLI copy that renders --manage is Tier 4.
    let _ = reconcile;
}

#[test]
#[ignore = "UNMAPPED: `skit params <name> --resync` CLI drift application (CliRunner + store) -> Tier 4 skit-cli"]
fn test_params_resync_reports_drift_after_edit() {
    // Python renames the const in the copy, then `--resync` reports and applies the drop.
    let _ = reconcile;
}

#[test]
fn test_analyzer_and_injector_share_one_read_enumeration() {
    struct Case {
        source: &'static str,
        expected_calls: &'static [&'static str],
        unchanged_reads: &'static [&'static str],
    }

    let cases = [
        Case {
            source: "read -n 3 CODE\nread NAME\n",
            expected_calls: &["_skit_read 0 'site-0' 0 '' NAME"],
            unchanged_reads: &["read -n 3 CODE"],
        },
        Case {
            source: "IFS=: read A B\nread NAME\n",
            expected_calls: &["_skit_read 0 'site-0' 0 '' NAME"],
            unchanged_reads: &["IFS=: read A B"],
        },
        Case {
            source: "read P\nread Q\nread R\n",
            expected_calls: &[
                "_skit_read 0 'site-0' 0 '' P",
                "_skit_read 1 'site-1' 0 '' Q",
                "_skit_read 2 'site-2' 0 '' R",
            ],
            unchanged_reads: &[],
        },
        Case {
            source: "cmd | while read x; do :; done\nread TOP\n",
            expected_calls: &["_skit_read 0 'site-0' 0 '' TOP"],
            unchanged_reads: &["cmd | while read x; do :; done"],
        },
    ];

    for case in cases {
        let declarations = reads(case.source)
            .into_iter()
            .map(|candidate| candidate.declaration)
            .collect::<Vec<_>>();
        assert_eq!(
            declarations
                .iter()
                .map(|declaration| (declaration.name.clone(), declaration.order))
                .collect::<Vec<_>>(),
            (0..case.expected_calls.len())
                .map(|order| (format!("input-{}", order + 1), order as i64))
                .collect::<Vec<_>>(),
            "analyzer enumeration for {:?}",
            case.source
        );

        let values = declarations
            .iter()
            .map(|declaration| {
                (
                    declaration.name.clone(),
                    format!("site-{}", declaration.order),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let rewritten = inject_values_for_interpreter(
            "shell",
            case.source,
            &declarations,
            &values,
            Some("bash"),
        )
        .unwrap();
        let calls = rewritten
            .lines()
            .filter(|line| line.starts_with("_skit_read "))
            .collect::<Vec<_>>();
        assert_eq!(
            calls, case.expected_calls,
            "injector sites for {:?}",
            case.source
        );
        for unchanged in case.unchanged_reads {
            assert!(
                rewritten.lines().any(|line| line == *unchanged),
                "excluded read changed in {:?}: {rewritten}",
                case.source
            );
        }
        assert!(
            matches!(parse_document("shell", &rewritten), ParseOutcome::Parsed(_)),
            "rewritten source must parse: {rewritten}"
        );
    }
}

#[test]
fn test_read_flags_do_not_read_letters_from_an_attached_value() {
    let cases = [
        (
            "read -pSure? X\n",
            "Sure?",
            "_skit_read 0 'a\\\\b' 0 'Sure?' -pSure? X",
        ),
        (
            "read -pEnter X\n",
            "Enter",
            "_skit_read 0 'a\\\\b' 0 'Enter' -pEnter X",
        ),
        (
            "read -idefault X\n",
            "",
            "_skit_read 0 'a\\\\b' 0 '' -idefault X",
        ),
    ];

    for (source, expected_prompt, expected_call) in cases {
        let candidates = reads(source);
        let [candidate] = candidates.as_slice() else {
            panic!("attached value must keep one input candidate: {source:?}");
        };
        assert_eq!(candidate.declaration.name, "input-1", "{source:?}");
        assert_eq!(candidate.declaration.order, 0, "{source:?}");
        assert_eq!(candidate.declaration.prompt, expected_prompt, "{source:?}");

        let declarations = [candidate.declaration.clone()];
        let rewritten = inject_values_for_interpreter(
            "shell",
            source,
            &declarations,
            &BTreeMap::from([("input-1".to_owned(), "a\\b".to_owned())]),
            Some("bash"),
        )
        .unwrap();
        let calls = rewritten
            .lines()
            .filter(|line| line.starts_with("_skit_read "))
            .collect::<Vec<_>>();
        assert_eq!(calls, [expected_call], "{source:?}");
    }

    let raw_candidates = reads("read -r X\n");
    let [raw_candidate] = raw_candidates.as_slice() else {
        panic!("a real raw flag must keep one input candidate");
    };
    let rewritten = inject_values_for_interpreter(
        "shell",
        "read -r X\n",
        std::slice::from_ref(&raw_candidate.declaration),
        &BTreeMap::from([("input-1".to_owned(), "a\\b".to_owned())]),
        Some("bash"),
    )
    .unwrap();
    let calls = rewritten
        .lines()
        .filter(|line| line.starts_with("_skit_read "))
        .collect::<Vec<_>>();
    assert_eq!(calls, ["_skit_read 0 'a\\b' 0 '' -r X"]);
}

#[test]
fn test_read_cluster_keeps_scanning_past_an_unknown_flag_letter() {
    // `-er`: 'e' (readline edit, no value) is unknown to the value-flag set, so the scan continues
    // to 'r', which registers raw. The value still delivers as a normal read varname.
    //
    // Ported via public output: the Python test's own public assertion
    // (`shell.analyze("read -er X\n").candidates[0].name == "input-1"`) is the observable contract.
    // The white-box `.raw is True` / `.varnames == ["X"]` checks map to this single input candidate
    // (`.raw` itself is not carried on the public candidate).
    let all = cands("read -er X\n");
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].declaration.name, "input-1");
    assert_eq!(all[0].declaration.binding, ParameterBinding::Input);
}
