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

fn exe_fixture(
    root: &Path,
    fields: &str,
) -> Result<(), Box<dyn std::error::Error>> {
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
    child_code: i32,
) -> Result<Output, Box<dyn std::error::Error>> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_skit"));
    command
        .args(args)
        .env("SKIT_DATA_DIR", root.join("data"))
        .env("SKIT_STATE_DIR", root.join("state"))
        .env("SKIT_CONFIG_DIR", root.join("config"))
        .env("SKIT_RUN_CHILD", "1")
        .env("SKIT_RUN_CHILD_CODE", child_code.to_string());
    if let Some(path) = result_path {
        command.env("SKIT_RUN_RESULT", path);
    }
    Ok(command.output()?)
}

fn child_tail() -> [&'static str; 4] {
    ["--", "--exact", "run_target_child", "--nocapture"]
}

#[test]
fn run_target_child() {
    if env::var_os("SKIT_RUN_CHILD").is_none() {
        return;
    }
    if let Some(path) = env::var_os("SKIT_RUN_RESULT") {
        let value = env::var("VALUE").unwrap_or_default();
        let token = env::var("TOKEN").unwrap_or_default();
        let body = format!("VALUE={value}\nTOKEN={token}\n");
        if fs::write(path, body).is_err() {
            process::exit(98);
        }
    }
    let code = env::var("SKIT_RUN_CHILD_CODE")
        .ok()
        .and_then(|value| value.parse::<i32>().ok())
        .unwrap_or(0);
    process::exit(code);
}

#[test]
fn set_value_reaches_child_and_is_remembered() -> Result<(), Box<dyn std::error::Error>> {
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
    let result_path = root.path().join("result.txt");
    let mut args = vec!["run", "Demo", "--set", "VALUE=a=b", "--no-input"];
    args.extend(child_tail());
    let output = run(root.path(), &args, Some(&result_path), 0)?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    assert_eq!(fs::read_to_string(result_path)?, "VALUE=a=b\nTOKEN=\n");

    let state = fs::read_to_string(root.path().join("state/values/demo.toml"))?;
    assert!(state.contains("VALUE = \"a=b\""));
    assert!(state.contains("exit = 0"));
    assert!(state.contains("--exact"));
    Ok(())
}

#[test]
fn remembered_extra_args_replay_on_the_next_run() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    exe_fixture(root.path(), "")?;
    let first_result = root.path().join("first.txt");
    let mut first_args = vec!["run", "Demo", "--no-input"];
    first_args.extend(child_tail());
    let first = run(root.path(), &first_args, Some(&first_result), 0)?;
    assert!(first.status.success());
    assert!(first_result.is_file());

    let second_result = root.path().join("second.txt");
    let second = run(
        root.path(),
        &["run", "Demo", "--no-input"],
        Some(&second_result),
        0,
    )?;
    assert!(
        second.status.success(),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );
    assert!(String::from_utf8(second.stderr)?.contains("Reusing remembered extra arguments"));
    assert!(second_result.is_file());
    Ok(())
}

#[test]
fn child_nonzero_exit_is_silent_and_passes_through() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    exe_fixture(root.path(), "")?;
    let mut args = vec!["run", "Demo", "--no-input"];
    args.extend(child_tail());
    let output = run(root.path(), &args, None, 7)?;
    assert_eq!(output.status.code(), Some(7));
    assert!(output.stderr.is_empty());
    let state = fs::read_to_string(root.path().join("state/values/demo.toml"))?;
    assert!(state.contains("exit = 7"));
    Ok(())
}

#[test]
fn save_preset_is_deferred_until_after_a_real_child_run()
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
    let mut args = vec![
        "run",
        "Demo",
        "--set",
        "VALUE=prod",
        "--save-preset",
        "release",
        "--no-input",
    ];
    args.extend(child_tail());
    let output = run(root.path(), &args, None, 0)?;
    assert!(output.status.success());
    let state = fs::read_to_string(root.path().join("state/values/demo.toml"))?;
    assert!(state.contains("[presets.release]"));
    assert!(state.contains("VALUE = \"prod\""));
    Ok(())
}

#[test]
fn secret_set_value_reaches_child_but_never_state_bytes()
-> Result<(), Box<dyn std::error::Error>> {
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
    let result_path = root.path().join("secret-result.txt");
    let mut args = vec!["run", "Demo", "--set", "TOKEN=s3cret-value", "--no-input"];
    args.extend(child_tail());
    let output = run(root.path(), &args, Some(&result_path), 0)?;
    assert!(output.status.success());
    assert_eq!(
        fs::read_to_string(result_path)?,
        "VALUE=\nTOKEN=s3cret-value\n"
    );
    let state = fs::read_to_string(root.path().join("state/values/demo.toml"))?;
    assert!(!state.contains("s3cret-value"));
    assert!(!state.contains("TOKEN"));
    Ok(())
}

#[test]
fn malformed_set_is_usage_and_never_launches() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    exe_fixture(root.path(), "")?;
    let output = run(
        root.path(),
        &["run", "Demo", "--set", "NOVALUE", "--no-input"],
        None,
        0,
    )?;
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr)?,
        "Malformed --set (expected NAME=VALUE): NOVALUE\n"
    );
    assert!(!root.path().join("state/values/demo.toml").exists());
    Ok(())
}

#[test]
fn unknown_set_name_is_usage_and_never_launches() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    exe_fixture(root.path(), "")?;
    let output = run(
        root.path(),
        &["run", "Demo", "--set", "OTHER=x", "--no-input"],
        None,
        0,
    )?;
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8(output.stderr)?.contains("unknown parameter: OTHER"));
    assert!(!root.path().join("state/values/demo.toml").exists());
    Ok(())
}

#[test]
fn missing_required_value_is_skit_125_and_never_launches()
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
    let output = run(root.path(), &["run", "Demo", "--no-input"], None, 0)?;
    assert_eq!(output.status.code(), Some(125));
    assert!(!output.stderr.is_empty());
    assert!(!root.path().join("state/values/demo.toml").exists());
    Ok(())
}

#[test]
fn command_and_prompt_launch_remain_explicit_125_refusals()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    write(
        &root.path().join("data/scripts/cmd/meta.toml"),
        r#"name = "Cmd"
kind = "command"
mode = "copy"
workdir = "invoke"
template = "echo hi"
"#,
    )?;
    let output = run(root.path(), &["run", "Cmd", "--no-input"], None, 0)?;
    assert_eq!(output.status.code(), Some(125));
    assert!(String::from_utf8(output.stderr)?.contains("not enabled"));
    assert!(!root.path().join("state/values/cmd.toml").exists());
    Ok(())
}
