//! Mechanical port of the Python oracle module `tests/test_template_context_quoting.py`
//! (`origin/main@206f9ef`): "Context-aware POSIX quoting for command templates (Fix A in
//! langs/launch.py)." Each `#[test]` keeps its Python `def test_*` name and its "WHY" comment
//! so it traces back to its origin.
//!
//! CRATE HINT OVERRIDE: the hint said `skit-language`, but the real API lives in `skit-runtime`
//! (`crates/skit-runtime/src/launch.rs`). The whole file is POSIX-only (`#![cfg(not(windows))]`),
//! mirroring the oracle's `_POSIX_ONLY` guard.
//!
//! Concept mapping used throughout:
//! - Python `launch.TemplateLaunch._substitute_posix(template, vals)` (the POSIX one-pass token
//!   walk) -> `render_command_template(template, &values)` (public). It is the shared core of the
//!   Python `_render`, so `launcher.build_command` and `launcher.describe_command` (both call
//!   `_render`) map to the SAME function; the store `add_command` + `Entry` machinery is only a
//!   vehicle to reach it and is elided.
//! - Python `_render`'s missing-value gate lives one layer OUT in Rust (`command_plan` in
//!   launch.rs), not in `render_command_template`. So the oracle's `entry.meta.params = None`
//!   detach trick needs no equivalent: an unfilled slot travels through this seam unchanged.
//! - Python `LaunchError` (the backtick-nested-double-quote refusal) ->
//!   `LaunchError::UnsafeTemplatePlaceholder`; its `to_string()` is the exact refusal message.
//! - Python extra-args append (`build_command(entry, extra, ...)` -> `_render` -> `shlex.join`)
//!   lives in `command_plan`/`append_shell_args`, reachable through `build_launch_plan`.
//! - Python `shlex.quote` (unquoted branch) -> `quote_posix_arg` (private); the two agree on
//!   every value these tests assert (`a b` -> `'a b'`, `$(id)` -> `'$(id)'`).
//!
//! Buckets:
//! - REAL (28): the 12 `test_value_*` context tests (each wraps the placeholder in the minimal
//!   template that puts it in the oracle's quote context; the expected escaped string is computed
//!   from the Python assertion), plus the 16 substitution / build / describe / refusal tests
//!   (string equality and, where the oracle runs `sh`, real `/bin/sh -c` execution).
//! - cross-crate `#[ignore]` (14): every `test_state_*` white-box unit of `_posix_quote_state`.
//!   The Rust twin is the PRIVATE `PosixQuoteState` (`Vec<char>` frames + `escape_pending: bool`);
//!   the oracle asserts Python's state-STRING encoding, which has no equal Rust value, and the
//!   pending/resume branch is unobservable through the public API. Their behavior is covered
//!   observably by the `test_value_*` and substitution tests. Python assertions kept as comments.
//! - cross-crate `#[ignore]` (2): the two `test_render_win32_*` tests exercise the Windows render
//!   branch, which Rust selects at COMPILE time (`#[cfg(windows)]`) and does not build here; the
//!   oracle reaches it by monkeypatching `sys.platform`, which a `cfg` cannot emulate.

// Every active test in this module asserts the POSIX arm of `render_command_template`, a
// compile-time host branch: on Windows the same wrapper renders through
// render_windows_command_template, so the POSIX expectations test nothing real there. The
// Windows arm's two owners remain cross-crate ignored stubs (the platform gap is recorded in
// the module doc); running this file on Windows would only fail honest POSIX assertions.
#![cfg(unix)]
#![cfg(not(windows))]

use std::collections::BTreeMap;
use std::process::Command;

use skit_application::delivery::Assembly;
use skit_domain::{Entry, EntryKind, EntryMeta, EntrySettings, Slug};
use skit_runtime::{
    LaunchError, LaunchPaths, ProgramProbe, build_launch_plan, render_command_template,
};

/// Build a value map from `&[(name, value)]` pairs.
fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

/// Render a template with the given values; panic on a refusal (callers that expect one call
/// `render_command_template` directly).
fn render(template: &str, pairs: &[(&str, &str)]) -> String {
    render_command_template(template, &map(pairs)).expect("template renders")
}

