//! The line that `skit --version` must print.
//!
//! The owner changed this contract on 2026-11-09. Version 0.4 printed `skit <version>`. The line
//! now also names the commit that the binary was built from: `skit 0.5.0 (1a2b3c4d5)`. The commit
//! is the first nine hex characters of the full commit id.
//!
//! The tests ask `git` for the commit. They do not read the value that the build script found, so a
//! defect in the build script cannot hide behind its own answer.

use std::{
    path::{Path, PathBuf},
    process::Command,
};

/// The line that `skit --version` prints for `version`, with its newline.
///
/// In a Git checkout whose top directory is this workspace, the line is
/// `skit <version> (<commit>)`. In other source trees, for example a source archive from PyPI,
/// the line is `skit <version>`.
pub(crate) fn version_line(version: &str) -> String {
    match checkout_commit() {
        Some(commit) => format!("skit {version} ({})\n", &commit[..9]),
        None => format!("skit {version}\n"),
    }
}

/// The full HEAD commit id of the Git checkout whose top directory is this workspace.
fn checkout_commit() -> Option<String> {
    let root = workspace_root();
    // A source archive from PyPI never names a commit, also when it is in a Git repository.
    if root.join("PKG-INFO").exists() {
        return None;
    }
    // A build directory outside the workspace can serve other checkouts too, so a build there
    // names no commit. Cargo gives this package and its build script the same OUT_DIR.
    let build_directory = Path::new(env!("OUT_DIR")).canonicalize().unwrap();
    if !build_directory.starts_with(&root) {
        return None;
    }
    // A Git repository above the workspace belongs to another project.
    let top = git(&root, &["rev-parse", "--show-toplevel"])?;
    if Path::new(&top).canonicalize().unwrap() != root {
        return None;
    }
    let commit = git(&root, &["rev-parse", "HEAD"]).expect("git names the HEAD commit");
    assert!(
        commit.len() >= 40 && commit.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "git rev-parse HEAD printed {commit:?}"
    );
    Some(commit)
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

/// The trimmed output of `git <arguments>` in `root`, or `None` when `root` is not in a Git
/// repository.
fn git(root: &Path, arguments: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(arguments)
        // A Git hook sets these values for the repository that runs the hook.
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_COMMON_DIR")
        .env("LC_ALL", "C")
        .output()
        .expect("the test runs git to find the commit of this checkout");
    if output.status.success() {
        return Some(String::from_utf8(output.stdout).unwrap().trim().to_owned());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not a git repository"),
        "git {arguments:?} failed in {}: {stderr}",
        root.display()
    );
    None
}
