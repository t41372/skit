//! Mechanical port of the Python oracle module `tests/test_tokens.py`
//! (`origin/main@206f9ef`): "Value-token engine: expansion, escapes, pass-through,
//! and error contracts." Each `#[test]` keeps its Python `def test_*` name so it
//! traces back to its origin, and each Python "WHY" comment is preserved above it.
//!
//! Concept mapping used throughout:
//! - Python `tokens.expand(text, cwd=CWD, env=env, now=NOW, brace_escapes=…)` ->
//!   `expand(text, &TokenContext { cwd, home, env, today, now }, brace_escapes)`.
//! - The oracle's `NOW = datetime(2026, 7, 9, 14, 30, 5)` renders to `{today}` via
//!   `strftime("%Y-%m-%d")` and `{now}` via `strftime("%H-%M-%S")`; the Rust
//!   `TokenContext` instead holds those already formatted -> `today = "2026-07-09"`,
//!   `now = "14-30-05"`.
//! - Python `CWD = Path("/work/dir")` and `str(CWD)` -> `cwd = "/work/dir"`.
//! - Python `os.path.expanduser` reads `HOME`/`USERPROFILE` (the monkeypatched home)
//!   -> the injected `TokenContext.home` field.
//! - Python `tokens.preview(...)` -> `preview(...)`, returning `(String, Option<String>)`
//!   (the string arm carries the localized message; `preview_typed` is a Rust-only
//!   superset and is not used here).
//! - Python `tokens.has_tokens(text)` -> `has_tokens(text)`.
//! - Python `tokens.TokenError` (a `ValueError`) -> `TokenError::MissingEnvironment
//!   { name, token }`; its `Display` reproduces the oracle sentence verbatim.
//!
//! Buckets:
//! - Bucket 1 (API exists): every test below except the one named next; each asserts
//!   the exact oracle input, output, and error string.
//! - Bucket 2 (cross-crate): `test_default_env_and_now_paths` exercises the Python
//!   default-argument path where `expand` reads `os.environ` / `datetime.now()` when
//!   `env`/`now` are omitted (`src/skit/tokens.py:52-55`). The Rust application layer
//!   receives all ambient state explicitly by design (see the `tokens.rs` module doc),
//!   so that real-environment/real-clock resolution lives in the composition root
//!   `skit-cli::run::command::token_context()` (`crates/skit-cli/src/run/command.rs:846`,
//!   `pub(crate)`), unreachable from this crate's integration tests without a forbidden
//!   dependency edit. skit-cli already covers it at `command.rs:1299-1300`. Comment-only
//!   `#[ignore]` stub.
//! - Bucket 3 (CLI/store integration): NONE. This oracle module drives only pure
//!   functions in `skit-application::tokens`.

use std::collections::BTreeMap;

use skit_application::tokens::{TokenContext, TokenError, expand, has_tokens, preview};

// --- The oracle's module-level fixtures ---
//
// NOW = datetime(2026, 7, 9, 14, 30, 5)
// CWD = Path("/work/dir")
//
// `home = Some("/home/u")` mirrors the monkeypatched HOME the tilde tests set. It is
// inert for every non-tilde input, so one fixture serves all tests.
fn context() -> TokenContext {
    TokenContext {
        cwd: "/work/dir".to_owned(),
        home: Some("/home/u".to_owned()),
        env: BTreeMap::new(),
        today: "2026-07-09".to_owned(),
        now: "14-30-05".to_owned(),
    }
}

/// Oracle `_expand(text, env=None)`: `tokens.expand(text, cwd=CWD, env=env or {}, now=NOW)`
/// (brace_escapes defaults to True).
fn expand_default(text: &str) -> String {
    expand(text, &context(), true).unwrap()
}

/// Oracle `_expand(text, env)` with a populated environment.
fn expand_with_env(text: &str, env: &[(&str, &str)]) -> String {
    let mut context = context();
    for (name, value) in env {
        context.env.insert((*name).to_owned(), (*value).to_owned());
    }
    expand(text, &context, true).unwrap()
}

