//! Mechanical port of the Python oracle module `tests/test_tokens.py`
//! (`main@206f9ef`). Each test keeps the Python function name and its rationale comment.
//!
//! Python reads ambient state when `cwd`, `env`, or `now` is omitted. The Rust application layer
//! intentionally requires a `TokenContext`; the one ambient-state test is therefore ported as a
//! black-box CLI test in `port_test_tokens_ambient.rs`. The other 20 tests exercise the same
//! scanner through its public deterministic surface here.

use std::{collections::BTreeMap, path::MAIN_SEPARATOR};

use skit_application::tokens::{TokenContext, expand, has_tokens, preview};

fn context() -> TokenContext {
    TokenContext {
        cwd: format!("{MAIN_SEPARATOR}work{MAIN_SEPARATOR}dir"),
        home: Some("/home/u".to_owned()),
        env: BTreeMap::new(),
        today: "2026-07-09".to_owned(),
        now: "14-30-05".to_owned(),
    }
}

fn expand_value(text: &str, environment: &[(&str, &str)]) -> String {
    let mut context = context();
    context.env.extend(
        environment
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned())),
    );
    expand(text, &context, true).unwrap()
}

#[test]
fn test_cwd_token() {
    // {cwd} expands to str(cwd), which is platform-native (backslashes on Windows) — so pin the
    // expectation to the context, not a hardcoded POSIX prefix.
    let context = context();
    assert_eq!(
        expand("{cwd}/out.png", &context, true).unwrap(),
        format!("{}/out.png", context.cwd)
    );
}

#[test]
fn test_today_token() {
    assert_eq!(
        expand_value("report_{today}.csv", &[]),
        "report_2026-07-09.csv"
    );
}

#[test]
fn test_now_token() {
    assert_eq!(expand_value("run_{now}.log", &[]), "run_14-30-05.log");
}

#[test]
fn test_env_token_present() {
    assert_eq!(
        expand_value("key={env:API_KEY}", &[("API_KEY", "abc123")]),
        "key=abc123"
    );
}

#[test]
fn test_env_token_missing_raises_with_names() {
    let error = expand("{env:MISSING_VAR}", &context(), true).unwrap_err();
    // Pin the whole sentence, not just the substrings: both interpolated fields (the bare name
    // and the full token) and the exact prose/casing must survive.
    assert_eq!(
        error.to_string(),
        "The environment variable MISSING_VAR isn't set (needed by {env:MISSING_VAR})."
    );
}

#[test]
fn test_multiple_tokens_in_one_value() {
    let context = context();
    assert_eq!(
        expand("{cwd}/out_{today}_{now}.png", &context, true).unwrap(),
        format!("{}/out_2026-07-09_14-30-05.png", context.cwd)
    );
}

#[test]
fn test_unknown_braces_pass_through() {
    // A value may carry braces meant for the script itself; only known tokens expand.
    assert_eq!(
        expand_value("pattern_{frame}.png", &[]),
        "pattern_{frame}.png"
    );
    assert_eq!(expand_value("{envelope}", &[]), "{envelope}");
}

#[test]
fn test_double_brace_escapes() {
    assert_eq!(expand_value("{{cwd}}", &[]), "{cwd}");
    assert_eq!(expand_value("a{{b}}c", &[]), "a{b}c");
}

#[test]
fn test_brace_escapes_false_keeps_double_braces_byte_identical() {
    // Placeholder-delivery mode: `{{`/`}}` pass through untouched. Prompt text is brace-heavy, so
    // unmanaged text must travel byte-identical.
    let context = context();
    assert_eq!(expand("{{cwd}}", &context, false).unwrap(), "{{cwd}}");
    assert_eq!(expand("a{{b}}c", &context, false).unwrap(), "a{{b}}c");
}

#[test]
fn test_brace_escapes_true_halves_the_pair() {
    assert_eq!(expand("{{cwd}}", &context(), true).unwrap(), "{cwd}");
}

#[test]
fn test_named_tokens_expand_in_both_brace_modes() {
    // The escape-pair policy is orthogonal to named tokens: {cwd} expands in either mode.
    let context = context();
    let expected = format!("{}/x", context.cwd);
    assert_eq!(expand("{cwd}/x", &context, false).unwrap(), expected);
    assert_eq!(expand("{cwd}/x", &context, true).unwrap(), expected);
}

#[test]
fn test_preview_threads_brace_escapes() {
    // The preview must use the same brace policy as delivery, or it shows a lie.
    let context = context();
    assert_eq!(
        preview("{{cwd}}", &context, false),
        ("{{cwd}}".to_owned(), None)
    );
    assert_eq!(
        preview("{{cwd}}", &context, true),
        ("{cwd}".to_owned(), None)
    );
}

#[test]
fn test_tilde_expansion_only_at_start() {
    assert_eq!(expand_value("~/x.txt", &[]), "/home/u/x.txt");
    assert_eq!(expand_value("~", &[]), "/home/u");
    assert_eq!(expand_value("a~b", &[]), "a~b");
}

#[test]
fn test_tilde_then_tokens_compose() {
    assert_eq!(
        expand_value("~/out_{today}.png", &[]),
        "/home/u/out_2026-07-09.png"
    );
}

#[test]
fn test_plain_text_unchanged() {
    assert_eq!(
        expand_value("just a value, nothing special", &[]),
        "just a value, nothing special"
    );
}

#[test]
fn test_preview_success_and_failure() {
    let context = context();
    let (expanded, error) = preview("x_{today}", &context, true);
    assert_eq!((expanded.as_str(), error), ("x_2026-07-09", None));

    let (original, error) = preview("{env:NOPE}", &context, true);
    assert_eq!(original, "{env:NOPE}");
    assert!(error.is_some_and(|message| message.contains("NOPE")));
}

#[test]
fn test_has_tokens() {
    assert!(has_tokens("{cwd}/x"));
    assert!(has_tokens("~/x"));
    assert!(has_tokens("a{{b"));
    assert!(has_tokens("a}}b"));
    assert!(has_tokens("{env:A}"));
    assert!(!has_tokens("plain"));
    assert!(!has_tokens("pattern_{frame}.png"));
}

#[test]
fn test_escape_sequences_mid_string_exact() {
    assert_eq!(expand_value("x{{y}}z", &[]), "x{y}z");
    assert_eq!(expand_value("{{{{", &[]), "{{");
    assert_eq!(expand_value("}}{{", &[]), "}{");
    assert_eq!(expand_value("a{{", &[]), "a{");
    assert_eq!(expand_value("{today}}}", &[]), "2026-07-09}");
}

#[test]
fn test_preview_forwards_every_argument() {
    let mut context = context();
    context.env.insert("K".to_owned(), "v".to_owned());
    assert_eq!(
        preview("{cwd}", &context, true),
        (context.cwd.clone(), None)
    );
    assert_eq!(preview("{env:K}", &context, true), ("v".to_owned(), None));
    assert_eq!(
        preview("{now}", &context, true),
        ("14-30-05".to_owned(), None)
    );
}

#[test]
fn test_escape_deep_in_string_exact() {
    // Escapes far from index 0/2 pin scanner advance arithmetic: a rewind or pinned index would
    // reread earlier characters and corrupt the output.
    assert_eq!(expand_value("abc{{d}}e", &[]), "abc{d}e");
    assert_eq!(
        expand_value("word {today} tail{{x}}", &[]),
        "word 2026-07-09 tail{x}"
    );
    assert_eq!(expand_value("plain tail", &[]), "plain tail");
}
