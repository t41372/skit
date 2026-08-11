//! Exact public-API ports of Python v0.4 `tests/test_tokens.py` except the one ambient-default
//! contract, which lives in `skit-cli/tests/port_test_tokens_runtime.rs` so it can exercise the
//! real process environment and clock boundary instead of faking them in a `TokenContext`.
//!
//! Python oracle: `origin/main@206f9ef946fc45835cb2479593794431f2620c32`.
//! Rust-only additive tests are named `rust_additive_*` and never count toward Python parity.

use std::collections::BTreeMap;

use skit_application::tokens::{TokenContext, TokenError, expand, has_tokens, preview};

fn native_cwd() -> String {
    if cfg!(windows) {
        r"\work\dir".to_owned()
    } else {
        "/work/dir".to_owned()
    }
}

fn context(env: &[(&str, &str)]) -> TokenContext {
    TokenContext {
        cwd: native_cwd(),
        home: Some("/home/u".to_owned()),
        env: env
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect(),
        today: "2026-07-09".to_owned(),
        now: "14-30-05".to_owned(),
    }
}

fn x(text: &str) -> String {
    expand(text, &context(&[]), true).unwrap()
}

#[test]
fn test_cwd_token() {
    assert_eq!(x("{cwd}/out.png"), format!("{}/out.png", native_cwd()));
}

#[test]
fn test_today_token() {
    assert_eq!(x("report_{today}.csv"), "report_2026-07-09.csv");
}

#[test]
fn test_now_token() {
    assert_eq!(x("run_{now}.log"), "run_14-30-05.log");
}

#[test]
fn test_env_token_present() {
    assert_eq!(
        expand(
            "key={env:API_KEY}",
            &context(&[("API_KEY", "abc123")]),
            true
        )
        .unwrap(),
        "key=abc123"
    );
}

#[test]
fn test_env_token_missing_raises_with_names() {
    let error = expand("{env:MISSING_VAR}", &context(&[]), true).unwrap_err();
    assert_eq!(
        error,
        TokenError::MissingEnvironment {
            name: "MISSING_VAR".to_owned(),
            token: "{env:MISSING_VAR}".to_owned(),
        }
    );
    assert_eq!(
        error.to_string(),
        "The environment variable MISSING_VAR isn't set (needed by {env:MISSING_VAR})."
    );
}

#[test]
fn test_multiple_tokens_in_one_value() {
    assert_eq!(
        x("{cwd}/out_{today}_{now}.png"),
        format!("{}/out_2026-07-09_14-30-05.png", native_cwd())
    );
}

#[test]
fn test_unknown_braces_pass_through() {
    assert_eq!(x("pattern_{frame}.png"), "pattern_{frame}.png");
    assert_eq!(x("{envelope}"), "{envelope}");
}

#[test]
fn test_double_brace_escapes() {
    assert_eq!(x("{{cwd}}"), "{cwd}");
    assert_eq!(x("a{{b}}c"), "a{b}c");
}

#[test]
fn test_brace_escapes_false_keeps_double_braces_byte_identical() {
    let ctx = context(&[]);
    assert_eq!(expand("{{cwd}}", &ctx, false).unwrap(), "{{cwd}}");
    assert_eq!(expand("a{{b}}c", &ctx, false).unwrap(), "a{{b}}c");
}

#[test]
fn test_brace_escapes_true_halves_the_pair() {
    assert_eq!(expand("{{cwd}}", &context(&[]), true).unwrap(), "{cwd}");
}

#[test]
fn test_named_tokens_expand_in_both_brace_modes() {
    let ctx = context(&[]);
    let expected = format!("{}/x", native_cwd());
    assert_eq!(expand("{cwd}/x", &ctx, false).unwrap(), expected);
    assert_eq!(expand("{cwd}/x", &ctx, true).unwrap(), expected);
}

#[test]
fn test_preview_threads_brace_escapes() {
    let ctx = context(&[]);
    assert_eq!(
        preview("{{cwd}}", &ctx, false),
        ("{{cwd}}".to_owned(), None)
    );
    assert_eq!(preview("{{cwd}}", &ctx, true), ("{cwd}".to_owned(), None));
}

#[test]
fn test_tilde_expansion_only_at_start() {
    assert_eq!(x("~/x.txt"), "/home/u/x.txt");
    assert_eq!(x("~"), "/home/u");
    assert_eq!(x("a~b"), "a~b");
}

#[test]
fn test_tilde_then_tokens_compose() {
    assert_eq!(x("~/out_{today}.png"), "/home/u/out_2026-07-09.png");
}

#[test]
fn test_plain_text_unchanged() {
    assert_eq!(
        x("just a value, nothing special"),
        "just a value, nothing special"
    );
}

#[test]
fn test_preview_success_and_failure() {
    let ctx = context(&[]);
    assert_eq!(
        preview("x_{today}", &ctx, true),
        ("x_2026-07-09".to_owned(), None)
    );
    let (original, error) = preview("{env:NOPE}", &ctx, true);
    assert_eq!(original, "{env:NOPE}");
    assert!(error.is_some());
    assert!(error.unwrap().contains("NOPE"));
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
    assert_eq!(x("x{{y}}z"), "x{y}z");
    assert_eq!(x("{{{{"), "{{");
    assert_eq!(x("}}{{"), "}{");
    assert_eq!(x("a{{"), "a{");
    assert_eq!(x("{today}}}"), "2026-07-09}");
}

#[test]
fn test_preview_forwards_every_argument() {
    let ctx = context(&[("K", "v")]);
    assert_eq!(
        preview("{cwd}", &ctx, true),
        (native_cwd(), None)
    );
    assert_eq!(preview("{env:K}", &ctx, true), ("v".to_owned(), None));
    assert_eq!(preview("{now}", &ctx, true), ("14-30-05".to_owned(), None));
}

#[test]
fn test_escape_deep_in_string_exact() {
    assert_eq!(x("abc{{d}}e"), "abc{d}e");
    assert_eq!(
        x("word {today} tail{{x}}"),
        "word 2026-07-09 tail{x}"
    );
    assert_eq!(x("plain tail"), "plain tail");
}

#[test]
fn rust_additive_missing_home_leaves_current_user_tilde_unchanged() {
    let ctx = TokenContext {
        home: None,
        ..context(&[])
    };
    assert_eq!(expand("~/x", &ctx, true).unwrap(), "~/x");
    assert_eq!(expand("~", &ctx, true).unwrap(), "~");
}

#[test]
fn rust_additive_invalid_environment_token_name_is_literal_not_an_error() {
    for text in ["{env:}", "{env:1BAD}", "{env:BAD-NAME}"] {
        assert_eq!(x(text), text);
        assert!(!has_tokens(text));
    }
}

#[test]
fn rust_additive_context_environment_is_explicit_and_does_not_read_process_global_values() {
    let ctx = TokenContext {
        env: BTreeMap::from([("A".to_owned(), "explicit".to_owned())]),
        ..context(&[])
    };
    assert_eq!(expand("{env:A}", &ctx, true).unwrap(), "explicit");
}
