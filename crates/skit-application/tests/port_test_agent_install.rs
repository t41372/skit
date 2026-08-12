//! Headless target-planning ports from Python `tests/test_agent_install.py` at `main@206f9ef`.
//!
//! The Rust use case exposes one public planner rather than Python's separate `named_target` helper.
//! These tests drive that public planner with synthetic roots and a deterministic directory probe;
//! no filesystem or CLI behavior is substituted for the target-selection contracts.

use std::path::{Path, PathBuf};

use skit_application::{
    AgentInstallError, AgentInstallPlan, AgentInstallRequest, AgentRoots, AgentScope,
    detect_agent_targets, plan_agent_install,
};

fn roots() -> AgentRoots {
    AgentRoots {
        home: Some(PathBuf::from("/home/tester")),
        cwd: PathBuf::from("/work/project"),
    }
}

fn ready_path(plan: AgentInstallPlan) -> PathBuf {
    let AgentInstallPlan::Ready { skills_dir } = plan else {
        panic!("explicit selection must be ready without a picker");
    };
    skills_dir
}

#[test]
fn test_detect_targets_reports_only_existing_marker_dirs() {
    let roots = roots();
    let existing = [
        roots.home.as_ref().unwrap().join(".claude"),
        roots.cwd.join(".agents"),
    ];

    let found = detect_agent_targets(&roots, |path| existing.iter().any(|item| item == path));

    assert_eq!(found.len(), 2);
    assert_eq!(found[0].name, "claude");
    assert_eq!(found[0].scope, AgentScope::User);
    assert_eq!(
        found[0].skills_dir(),
        roots.home.as_ref().unwrap().join(".claude/skills")
    );
    assert_eq!(found[1].name, "agents");
    assert_eq!(found[1].scope, AgentScope::Project);
    assert_eq!(found[1].skills_dir(), roots.cwd.join(".agents/skills"));
}

#[test]
fn test_detect_targets_empty_when_nothing_exists() {
    assert!(detect_agent_targets(&roots(), |_| false).is_empty());
}

#[test]
fn test_named_target_user_and_project_scopes() {
    let roots = roots();
    let user = plan_agent_install(
        &AgentInstallRequest {
            target: Some("claude".to_owned()),
            directory: None,
            project: false,
            interactive: false,
        },
        &roots,
        |_| false,
    )
    .unwrap();
    let project = plan_agent_install(
        &AgentInstallRequest {
            target: Some("codex".to_owned()),
            directory: None,
            project: true,
            interactive: false,
        },
        &roots,
        |_| false,
    )
    .unwrap();

    assert_eq!(
        ready_path(user),
        roots.home.as_ref().unwrap().join(".claude/skills")
    );
    assert_eq!(ready_path(project), roots.cwd.join(".codex/skills"));
}

#[test]
fn test_named_target_agents_is_always_project_scoped() {
    let roots = roots();
    for project in [false, true] {
        let plan = plan_agent_install(
            &AgentInstallRequest {
                target: Some("agents".to_owned()),
                directory: None,
                project,
                interactive: false,
            },
            &roots,
            |_| false,
        )
        .unwrap();
        assert_eq!(ready_path(plan), roots.cwd.join(".agents/skills"));
    }
}

#[test]
fn test_named_target_unknown_is_none() {
    let roots = roots();
    for project in [false, true] {
        let error = plan_agent_install(
            &AgentInstallRequest {
                target: Some("cursor".to_owned()),
                directory: None,
                project,
                interactive: false,
            },
            &roots,
            |_: &Path| false,
        )
        .unwrap_err();
        assert_eq!(
            error,
            AgentInstallError::UnknownTarget {
                name: "cursor".to_owned()
            }
        );
    }
}
