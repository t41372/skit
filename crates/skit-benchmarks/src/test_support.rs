use std::{fs, path::Path, process::Command};

pub(crate) fn initialized_git_repository(path: &Path) {
    fs::create_dir_all(path).unwrap();
    fs::write(path.join("README.md"), "benchmark fixture\n").unwrap();

    let status = Command::new("git")
        .args(["init", "--quiet", "--initial-branch=main", "--template="])
        .arg(path)
        .current_dir(path.parent().unwrap())
        .status()
        .unwrap();
    assert!(status.success(), "git fixture initialization failed");

    run_git(path, &["add", "--", "README.md"]);
    run_git(
        path,
        &[
            "-c",
            "user.name=skit tests",
            "-c",
            "user.email=tests@skit.invalid",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=",
            "commit",
            "--quiet",
            "--message=fixture",
        ],
    );
}

fn run_git(cwd: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .status()
        .unwrap();
    assert!(status.success(), "git fixture command failed: {args:?}");
}
