use std::{cell::RefCell, fs, io, path::Path, sync::Arc, thread};

use skit_store::{AgentSkillInstallPoint, FileAgentSkillStore};
use tempfile::TempDir;

#[test]
fn installation_writes_and_atomically_upgrades_the_named_skill() {
    let root = TempDir::new().unwrap();
    let skills = root.path().join("skills");
    let store = FileAgentSkillStore;

    let installed = store.install(&skills, b"first\n").unwrap();
    assert_eq!(installed, skills.join("skit/SKILL.md"));
    assert_eq!(fs::read(&installed).unwrap(), b"first\n");

    store.install(&skills, b"second\n").unwrap();
    assert_eq!(fs::read(&installed).unwrap(), b"second\n");
    assert!(
        fs::read_dir(installed.parent().unwrap())
            .unwrap()
            .all(|item| !item.unwrap().file_name().to_string_lossy().contains(".tmp"))
    );
}

#[cfg(unix)]
#[test]
fn an_upgrade_preserves_existing_permissions_and_follows_a_skill_file_symlink() {
    use std::os::unix::fs::PermissionsExt as _;

    let root = TempDir::new().unwrap();
    let skills = root.path().join("skills");
    let skill_dir = skills.join("skit");
    fs::create_dir_all(&skill_dir).unwrap();
    let target = root.path().join("shared-skill.md");
    fs::write(&target, b"old").unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o640)).unwrap();
    std::os::unix::fs::symlink(&target, skill_dir.join("SKILL.md")).unwrap();

    FileAgentSkillStore.install(&skills, b"new").unwrap();

    assert_eq!(fs::read(&target).unwrap(), b"new");
    assert!(skill_dir.join("SKILL.md").is_symlink());
    assert_eq!(
        fs::metadata(&target).unwrap().permissions().mode() & 0o777,
        0o640
    );
}

#[cfg(unix)]
#[test]
fn relative_skill_links_are_followed_and_cycles_are_refused() {
    let root = TempDir::new().unwrap();
    let skills = root.path().join("skills");
    let skill_dir = skills.join("skit");
    fs::create_dir_all(&skill_dir).unwrap();
    let shared = skill_dir.join("shared.md");
    fs::write(&shared, b"old").unwrap();
    std::os::unix::fs::symlink("shared.md", skill_dir.join("SKILL.md")).unwrap();

    FileAgentSkillStore.install(&skills, b"new").unwrap();
    assert_eq!(fs::read(&shared).unwrap(), b"new");

    fs::remove_file(skill_dir.join("SKILL.md")).unwrap();
    std::os::unix::fs::symlink("cycle.md", skill_dir.join("SKILL.md")).unwrap();
    std::os::unix::fs::symlink("SKILL.md", skill_dir.join("cycle.md")).unwrap();
    let error = FileAgentSkillStore
        .install(&skills, b"blocked")
        .unwrap_err();
    assert!(error.to_string().contains("symbolic link cycle"));
}

#[test]
fn a_blocking_file_refuses_without_changing_it() {
    let root = TempDir::new().unwrap();
    let blocker = root.path().join("skills");
    fs::write(&blocker, b"owned by user").unwrap();

    assert!(FileAgentSkillStore.install(&blocker, b"skill").is_err());
    assert_eq!(fs::read(&blocker).unwrap(), b"owned by user");
}

#[test]
fn a_late_raw_replace_fault_keeps_old_bytes_and_removes_owned_staging() {
    for had_existing_target in [false, true] {
        let root = TempDir::new().unwrap();
        let skills = root.path().join("skills");
        let target = skills.join("skit/SKILL.md");
        if had_existing_target {
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::write(&target, b"old skill bytes\n").unwrap();
        }
        let calls = RefCell::new(Vec::new());

        let error = FileAgentSkillStore
            .install_with_checkpoint(&skills, b"new skill bytes\n", |point, path| {
                calls.borrow_mut().push((point, path.to_path_buf()));
                if point == AgentSkillInstallPoint::BeforeReplace {
                    Err(io::Error::other("raw replace fault"))
                } else {
                    Ok(())
                }
            })
            .unwrap_err();

        assert!(matches!(
            error,
            skit_application::RepositoryError::Io {
                operation: "replace",
                ref path,
                ref reason,
            } if Path::new(path) == target && reason == "raw replace fault"
        ));
        let mut expected = vec![(AgentSkillInstallPoint::Inspect, target.clone())];
        if !had_existing_target {
            expected.extend([
                (
                    AgentSkillInstallPoint::BeforeCreateDirectory,
                    skills.clone(),
                ),
                (
                    AgentSkillInstallPoint::BeforeCreateDirectory,
                    skills.join("skit"),
                ),
            ]);
        }
        expected.push((AgentSkillInstallPoint::BeforeReplace, target.clone()));
        assert_eq!(calls.into_inner(), expected);
        if had_existing_target {
            assert_eq!(fs::read(&target).unwrap(), b"old skill bytes\n");
            assert!(
                fs::read_dir(target.parent().unwrap())
                    .unwrap()
                    .all(|item| !item.unwrap().file_name().to_string_lossy().contains(".tmp"))
            );
        } else {
            assert!(!skills.exists());
        }
    }
}

