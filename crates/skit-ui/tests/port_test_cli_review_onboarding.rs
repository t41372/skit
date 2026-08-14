use std::path::PathBuf;

use skit_application::SourcePermissions;
use skit_domain::parameters::ParameterValue;
use skit_form::onboarding_plan;
use skit_ui::{KnownEntryKind, ReviewDefaults, ReviewState, SourceSnapshot};

fn snapshot(name: &str, body: &str) -> SourceSnapshot {
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

fn python_review(name: &str, body: &str) -> ReviewState {
    ReviewState::from_source(
        snapshot(name, body),
        KnownEntryKind::Python,
        ReviewDefaults::default(),
    )
}

#[test]
fn test_resolve_metadata_interactive() {
    let mut review = python_review("job.py", "import requests\nprint(requests)\n");
    assert_eq!(review.dependencies_text(), "requests", "the interactive field starts from analyzer suggestions");
    review.set_dependencies_text("requests, rich");
    review.set_requires_python(">=3.12");
    let create = review.create_entry().expect("interactive metadata must validate into one atomic create request");
    assert_eq!(create.settings.dependencies, ["requests", "rich"]);
    assert_eq!(create.settings.requires_python.as_deref(), Some(">=3.12"));
}

#[test]
fn test_resolve_metadata_interactive_dash_clears_deps() {
    let mut review = python_review("job.py", "import requests\nprint(requests)\n");
    assert_eq!(review.dependencies_text(), "requests");
    review.set_dependencies_text("-");
    review.set_requires_python("");
    let create = review
        .create_entry()
        .expect("the frozen line-prompt '-' spelling means install nothing, not a literal package");
    assert!(create.settings.dependencies.is_empty());
    assert!(create.settings.requires_python.as_deref().unwrap_or_default().is_empty());
}

#[test]
fn test_resolve_metadata_interactive_none_word_clears_deps() {
    let mut review = python_review("job.py", "import requests\nprint(requests)\n");
    review.set_dependencies_text("None");
    let create = review
        .create_entry()
        .expect("the frozen interactive 'None' spelling means install nothing");
    assert!(create.settings.dependencies.is_empty());
}

#[test]
fn test_prompt_identity_prompts_name_and_description() {
    let mut review = python_review("image_stitch.py", "\"\"\"doc first line.\"\"\"\n");
    review.set_name("stitch");
    review.set_description("Stack images vertically");
    let create = review.create_entry().expect("identity review");
    assert_eq!(create.name, "stitch");
    assert_eq!(create.description, "Stack images vertically");
}

#[test]
fn test_prompt_identity_blank_name_falls_back_to_stem() {
    let review = python_review("worker.py", "print(1)\n");
    assert_eq!(
        review.name(),
        "worker",
        "the Rust review pre-fills the same stem that Python's blank answer delegated back to the store"
    );
    assert_eq!(review.description(), "");
    let create = review.create_entry().expect("default identity");
    assert_eq!(create.name, "worker");
}

#[test]
fn test_onboard_params_framework_detected() {
    let plan = onboarding_plan(
        "python",
        "import argparse\np = argparse.ArgumentParser()\n",
    );
    assert!(plan.uses_cli_framework(), "argparse must be recognized as the source's CLI interface");
    assert!(
        plan.offered_candidates().is_empty(),
        "framework-owned CLI fields must suppress const/input onboarding rather than creating a second interface"
    );
}

#[test]
fn test_onboard_params_interactive_selection() {
    let mut review = python_review(
        "x.py",
        "CITY = \"Taipei\"\nRETRIES = 3\nwho = input(\"Name: \")\nprint(CITY, RETRIES, who)\n",
    );
    assert!(review.candidates().len() >= 2, "the frozen source must expose at least its constant candidates");
    assert!(review.candidate("CITY").is_some(), "CITY must be offered before the user selects all");
    let names = review
        .candidates()
        .iter()
        .map(|candidate| candidate.declaration.name.clone())
        .collect::<Vec<_>>();
    for name in names {
        review.set_candidate_selected(&name, true);
    }
    let create = review.create_entry().expect("selected onboarding candidates must survive into the atomic request");
    assert!(
        create.settings.parameters.iter().any(|parameter| parameter.name == "CITY"),
        "CITY vanished after interactive all-selection"
    );
}

#[test]
fn test_paramspec_from_candidate_roundtrip() {
    let plan = onboarding_plan("python", "CITY = \"Taipei\"\nprint(CITY)\n");
    let candidate = plan.candidates.first().expect("CITY candidate");
    assert_eq!(candidate.declaration.name, "CITY");
    assert_eq!(
        candidate.declaration.default,
        Some(ParameterValue::String("Taipei".to_owned())),
        "the frontend-neutral declaration must preserve the analyzer candidate rather than re-invent it"
    );
}