/// The oracle's `_run_sh`: run one shell string through absolute `/bin/sh -c` and return
/// (success, stdout). Absolute `/bin/sh` mirrors the oracle exactly.
fn run_sh(command: &str) -> (bool, String) {
    let output = Command::new("/bin/sh")
        .arg("-c")
        .arg(command)
        .output()
        .expect("spawn /bin/sh");
    (
        output.status.success(),
        String::from_utf8(output.stdout).expect("utf-8 stdout"),
    )
}

// ==========================================================================
// _posix_quote_state — the quote-context state machine (exact transitions).
// White-box unit of the private PosixQuoteState; see the file header bucket note.
// ==========================================================================

#[test]
#[ignore = "cross-crate: white-box unit of skit-runtime's private PosixQuoteState (crates/skit-runtime/src/launch.rs); its Vec<char>+bool state has no equal to Python's state-STRING return value. Covered observably by the value_* tests."]
fn test_state_open_and_close_single_quote() {
    // launch._posix_quote_state("'", "") == "'"  (a bare ' opens single-quote context)
    // launch._posix_quote_state("'", "'") == ""  (the matching ' closes it)
}

#[test]
#[ignore = "cross-crate: white-box unit of skit-runtime's private PosixQuoteState (crates/skit-runtime/src/launch.rs); no equal Rust value to Python's state-STRING. Covered observably by the value_* tests."]
fn test_state_open_and_close_double_quote() {
    // launch._posix_quote_state('"', "") == '"'  (a bare " opens double-quote context)
    // launch._posix_quote_state('"', '"') == ""  (the matching " closes it)
}

#[test]
#[ignore = "cross-crate: white-box unit of skit-runtime's private PosixQuoteState (crates/skit-runtime/src/launch.rs); no equal Rust value to Python's state-STRING. Covered observably by the value_* tests."]
fn test_state_backslash_skips_next_char_in_unquoted_so_quote_stays_shut() {
    // Outside single quotes a backslash consumes the following char, so `\"` / `\'` do NOT open a
    // quote context (they are an escaped literal quote).
    // launch._posix_quote_state('\\"', "") == ""
    // launch._posix_quote_state("\\'", "") == ""
    // And having consumed the char, a subsequent real quote still opens as normal.
    // launch._posix_quote_state("\\a'", "") == "'"
}

#[test]
#[ignore = "cross-crate: white-box unit of skit-runtime's private PosixQuoteState (crates/skit-runtime/src/launch.rs); no equal Rust value to Python's state-STRING. Covered observably by the value_* tests."]
fn test_state_backslash_skips_closing_quote_inside_double() {
    // Inside "..." an escaped `\"` does not close the context...
    // launch._posix_quote_state('\\"', '"') == '"'
    // ...but the next *bare* " does.
    // launch._posix_quote_state('\\""', '"') == ""
}

#[test]
#[ignore = "cross-crate: white-box unit of skit-runtime's private PosixQuoteState (crates/skit-runtime/src/launch.rs); no equal Rust value to Python's state-STRING. Covered observably by the value_* tests."]
fn test_state_backslash_is_literal_inside_single_quotes() {
    // POSIX single quotes have no escapes: a backslash is an ordinary literal that does NOT consume
    // the following char, so the very next ' still closes the context.
    // launch._posix_quote_state("\\'", "'") == ""
    // A lone backslash inside single quotes therefore leaves the context open.
    // launch._posix_quote_state("\\", "'") == "'"
}

#[test]
#[ignore = "cross-crate: white-box unit of skit-runtime's private PosixQuoteState (crates/skit-runtime/src/launch.rs); no equal Rust value to Python's state-STRING. Covered observably by the value_* tests."]
fn test_state_the_other_quote_kind_is_literal() {
    // A " inside '...' is a literal (no state change); a ' inside "..." likewise.
    // launch._posix_quote_state('"', "'") == "'"
    // launch._posix_quote_state("'", '"') == '"'
}

#[test]
#[ignore = "cross-crate: white-box unit of skit-runtime's private PosixQuoteState (crates/skit-runtime/src/launch.rs); no equal Rust value to Python's state-STRING. Covered observably by the value_* tests."]
fn test_state_carries_across_successive_chunks() {
    // Quote-neutral text leaves whatever context we were handed untouched (the between-token carry
    // that _substitute_posix relies on)...
    // launch._posix_quote_state("plain text", "'") == "'"
    // launch._posix_quote_state("plain text", '"') == '"'
    // launch._posix_quote_state("plain text", "") == ""
    // ...and a later chunk can close a context an earlier chunk opened.
    // opened = launch._posix_quote_state("open '", "") == "'"
    // launch._posix_quote_state("close '", opened) == ""
}