#[test]
fn a_concurrently_created_directory_is_not_cleanup_owned() {
    let root = TempDir::new().unwrap();
    let skills = root.path().join("skills");

    FileAgentSkillStore
        .install_with_checkpoint(&skills, b"new skill bytes\n", |point, path| {
            if point == AgentSkillInstallPoint::BeforeCreateDirectory && path == skills {
                fs::create_dir(&skills).unwrap();
            }
            if point == AgentSkillInstallPoint::BeforeReplace {
                Err(io::Error::other("raw replace fault"))
            } else {
                Ok(())
            }
        })
        .unwrap_err();

    assert!(skills.is_dir());
    assert_eq!(fs::read_dir(&skills).unwrap().count(), 0);
}

#[test]
fn installation_accepts_an_explicit_relative_skills_directory() {
    let root = tempfile::Builder::new()
        .prefix(".skit-agent-relative-")
        .tempdir_in(".")
        .unwrap();
    let cwd = std::env::current_dir().unwrap();
    let skills = root.path().strip_prefix(&cwd).unwrap().join("skills");
    assert!(skills.is_relative());

    let installed = FileAgentSkillStore
        .install(&skills, b"relative skill\n")
        .unwrap();

    assert_eq!(installed, skills.join("skit/SKILL.md"));
    assert_eq!(fs::read(installed).unwrap(), b"relative skill\n");
}

#[cfg(unix)]
#[test]
fn installation_follows_a_parent_directory_symlink() {
    let root = TempDir::new().unwrap();
    let target = root.path().join("shared-agent");
    fs::create_dir(&target).unwrap();
    let linked_base = root.path().join("linked-agent");
    std::os::unix::fs::symlink(&target, &linked_base).unwrap();
    let skills = linked_base.join("skills");

    let installed = FileAgentSkillStore
        .install(&skills, b"linked parent\n")
        .unwrap();

    assert_eq!(installed, skills.join("skit/SKILL.md"));
    assert!(fs::symlink_metadata(&linked_base).unwrap().is_symlink());
    assert_eq!(
        fs::read(target.join("skills/skit/SKILL.md")).unwrap(),
        b"linked parent\n"
    );
}

#[test]
fn late_fault_cleanup_does_not_remove_a_replacement_directory() {
    let root = TempDir::new().unwrap();
    let skills = root.path().join("skills");
    let skill_dir = skills.join("skit");

    FileAgentSkillStore
        .install_with_checkpoint(&skills, b"new skill bytes\n", |point, path| {
            if point == AgentSkillInstallPoint::BeforeReplace {
                for item in fs::read_dir(&skill_dir).unwrap() {
                    fs::remove_file(item.unwrap().path()).unwrap();
                }
                fs::remove_dir(&skill_dir).unwrap();
                fs::create_dir(&skill_dir).unwrap();
                return Err(io::Error::other("raw replace fault"));
            }
            assert_ne!(path, Path::new(""));
            Ok(())
        })
        .unwrap_err();

    assert!(skill_dir.is_dir());
    assert_eq!(fs::read_dir(&skill_dir).unwrap().count(), 0);
    assert!(skills.is_dir());
}

#[test]
fn late_fault_cleanup_owns_each_created_directory_across_parent_components() {
    let root = TempDir::new().unwrap();
    let missing = root.path().join("missing");
    let skills = missing.join("../skills");

    FileAgentSkillStore
        .install_with_checkpoint(&skills, b"new skill bytes\n", |point, _| {
            if point == AgentSkillInstallPoint::BeforeReplace {
                Err(io::Error::other("raw replace fault"))
            } else {
                Ok(())
            }
        })
        .unwrap_err();

    assert!(!missing.exists());
    assert!(!root.path().join("skills").exists());
}

