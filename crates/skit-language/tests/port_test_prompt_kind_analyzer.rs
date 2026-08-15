use skit_language::placeholder_params;

fn names(text: &str) -> Vec<String> {
    placeholder_params("prompt", text)
        .into_iter()
        .map(|field| field.name)
        .collect()
}

#[test]
fn test_placeholder_names_dedupes_in_body_order() {
    let text = "a {{b}} c {{a}} d {{b}} {{_x1}} {{9bad}} {{ spaced }} {{a-b}}";
    assert_eq!(names(text), ["b", "a", "_x1"]);
}

#[test]
fn test_placeholder_names_single_braces_are_never_candidates() {
    let text = r#"JSON {"key": 1} f-string {value} shell ${HOME} empty {} plain {word}"#;
    assert!(names(text).is_empty());
}

#[test]
fn test_placeholder_names_brace_adjacent_is_not_a_candidate() {
    assert!(names("{{{raw}}} and {{{x}} and {{y}}}").is_empty());
}

#[test]
fn test_placeholder_names_reserved_name_excluded() {
    assert_eq!(names("{{prompt}} {{real}}"), ["real"]);
}

#[test]
fn test_placeholder_names_accept_unicode_identifiers_and_reject_non_names() {
    let text = "{{任务}} {{café}} {{é}} {{not-a-name}} {{💥}} {{}}";
    assert_eq!(names(text), ["任务", "café", "é"]);
}

#[test]
fn test_placeholder_names_high_cardinality_stays_ordered_and_complete() {
    let expected = (0..10_000)
        .map(|index| format!("field_{index}"))
        .collect::<Vec<_>>();
    let text = expected
        .iter()
        .map(|name| format!("{{{{{name}}}}}"))
        .collect::<Vec<_>>()
        .join(" ");
    assert_eq!(names(&text), expected);
}

#[test]
fn test_prompt_grammar_is_independent_of_command_templates() {
    assert!(names("{name}").is_empty());
    assert_eq!(
        placeholder_params("command", "{name}")
            .into_iter()
            .map(|field| field.name)
            .collect::<Vec<_>>(),
        ["name"],
        "prompt double-brace grammar must never collapse into command single-brace grammar"
    );
    assert!(
        placeholder_params("command", "{{name}}")
            .into_iter()
            .all(|field| field.name != "name"),
        "the command grammar must not start accepting prompt-style double braces"
    );
}
