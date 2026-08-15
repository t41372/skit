use skit_language::suggest_description;

#[test]
fn test_prompt_description_takes_first_line_minus_heading() {
    assert_eq!(suggest_description("prompt",b"\n\n## A title ##\nbody"),"A title ##");
    assert_eq!(suggest_description("prompt",b"plain line\n"),"plain line");
    assert_eq!(suggest_description("prompt",b"\n\n"),"");
}

#[test]
fn test_prompt_description_caps_derived_metadata_without_breaking_unicode() {
    const FROZEN_LIMIT:usize=120;
    let exact=format!("{}🙂","界".repeat(FROZEN_LIMIT-1));
    assert_eq!(exact.chars().count(),FROZEN_LIMIT);
    assert_eq!(suggest_description("prompt",format!("# {exact}\nbody").as_bytes()),exact);
    let over=format!("{exact}尾");
    let expected=format!("{}…",exact.chars().take(FROZEN_LIMIT-1).collect::<String>());
    assert_eq!(suggest_description("prompt",over.as_bytes()),expected);
    assert_eq!(suggest_description("prompt",over.as_bytes()).chars().count(),FROZEN_LIMIT);
    let huge="提示🙂".repeat(40_000);
    let derived=suggest_description("prompt",huge.as_bytes());
    assert_eq!(derived.chars().count(),FROZEN_LIMIT);
    assert!(derived.ends_with('…'));
    assert_eq!(derived,format!("{}…",huge.chars().take(FROZEN_LIMIT-1).collect::<String>()));
}
