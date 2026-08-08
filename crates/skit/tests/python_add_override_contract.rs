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

    fn add(&self, args: &[&str]) -> Result<Output, Box<dyn std::error::Error>> {
        Ok(self.command().arg("add").args(args).output()?)
    }

    fn show(&self, name: &str) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
        let output = self.command().args(["show", name, "--json"]).output()?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(serde_json::from_slice(&output.stdout)?)
    }

    fn list(&self) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
        let output = self.command().args(["list", "--json"]).output()?;
        assert!(output.status.success());
        Ok(serde_json::from_slice(&output.stdout)?)
    }
}

#[test]
fn explicit_dep_and_python_override_auto_detection() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let roots = Roots::new(root.path());
    let source = root.path().join("job.py");
    fs::write(
        &source,
        "#!/usr/bin/env python3.12\nimport requests\nprint('ok')\n",
    )?;

    let output = roots.add(&[
        source.to_string_lossy().as_ref(),
        "-n",
        "job",
        "--dep",
        "rich>=13",
        "--python",
        ">=3.11,<3.12",
    ])?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains("Dependencies: rich>=13"));
    assert!(stdout.contains("Python constraint: >=3.11,<3.12"));
    assert!(!stdout.contains("Dependencies: requests"));

    let shown = roots.show("job")?;
    assert_eq!(shown["dependencies"], serde_json::json!(["rich>=13"]));
    assert_eq!(shown["requires_python"], ">=3.11,<3.12");
    let stored = fs::read_to_string(roots.data.join("scripts/job/script.py"))?;
    assert!(stored.contains("#     \"rich>=13\","));
    assert!(stored.contains("# requires-python = \">=3.11,<3.12\""));
    Ok(())
}

#[test]
fn python_dash_clears_versioned_shebang_pin() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let roots = Roots::new(root.path());
    let source = root.path().join("job.py");
    let text = "#!/usr/bin/env python3.12\nprint('ok')\n";
    fs::write(&source, text)?;

    let output = roots.add(&[
        source.to_string_lossy().as_ref(),
        "-n",
        "job",
        "--python",
        "-",
    ])?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let shown = roots.show("job")?;
    assert_eq!(shown["requires_python"], "");
    assert_eq!(
        fs::read_to_string(roots.data.join("scripts/job/script.py"))?,
        text
    );
    Ok(())
}

#[test]
fn invalid_dep_is_usage_error_before_any_store_write() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let roots = Roots::new(root.path());
    let source = root.path().join("job.py");
    fs::write(&source, "print('ok')\n")?;

    let output = roots.add(&[source.to_string_lossy().as_ref(), "--dep", "requests => 2"])?;
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8(output.stderr)?.contains("invalid Python dependency"));
    assert_eq!(roots.list()?, serde_json::json!([]));
    Ok(())
}

#[test]
fn invalid_python_constraint_is_usage_error_before_any_store_write()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let roots = Roots::new(root.path());
    let source = root.path().join("job.py");
    fs::write(&source, "print('ok')\n")?;

    let output = roots.add(&[source.to_string_lossy().as_ref(), "--python", "3.12"])?;
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8(output.stderr)?.contains("invalid Python constraint"));
    assert_eq!(roots.list()?, serde_json::json!([]));
    Ok(())
}

#[test]
fn explicit_metadata_conflicts_with_source_pep723_and_writes_nothing()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let roots = Roots::new(root.path());
    let source = root.path().join("job.py");
    fs::write(
        &source,
        "# /// script\n# dependencies = [\"source-dep\"]\n# ///\nprint('ok')\n",
    )?;

    let output = roots.add(&[source.to_string_lossy().as_ref(), "--dep", "rich"])?;
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8(output.stderr)?.contains("already declares PEP 723 metadata"));
    assert_eq!(roots.list()?, serde_json::json!([]));
    Ok(())
}

#[test]
fn python_metadata_flags_on_non_python_kind_are_refused() -> Result<(), Box<dyn std::error::Error>>
{
    let root = tempdir()?;
    let roots = Roots::new(root.path());
    let source = root.path().join("job.sh");
    fs::write(&source, "echo ok\n")?;

    let dep = roots.add(&[source.to_string_lossy().as_ref(), "--dep", "rich"])?;
    assert_eq!(dep.status.code(), Some(2));
    assert!(String::from_utf8(dep.stderr)?.contains("drop --dep"));
    assert_eq!(roots.list()?, serde_json::json!([]));

    let python = roots.add(&[source.to_string_lossy().as_ref(), "--python", ">=3.12"])?;
    assert_eq!(python.status.code(), Some(2));
    assert!(String::from_utf8(python.stderr)?.contains("doesn't apply to shell scripts"));
    assert_eq!(roots.list()?, serde_json::json!([]));
    Ok(())
}
