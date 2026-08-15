use skit_application::{library_detail::LibrarySurface, SourcePermissions};
use skit_domain::Slug;
use skit_i18n::{Localize, Locale, Message};
use skit_tui::run_add_workflow;
use skit_ui::{
    Action, AddAction, AddEffect, AddWorkflowState, Effect, KnownEntryKind, ReviewDefaults,
    ReviewState, SourceSnapshot,
};

#[derive(Debug)]
struct HostError;
impl Localize for HostError {
    fn message(&self) -> Message {
        Message::new("unexpected Prompt-TUI wrapper host failure")
    }
}

#[test]
fn test_run_prompt_review_returns_the_apps_result() {
    let review = ReviewState::from_source(
        SourceSnapshot {
            path: "h.prompt.md".into(),
            source_record: "h.prompt.md".to_owned(),
            bytes: b"x\n".to_vec(),
            permissions: SourcePermissions::default(),
            is_regular: true,
            is_directory: false,
            is_draft: false,
        },
        KnownEntryKind::Prompt,
        ReviewDefaults {
            name: Some("n".to_owned()),
            ..ReviewDefaults::default()
        },
    );
    let expected = Slug::parse("slug-sentinel").unwrap();
    let returned = expected.clone();
    let mut host_calls = 0_usize;

    let result = run_add_workflow(
        AddWorkflowState::from_review(review),
        vec![Action::Add(AddAction::Save)],
        |effect| -> Result<Action, HostError> {
            host_calls += 1;
            let Effect::Add(effects) = effect else {
                panic!("Prompt review wrapper requested unrelated host work: {effect:?}")
            };
            let [AddEffect::Commit { entry, .. }] = effects.as_slice() else {
                panic!("Prompt review wrapper did not submit exactly one atomic add: {effects:?}")
            };
            assert_eq!(entry.name, "n");
            assert_eq!(entry.kind.as_str(), "prompt");
            Ok(Action::AddCompleted {
                surface: LibrarySurface::default(),
                rerunnable: Vec::new(),
                slug: returned.clone(),
                message: "Added".to_owned(),
            })
        },
        Locale::En,
    )
    .expect("Prompt review wrapper failed before returning its app result");

    assert_eq!(result, Some(expected), "the wrapper discarded the add app's typed result");
    assert_eq!(host_calls, 1, "the wrapper performed extra host cycles after completion");
}
