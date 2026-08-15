use std::path::PathBuf;

use skit_application::SourcePermissions;
use skit_domain::StorageMode;
use skit_ui::{KnownEntryKind, ReviewDefaults, ReviewState, SourceSnapshot};

fn prompt_review(name: &str, body: &str, defaults: ReviewDefaults) -> ReviewState {
    ReviewState::from_source(
        SourceSnapshot {
            path: PathBuf::from(format!("/work/{name}")),
            source_record: format!("/work/{name}"),
            bytes: body.as_bytes().to_vec(),
            permissions: SourcePermissions::default(),
            is_regular: true,
            is_directory: false,
            is_draft: false,
        },
        KnownEntryKind::Prompt,
        defaults,
    )
}

#[test]
fn test_add_prompt_interactive_selection() {
    let mut review = prompt_review(
        "p.prompt.md",
        "{{a}} {{b}} {{c}}\n",
        ReviewDefaults::default(),
    );
    review.set_prompt_selected("a", true);
    review.set_prompt_selected("b", false);
    review.set_prompt_selected("c", true);
    review.set_runner("", false);
    let create = review.create_entry().expect("selected prompt review");
    assert_eq!(create.settings.params, ["a", "c"]);
    assert_eq!(create.settings.runner, "");
    assert!(!review.runner_was_picked(), "skipping the runner must not become a remembered pick");
}

#[test]
fn test_add_prompt_plain_identity_defaults_drop_compound_suffix() {
    let review = prompt_review(
        "review.prompt.md",
        "# Review pull requests\n",
        ReviewDefaults::default(),
    );
    assert_eq!(review.name(), "review", "compound .prompt.md must default to the logical prompt name");
    assert_eq!(review.description(), "Review pull requests");
    let create = review.create_entry().expect("default identity");
    assert_eq!(create.name, "review");
    assert_eq!(create.description, "Review pull requests");
}

#[test]
fn test_add_prompt_plain_identity_accepts_user_overrides() {
    let mut review = prompt_review(
        "review.prompt.md",
        "Review this change\n",
        ReviewDefaults::default(),
    );
    review.set_name("pr-review");
    review.set_description("Team review prompt");
    let create = review.create_entry().expect("identity override");
    assert_eq!(create.name, "pr-review");
    assert_eq!(create.description, "Team review prompt");
}

#[test]
fn test_add_prompt_interactive_tui_form_opens_review_panel() {
    let review = prompt_review(
        "p.prompt.md",
        "Do {{a}}\n",
        ReviewDefaults {
            runner: Some("claude".to_owned()),
            runner_names: vec!["claude".to_owned()],
            reference: true,
            interpolate: Some(false),
            ..ReviewDefaults::default()
        },
    );
    assert_eq!(review.runner(), "claude", "CLI --runner must arrive in the hosted panel");
    assert!(!review.interpolate(), "--no-interpolate must arrive in the hosted panel");
    assert_eq!(review.storage(), StorageMode::Reference, "--ref must arrive in the hosted panel");
    let create = review.create_entry().expect("hosted prompt review");
    assert_eq!(create.settings.runner, "claude");
    assert!(!create.settings.interpolate);
    assert_eq!(create.mode, StorageMode::Reference);
}

#[test]
fn test_add_interactive_off_answer_disables_insertion() {
    let mut review = prompt_review(
        "quiet.prompt.md",
        "{{a}} {{b}}\n",
        ReviewDefaults::default(),
    );
    review.set_interpolate(false);
    let create = review.create_entry().expect("interpolation-off prompt");
    assert!(!create.settings.interpolate);
    assert!(create.settings.params.is_empty(), "insertion-off prompt must not secretly manage placeholders");
    assert!(create.settings.parameters.is_empty());
}

#[test]
fn test_add_interactive_explicit_all_beats_the_flood_cap() {
    let body = (0..32)
        .map(|index| format!("{{{{h{index}}}}}"))
        .collect::<Vec<_>>()
        .join(" ");
    let mut review = prompt_review("all-in.prompt.md", &body, ReviewDefaults::default());
    assert!(review.prompt_is_flooded(), "fixture must exceed the auto-management limit");
    assert!(review.selected_prompt_names().is_empty(), "flooded review must start with nothing selected");
    let names = review
        .prompt_candidates()
        .iter()
        .map(|candidate| candidate.name.clone())
        .collect::<Vec<_>>();
    review.set_prompt_selection(&names);
    let create = review.create_entry().expect("explicit all selection");
    assert_eq!(create.settings.params.len(), names.len(), "explicit all must beat the flood default");
    assert_eq!(create.settings.params, names);
}
