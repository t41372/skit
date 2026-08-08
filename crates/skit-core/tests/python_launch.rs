use std::fs;
use std::path::{Path, PathBuf};

use skit_core::{Assembly, Entry, LaunchOptions, Platform, ScriptMeta, build_launch_plan};
use tempfile::tempdir;

fn entry(root: &Path, mode: &str) -> Entry {
    Entry {
        slug: "py".to_owned(),
        meta: ScriptMeta {
            schema: 1,
            name: "py".to_owned(),
            kind: "python".to_owned(),
            mode: mode.to_owned(),
            source: root.join("origin.py").to_string_lossy().into_owned(),
            source_hash: String::new(),
            added_at: String::new(),
            workdir: if mode == "reference" {
                "origin".to_owned()
            } else {
                "invoke".to_owned()
            },
            description: String::new(),
            template: String::new(),
            dependencies: None,
            requires_python: String::new(),
            params: None,
            interpreter: String::new(),
            runner: String::new(),
            interpolate: true,
            needs: None,
            parameters: None,
            extra: Default::default(),
        },
        dir: root.join("data/scripts/py"),
    }
}

fn resolver(name: &str) -> Option<PathBuf> {
    (name == "uv").then(|| PathBuf::from("/tools/uv"))
}

#[test]
fn copy_python_uses_effective_block_metadata_in_uv_argv()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let entry = entry(root.path(), "copy");
    fs::create_dir_all(&entry.dir)?;
    fs::write(
        entry.dir.join("script.py"),
        "# /// script\n# requires-python = \">=3.12,<3.13\"\n# dependencies = [\"requests>=2,<3\", \"rich>=13\"]\n# ///\nprint(1)\n",
    )?;
    let options = LaunchOptions::new(Platform::Linux, root.path());
    let assembly = Assembly {
        args: vec!["--verbose".to_owned()],
        ..Assembly::default()
    };

    let plan = build_launch_plan(&entry, &assembly, &options, &resolver)?;
    assert_eq!(
        plan.argv,
        [
            "/tools/uv",
            "run",
            "--no-project",
            "--python",
            ">=3.12,<3.13",
            "--with",
            "requests>=2,<3",
            "--with",
            "rich>=13",
            "--script",
            entry.dir.join("script.py").to_string_lossy().as_ref(),
            "--verbose"
        ]
    );
    assert_eq!(plan.cwd, root.path());
    Ok(())
}

#[test]
fn reference_python_uses_meta_axes_and_original_workdir()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let mut entry = entry(root.path(), "reference");
    fs::write(&entry.meta.source, "print(1)\n")?;
    entry.meta.dependencies = Some(vec!["rich".to_owned()]);
    entry.meta.requires_python = ">=3.11".to_owned();
    let options = LaunchOptions::new(Platform::Linux, root.path());

    let plan = build_launch_plan(&entry, &Assembly::default(), &options, &resolver)?;
    assert_eq!(
        plan.argv,
        [
            "/tools/uv",
            "run",
            "--no-project",
            "--python",
            ">=3.11",
            "--with",
            "rich",
            "--script",
            entry.meta.source.as_str()
        ]
    );
    assert_eq!(plan.cwd, root.path());
    Ok(())
}

#[test]
fn script_override_is_the_exact_snapshot_passed_to_uv() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let entry = entry(root.path(), "copy");
    fs::create_dir_all(&entry.dir)?;
    fs::write(entry.dir.join("script.py"), "print('stored')\n")?;
    let injected = root.path().join("skit-injected.py");
    fs::write(&injected, "print('injected')\n")?;
    let mut options = LaunchOptions::new(Platform::Linux, root.path());
    options.script_override = Some(injected.clone());

    let plan = build_launch_plan(&entry, &Assembly::default(), &options, &resolver)?;
    let script_index = plan
        .argv
        .iter()
        .position(|arg| arg == "--script")
        .ok_or("missing --script")?;
    assert_eq!(plan.argv[script_index + 1], injected.to_string_lossy());
    Ok(())
}

#[test]
fn missing_uv_is_a_named_runtime_refusal() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let entry = entry(root.path(), "copy");
    fs::create_dir_all(&entry.dir)?;
    fs::write(entry.dir.join("script.py"), "print(1)\n")?;
    let options = LaunchOptions::new(Platform::Linux, root.path());
    let result = build_launch_plan(&entry, &Assembly::default(), &options, &|_: &str| None);
    assert!(matches!(
        result,
        Err(skit_core::LaunchPlanError::MissingInterpreter(name)) if name == "uv"
    ));
    Ok(())
}
