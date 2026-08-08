use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use skit_core::{Binding, Delivery, ParamDecl, ParamDefault, ParamType, write_python_params};
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

    fn add(&self, source: &Path, name: &str) -> Result<Output, Box<dyn std::error::Error>> {
        Ok(self
            .command()
            .args(["add", source.to_string_lossy().as_ref(), "-n", name])
            .output()?)
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
}

#[test]
fn non_tty_python_add_accepts_dependency_and_shebang_suggestions_but_not_new_params()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let roots = Roots::new(root.path());
    let source = root.path().join("job.py");
    fs::write(
        &source,
        "#!/usr/bin/env python3.12\nimport os\nimport requests\nCITY = 'Taipei'\n",
    )?;

    let output = roots.add(&source, "job")?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains("Added: job (copy mode)"));
    assert!(stdout.contains("Dependencies: requests"));
    assert!(stdout.contains("Python constraint: >=3.12,<3.13"));
    assert!(stdout.contains("Parameter candidates left unmanaged: CITY"));

    let shown = roots.show("job")?;
    assert_eq!(shown["dependencies"], serde_json::json!(["requests"]));
    assert_eq!(shown["requires_python"], ">=3.12,<3.13");
    assert_eq!(shown["param_source"], "none");
    assert_eq!(shown["param_origin"], "none");

    let stored = fs::read_to_string(roots.data.join("scripts/job/script.py"))?;
    assert!(stored.contains("# requires-python = \">=3.12,<3.13\""));
    assert!(stored.contains("#     \"requests\","));
    assert!(!stored.contains("[tool.skit]"));
    Ok(())
}

#[cfg(unix)]
fn install_fake_uv(roots: &Roots) -> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::PermissionsExt;

    let path = roots.data.join("bin/uv");
    fs::create_dir_all(path.parent().ok_or("missing bin parent")?)?;
    fs::write(
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

#[cfg(unix)]
#[test]
fn portable_frozen_python_schema_survives_add_show_and_real_managed_run()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let roots = Roots::new(root.path());
    let source = root.path().join("portable.py");
    let params = vec![ParamDecl {
        name: "CITY".to_owned(),
        binding: Binding::Const,
        delivery: Delivery::Inject,
        param_type: ParamType::String,
        default: Some(ParamDefault::String("Taipei".to_owned())),
        ..ParamDecl::default()
    }];
    let portable = write_python_params("CITY = 'Taipei'\nprint(CITY)\n", &params);
    fs::write(&source, &portable)?;

    let added = roots.add(&source, "portable")?;
    assert!(
        added.status.success(),
        "{}",
        String::from_utf8_lossy(&added.stderr)
    );
    assert_eq!(
        fs::read_to_string(roots.data.join("scripts/portable/script.py"))?,
        portable
    );
    let shown = roots.show("portable")?;
    assert_eq!(shown["param_source"], "inject");
    assert_eq!(shown["param_origin"], "managed");
    assert_eq!(shown["fields"][0]["key"], "CITY");
    assert_eq!(shown["fields"][0]["default"], "Taipei");

    install_fake_uv(&roots)?;
    let log = root.path().join("uv-temp.txt");
    let run = roots
        .command()
        .args(["run", "portable", "--set", "CITY=Paris", "--no-input"])
        .env("PATH", "")
        .env("SKIT_FAKE_UV_LOG", &log)
        .output()?;
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(run.stderr.is_empty());
    let temp_path = fs::read_to_string(&log)?.trim().to_owned();
    assert!(!Path::new(&temp_path).exists());
    assert_eq!(
        fs::read_to_string(roots.data.join("scripts/portable/script.py"))?,
        portable
    );
    Ok(())
}
