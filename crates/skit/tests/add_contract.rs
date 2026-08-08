use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use tempfile::tempdir;

struct Roots {
    data: PathBuf,
    state: PathBuf,
    config: PathBuf,
}

impl Roots {
    fn new(root: &Path) -> Self {
        Self {
            data: root.join("data"),
            state: root.join("state"),
            config: root.join("config"),
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_skit"));
        command
            .env("SKIT_DATA_DIR", &self.data)
            .env("SKIT_STATE_DIR", &self.state)
            .env("SKIT_CONFIG_DIR", &self.config);
        command
    }

    fn list_json(&self) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
        let output = self.command().args(["list", "--json"]).output()?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(serde_json::from_slice(&output.stdout)?)
    }

    fn show_json(&self, name: &str) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
        let output = self.command().args(["show", name, "--json"]).output()?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(serde_json::from_slice(&output.stdout)?)
    }
}

fn run_add(roots: &Roots, args: &[&str]) -> Result<Output, Box<dyn std::error::Error>> {
    Ok(roots.command().arg("add").args(args).output()?)
}

#[test]
fn shell_copy_infers_identity_description_interpreter_and_store_shape()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let roots = Roots::new(root.path());
    let source = root.path().join("deploy.sh");
    let bytes = b"#!/usr/bin/env zsh\n# Ship it\necho hi\n";
    fs::write(&source, bytes)?;

    let output = run_add(&roots, &[source.to_string_lossy().as_ref(), "-n", "deploy"])?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout)?,
        "Added: deploy (copy mode)\n  Description: Ship it\n  Run it: skit run deploy\n"
    );

    let shown = roots.show_json("deploy")?;
    assert_eq!(shown["kind"], "shell");
    assert_eq!(shown["mode"], "copy");
    assert_eq!(shown["description"], "Ship it");
    assert_eq!(shown["interpreter"], "zsh");
    assert_eq!(shown["workdir"], "invoke");
    assert_eq!(
        fs::read(roots.data.join("scripts/deploy/script.sh"))?,
        bytes
    );
    Ok(())
}

#[test]
fn reference_mode_keeps_original_and_materializes_no_payload()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let roots = Roots::new(root.path());
    let source = root.path().join("task.rb");
    let bytes = b"# A task\nputs 'ok'\n";
    fs::write(&source, bytes)?;

    let output = run_add(
        &roots,
        &[source.to_string_lossy().as_ref(), "--ref", "-n", "task"],
    )?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fs::read(&source)?, bytes);
    assert!(!roots.data.join("scripts/task/script.rb").exists());

    let shown = roots.show_json("task")?;
    assert_eq!(shown["kind"], "ruby");
    assert_eq!(shown["mode"], "reference");
    assert_eq!(shown["workdir"], "origin");
    assert_eq!(shown["missing"], false);
    Ok(())
}

#[test]
fn explicit_kind_handles_extensionless_script() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let roots = Roots::new(root.path());
    let source = root.path().join("build");
    fs::write(&source, "echo building\n")?;

    let output = run_add(
        &roots,
        &[
            source.to_string_lossy().as_ref(),
            "--kind",
            "shell",
            "-n",
            "builder",
            "-d",
            "Build project",
        ],
    )?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let shown = roots.show_json("builder")?;
    assert_eq!(shown["kind"], "shell");
    assert_eq!(shown["description"], "Build project");
    assert!(roots.data.join("scripts/builder/script.sh").is_file());
    Ok(())
}

#[test]
fn exe_is_always_reference_and_does_not_claim_copy_mode() -> Result<(), Box<dyn std::error::Error>>
{
    let root = tempdir()?;
    let roots = Roots::new(root.path());
    let source = root.path().join("program.bin");
    fs::write(&source, b"opaque program bytes")?;

    let output = run_add(
        &roots,
        &[source.to_string_lossy().as_ref(), "--exe", "-n", "program"],
    )?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout)?,
        "Added: program\n  Run it: skit run program\n"
    );
    let shown = roots.show_json("program")?;
    assert_eq!(shown["kind"], "exe");
    assert_eq!(shown["mode"], "reference");
    assert!(!roots.data.join("scripts/program/payload").exists());
    Ok(())
}

#[test]
fn unknown_file_is_usage_error_and_writes_nothing() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let roots = Roots::new(root.path());
    let source = root.path().join("notes");
    fs::write(&source, "plain text\n")?;

    let output = run_add(&roots, &[source.to_string_lossy().as_ref()])?;
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr)?;
    assert!(stderr.contains("--kind <language>"));
    assert!(stderr.contains("--exe"));
    assert_eq!(roots.list_json()?, serde_json::json!([]));
    Ok(())
}

#[test]
fn conflicting_kind_selectors_are_usage_error_and_write_nothing()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let roots = Roots::new(root.path());
    let source = root.path().join("tool.sh");
    fs::write(&source, "echo hi\n")?;

    let output = run_add(
        &roots,
        &[
            source.to_string_lossy().as_ref(),
            "--kind",
            "shell",
            "--exe",
        ],
    )?;
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8(output.stderr)?.contains("Use --kind or --exe, not both"));
    assert_eq!(roots.list_json()?, serde_json::json!([]));
    Ok(())
}

#[test]
fn ordinary_python_file_is_accepted_without_fabricating_metadata()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let roots = Roots::new(root.path());
    let source = root.path().join("job.py");
    let bytes = b"print('hi')\n";
    fs::write(&source, bytes)?;

    let output = run_add(&roots, &[source.to_string_lossy().as_ref()])?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout)?,
        "Added: job (copy mode)\n  Run it: skit run job\n"
    );
    assert_eq!(
        fs::read(roots.data.join("scripts/job/script.py"))?,
        bytes
    );
    let shown = roots.show_json("job")?;
    assert_eq!(shown["kind"], "python");
    assert_eq!(shown["mode"], "copy");
    assert_eq!(shown["dependencies"], serde_json::json!([]));
    assert_eq!(shown["requires_python"], "");
    assert_eq!(shown["param_source"], "none");
    Ok(())
}
