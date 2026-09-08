//! Long and environment-driven owners for the four-profile review corpus.
//!
//! Both owners are ignored. The first one regenerates the canonical corpus twice. The second
//! one needs a path from the environment. Every line they use has a fast contract in
//! `tui_walker_bundle.rs`.

// Keep this ignored owner compiled on every host to check the complete corpus path.
#[test]
#[ignore = "generates the stable four-profile review corpus twice; re-measure the wall time"]
fn generates_and_reinstalls_the_stable_review_corpus() {
    use std::fs;

    use skit_tui_walker_support::TransitionCause;
    use skit_ui::{AddEffect, Effect};

    use super::tui_real_host::StableSandboxNamespace;
    use super::tui_real_walker::canonical_corpus_operations;
    use super::tui_walker_bundle::{
        checkout_root, generate_and_install_review_corpus, read_review_corpus, source_identity,
        staged_residue, validate_final_review_corpus,
    };

    let namespace = StableSandboxNamespace::system().unwrap();
    let parent = checkout_root()
        .unwrap()
        .join("target/ui-walker-review-corpus");
    fs::create_dir_all(&parent).unwrap();
    let mut fingerprint = source_identity;
    let (installed, pair) = generate_and_install_review_corpus(
        namespace,
        &parent,
        &canonical_corpus_operations(),
        &mut fingerprint,
    )
    .unwrap();

    let corpus = read_review_corpus(&installed).unwrap();
    validate_final_review_corpus(
        &corpus.operations_bytes,
        corpus.run.operation_count,
        &corpus.profile_coverage,
        &corpus.coverage,
    )
    .unwrap();
    for readback in &corpus.profiles {
        let profile = &readback.profile.id;
        let requests = readback
            .rows
            .iter()
            .filter_map(|row| match &row.cause {
                TransitionCause::Host { request, .. } => Some(request.clone()),
                TransitionCause::Initial
                | TransitionCause::Session { .. }
                | TransitionCause::Reducer { .. } => None,
            })
            .map(|request| serde_json::from_value::<Effect>(request).unwrap())
            .collect::<Vec<_>>();
        assert!(
            requests.iter().any(|request| matches!(
                request,
                Effect::SaveRunner { request, .. } if request.name == "new-agent"
            )),
            "{profile} did not save new-agent"
        );
        assert!(
            requests.iter().any(|request| matches!(
                request,
                Effect::Add(effects) if effects.iter().any(|effect| matches!(
                    effect,
                    AddEffect::RememberRunner(name) if name == "new-agent"
                ))
            )),
            "{profile} did not remember new-agent"
        );
    }
    assert_eq!(pair.main.profiles.len(), 4);
    assert!(staged_residue(&parent).is_empty());
    eprintln!("installed review corpus: {}", installed.display());
}

#[test]
#[ignore = "set SKIT_WALKER_REVIEW to validate one completed review corpus"]
fn validates_a_completed_review_corpus_from_env() {
    use std::{env, path::PathBuf};

    use super::tui_walker_bundle::validate_completed_review_corpus;

    let root = PathBuf::from(
        env::var_os("SKIT_WALKER_REVIEW").expect("SKIT_WALKER_REVIEW must name one review corpus"),
    );
    let revision = env::var("SKIT_WALKER_EXPECT_REVISION").ok();
    let corpus = validate_completed_review_corpus(&root, revision.as_deref(), true).unwrap();
    assert!(corpus.progress.complete);
}
