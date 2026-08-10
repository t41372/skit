//! Public-API ports of Python v0.4 `tests/test_tokens.py`.

use std::collections::BTreeMap;

use skit_application::tokens::{TokenContext, TokenError, expand, has_tokens, preview};

fn context(env: &[(&str, &str)]) -> TokenContext {
    TokenContext {
        cwd: "/work/dir".to_owned(),
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
fn test_cwd_today_now_and_multiple_named_tokens_expand() {
    assert_eq!(x("{cwd}/out.png"), "/work/dir/out.png");
    assert_eq!(x("report_{today}.csv"), "report_2026-07-09.csv");
    assert_eq!(x("run_{now}.log"), "run_14-30-05.log");
    assert_eq!(
        x("{cwd}/out_{today}_{now}.png"),
        "/work/dir/out_2026-07-09_14-30-05.png"
    );
}

#[test]
fn test_env_token_present_and_missing_exact_error_shape() {
    assert_eq!(
        expand("key={env:API_KEY}", &context(&[("API_KEY", "abc123")]), true).unwrap(),
        "key=abc123"
    );

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
fn test_unknown_braces_pass_through() {
    assert_eq!(x("pattern_{frame}.png"), "pattern_{frame}.png");
    assert_eq!(x("{envelope}"), "{envelope}");
}

#[test]
fn test_double_brace_escapes_and_mid_string_escape_sequences_are_exact() {
    for (source, expected) in [
        ("{{cwd}}", "{cwd}"),
        ("a{{b}}c", "a{b}c"),
        ("x{{y}}z", "x{y}z"),
        ("{{{{", "{{"),
        ("}}{{", "}{"),
        ("a{{", "a{"),
        ("{today}}}", "2026-07-09}"),
        ("abc{{d}}e", "abc{d}e"),
        ("word {today} tail{{x}}", "word 2026-07-09 tail{x}"),
    ] {
        assert_eq!(x(source), expected, "{source:?}");
    }
}

#[test]
fn test_brace_escape_policy_is_orthogonal_to_named_token_expansion() {
    let ctx = context(&[]);
    assert_eq!(expand("{{cwd}}", &ctx, false).unwrap(), "{{cwd}}");
    assert_eq!(expand("a{{b}}c", &ctx, false).unwrap(), "a{{b}}c");
    assert_eq!(expand("{{cwd}}", &ctx, true).unwrap(), "{cwd}");
    assert_eq!(expand("{cwd}/x", &ctx, false).unwrap(), "/work/dir/x");
    assert_eq!(expand("{cwd}/x", &ctx, true).unwrap(), "/work/dir/x");
}

#[test]
fn test_preview_threads_context_and_brace_policy_and_never_raises() {
    let ctx = context(&[("K", "v")]);
    assert_eq!(preview("{{cwd}}", &ctx, false), ("{{cwd}}".to_owned(), None));
    assert_eq!(preview("{{cwd}}", &ctx, true), ("{cwd}".to_owned(), None));
    assert_eq!(preview("{cwd}", &ctx, true), ("/work/dir".to_owned(), None));
    assert_eq!(preview("{env:K}", &ctx, true), ("v".to_owned(), None));
    assert_eq!(preview("{now}", &ctx, true), ("14-30-05".to_owned(), None));

    let (original, error) = preview("{env:NOPE}", &ctx, true);
    assert_eq!(original, "{env:NOPE}");
    assert!(error.is_some_and(|message| message.contains("NOPE")));
}

#[test]
fn test_tilde_expands_only_at_the_start_and_composes_with_tokens() {
    assert_eq!(x("~/x.txt"), "/home/u/x.txt");
    assert_eq!(x("~"), "/home/u");
    assert_eq!(x("a~b"), "a~b");
    assert_eq!(x("~/out_{today}.png"), "/home/u/out_2026-07-09.png");
}

#[test]
fn test_missing_home_leaves_current_user_tilde_unchanged() {
    let ctx = TokenContext {
        home: None,
        ..context(&[])
    };
    assert_eq!(expand("~/x", &ctx, true).unwrap(), "~/x");
    assert_eq!(expand("~", &ctx, true).unwrap(), "~");
}

#[test]
fn test_plain_text_is_unchanged() {
    assert_eq!(x("just a value, nothing special"), "just a value, nothing special");
    assert_eq!(x("plain tail"), "plain tail");
}

#[test]
fn test_has_tokens_matches_actionable_scanner_shapes() {
    for text in ["{cwd}/x", "~/x", "a{{b", "a}}b", "{env:A}"] {
        assert!(has_tokens(text), "{text:?}");
    }
    for text in ["plain", "pattern_{frame}.png"] {
        assert!(!has_tokens(text), "{text:?}");
    }
}

#[test]
fn test_invalid_environment_token_name_is_literal_not_an_error() {
    for text in ["{env:}", "{env:1BAD}", "{env:BAD-NAME}"] {
        assert_eq!(x(text), text);
        assert!(!has_tokens(text));
    }
}

#[test]
fn test_context_environment_is_explicit_and_does_not_read_process_global_values() {
    let ctx = TokenContext {
        env: BTreeMap::from([("A".to_owned(), "explicit".to_owned())]),
        ..context(&[])
    };
    assert_eq!(expand("{env:A}", &ctx, true).unwrap(), "explicit");
}
