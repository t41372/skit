//! Exact public-domain port of Python v0.4 `tests/test_store_mut.py::test_slugify`.
//!
//! The parent test preserves the Python parametrized contract; additive row tests ensure one bad
//! case cannot hide later variants. These are domain/address semantics only and do not patch slug
//! generation on this branch.

use skit_domain::Slug;

const CASES: &[(&str, &str)] = &[
    ("Hello World!", "hello-world"),
    ("  many   spaces  ", "many-spaces"),
    ("a___b---c", "a-b-c"),
    ("!!!", "script"),
    ("My.Script_v2", "my-script-v2"),
];

#[test]
fn test_slugify() {
    assert_eq!(CASES.len(), 5);
    for (raw, expected) in CASES {
        assert_eq!(Slug::from_display_name(raw).as_str(), *expected, "{raw:?}");
    }
}

#[test]
fn rust_additive_slugify_hello_world() {
    assert_eq!(Slug::from_display_name("Hello World!").as_str(), "hello-world");
}

#[test]
fn rust_additive_slugify_many_spaces() {
    assert_eq!(
        Slug::from_display_name("  many   spaces  ").as_str(),
        "many-spaces"
    );
}

#[test]
fn rust_additive_slugify_mixed_separators() {
    assert_eq!(Slug::from_display_name("a___b---c").as_str(), "a-b-c");
}

#[test]
fn rust_additive_slugify_all_punctuation_falls_back_to_script() {
    assert_eq!(Slug::from_display_name("!!!").as_str(), "script");
}

#[test]
fn rust_additive_slugify_dot_and_underscore() {
    assert_eq!(
        Slug::from_display_name("My.Script_v2").as_str(),
        "my-script-v2"
    );
}
