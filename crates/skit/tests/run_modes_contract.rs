use std::env;
use std::fs;
use std::path::Path;
use std::process::{self, Command, Output};

use tempfile::tempdir;

fn write(path: &Path, content: &str) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)?;
    Ok(())
}

fn exe_fixture(root: &Path, fields: &str) -> Result<(), Box<dyn std::error::Error>> {
    let source = env::current_exe()?.to_string_lossy().into_owned();
    write(
        &root.join("data/scripts/demo/meta.toml"),
        &format!(
            "schema = 1\nname = \"Demo\"\nkind = \"exe\"\nmode = \"reference\"\nsource = {source:?}\nworkdir = \"invoke\"\n{fields}"
        ),
    )
}

fn run(
    root: &Path,
    args: &[&str],
    result_path: Option<&Path>,
) -> Result<Output, Box<dyn std::error::Error>> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_skit"));
    command
        .args(args)
        .env("SKIT_DATA_DIR", root.join("data"))
        .env("SKIT_STATE_DIR", root.join("state"))
        .env("SKIT_CONFIG_DIR", root.join("config"))
        .env("SKIT_RUN_MODES_CHILD", "1");
    if let Some(path) = result_path {
        command.env("SKIT_RUN_MODES_RESULT", path);
    }
    Ok(command.output()?)
}

fn child_tail() -> [&'static str; 4] {
    ["--", "--exact", "run_modes_target_child", "--nocapture"]
}

#[test]
fn run_modes_target_child() {
    if env::var_os("SKIT_RUN_MODES_CHILD").is_none() {
        return;
    }
    if let Some(path) = env::var_os("SKIT_RUN_MODES_RESULT") {
        let value = env::var("VALUE").unwrap_or_default();
        if fs::write(path, format!("VALUE={value}\n")).is_err() {
            process::exit(98);
        }
    }
    process::exit(0);
}

#[test]
fn dry_run_masks_secret_and_does_not_create_state() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    exe_fixture(
        root.path(),
        r#"
[[parameters]]
name = "TOKEN"
delivery = "env"
type = "str"
required = true
secret = true
"#,
    )?;
    let output = run(
        root.path(),
        &[
            "run",
            "Demo",
            "--set",
            "TOKEN=dry-secret-value",
            "--dry-run",
            "--no-input",
        ],
        None,
    )?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout)?;
    let stderr = String::from_utf8(output.stderr)?;
    assert!(stdout.contains("env={"));
    assert!(stdout.contains("•••"));
    assert!(!stdout.contains("dry-secret-value"));
    assert!(!stderr.contains("dry-secret-value"));
    assert!(!root.path().join("state/values/demo.toml").exists());
    Ok(())
}

#[test]
fn dry_run_save_preset_writes_only_the_validated_preset() -> Result<(), Box<dyn std::error::Error>>
{
    let root = tempdir()?;
    exe_fixture(
        root.path(),
        r#"
[[parameters]]
name = "VALUE"
delivery = "env"
type = "str"
required = true
"#,
    )?;
    let output = run(
        root.path(),
        &[
            "run",
            "Demo",
            "--set",
            "VALUE=preview",
            "--save-preset",
            "preview",
            "--dry-run",
            "--no-input",
        ],
        None,
    )?;
    assert!(output.status.success());
    assert!(String::from_utf8(output.stderr)?.contains("Preset \"preview\" saved"));
    let state = fs::read_to_string(root.path().join("state/values/demo.toml"))?;
    assert!(state.contains("[presets.preview]"));
    assert!(state.contains("VALUE = \"preview\""));
    assert!(!state.contains("[last_run]"));
    assert!(!state.contains("extra_args"));
    Ok(())
}

#[test]
fn raw_bypasses_required_form_and_does_not_remember_raw_tail()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    exe_fixture(
        root.path(),
        r#"
[[parameters]]
name = "VALUE"
delivery = "env"
type = "str"
required = true
"#,
    )?;
    let result_path = root.path().join("raw-result.txt");
    let mut args = vec!["run", "Demo", "--raw", "--no-input"];
    args.extend(child_tail());
    let output = run(root.path(), &args, Some(&result_path))?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fs::read_to_string(result_path)?, "VALUE=\n");
    let state = fs::read_to_string(root.path().join("state/values/demo.toml"))?;
    assert!(state.contains("[last_run]"));
    assert!(!state.contains("VALUE"));
    assert!(!state.contains("extra_args"));
    assert!(!state.contains("--exact"));
    Ok(())
}

#[test]
fn raw_is_incompatible_with_form_value_surfaces_and_writes_nothing()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    exe_fixture(root.path(), "")?;
    for conflicting in [
        vec!["run", "Demo", "--raw", "--set", "X=1"],
        vec!["run", "Demo", "--raw", "--preset", "p"],
        vec!["run", "Demo", "--raw", "--save-preset", "p"],
    ] {
        let output = run(root.path(), &conflicting, None)?;
        assert_eq!(output.status.code(), Some(2));
        assert!(
            String::from_utf8(output.stderr)?
                .contains("--raw cannot be combined with --set, --preset, or --save-preset")
        );
    }
    assert!(!root.path().join("state/values/demo.toml").exists());
    Ok(())
}
