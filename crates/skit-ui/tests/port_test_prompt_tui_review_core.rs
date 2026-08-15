use std::path::PathBuf;

use skit_application::SourcePermissions;
use skit_domain::StorageMode;
use skit_ui::{KnownEntryKind, ReviewDefaults, ReviewState, SourceSnapshot};

fn source(name: &str, body: &str) -> SourceSnapshot {
    SourceSnapshot {
        path: PathBuf::from(format!("/work/{name}")),
        source_record: format!("/work/{name}"),
        bytes: body.as_bytes().to_vec(),
        permissions: SourcePermissions::default(),
        is_regular: true,
        is_directory: false,
        is_draft: false,
    }
}

fn review(name: &str, body: &str, defaults: ReviewDefaults) -> ReviewState {
    ReviewState::from_source(source(name, body), KnownEntryKind::Prompt, defaults)
}

#[test]
fn test_review_insertion_switch_off_hides_ticks_and_stores_off() {
    let mut review = review("raw.prompt.md", "Use {{tool}} literally\n", ReviewDefaults::default());
    assert_eq!(review.selected_prompt_names(), ["tool"]);
    review.set_interpolate(false);
    let create = review.create_entry().expect("insertion-off prompt review");
    assert!(!create.settings.interpolate);
    assert!(create.settings.params.is_empty(), "insertion-off prompt retained managed placeholders");
    assert!(create.settings.parameters.is_empty(), "insertion-off prompt retained declared placeholder rows");
}

#[test]
fn test_review_runner_pick_pins_and_remembers() {
    let mut review = review(
        "r.prompt.md",
        "Go {{a}}\n",
        ReviewDefaults {
            runner_names: vec!["claude".to_owned(), "codex".to_owned()],
            ..ReviewDefaults::default()
        },
    );
    assert_eq!(review.runner(), "");
    assert!(!review.runner_was_picked());
    review.set_runner("claude", true);
    assert_eq!(review.runner(), "claude");
    assert!(review.runner_was_picked(), "an active review pick was not marked for RememberRunner");
    let create = review.create_entry().expect("picked prompt runner");
    assert_eq!(create.settings.runner, "claude");
}

#[test]
fn test_review_prefills_last_picked_and_explicit_runner_wins() {
    let last = review(
        "l.prompt.md",
        "x {{a}}\n",
        ReviewDefaults {
            runner_names: vec!["amp".to_owned(), "codex".to_owned()],
            last_runner: Some("amp".to_owned()),
            ..ReviewDefaults::default()
        },
    );
    assert_eq!(last.runner(), "amp");
    assert!(!last.runner_was_picked(), "a last-picked prefill is not a new pick");

    let explicit = review(
        "l.prompt.md",
        "x {{a}}\n",
        ReviewDefaults {
            runner: Some("codex".to_owned()),
            runner_names: vec!["amp".to_owned(), "codex".to_owned()],
            last_runner: Some("amp".to_owned()),
            interpolate: Some(false),
            ..ReviewDefaults::default()
        },
    );
    assert_eq!(explicit.runner(), "codex", "explicit add-time runner did not beat last-picked state");
    assert!(!explicit.runner_was_picked(), "an untouched explicit add-time pin must not overwrite picker history");
    let create = explicit.create_entry().expect("explicit runner review");
    assert_eq!(create.settings.runner, "codex");
    assert!(!create.settings.interpolate);
}

#[test]
fn test_review_reference_mode_links_the_original() {
    let mut review = review("linked.prompt.md", "{{a}}\n", ReviewDefaults::default());
    review.set_storage(StorageMode::Reference);
    let create = review.create_entry().expect("reference prompt review");
    assert_eq!(create.mode, StorageMode::Reference);
    assert_eq!(create.source, "/work/linked.prompt.md");
    assert!(create.payload.is_none(), "reference review incorrectly materialized an owned prompt body");
    assert_eq!(create.workdir, "invoke");
}

#[test]
fn test_review_description_prefill_and_toggle_action() {
    let mut review = review(
        "d.prompt.md",
        "{{a}}\n",
        ReviewDefaults {
            description: Some("hand-written".to_owned()),
            ..ReviewDefaults::default()
        },
    );
    assert_eq!(review.description(), "hand-written");
    assert_eq!(review.selected_prompt_names(), ["a"]);
    review.set_prompt_selected("a", false);
    assert!(review.selected_prompt_names().is_empty());
    let create = review.create_entry().expect("toggled prompt review");
    assert_eq!(create.description, "hand-written");
    assert!(create.settings.params.is_empty());
}

#[test]
fn test_review_ctrl_e_keeps_placeholder_ticks_by_name_across_flood_transitions() {
    const FROZEN_AUTO_MANAGE_LIMIT: usize = 30;
    let mut review = review("ticks.prompt.md", "{{keep_off}} {{removed}}\n", ReviewDefaults::default());
    review.set_prompt_selected("keep_off", false);

    let flood_names = std::iter::once("flood_on".to_owned())
        .chain(std::iter::once("keep_off".to_owned()))
        .chain((0..FROZEN_AUTO_MANAGE_LIMIT - 1).map(|index| format!("new_{index}")))
        .collect::<Vec<_>>();
    assert_eq!(flood_names.len(), FROZEN_AUTO_MANAGE_LIMIT + 1);
    let flooded = flood_names
        .iter()
        .map(|name| format!("{{{{{name}}}}}"))
        .collect::<Vec<_>>()
        .join(" ");
    review.rescan((flooded + "\n").into_bytes());

    assert!(review.prompt_is_flooded());
    assert!(!review.prompt_candidates().iter().any(|candidate| candidate.name == "removed"));
    assert!(!review.prompt_candidates().iter().find(|candidate| candidate.name == "keep_off").unwrap().selected);
    assert!(!review.prompt_candidates().iter().find(|candidate| candidate.name == "flood_on").unwrap().selected,
        "a genuinely new placeholder crossing into flood must default off");
    review.set_prompt_selected("flood_on", true);

    review.rescan(b"{{fresh_below}} {{flood_on}} {{keep_off}}\n".to_vec());
    assert!(!review.prompt_is_flooded());
    assert_eq!(
        review.prompt_candidates().iter().map(|candidate| candidate.name.as_str()).collect::<Vec<_>>(),
        ["fresh_below", "flood_on", "keep_off"]
    );
    assert!(review.prompt_candidates()[0].selected, "new below-cap placeholder did not default on");
    assert!(review.prompt_candidates()[1].selected, "explicit flood selection did not follow its name across rescan");
    assert!(!review.prompt_candidates()[2].selected, "explicit off decision did not follow reordered placeholder name");
}
