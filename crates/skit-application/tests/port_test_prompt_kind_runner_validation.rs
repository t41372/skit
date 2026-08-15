use skit_application::runner_management::validate_runner_argv;

fn argv(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn reason(values: &[&str]) -> Option<&'static str> {
    validate_runner_argv(&argv(values))
        .err()
        .map(|error| error.reason_code())
}

#[test]
fn test_validate_prompt_runner_argv_rules() {
    assert_eq!(reason(&["claude", "{{prompt}}"]), None);
    assert_eq!(reason(&["a", "--m={{prompt}}"]), None);
    assert_eq!(reason(&["a", "{lit}", "{{prompt}}"]), None);
    assert_eq!(reason(&["a", "{lit} {{prompt}}"]), None);

    assert_eq!(reason(&[]), Some("empty"));
    assert_eq!(reason(&[""]), Some("empty"));
    assert_eq!(reason(&["claude"]), Some("prompt-slot-count"));
    assert_eq!(
        reason(&["a", "{{prompt}}", "{{prompt}}"]),
        Some("prompt-slot-count")
    );
    assert_eq!(reason(&["{{prompt}}"]), Some("prompt-in-binary"));
    assert_eq!(reason(&["a", "{{other}}"]), Some("stray-hole"));
    assert_eq!(
        reason(&["a", "{{占位符}}", "{{prompt}}"]),
        Some("stray-hole")
    );
    assert_eq!(
        reason(&["a", "{{not-a-name}}", "{{prompt}}"]),
        Some("stray-hole")
    );
    assert_eq!(
        reason(&["a", "{{💥}}", "{{prompt}}"]),
        Some("stray-hole")
    );
}
