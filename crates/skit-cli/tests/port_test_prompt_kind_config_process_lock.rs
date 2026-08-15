use std::{
    env,
    fs::{self, OpenOptions},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use skit_store::{FileConfigStore, PromptRunner};
use tempfile::TempDir;

#[test]
fn rust_additive_config_lock_child() {
    let Some(root) = env::var_os("SKIT_CONFIG_LOCK_CHILD_ROOT") else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    fs::write(root.join("attempted"), b"yes").unwrap();
    FileConfigStore::new(&root)
        .set_runner(
            PromptRunner {
                name: "child".to_owned(),
                argv: vec!["child".to_owned(), "{{prompt}}".to_owned()],
            },
            false,
        )
        .unwrap();
    fs::write(root.join("acquired"), b"yes").unwrap();
}

#[test]
fn test_config_lock_serializes_a_real_subprocess() {
    let config = TempDir::new().unwrap();
    let lock_path = config.path().join("config.lock");
    fs::create_dir_all(config.path()).unwrap();
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .unwrap();
    if lock.metadata().unwrap().len() == 0 {
        lock.set_len(1).unwrap();
    }
    lock.lock().unwrap();

    // Spawn this integration-test executable itself so the child can call the *real*
    // FileConfigStore::set_runner adapter. The marker is written immediately before that call,
    // exactly like frozen Python writes `attempted` immediately before entering _config_lock().
    let mut child = Command::new(env::current_exe().unwrap())
        .args(["--exact", "rust_additive_config_lock_child", "--nocapture"])
        .env("SKIT_CONFIG_LOCK_CHILD_ROOT", config.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let attempted = config.path().join("attempted");
    let acquired = config.path().join("acquired");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !attempted.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(attempted.exists(), "subprocess never reached FileConfigStore::set_runner");

    // Give an implementation that ignores config.lock enough time to complete; because the child
    // marker is already present, this interval no longer conflates process startup with blocking.
    thread::sleep(Duration::from_millis(100));
    assert!(!acquired.exists(), "subprocess completed the config transaction while config.lock was held");
    assert!(child.try_wait().unwrap().is_none(), "subprocess exited instead of waiting for config.lock");

    lock.unlock().unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    assert_eq!(fs::read_to_string(&acquired).unwrap(), "yes");
    let runners = FileConfigStore::new(config.path()).runners().unwrap();
    assert!(runners.iter().any(|runner| runner.name == "child" && runner.argv == ["child", "{{prompt}}"]));
}