#[cfg(unix)]
#[test]
fn a_late_fault_preserves_the_final_symlink_target_bytes_and_mode() {
    use std::os::unix::fs::PermissionsExt as _;

    let root = TempDir::new().unwrap();
    let skills = root.path().join("skills");
    let destination = skills.join("skit/SKILL.md");
    fs::create_dir_all(destination.parent().unwrap()).unwrap();
    let target = root.path().join("shared.md");
    fs::write(&target, b"old linked bytes\n").unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o640)).unwrap();
    std::os::unix::fs::symlink(&target, &destination).unwrap();
    let link_target = fs::read_link(&destination).unwrap();

    FileAgentSkillStore
        .install_with_checkpoint(&skills, b"new linked bytes\n", |point, _| {
            if point == AgentSkillInstallPoint::BeforeReplace {
                Err(io::Error::other("raw replace fault"))
            } else {
                Ok(())
            }
        })
        .unwrap_err();

    assert!(fs::symlink_metadata(&destination).unwrap().is_symlink());
    assert_eq!(fs::read_link(&destination).unwrap(), link_target);
    assert_eq!(fs::read(&target).unwrap(), b"old linked bytes\n");
    assert_eq!(
        fs::metadata(&target).unwrap().permissions().mode() & 0o777,
        0o640
    );
    assert!(
        fs::read_dir(target.parent().unwrap())
            .unwrap()
            .all(|item| !item.unwrap().file_name().to_string_lossy().contains(".tmp"))
    );
}

#[test]
fn concurrent_upgrades_never_leave_partial_skill_bytes() {
    let root = TempDir::new().unwrap();
    let skills = Arc::new(root.path().join("skills"));
    let first = vec![b'a'; 128 * 1024];
    let second = vec![b'b'; 128 * 1024];
    let workers = [first.clone(), second.clone()]
        .into_iter()
        .map(|bytes| {
            let skills = Arc::clone(&skills);
            thread::spawn(move || FileAgentSkillStore.install(&skills, &bytes))
        })
        .collect::<Vec<_>>();
    for worker in workers {
        worker.join().unwrap().unwrap();
    }

    let installed = fs::read(skills.join("skit/SKILL.md")).unwrap();
    assert!(installed == first || installed == second);
}

#[test]
fn a_blocking_ancestor_file_refuses_before_the_named_directory_is_created() {
    let root = TempDir::new().unwrap();
    let skills = root.path().join("skills");
    let blocked = skills.join("skit");

    let error = FileAgentSkillStore
        .install_with_checkpoint(&skills, b"new skill bytes\n", |point, path| {
            if point == AgentSkillInstallPoint::BeforeCreateDirectory && path == skills {
                fs::create_dir(&skills).unwrap();
                fs::write(skills.join("skit"), b"owned by user").unwrap();
            }
            Ok(())
        })
        .unwrap_err();

    assert!(matches!(
        error,
        skit_application::RepositoryError::Io {
            operation: "create",
            ref path,
            ref reason,
        } if path == &blocked.display().to_string()
            && reason.contains("the path is not a directory")
    ));
    assert_eq!(fs::read(&blocked).unwrap(), b"owned by user");
}

#[cfg(unix)]
#[test]
fn an_unreadable_ancestor_refuses_with_the_inspect_operation() {
    use std::os::unix::fs::PermissionsExt as _;

    let root = TempDir::new().unwrap();
    let skills = root.path().join("skills");

    let outcome = FileAgentSkillStore.install_with_checkpoint(
        &skills,
        b"new skill bytes\n",
        |point, path| {
            if point == AgentSkillInstallPoint::BeforeCreateDirectory && path == skills {
                fs::create_dir_all(skills.join("skit")).unwrap();
                fs::set_permissions(&skills, fs::Permissions::from_mode(0o000)).unwrap();
            }
            Ok(())
        },
    );

    fs::set_permissions(&skills, fs::Permissions::from_mode(0o755)).unwrap();
    let error = outcome.unwrap_err();
    assert!(matches!(
        error,
        skit_application::RepositoryError::Io {
            operation: "inspect",
            ref path,
            ..
        } if path == &skills.join("skit").display().to_string()
    ));
}

