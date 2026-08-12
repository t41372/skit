//! Storage-side install contract from Python `tests/test_agent_install.py` at `main@206f9ef`.

use std::fs;

use skit_store::FileAgentSkillStore;
use tempfile::TempDir;

#[test]
fn test_install_into_writes_and_upgrades() {
    let root = TempDir::new().unwrap();
    let skills = root.path().join("skills");
    let shipped = include_bytes!("../../../skills/skit/SKILL.md");

    let installed = FileAgentSkillStore.install(&skills, shipped).unwrap();
    assert_eq!(installed, skills.join("skit/SKILL.md"));
    assert_eq!(fs::read(&installed).unwrap(), shipped);

    fs::write(&installed, b"stale").unwrap();
    let again = FileAgentSkillStore.install(&skills, shipped).unwrap();

    assert_eq!(again, installed);
    assert_eq!(fs::read(&installed).unwrap(), shipped);
    assert!(
        fs::read_dir(installed.parent().unwrap())
            .unwrap()
            .all(|item| !item.unwrap().file_name().to_string_lossy().contains(".tmp")),
        "upgrade leaked a temporary file into the installed skill directory"
    );
}