#[test]
#[ignore = "cross-crate: white-box unit of skit-runtime's private PosixQuoteState (crates/skit-runtime/src/launch.rs); pending-escape (\"\\\") suffix is a Python state-STRING encoding with no Rust equal, and the resume branch is unobservable through render_command_template."]
fn test_state_dangling_backslash_pends_across_the_boundary() {
    // A chunk ENDING on an unconsumed backslash reports it: that escape applies to the first char
    // the caller emits next, so swallowing it here would hand the escape to the substituted value.
    // launch._posix_quote_state("foo\\", "") == "\\"
    // launch._posix_quote_state("\\", '"') == '"\\'
    // An EVEN run of backslashes self-consumes: nothing pends.
    // launch._posix_quote_state("foo\\\\", "") == ""
    // launch._posix_quote_state("\\\\", '"') == '"'
    // Inside single quotes a backslash is literal — never pending.
    // launch._posix_quote_state("foo\\", "'") == "'"
}

#[test]
#[ignore = "cross-crate: white-box unit of skit-runtime's private PosixQuoteState (crates/skit-runtime/src/launch.rs); the resume-from-pending branch is unobservable through the public API (render_posix_command_template clears escape_pending after every advance)."]
fn test_state_resumes_a_pending_backslash_by_consuming_the_first_char() {
    // Resuming with pending state: the first character is the escaped one — even a quote character,
    // which therefore must NOT open a context.
    // launch._posix_quote_state("x", '"\\') == '"'
    // launch._posix_quote_state("'abc", "\\") == ""
    // launch._posix_quote_state('"abc', "\\") == ""
    // An empty resumed chunk keeps the escape pending.
    // launch._posix_quote_state("", '"\\') == '"\\'
    // launch._posix_quote_state("", "\\") == "\\"
    // A two-char resume where the second char is a real quote must still act on it (i=1).
    // launch._posix_quote_state('x"', '"\\') == ""
    // launch._posix_quote_state('x"', "\\") == '"'
}

// ==========================================================================
// _posix_quote_value — position-aware escaping (exact output + escape ORDER).
// Promoted to REAL: each wraps the placeholder in the minimal template that puts it in the
// oracle's quote context; the expected escaped substring is the oracle's asserted value.
// ==========================================================================

#[test]
fn test_value_single_context_escapes_embedded_apostrophe() {
    // Inside '...', an embedded ' becomes '\'' (close, escaped-quote, reopen).
    // oracle: _posix_quote_value("a'b", "'") == "a'\\''b"
    assert_eq!(render("'{v}'", &[("v", "a'b")]), r"'a'\''b'");
}

#[test]
fn test_value_single_context_plain_value_is_verbatim() {
    // oracle: _posix_quote_value("plain", "'") == "plain"
    assert_eq!(render("'{v}'", &[("v", "plain")]), "'plain'");
}

