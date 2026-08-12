//! Domain-level ports of the two slugify regressions in Python `tests/test_review_fixes.py`.

use skit_domain::Slug;

#[test]
fn test_slugify_all_special_chars_fallback() {
    for name in ["---", "!!!", "  "] {
        assert_eq!(Slug::from_display_name(name).as_str(), "script", "name={name:?}");
    }
}

#[test]
fn test_slugify_leading_trailing_special() {
    assert_eq!(Slug::from_display_name("-hello-").as_str(), "hello");
    assert_eq!(Slug::from_display_name("hello  world").as_str(), "hello-world");
    assert_eq!(Slug::from_display_name("--hello---world--").as_str(), "hello-world");
}
