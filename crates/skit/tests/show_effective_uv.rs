use std::fs;
use std::path::Path;
use std::process::Command;

use serde_json::json;
use tempfile::tempdir;

fn write(path: &Path, content: &str) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)?;
    Ok(())
}

#[test]
fn show_json_reports_block_only_python_metadata_that_uv_would_enforce()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let data = root.path().join("data");
    write(
        &data.join("scripts/py/meta.toml"),
        r#"name = "py"
kind = "python"
mode = "copy"
source = "/gone/origin.py"
workdir = "invoke"
"#,
    )?;
    write(
        &data.join("scripts/py/script.py"),
        "# /// script\n# requires-python = \">=3.12\"\n# dependencies = [\"rich>=13\"]\n# ///\nprint(1)\n",
    )?;

    let output = Command::new(env!("CARGO_BIN_EXE_skit"))
        .args(["show", "py", "--json"])
        .env("SKIT_DATA_DIR", &data)
        .env("SKIT_STATE_DIR", root.path().join("state"))
        .env("SKIT_CONFIG_DIR", root.path().join("config"))
        .output()?;
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(output.stderr.is_empty());
    let payload: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(payload["dependencies"], json!(["rich>=13"]));
    assert_eq!(payload["requires_python"], ">=3.12");
    assert_eq!(payload["missing"], false);
    Ok(())
}

#[test]
fn show_human_reports_same_effective_metadata() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let data = root.path().join("data");
    write(
        &data.join("scripts/py/meta.toml"),
        r#"name = "py"
kind = "python"
mode = "copy"
source = "/gone/origin.py"
workdir = "invoke"
"#,
    )?;
    write(
        &data.join("scripts/py/script.py"),
        "# /// script\n# requires-python = \">=3.12\"\n# dependencies = [\"rich>=13\"]\n# ///\nprint(1)\n",
    )?;

    let output = Command::new(env!("CARGO_BIN_EXE_skit"))
        .args(["show", "py"])
        .env("SKIT_DATA_DIR", &data)
        .env("SKIT_STATE_DIR", root.path().join("state"))
        .env("SKIT_CONFIG_DIR", root.path().join("config"))
        .output()?;
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let text = String::from_utf8(output.stdout)?;
    assert!(text.contains("Dependencies: rich>=13"));
    assert!(text.contains("Python constraint: >=3.12"));
    Ok(())
}
