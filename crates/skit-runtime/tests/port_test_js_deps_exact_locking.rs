//! Exact locking/lifecycle ports from Python v0.4 `tests/test_js_deps.py`.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
    time::Duration,
};

use skit_runtime::{
    DependencyCommand, DependencyCommandRunner, ProgramProbe, clear_javascript_dependencies,
    ensure_javascript_dependencies,
};
use tempfile::TempDir;

#[derive(Debug, Default)]
struct Probe;

impl ProgramProbe for Probe {
    fn find_program(&self, name: &str) -> Option<PathBuf> {
        (name == "npm").then(|| PathBuf::from("/bin/npm"))
    }
    fn is_file(&self, path: &Path) -> bool {
        path.is_file()
    }
    fn is_dir(&self, path: &Path) -> bool {
        path.is_dir()
    }
    fn is_executable(&self, _path: &Path) -> bool {
        true
    }
}

fn lock_path(entry_dir: &Path) -> PathBuf {
    let parent = entry_dir.parent().expect("fixture entry has a parent");
    let root = if parent.file_name().is_some_and(|name| name == "scripts") {
        parent.parent().unwrap_or(parent)
    } else {
        parent
    };
    root.join(".locks").join(format!(
        "{}.skit-deps.lock",
        entry_dir.file_name().unwrap().to_string_lossy()
    ))
}

fn entry(root: &TempDir) -> PathBuf {
    let entry = root.path().join("entry");
    fs::create_dir(&entry).unwrap();
    entry
}

#[derive(Debug)]
struct LockAssertingRunner {
    expected_lock: PathBuf,
}

impl DependencyCommandRunner for LockAssertingRunner {
    fn run(&self, command: &DependencyCommand) -> std::io::Result<bool> {
        assert!(
            self.expected_lock.is_file(),
            "installer ran without the persistent dependency lock: {}",
            self.expected_lock.display()
        );
        fs::create_dir_all(command.cwd.join("node_modules"))?;
        Ok(true)
    }
}

#[derive(Debug)]
struct BlockingRunner {
    entered: mpsc::Sender<()>,
    release: mpsc::Receiver<()>,
}

impl DependencyCommandRunner for BlockingRunner {
    fn run(&self, command: &DependencyCommand) -> std::io::Result<bool> {
        self.entered.send(()).unwrap();
        self.release.recv().unwrap();
        fs::create_dir_all(command.cwd.join("node_modules"))?;
        Ok(true)
    }
}

#[test]
fn test_install_lock_uses_a_persistent_inode_outside_the_entry() {
    let root = TempDir::new().unwrap();
    let entry = entry(&root);
    let lock = lock_path(&entry);

    clear_javascript_dependencies(&entry).unwrap();

    assert!(lock.is_file());
    assert!(!lock.starts_with(&entry));
    assert!(lock.is_file(), "lock inode must survive release");
    assert!(!entry.join(".skit-deps.lock").exists());
}

#[test]
fn test_install_lock_waits_for_a_live_holder() {
    let root = TempDir::new().unwrap();
    let entry = entry(&root);
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let held_entry = entry.clone();
    let holder = thread::spawn(move || {
        ensure_javascript_dependencies(
            &held_entry,
            "node",
            &["chalk".to_owned()],
            &Probe,
            &BlockingRunner {
                entered: entered_tx,
                release: release_rx,
            },
        )
        .unwrap();
    });
    entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();

    let (waiter_tx, waiter_rx) = mpsc::channel();
    let waited_entry = entry.clone();
    let waiter = thread::spawn(move || {
        clear_javascript_dependencies(&waited_entry).unwrap();
        waiter_tx.send(()).unwrap();
    });

    assert!(
        waiter_rx.recv_timeout(Duration::from_millis(100)).is_err(),
        "a second dependency operation entered while the first lock was live"
    );
    release_tx.send(()).unwrap();
    holder.join().unwrap();
    waiter.join().unwrap();
    waiter_rx.recv_timeout(Duration::from_secs(2)).unwrap();
}

#[test]
fn test_ensure_installed_serializes_under_the_entry_lock() {
    let root = TempDir::new().unwrap();
    let entry = entry(&root);
    let lock = lock_path(&entry);
    ensure_javascript_dependencies(
        &entry,
        "node",
        &["chalk".to_owned()],
        &Probe,
        &LockAssertingRunner {
            expected_lock: lock.clone(),
        },
    )
    .unwrap();
    assert!(lock.is_file());
}

#[test]
fn test_install_lock_path_survives_entry_directory_removal() {
    let root = TempDir::new().unwrap();
    let entry = entry(&root);
    let lock = lock_path(&entry);
    clear_javascript_dependencies(&entry).unwrap();
    fs::remove_dir(&entry).unwrap();
    assert!(lock.is_file());
}

#[test]
fn test_clear_takes_the_install_lock() {
    let root = TempDir::new().unwrap();
    let entry = entry(&root);
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let held_entry = entry.clone();
    let holder = thread::spawn(move || {
        ensure_javascript_dependencies(
            &held_entry,
            "node",
            &["chalk".to_owned()],
            &Probe,
            &BlockingRunner {
                entered: entered_tx,
                release: release_rx,
            },
        )
        .unwrap();
    });
    entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();

    let (clear_tx, clear_rx) = mpsc::channel();
    let clear_entry = entry.clone();
    let clearer = thread::spawn(move || {
        clear_javascript_dependencies(&clear_entry).unwrap();
        clear_tx.send(()).unwrap();
    });
    assert!(
        clear_rx.recv_timeout(Duration::from_millis(100)).is_err(),
        "clear bypassed the live installer lock"
    );
    release_tx.send(()).unwrap();
    holder.join().unwrap();
    clearer.join().unwrap();
    clear_rx.recv_timeout(Duration::from_secs(2)).unwrap();
}

#[test]
fn test_install_lock_never_unlinks_its_persistent_inode() {
    let root = TempDir::new().unwrap();
    let entry = entry(&root);
    let lock = lock_path(&entry);
    clear_javascript_dependencies(&entry).unwrap();
    fs::write(&lock, b"persistent sentinel").unwrap();
    clear_javascript_dependencies(&entry).unwrap();
    assert_eq!(fs::read(&lock).unwrap(), b"persistent sentinel");
}

#[test]
fn test_install_lock_reuses_the_same_persistent_inode() {
    let root = TempDir::new().unwrap();
    let entry = entry(&root);
    let lock = lock_path(&entry);
    let alias = root.path().join("lock-hardlink");
    clear_javascript_dependencies(&entry).unwrap();
    fs::hard_link(&lock, &alias).unwrap();

    clear_javascript_dependencies(&entry).unwrap();
    fs::write(&alias, b"same inode").unwrap();
    assert_eq!(
        fs::read(&lock).unwrap(),
        b"same inode",
        "dependency lock path was replaced instead of reusing the persistent inode"
    );
}

#[test]
fn test_clean_sweeps_aged_injected_leftovers() {
    let root = TempDir::new().unwrap();
    let entry = entry(&root);
    let stranded = entry.join(".injected-crash.js");
    fs::write(&stranded, "secret").unwrap();

    clear_javascript_dependencies(&entry).unwrap();

    assert!(
        !stranded.exists(),
        "dependency cleanup left a secret-bearing injected copy behind"
    );
}