#[test]
fn a_create_checkpoint_fault_stops_the_rollback_at_a_removed_owned_directory() {
    let root = TempDir::new().unwrap();
    let base = root.path().join("agent");
    let skills = base.join("skills");

    let error = FileAgentSkillStore
        .install_with_checkpoint(&skills, b"new skill bytes\n", |point, path| {
            if point == AgentSkillInstallPoint::BeforeCreateDirectory && path == skills {
                fs::remove_dir(&base).unwrap();
                return Err(io::Error::other("injected create fault"));
            }
            Ok(())
        })
        .unwrap_err();

    assert!(matches!(
        error,
        skit_application::RepositoryError::Io {
            operation: "create",
            ref path,
            ref reason,
        } if path == &skills.display().to_string() && reason == "injected create fault"
    ));
    assert!(!base.exists());
}

#[test]
fn a_racing_file_at_the_named_directory_refuses_and_keeps_its_bytes() {
    let root = TempDir::new().unwrap();
    let skills = root.path().join("skills");
    fs::create_dir(&skills).unwrap();
    let blocked = skills.join("skit");

    let error = FileAgentSkillStore
        .install_with_checkpoint(&skills, b"new skill bytes\n", |point, path| {
            if point == AgentSkillInstallPoint::BeforeCreateDirectory && path == blocked {
                fs::write(&blocked, b"owned by user").unwrap();
            }
            Ok(())
        })
        .unwrap_err();

    assert!(matches!(
        error,
        skit_application::RepositoryError::Io {
            operation: "create",
            ref path,
            ..
        } if path == &blocked.display().to_string()
    ));
    assert_eq!(fs::read(&blocked).unwrap(), b"owned by user");
}

#[cfg(unix)]
#[test]
fn a_read_only_skills_directory_refuses_to_create_the_named_directory() {
    use std::os::unix::fs::PermissionsExt as _;

    let root = TempDir::new().unwrap();
    let skills = root.path().join("skills");
    fs::create_dir(&skills).unwrap();
    fs::set_permissions(&skills, fs::Permissions::from_mode(0o500)).unwrap();

    let outcome = FileAgentSkillStore.install(&skills, b"new skill bytes\n");

    fs::set_permissions(&skills, fs::Permissions::from_mode(0o755)).unwrap();
    let error = outcome.unwrap_err();
    assert!(matches!(
        error,
        skit_application::RepositoryError::Io {
            operation: "create",
            ref path,
            ..
        } if path == &skills.join("skit").display().to_string()
    ));
    assert!(!skills.join("skit").exists());
}

/// The environment name that marks the child process of the directory-identity contract.
#[cfg(unix)]
const IDENTITY_CHILD: &str = "SKIT_TEST_AGENT_SKILL_IDENTITY_CHILD";

/// The identity of a new directory needs one file descriptor. A process that holds every
/// descriptor makes that step fail after the directory is already on disk. The descriptor limit is
/// a process property, so the contract runs in one child process with a small limit.
#[cfg(unix)]
#[test]
fn a_new_directory_without_an_identity_is_removed_and_the_install_refuses() {
    if std::env::var_os(IDENTITY_CHILD).is_some() {
        identity_failure_contract();
        return;
    }

    let program = std::env::current_exe().unwrap();
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg("ulimit -n 128 && exec \"$0\" --exact --nocapture --test-threads=1 \"$1\"")
        .arg(&program)
        .arg("a_new_directory_without_an_identity_is_removed_and_the_install_refuses")
        .env(IDENTITY_CHILD, "1")
        .status()
        .unwrap();

    assert!(status.success(), "the child contract must pass");
}

#[cfg(unix)]
fn identity_failure_contract() {
    let root = TempDir::new().unwrap();
    let skills = root.path().join("skills");
    fs::create_dir(&skills).unwrap();
    let source = skills.join("open-me.txt");
    fs::write(&source, b"open me\n").unwrap();

    let mut held = Vec::new();
    while let Ok(file) = fs::File::open(&source) {
        held.push(file);
    }

    let error = FileAgentSkillStore
        .install(&skills, b"new skill bytes\n")
        .unwrap_err();
    drop(held);

    assert!(matches!(
        error,
        skit_application::RepositoryError::Io {
            operation: "inspect",
            ref path,
            ..
        } if path == &skills.join("skit").display().to_string()
    ));
    assert!(!skills.join("skit").exists());
}
