//! Build script of the `skit` command.
//!
//! `skit --version` names the commit that the binary was built from: `skit 0.5.0 (1a2b3c4d5)`.
//! This script finds that commit and gives its first nine hex characters to the compiler as
//! `SKIT_BUILD_COMMIT`. When the commit is not known, the value is empty, and `skit --version`
//! shows the version alone. The script always sets the value, so a `SKIT_BUILD_COMMIT` in the
//! environment of the build cannot get into the binary.
//!
//! A wrong commit is worse than no commit. These rules prevent a wrong commit:
//!
//! - The script reads only the Git checkout whose top directory is the workspace root. It never
//!   looks in a directory above the root. A repository there belongs to another project, for
//!   example a monorepo that contains a copy of skit.
//! - A source archive from PyPI has `PKG-INFO` at its root. A build from the archive names no
//!   commit, also when someone puts the archive into a Git repository.
//! - The script names a commit only when the build directory is inside the workspace. Cargo
//!   decides by file times if a result is current, and checkouts that share a build directory
//!   share the results. Thus a build directory outside all checkouts, for example one
//!   `CARGO_TARGET_DIR` for all projects, gets no commit. The rule cannot see a different checkout
//!   that builds in the build directory of this checkout, for example with `--target-dir`. Cargo
//!   can then use the result of this script, and code of this checkout, without a new build. The
//!   build of the other checkout then shows the commit of this checkout. Give each checkout its
//!   own build directory.
//! - The script reads no environment value other than the two directories that Cargo gives it.
//!   For example, `GITHUB_SHA` belongs to the workflow that runs the build, and that workflow can
//!   belong to another project.
//!
//! The script reads the files in `.git`. It does not run `git`. The Linux release wheels build in
//! a container as root. In the container, `git` can be missing, and `git` refuses a checkout that
//! another user owns.

use std::{
    env, fs, io,
    path::{Path, PathBuf},
};

/// The number of hex characters that `skit --version` shows. rustc, Cargo, and uv show the same
/// number.
const SHORT_COMMIT_LENGTH: usize = 9;

fn main() {
    let package = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("set by Cargo"));
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("set by Cargo"));
    let mut watched = Vec::new();
    let commit = workspace_root(&package)
        .filter(|root| is_inside(&out_dir, root))
        .and_then(|root| head_commit(root, &mut watched));
    println!("cargo::rerun-if-changed=build.rs");
    // Cargo runs this script again when one of these files changes, for example after a commit.
    // Each path exists: Cargo runs the script on every build for a path that does not exist.
    for path in &watched {
        println!("cargo::rerun-if-changed={}", path.display());
    }
    let short = commit
        .as_deref()
        .map_or("", |id| &id[..SHORT_COMMIT_LENGTH]);
    println!("cargo::rustc-env=SKIT_BUILD_COMMIT={short}");
}

/// The workspace root. This package is at `<root>/crates/skit-cli`. A copy of the package in a
/// different place has no workspace root.
fn workspace_root(package: &Path) -> Option<&Path> {
    let crates = package.parent()?;
    if package.file_name()? != "skit-cli" || crates.file_name()? != "crates" {
        return None;
    }
    crates.parent()
}

/// True when `path` is inside `directory`. Links in both paths are resolved first.
fn is_inside(path: &Path, directory: &Path) -> bool {
    match (path.canonicalize(), directory.canonicalize()) {
        (Ok(path), Ok(directory)) => path.starts_with(directory),
        _ => false,
    }
}

/// The full id of the commit that HEAD names in the Git checkout at `root`.
///
/// The files that give the answer go into `watched`.
fn head_commit(root: &Path, watched: &mut Vec<PathBuf>) -> Option<String> {
    if root.join("PKG-INFO").exists() {
        return None;
    }
    let dot_git = root.join(".git");
    let git_dir = if dot_git.is_dir() {
        dot_git
    } else {
        // A linked worktree or a submodule has a `.git` file that names its Git directory.
        let text = fs::read_to_string(&dot_git).ok()?;
        watched.push(dot_git);
        root.join(text.strip_prefix("gitdir:")?.trim())
    };
    let head_path = git_dir.join("HEAD");
    let head = fs::read_to_string(&head_path).ok()?;
    watched.push(head_path);
    let head = head.trim_end();
    match head.strip_prefix("ref: ") {
        Some(branch) if branch.starts_with("refs/heads/") => {
            branch_commit(&common_dir(&git_dir)?, branch, watched)
        }
        Some(_) => None,
        // A detached HEAD holds the commit id.
        None => object_id(head),
    }
}

/// The directory that holds the branches. A linked worktree names it in its `commondir` file.
/// Other checkouts keep the branches in the Git directory.
fn common_dir(git_dir: &Path) -> Option<PathBuf> {
    match fs::read_to_string(git_dir.join("commondir")) {
        Ok(text) => Some(git_dir.join(text.trim_end())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Some(git_dir.to_path_buf()),
        Err(_) => None,
    }
}

/// The commit id of `branch`, for example `refs/heads/main`. Git keeps it in a file of its own
/// (a loose ref) or in a line of `packed-refs`. A loose ref replaces a packed one.
fn branch_commit(common_dir: &Path, branch: &str, watched: &mut Vec<PathBuf>) -> Option<String> {
    let loose = common_dir.join(branch);
    if let Ok(text) = fs::read_to_string(&loose) {
        watched.push(loose);
        return object_id(text.trim_end());
    }
    let packed_path = common_dir.join("packed-refs");
    let packed = fs::read_to_string(&packed_path).ok();
    // The branch has no loose ref, so it has no file of its own. `packed-refs` and the directory
    // of the loose refs also change for other refs, and each change rebuilds skit-cli. Git adds a
    // line to the reflog of the branch at each change of the branch, so watch the reflog when it
    // exists.
    let reflog = common_dir.join("logs").join(branch);
    if reflog.is_file() {
        watched.push(reflog);
    } else {
        // A commit on the branch makes a loose ref. A watched path that does not exist makes Cargo
        // run this script on every build, so watch the nearest directory on its path that exists.
        let refs = common_dir.join("refs");
        watched.extend(
            loose
                .ancestors()
                .skip(1)
                .take_while(|directory| directory.starts_with(&refs))
                .find(|directory| directory.is_dir())
                .map(Path::to_path_buf),
        );
        if packed.is_some() {
            watched.push(packed_path);
        }
    }
    packed?
        .lines()
        .find_map(|line| {
            let (id, name) = line.split_once(' ')?;
            (name == branch).then_some(id)
        })
        .and_then(object_id)
}

/// `text` as a full commit id: 40 hex digits (SHA-1) or 64 hex digits (SHA-256), in lowercase.
fn object_id(text: &str) -> Option<String> {
    let full = matches!(text.len(), 40 | 64)
        && text
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'));
    full.then(|| text.to_owned())
}
