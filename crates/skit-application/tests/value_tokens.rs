use std::collections::BTreeMap;

use skit_application::tokens::{TokenContext, TokenError, expand, has_tokens, preview};

fn context() -> TokenContext {
    TokenContext {
        cwd: "/work/dir".to_owned(),
        home: Some("/home/u".to_owned()),
        env: BTreeMap::new(),
        today: "2026-07-09".to_owned(),
        now: "14-30-05".to_owned(),
    }
}

#[test]
fn named_tokens_expand_and_unknown_braces_pass_through() {
    let mut context = context();
    context
        .env
        .insert("API_KEY".to_owned(), "abc123".to_owned());

    assert_eq!(
        expand("{cwd}/out_{today}_{now}.png", &context, true).unwrap(),
        "/work/dir/out_2026-07-09_14-30-05.png"
    );
    assert_eq!(
        expand("key={env:API_KEY}", &context, true).unwrap(),
        "key=abc123"
    );
    assert_eq!(
        expand("pattern_{frame}.png/{envelope}", &context, true).unwrap(),
        "pattern_{frame}.png/{envelope}"
    );
}

#[test]
fn missing_environment_variables_fail_with_the_exact_named_token() {
    assert_eq!(
        expand("{env:MISSING_VAR}", &context(), true),
        Err(TokenError::MissingEnvironment {
            name: "MISSING_VAR".to_owned(),
            token: "{env:MISSING_VAR}".to_owned(),
        })
    );
    assert_eq!(
        expand("{env:MISSING_VAR}", &context(), true)
            .unwrap_err()
            .to_string(),
        "The environment variable MISSING_VAR isn't set (needed by {env:MISSING_VAR})."
    );
}

#[test]
fn brace_escape_policy_is_orthogonal_to_named_token_expansion() {
    let context = context();

    assert_eq!(expand("{{cwd}}", &context, true).unwrap(), "{cwd}");
    assert_eq!(expand("a{{b}}c", &context, true).unwrap(), "a{b}c");
    assert_eq!(expand("{{{{", &context, true).unwrap(), "{{");
    assert_eq!(expand("}}{{", &context, true).unwrap(), "}{");
    assert_eq!(expand("a{{", &context, true).unwrap(), "a{");
    assert_eq!(expand("{today}}}", &context, true).unwrap(), "2026-07-09}");

    assert_eq!(expand("{{cwd}}", &context, false).unwrap(), "{{cwd}}");
    assert_eq!(expand("a{{b}}c", &context, false).unwrap(), "a{{b}}c");
    assert_eq!(expand("{cwd}/x", &context, false).unwrap(), "/work/dir/x");
}

#[test]
fn current_user_tilde_expands_only_at_the_start_and_composes_with_tokens() {
    let context = context();

    assert_eq!(expand("~/x.txt", &context, true).unwrap(), "/home/u/x.txt");
    assert_eq!(expand("~", &context, true).unwrap(), "/home/u");
    assert_eq!(
        expand("~/out_{today}.png", &context, true).unwrap(),
        "/home/u/out_2026-07-09.png"
    );
    assert_eq!(expand("a~b", &context, true).unwrap(), "a~b");

    let mut no_home = context;
    no_home.home = None;
    assert_eq!(expand("~/x", &no_home, true).unwrap(), "~/x");
}

#[test]
fn preview_never_raises_and_preserves_the_original_on_failure() {
    let context = context();

    assert_eq!(
        preview("x_{today}", &context, true),
        ("x_2026-07-09".to_owned(), None)
    );
    assert_eq!(
        preview("{{cwd}}", &context, false),
        ("{{cwd}}".to_owned(), None)
    );
    let (original, error) = preview("{env:NOPE}", &context, true);
    assert_eq!(original, "{env:NOPE}");
    assert!(error.unwrap().contains("NOPE"));
}

#[test]
fn has_tokens_matches_exactly_the_syntax_that_expansion_changes() {
    assert!(has_tokens("{cwd}/x"));
    assert!(has_tokens("~/x"));
    assert!(has_tokens("a{{b"));
    assert!(has_tokens("a}}b"));
    assert!(has_tokens("{env:A}"));
    assert!(!has_tokens("plain"));
    assert!(!has_tokens("pattern_{frame}.png"));
    assert!(!has_tokens("a~b"));
}

#[test]
fn scanner_advance_is_exact_deep_inside_values() {
    let context = context();

    assert_eq!(expand("abc{{d}}e", &context, true).unwrap(), "abc{d}e");
    assert_eq!(
        expand("word {today} tail{{x}}", &context, true).unwrap(),
        "word 2026-07-09 tail{x}"
    );
    assert_eq!(
        expand("just a value, nothing special", &context, true).unwrap(),
        "just a value, nothing special"
    );
}

#[test]
fn malformed_environment_tokens_and_escaped_prefixes_remain_literal() {
    let context = context();
    for (value, expected) in [
        ("{env:", "{env:"),
        ("{env:}", "{env:}"),
        ("{env:9BAD}", "{env:9BAD}"),
        ("{{{cwd}", "{/work/dir"),
    ] {
        assert_eq!(expand(value, &context, true).unwrap(), expected);
    }
    assert!(!has_tokens("{env:"));
    assert!(!has_tokens("{env:}"));
    assert!(!has_tokens("{env:9BAD}"));
}
