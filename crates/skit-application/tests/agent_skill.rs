use std::{collections::BTreeSet, path::PathBuf};

use skit_application::{
    AgentInstallError, AgentInstallPlan, AgentInstallRequest, AgentRoots, AgentScope,
    detect_agent_targets, plan_agent_install,
};

fn roots() -> AgentRoots {
    AgentRoots {
        home: Some(PathBuf::from("/home/demo")),
        cwd: PathBuf::from("/work/project"),
    }
}

#[test]
fn named_targets_preserve_v040_scope_rules() {
    for (name, project, expected) in [
        ("claude", false, "/home/demo/.claude/skills"),
        ("codex", true, "/work/project/.codex/skills"),
        ("agents", false, "/work/project/.agents/skills"),
        ("agents", true, "/work/project/.agents/skills"),
    ] {
        let plan = plan_agent_install(
            &AgentInstallRequest {
                target: Some(name.to_owned()),
                directory: None,
                project,
                interactive: false,
            },
            &roots(),
            |_| false,
        )
        .unwrap();
        assert_eq!(
            plan,
            AgentInstallPlan::Ready {
                skills_dir: PathBuf::from(expected)
            }
        );
    }
}

#[test]
fn explicit_directory_is_consent_and_expands_the_current_user() {
    let plan = plan_agent_install(
        &AgentInstallRequest {
            target: None,
            directory: Some(PathBuf::from("~/agent-skills")),
            project: false,
            interactive: false,
        },
        &roots(),
        |_| false,
    )
    .unwrap();
    assert_eq!(
        plan,
        AgentInstallPlan::Ready {
            skills_dir: PathBuf::from("/home/demo/agent-skills")
        }
    );
}

#[test]
fn conflicting_and_unknown_explicit_selections_are_usage_errors() {
    let conflict = plan_agent_install(
        &AgentInstallRequest {
            target: Some("claude".to_owned()),
            directory: Some(PathBuf::from("/tmp/skills")),
            project: false,
            interactive: false,
        },
        &roots(),
        |_| false,
    )
    .unwrap_err();
    assert_eq!(conflict, AgentInstallError::ConflictingSelection);

    let unknown = plan_agent_install(
        &AgentInstallRequest {
            target: Some("future".to_owned()),
            directory: None,
            project: false,
            interactive: false,
        },
        &roots(),
        |_| false,
    )
    .unwrap_err();
    assert_eq!(
        unknown,
        AgentInstallError::UnknownTarget {
            name: "future".to_owned()
        }
    );
}

#[test]
fn bare_noninteractive_install_never_guesses_even_one_existing_target() {
    let existing = BTreeSet::from([PathBuf::from("/home/demo/.codex")]);
    let error = plan_agent_install(
        &AgentInstallRequest {
            target: None,
            directory: None,
            project: false,
            interactive: false,
        },
        &roots(),
        |path| existing.contains(path),
    )
    .unwrap_err();
    assert_eq!(error, AgentInstallError::ExplicitSelectionRequired);
}

#[test]
fn bare_interactive_install_returns_every_existing_target_in_stable_order() {
    let existing = BTreeSet::from([
        PathBuf::from("/home/demo/.claude"),
        PathBuf::from("/work/project/.codex"),
        PathBuf::from("/work/project/.agents"),
    ]);
    let plan = plan_agent_install(
        &AgentInstallRequest {
            target: None,
            directory: None,
            project: false,
            interactive: true,
        },
        &roots(),
        |path| existing.contains(path),
    )
    .unwrap();
    let AgentInstallPlan::Choose { candidates } = plan else {
        panic!("interactive detection must return a choice plan");
    };
    assert_eq!(
        candidates
            .iter()
            .map(|target| (target.name.as_str(), target.scope))
            .collect::<Vec<_>>(),
        [
            ("claude", AgentScope::User),
            ("codex", AgentScope::Project),
            ("agents", AgentScope::Project),
        ]
    );
}

#[test]
fn bare_interactive_install_reports_no_existing_targets() {
    let error = plan_agent_install(
        &AgentInstallRequest {
            target: None,
            directory: None,
            project: false,
            interactive: true,
        },
        &roots(),
        |_| false,
    )
    .unwrap_err();
    assert_eq!(error, AgentInstallError::NoTargetsDetected);
}

#[test]
fn a_frontend_can_present_an_empty_read_only_detection_result() {
    assert!(detect_agent_targets(&roots(), |_| false).is_empty());

    let existing = BTreeSet::from([
        PathBuf::from("/home/demo/.codex"),
        PathBuf::from("/work/project/.agents"),
    ]);
    let targets = detect_agent_targets(&roots(), |path| existing.contains(path));
    assert_eq!(
        targets
            .iter()
            .map(|target| (target.name.as_str(), target.scope))
            .collect::<Vec<_>>(),
        [("codex", AgentScope::User), ("agents", AgentScope::Project),]
    );
}