#[test]
fn test_cwd_token() {
    // {cwd} expands to str(cwd), which is platform-native (backslashes on Windows) — so pin the
    // expectation to str(CWD), not a hardcoded POSIX prefix. The Rust cwd is a String already, so
    // "/work/dir" is that value verbatim.
    assert_eq!(expand_default("{cwd}/out.png"), "/work/dir/out.png");
}

#[test]
fn test_today_token() {
    assert_eq!(
        expand_default("report_{today}.csv"),
        "report_2026-07-09.csv"
    );
}

#[test]
fn test_now_token() {
    assert_eq!(expand_default("run_{now}.log"), "run_14-30-05.log");
}

#[test]
fn test_env_token_present() {
    assert_eq!(
        expand_with_env("key={env:API_KEY}", &[("API_KEY", "abc123")]),
        "key=abc123"
    );
}

#[test]
fn test_env_token_missing_raises_with_names() {
    let error = expand("{env:MISSING_VAR}", &context(), true).unwrap_err();
    // The typed variant carries both interpolated fields intact.
    assert_eq!(
        error,
        TokenError::MissingEnvironment {
            name: "MISSING_VAR".to_owned(),
            token: "{env:MISSING_VAR}".to_owned(),
        }
    );
    // Pin the whole sentence, not just the substrings: both interpolated fields (the bare name
    // and the full token) *and* the exact prose/casing, so a corrupted or down-cased message
    // string cannot slip through.
    assert_eq!(
        error.to_string(),
        "The environment variable MISSING_VAR isn't set (needed by {env:MISSING_VAR})."
    );
}

#[test]
fn test_multiple_tokens_in_one_value() {
    assert_eq!(
        expand_default("{cwd}/out_{today}_{now}.png"),
        "/work/dir/out_2026-07-09_14-30-05.png"
    );
}

#[test]
fn test_unknown_braces_pass_through() {
    // A value may carry braces meant for the script itself; only known tokens expand.
    assert_eq!(expand_default("pattern_{frame}.png"), "pattern_{frame}.png");
    assert_eq!(expand_default("{envelope}"), "{envelope}");
}

#[test]
fn test_double_brace_escapes() {
    assert_eq!(expand_default("{{cwd}}"), "{cwd}");
    assert_eq!(expand_default("a{{b}}c"), "a{b}c");
}

#[test]
fn test_brace_escapes_false_keeps_double_braces_byte_identical() {
    // Placeholder-delivery mode: `{{`/`}}` pass through untouched (prompt text is
    // brace-heavy; unmanaged text travels byte-identical).
    assert_eq!(expand("{{cwd}}", &context(), false).unwrap(), "{{cwd}}");
    assert_eq!(expand("a{{b}}c", &context(), false).unwrap(), "a{{b}}c");
}

#[test]
fn test_brace_escapes_true_halves_the_pair() {
    assert_eq!(expand("{{cwd}}", &context(), true).unwrap(), "{cwd}");
}

#[test]
fn test_named_tokens_expand_in_both_brace_modes() {
    // The escape-pair policy is orthogonal to the named tokens: {cwd} expands regardless.
    assert_eq!(expand("{cwd}/x", &context(), false).unwrap(), "/work/dir/x");
    assert_eq!(expand("{cwd}/x", &context(), true).unwrap(), "/work/dir/x");
}

#[test]
fn test_preview_threads_brace_escapes() {
    // The preview must take the SAME brace_escapes the delivery will, or it shows a lie.
    assert_eq!(
        preview("{{cwd}}", &context(), false),
        ("{{cwd}}".to_owned(), None)
    );
    assert_eq!(
        preview("{{cwd}}", &context(), true),
        ("{cwd}".to_owned(), None)
    );
}

#[test]
fn test_tilde_expansion_only_at_start() {
    // The oracle monkeypatches HOME=/home/u (POSIX) and USERPROFILE=/home/u (Windows, which
    // os.path.expanduser reads); the Rust fixture injects the same via context.home.
    assert_eq!(expand_default("~/x.txt"), "/home/u/x.txt");
    assert_eq!(expand_default("~"), "/home/u");
    assert_eq!(expand_default("a~b"), "a~b"); // not a home reference; untouched
}

