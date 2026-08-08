#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Output};

use skit_core::{Binding, Delivery, ParamDecl, ParamDefault, ParamType, write_python_params};
use tempfile::tempdir;

fn write(path: &Path, content: &str) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)?;
    Ok(())
}

fn python_fixture(
    root: &Path,
    body: &str,
    params: &[ParamDecl],
) -> Result<String, Box<dyn std::error::Error>> {
    let stored = write_python_params(body, params);
    write(&root.join("data/scripts/managed/script.py"), &stored)?;
    write(
        &root.join("data/scripts/managed/meta.toml"),
        r#"schema = 1
name = "Managed"
kind = "python"
mode = "copy"
source = "/missing/original.py"
workdir = "invoke"
"#,
    )?;
    Ok(stored)
}

fn install_fake_uv(root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let path = root.join("data/bin/uv");
    write(
        &path,
        r#"#!/bin/sh
script=
while [ "$#" -gt 0 ]; do
    if [ "$1" = "--script" ]; then
        shift
        script=$1
        break
    fi
    shift
done
[ -n "$script" ] || exit 90
found=
while IFS= read -r line; do
    case "$line" in
        'CITY = "Paris"') found=1 ;;
    esac
done < "$script"
[ -n "$found" ] || exit 91
printf '%s\n' "$script" > "$SKIT_FAKE_UV_LOG" || exit 92
"#,
    )?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755))?;
    Ok(())
}

fn run(root: &Path, args: &[&str], log: &Path) -> Result<Output, Box<dyn std::error::Error>> {
    Ok(Command::new(env!("CARGO_BIN_EXE_skit"))
        .args(args)
        .env("SKIT_DATA_DIR", root.join("data"))
        .env("SKIT_STATE_DIR", root.join("state"))
        .env("SKIT_CONFIG_DIR", root.join("config"))
        .env("SKIT_FAKE_UV_LOG", log)
        .env("PATH", "")
        .output()?)
}

#[test]
fn managed_const_reaches_real_child_via_ephemeral_snapshot_and_store_stays_untouched()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let params = vec![ParamDecl {
        name: "CITY".to_owned(),
        binding: Binding::Const,
        delivery: Delivery::Inject,
        param_type: ParamType::String,
        default: Some(ParamDefault::String("Taipei".to_owned())),
        ..ParamDecl::default()
    }];
    let stored = python_fixture(root.path(), "CITY = 'Taipei'\nprint(CITY)\n", &params)?;
    install_fake_uv(root.path())?;
    let log = root.path().join("uv-script-path.txt");

    let output = run(
        root.path(),
        &["run", "Managed", "--set", "CITY=Paris", "--no-input"],
        &log,
    )?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());

    let temp_path = fs::read_to_string(&log)?.trim().to_owned();
    assert!(!temp_path.is_empty());
    assert!(!Path::new(&temp_path).exists());
    assert_eq!(
        fs::read_to_string(root.path().join("data/scripts/managed/script.py"))?,
        stored
    );
    let state = fs::read_to_string(root.path().join("state/values/managed.toml"))?;
    assert!(state.contains("CITY = \"Paris\""));
    assert!(state.contains("exit = 0"));
    Ok(())
}

#[test]
fn managed_input_value_is_a_125_refusal_and_never_spawns_fake_uv()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let params = vec![ParamDecl {
        name: "input-1".to_owned(),
        binding: Binding::Input,
        delivery: Delivery::Inject,
        prompt: "Name: ".to_owned(),
        order: 0,
        ..ParamDecl::default()
    }];
    python_fixture(root.path(), "name = input('Name: ')\n", &params)?;
    install_fake_uv(root.path())?;
    let log = root.path().join("should-not-exist.txt");

    let output = run(
        root.path(),
        &["run", "Managed", "--set", "input-1=Ada", "--no-input"],
        &log,
    )?;
    assert_eq!(output.status.code(), Some(125));
    assert!(String::from_utf8(output.stderr)?.contains("managed input() injection is not enabled"));
    assert!(!log.exists());
    assert!(!root.path().join("state/values/managed.toml").exists());
    Ok(())
}