#[test]
fn test_value_double_context_escapes_backslash() {
    // oracle: _posix_quote_value("\\", '"') == "\\\\"  (one backslash -> two)
    assert_eq!(render(r#""{v}""#, &[("v", "\\")]), r#""\\""#);
}

#[test]
fn test_value_double_context_escapes_double_quote() {
    // oracle: _posix_quote_value('"', '"') == '\\"'
    assert_eq!(render(r#""{v}""#, &[("v", "\"")]), r#""\"""#);
}

#[test]
fn test_value_double_context_escapes_dollar() {
    // oracle: _posix_quote_value("$", '"') == "\\$"
    assert_eq!(render(r#""{v}""#, &[("v", "$")]), r#""\$""#);
}

#[test]
fn test_value_double_context_escapes_backtick() {
    // oracle: _posix_quote_value("`", '"') == "\\`"
    assert_eq!(render(r#""{v}""#, &[("v", "`")]), r#""\`""#);
}

#[test]
fn test_value_double_context_neutralizes_command_substitution() {
    // The reproduced bug: $(...) must arrive dead, not live, inside double quotes.
    // oracle: _posix_quote_value("$(printf unexpected)", '"') == "\\$(printf unexpected)"
    assert_eq!(
        render(r#""{v}""#, &[("v", "$(printf unexpected)")]),
        r#""\$(printf unexpected)""#
    );
}

#[test]
fn test_value_double_context_backslash_doubling_precedes_dollar_escape() {
    // Value is literally backslash-dollar (`\$`). Backslash-doubling MUST run first: else the
    // backslash the $->\$ step injects would itself get doubled. Correct result is `\\\$`.
    // oracle: _posix_quote_value("\\$", '"') == "\\\\\\$"
    assert_eq!(render(r#""{v}""#, &[("v", "\\$")]), r#""\\\$""#);
}

#[test]
fn test_value_double_context_backslash_before_double_quote_order() {
    // A value carrying both \ and ": backslash-doubling must precede the "-escape.
    // \" -> \\ + \" (doubled backslash, then escaped quote), never \\\\ + ".
    // oracle: _posix_quote_value('\\"', '"') == "\\\\" + '\\"'
    assert_eq!(render(r#""{v}""#, &[("v", "\\\"")]), r#""\\\"""#);
}

#[test]
fn test_value_double_context_backtick_after_dollar() {
    // A value carrying both $ and ` — both are neutralized, order-independently here.
    // oracle: _posix_quote_value("$x`y`", '"') == "\\$x\\`y\\`"
    assert_eq!(render(r#""{v}""#, &[("v", "$x`y`")]), r#""\$x\`y\`""#);
}

#[test]
fn test_value_unquoted_context_defers_to_shlex_quote() {
    // In unquoted position the value defers to shlex.quote; quote_posix_arg agrees on both.
    // oracle: _posix_quote_value("a b", "") == "'a b'"; _posix_quote_value("$(id)", "") == "'$(id)'"
    assert_eq!(render("{v}", &[("v", "a b")]), "'a b'");
    assert_eq!(render("{v}", &[("v", "$(id)")]), "'$(id)'");
}

#[test]
fn test_value_inside_a_substitution_takes_the_unquoted_branch() {
    // However many double quotes wrap the substitution, the value sits in the nested command's own
    // unquoted context — the shlex.quote branch. Context `"(` and `"`` both take it...
    // oracle: _posix_quote_value("a b", '"(') == "'a b'"; _posix_quote_value("a b", '"`') == "'a b'"
    assert_eq!(render(r#""$(a {v}"#, &[("v", "a b")]), r#""$(a 'a b'"#);
    assert_eq!(render(r#""`{v}"#, &[("v", "a b")]), r#""`'a b'"#);
    // ...and double quotes INSIDE the substitution take the double-quote branch again: only the
    // innermost frame decides.
    // oracle: _posix_quote_value("$x", '"("') == "\\$x"
    assert_eq!(render(r#""$(a "{v}"#, &[("v", "$x")]), r#""$(a "\$x"#);
}

// ==========================================================================
// _substitute_posix via build_command / describe_command / real execution.
// ==========================================================================

#[test]
fn test_double_quoted_placeholder_neutralizes_command_substitution() {
    // The $ is backslash-escaped INSIDE the double quotes (old code left it live).
    let cmd = render(
        r#"printf "%s\n" "{value}""#,
        &[("value", "$(printf unexpected)")],
    );
    assert_eq!(cmd, r#"printf "%s\n" "\$(printf unexpected)""#);
    // The user-visible proof: the child prints the value literally, no substitution.
    let (ok, out) = run_sh(&cmd);
    assert!(ok);
    assert_eq!(out, "$(printf unexpected)\n");
}

#[test]
fn test_single_quoted_placeholder_stays_literal_with_apostrophe_and_substitution() {
    let cmd = render("echo '{v}'", &[("v", "a'b $(id)")]);
    assert_eq!(cmd, r"echo 'a'\''b $(id)'");
    let (ok, out) = run_sh(&cmd);
    assert!(ok);
    assert_eq!(out, "a'b $(id)\n");
}

#[test]
fn test_unquoted_placeholder_embedded_in_a_word() {
    let cmd = render("echo scale={width}:-1", &[("width", "640")]);
    assert_eq!(cmd, "echo scale=640:-1");
    let (ok, out) = run_sh(&cmd);
    assert!(ok);
    assert_eq!(out, "scale=640:-1\n");
}

#[test]
fn test_unquoted_placeholder_hostile_value_cannot_escape_the_word() {
    let cmd = render("echo scale={width}:-1", &[("width", "640 $(id)")]);
    // shlex.quote wraps the whole value, so the space and $(...) stay inside one word.
    assert_eq!(cmd, "echo scale='640 $(id)':-1");
    let (ok, out) = run_sh(&cmd);
    assert!(ok);
    assert_eq!(out, "scale=640 $(id):-1\n"); // $(id) never ran
}

#[test]
fn test_unfilled_placeholder_travels_through_unchanged() {
    // The missing-value gate lives in command_plan, not this seam, so no `meta.params = None` trick
    // is needed: an unfilled placeholder that is simply absent from the map travels as-is.
    let cmd = render("echo {leftover}", &[]);
    assert_eq!(cmd, "echo {leftover}");
}

#[test]
fn test_brace_escapes_collapse_inside_quotes_without_disturbing_state() {
    let cmd = render(r#"echo "{{x}} {v}""#, &[("v", "$X")]);
    // {{ }} collapse to literal single braces even inside the double quotes...
    assert_eq!(cmd, r#"echo "{x} \$X""#);
    assert!(!cmd.contains("{{x}}"));
    // ...and the intervening braces are state-neutral: {v} still gets DOUBLE-context escaping.
    let (ok, out) = run_sh(&cmd);
    assert!(ok);
    assert_eq!(out, "{x} $X\n");
}

#[test]
fn test_substituted_value_containing_double_braces_is_not_rescanned() {
    let cmd = render(r#"echo "{v}""#, &[("v", "{{x}}")]);
    // One-pass substitution: the value's own "{{" is NOT treated as a template escape.
    assert_eq!(cmd, r#"echo "{{x}}""#);
    let (ok, out) = run_sh(&cmd);
    assert!(ok);
    assert_eq!(out, "{{x}}\n");
}

#[test]
fn test_extra_args_are_appended_shell_quoted_after_the_template() {
    // Extra args are appended after the rendered template, each shell-quoted (shlex on POSIX). This
    // append lives in command_plan/append_shell_args, reachable through build_launch_plan.
    let mut command = command_entry("echo {v}");
    let assembly = Assembly {
        args: vec!["a b".to_owned(), "$X".to_owned()],
        masked_args: vec!["a b".to_owned(), "$X".to_owned()],
        command_values: map(&[("v", "hi")]),
        masked_command_values: map(&[("v", "hi")]),
        ..Assembly::default()
    };
    let mut probe = probe_for("/unused");
    probe.programs.insert("sh".to_owned(), "/bin/sh".into());
    command.meta.workdir = "invoke".to_owned();

    let plan =
        build_launch_plan(&command, &paths("/unused"), &assembly, None, None, &probe).unwrap();
    // build_command(entry, ["a b", "$X"], values={"v": "hi"}) == "echo hi " + shlex.join(...)
    assert_eq!(plan.args, ["-c", "echo hi 'a b' '$X'"]);
    let (ok, out) = run_sh(&plan.args[1]);
    assert!(ok);
    assert_eq!(out, "hi a b $X\n"); // extra args quoted, $X not expanded
}

#[test]
fn test_quote_state_affects_only_later_placeholders() {
    // {a} sits in double quotes (backslash-escaped $), {b} lands unquoted after the closing quote
    // (shlex single-quoting): different escaping proves the first slot's context did not leak.
    let cmd = render(r#"echo "{a}" {b}"#, &[("a", "$A"), ("b", "$B")]);
    assert_eq!(cmd, r#"echo "\$A" '$B'"#);
    let (ok, out) = run_sh(&cmd);
    assert!(ok);
    assert_eq!(out, "$A $B\n");
}

#[test]
fn test_describe_command_uses_the_same_context_aware_quoting() {
    // Python build and describe both call _render; Rust command_plan renders both the command and
    // its masked twin through render_command_template — so this shared function IS the describe
    // path. The transparency/dry-run line escapes the value for its double-quoted slot too.
    let line = render(r#"echo "{v}""#, &[("v", "$(id)")]);
    assert_eq!(line, r#"echo "\$(id)""#);
}

#[test]
fn test_dangling_backslash_before_a_placeholder_cannot_eat_the_value_escape() {
    // A template backslash immediately before a double-quoted placeholder would otherwise consume
    // the `\` guarding the value's `$`. The renderer completes the template's backslash into a
    // literal `\\` pair, keeping the escape intact.
    let cmd = render(
        r#"printf "%s\n" "foo\{name}""#,
        &[("name", "$(printf pwned)")],
    );
    assert_eq!(cmd, r#"printf "%s\n" "foo\\\$(printf pwned)""#);
    let (ok, out) = run_sh(&cmd);
    assert!(ok);
    // The author's backslash survives as a literal; the substitution stays dead.
    assert_eq!(out, "foo\\$(printf pwned)\n");
}

#[test]
fn test_dangling_backslash_in_unquoted_position_is_neutralized_too() {
    let cmd = render(r"printf %s\\n foo\{name}", &[("name", "$(printf pwned)")]);
    // The completed `\\` is a literal backslash; the value keeps its own single quotes.
    assert_eq!(cmd, r"printf %s\\n foo\\'$(printf pwned)'");
    let (ok, out) = run_sh(&cmd);
    assert!(ok);
    assert_eq!(out, "foo\\$(printf pwned)\n");
}

#[test]
fn test_even_backslash_run_before_a_placeholder_adds_no_neutralizer() {
    // `\\` before the slot is a self-contained escaped backslash — no pending escape, so the
    // renderer must NOT add another one (that would grow the run and re-arm the $).
    let cmd = render(
        r#"printf "%s\n" "a\\{name}""#,
        &[("name", "$(printf pwned)")],
    );
    assert_eq!(cmd, r#"printf "%s\n" "a\\\$(printf pwned)""#);
    let (_ok, out) = run_sh(&cmd);
    assert_eq!(out, "a\\$(printf pwned)\n");
}

#[test]
fn test_dangling_backslash_before_brace_escape_and_unfilled_placeholder_is_absorbed() {
    // A `{{` emission (and an unfilled `{name}`) begins with a brace: the template's dangling
    // backslash escapes THAT — `\{` and `\\{` are the same literal two chars in sh — so no
    // neutralizer is added, and the state resolves for everything that follows.
    let cmd = render(
        r#"printf "%s\n" "\{{x}} {later}""#,
        &[("later", "$(printf pwned)")],
    );
    assert_eq!(cmd, r#"printf "%s\n" "\{x} \$(printf pwned)""#);
    let (_ok, out) = run_sh(&cmd);
    assert_eq!(out, "\\{x} $(printf pwned)\n");
    // The unfilled twin, driven through the substitutor directly: `{never}` stays as-is, its brace
    // absorbs the dangling escape, and the LATER filled placeholder still gets clean double-quote
    // escaping. (Python: _substitute_posix('"\\{never} {later}"', {"later": "$(x)"}).)
    let rendered = render(r#""\{never} {later}""#, &[("later", "$(x)")]);
    assert_eq!(rendered, r#""\{never} \$(x)""#);
}

// ==========================================================================
// _render — the win32 branch (list2cmdline). Rust selects it at COMPILE time via #[cfg(windows)];
// it is not built on this POSIX target, and no runtime fake replaces sys.platform. cross-crate.
// ==========================================================================

#[test]
#[ignore = "cross-crate: the Windows render branch (render_windows_command_template, #[cfg(windows)] in crates/skit-runtime/src/launch.rs) is not compiled on this POSIX target. Python reaches it by monkeypatching sys.platform, which a compile-time cfg cannot emulate."]
fn test_render_win32_uses_list2cmdline_not_posix_quoting() {
    // entry "echo {v}"; extra=["c d"]; values={"v": "a b"}
    // Windows list2cmdline wraps spaced values AND spaced extra args in double quotes:
    //   cmd == 'echo "a b" "c d"'  and  "'a b'" not in cmd  (POSIX would single-quote).
}

#[test]
#[ignore = "cross-crate: the Windows render branch (render_windows_command_template, #[cfg(windows)] in crates/skit-runtime/src/launch.rs) is not compiled on this POSIX target. Python reaches it by monkeypatching sys.platform, which a compile-time cfg cannot emulate."]
fn test_render_win32_repl_handles_brace_escapes_and_unfilled_placeholders() {
    // entry "echo {{x}} {filled} {unfilled}"; values={"filled": "v"}
    // Every repl branch: {{ -> {, }} -> }, a filled placeholder, and an untouched one:
    //   cmd == "echo {x} v {unfilled}"
}

// ==========================================================================
// Command substitution frames: `$( … )` and `` ` … ` `` restart quoting, so the state machine is
// a FRAME STACK, not one character. White-box units of the private PosixQuoteState. cross-crate.
// ==========================================================================

#[test]
#[ignore = "cross-crate: white-box unit of skit-runtime's private PosixQuoteState (crates/skit-runtime/src/launch.rs); no equal Rust value to Python's state-STRING. Frame behavior is covered observably by test_value_inside_a_substitution_takes_the_unquoted_branch and the nested-substitution execution tests."]
fn test_state_pushes_and_pops_a_command_substitution_frame() {
    // `$(` opens a frame even inside double quotes, and `)` pops it — so the template's own `"`
    // after the substitution closes the OUTER context, not the inner one.
    // launch._posix_quote_state('"$(', "") == '"('
    // launch._posix_quote_state('"$(cmd)', "") == '"'
    // launch._posix_quote_state('"$(cmd "', "") == '"("'
    // A `)` with no frame open is left alone.
    // launch._posix_quote_state('"a)b"', "") == ""
}

#[test]
#[ignore = "cross-crate: white-box unit of skit-runtime's private PosixQuoteState (crates/skit-runtime/src/launch.rs); no equal Rust value to Python's state-STRING. Covered observably by the nested-substitution tests."]
fn test_state_treats_backticks_as_a_frame_that_one_character_opens_and_closes() {
    // launch._posix_quote_state('"`', "") == '"`'
    // launch._posix_quote_state('"`cmd`', "") == '"'
}

#[test]
#[ignore = "cross-crate: white-box unit of skit-runtime's private PosixQuoteState (crates/skit-runtime/src/launch.rs); no equal Rust value to Python's state-STRING. Covered observably by the nested-substitution tests."]
fn test_state_pops_exactly_one_frame_off_a_deep_stack() {
    // Every pop takes the innermost frame and nothing else (asserted on stacks >= three deep).
    // launch._posix_quote_state("\"$(a 'b'", "") == '"('
    // launch._posix_quote_state('"$(a "b"', "") == '"('
    // launch._posix_quote_state('"$(a `b`', "") == '"('
    // And `)` pops the substitution frame only — the `"(` it was nested in survives.
    // launch._posix_quote_state('"$("$(a)', "") == '"("'
}

#[test]
#[ignore = "cross-crate: white-box unit of skit-runtime's private PosixQuoteState (crates/skit-runtime/src/launch.rs); no equal Rust value to Python's state-STRING. Covered observably by the nested-substitution tests."]
fn test_state_opening_a_frame_never_discards_the_ones_below_it() {
    // Each opener PUSHES; replacing the stack instead would silently drop the enclosing context.
    // launch._posix_quote_state("\"$(a '", "") == "\"('"
    // launch._posix_quote_state('"$(a "', "") == '"("'
    // launch._posix_quote_state('"$(a `', "") == '"(`'
    // launch._posix_quote_state('"$(a $(', "") == '"(('
}

#[test]
#[ignore = "cross-crate: white-box unit of skit-runtime's private PosixQuoteState (crates/skit-runtime/src/launch.rs); no equal Rust value to Python's state-STRING. Covered observably by the nested-substitution tests."]
fn test_state_only_a_dollar_followed_by_paren_opens_a_substitution() {
    // A bare `$` (a variable reference, a literal) is ordinary text: treating it as an opener would
    // both push a phantom frame and swallow the next character.
    // launch._posix_quote_state('"$HOME', "") == '"'
    // launch._posix_quote_state("$x", "") == ""
    // ...and a `(` that no `$` introduces is ordinary text too.
    // launch._posix_quote_state('"(a)', "") == '"'
}

#[test]
fn test_value_survives_a_nested_command_substitution_verbatim() {
    // The value lands inside a command substitution and must round-trip byte-exact through real
    // execution — `;`, spaces, `$`, apostrophes and PWNED attempts all arrive dead as data.
    let cases: [(&str, &str); 7] = [
        // The value lands UNQUOTED inside the substitution: `;` would have been live.
        (
            r#"printf "%s\n" "$(printf %s {v})""#,
            "safe; printf INJECTED",
        ),
        // ...and word-splitting would have eaten the space.
        (r#"printf "%s\n" "$(printf %s {v})""#, "a b"),
        // The value lands in double quotes INSIDE the substitution: the old tracker read that `"`
        // as closing the outer one and shlex-quoted, adding literal apostrophes.
        (r#"printf "%s\n" "$(printf %s "{v}")""#, "$(printf PWNED)"),
        (r#"printf "%s\n" "$(printf %s "{v}")""#, "a b"),
        // Backticks, unquoted and single-quoted inner contexts.
        (
            r#"printf "%s\n" "`printf %s {v}`""#,
            "safe; printf INJECTED",
        ),
        (r#"printf "%s\n" "`printf %s '{v}'`""#, "it's $HOME"),
        // Two levels deep.
        (
            r#"printf "%s\n" "$(printf %s "$(printf %s "{v}")")""#,
            "deep 'a b' $X",
        ),
    ];
    for (template, value) in cases {
        let cmd = render(template, &[("v", value)]);
        let (ok, out) = run_sh(&cmd);
        assert!(ok, "template={template:?} value={value:?} cmd={cmd:?}");
        assert_eq!(
            out,
            format!("{value}\n"),
            "template={template:?} cmd={cmd:?}"
        );
    }
}

#[test]
fn test_double_quotes_nested_in_backticks_are_refused_not_guessed() {
    // The one context skit cannot quote for: the backtick form strips one layer of backslashes
    // before parsing the inner command, so the `\$` the double-quote branch emits arrives bare and
    // `$(cmd)` runs. The render refuses instead of assembling a command that means something else.
    let error = render_command_template(
        r#"printf "%s\n" "`printf %s "{v}"`""#,
        &map(&[("v", "$(printf PWNED)")]),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        LaunchError::UnsafeTemplatePlaceholder { .. }
    ));
    // The whole rendered message, not a substring (the oracle asserts full-string equality).
    assert_eq!(
        error.to_string(),
        "Can't safely fill in a value inside double quotes nested in a `…` command \
         substitution — the shell strips one layer of escaping there. Rewrite that part \
         of the template with $(…) instead of backticks."
    );
}

// --- Harness for the extra-args test (mirrors crates/skit-runtime/tests/launch_plan.rs) ---

#[derive(Debug, Default)]
struct FakeProbe {
    programs: BTreeMap<String, std::path::PathBuf>,
    files: Vec<std::path::PathBuf>,
    dirs: Vec<std::path::PathBuf>,
}

impl ProgramProbe for FakeProbe {
    fn find_program(&self, name: &str) -> Option<std::path::PathBuf> {
        self.programs.get(name).cloned()
    }

    fn is_file(&self, path: &std::path::Path) -> bool {
        self.files.iter().any(|item| item == path)
    }

    fn is_dir(&self, path: &std::path::Path) -> bool {
        self.dirs.iter().any(|item| item == path)
    }

    fn is_executable(&self, path: &std::path::Path) -> bool {
        self.files.iter().any(|item| item == path)
    }
}

fn command_entry(template: &str) -> Entry {
    let mut entry = Entry {
        slug: Slug::parse("demo").unwrap(),
        meta: EntryMeta::minimal("Demo", EntryKind::parse("command").unwrap()),
    };
    EntrySettings {
        template: template.to_owned(),
        ..EntrySettings::default()
    }
    .write_to_meta(&mut entry.meta);
    entry
}

fn paths(script: &str) -> LaunchPaths {
    LaunchPaths {
        script: std::path::PathBuf::from(script),
        entry_dir: std::path::PathBuf::from("/data/scripts/demo"),
        invoke_cwd: std::path::PathBuf::from("/invoke"),
    }
}

fn probe_for(script: &str) -> FakeProbe {
    FakeProbe {
        files: vec![std::path::PathBuf::from(script)],
        dirs: vec![
            std::path::PathBuf::from("/invoke"),
            std::path::PathBuf::from("/data/scripts/demo"),
        ],
        ..FakeProbe::default()
    }
}