#[test]
fn test_tilde_then_tokens_compose() {
    assert_eq!(
        expand_default("~/out_{today}.png"),
        "/home/u/out_2026-07-09.png"
    );
}

#[test]
fn test_plain_text_unchanged() {
    assert_eq!(
        expand_default("just a value, nothing special"),
        "just a value, nothing special"
    );
}

#[test]
fn test_preview_success_and_failure() {
    let (ok, err) = preview("x_{today}", &context(), true);
    assert_eq!((ok, err), ("x_2026-07-09".to_owned(), None));
    let (orig, err) = preview("{env:NOPE}", &context(), true);
    assert_eq!(orig, "{env:NOPE}");
    assert!(err.is_some());
    assert!(err.unwrap().contains("NOPE"));
}

#[test]
fn test_has_tokens() {
    assert!(has_tokens("{cwd}/x"));
    assert!(has_tokens("~/x"));
    assert!(has_tokens("a{{b"));
    assert!(has_tokens("a}}b")); // a closing-brace escape alone still means expand() acts
    assert!(has_tokens("{env:A}"));
    assert!(!has_tokens("plain"));
    assert!(!has_tokens("pattern_{frame}.png")); // unknown token: expand() is a no-op
}

#[test]
#[ignore = "CROSS-CRATE (bucket 2): the Python default-argument path — expand() reading os.environ / datetime.now() when env/now are omitted (src/skit/tokens.py:52-55) — is deliberately not in skit-application. The tokens.rs module doc injects all ambient state explicitly; the real-environment/real-clock resolution lives in the composition root skit-cli::run::command::token_context() (crates/skit-cli/src/run/command.rs:846, pub(crate)), unreachable from this crate's integration tests. skit-cli covers it at command.rs:1299-1300."]
fn test_default_env_and_now_paths() {
    // Defaults resolve from the real environment/clock; pin just the env var.
    // monkeypatch.setenv("SKIT_TOKEN_TEST", "v")
    // assert tokens.expand("{env:SKIT_TOKEN_TEST}", cwd=CWD) == "v"
    // out = tokens.expand("{today}", cwd=CWD)
    // assert len(out) == 10 and out[4] == "-" and out[7] == "-"
}

// --------------------------------------------------------------------------
// mutation hardening
// --------------------------------------------------------------------------

#[test]
fn test_escape_sequences_mid_string_exact() {
    assert_eq!(expand_default("x{{y}}z"), "x{y}z");
    assert_eq!(expand_default("{{{{"), "{{"); // two escapes back to back
    assert_eq!(expand_default("}}{{"), "}{");
    assert_eq!(expand_default("a{{"), "a{"); // trailing opener escape
    assert_eq!(expand_default("{today}}}"), "2026-07-09}"); // token then escape
}

#[test]
fn test_preview_forwards_every_argument() {
    // cwd forwarded
    assert_eq!(
        preview("{cwd}", &context(), true),
        ("/work/dir".to_owned(), None)
    );
    // env forwarded (a dropped env kwarg would fall back to os.environ and miss K)
    assert_eq!(
        {
            let mut context = context();
            context.env.insert("K".to_owned(), "v".to_owned());
            preview("{env:K}", &context, true)
        },
        ("v".to_owned(), None)
    );
    // now forwarded
    assert_eq!(
        preview("{now}", &context(), true),
        ("14-30-05".to_owned(), None)
    );
}

#[test]
fn test_escape_deep_in_string_exact() {
    // Escapes far from index 0/2 pin the scanner's advance arithmetic: a mutant that
    // rewinds or pins the index re-reads earlier characters and corrupts the output.
    assert_eq!(expand_default("abc{{d}}e"), "abc{d}e");
    assert_eq!(
        expand_default("word {today} tail{{x}}"),
        "word 2026-07-09 tail{x}"
    );
    assert_eq!(expand_default("plain tail"), "plain tail");
}
