use std::{fs, sync::Arc, thread};

use skit_store::FileAgentSkillStore;
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
