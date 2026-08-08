use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use skit_core::{
    Assembly, Entry, LaunchOptions, LaunchPlanError, Platform, ScriptMeta, build_launch_plan,
};
use tempfile::tempdir;

fn entry(
    root: &Path,
    kind: &str,
    mode: &str,
    source: &Path,
    workdir: &str,
    interpreter: &str,
) -> Result<Entry, Box<dyn std::error::Error>> {
    let text = format!(
        "name = \"demo\"\nkind = \"{kind}\"\nmode = \"{mode}\"\nsource = {source:?}\nworkdir = \"{workdir}\"\ninterpreter = \"{interpreter}\"\n",
        source = source.to_string_lossy()
    );
    let meta: ScriptMeta = toml::from_str(&text)?;
    Ok(Entry {
        slug: "demo".to_owned(),
        meta,
        dir: root.join("data/scripts/demo"),
    })
}

fn resolver<'a>(items: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<PathBuf> + 'a {
    move |name| {
        items
            .iter()
            .find(|(candidate, _)| *candidate == name)
            .map(|(_, path)| PathBuf::from(path))
    }
}

#[test]
fn interpreter_launch_honors_pin_and_routes_env_and_args() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let source = root.path().join("origin.sh");
    fs::write(&source, "#!/bin/zsh\necho ok\n")?;
    let entry = entry(root.path(), "shell", "reference", &source, "origin", "zsh")?;
    let assembly = Assembly {
        args: vec!["--fast".to_owned()],
        env_values: BTreeMap::from([("MODE".to_owned(), "prod".to_owned())]),
        ..Assembly::default()
    };
    let options = LaunchOptions::new(Platform::Linux, root.path());

    let plan = build_launch_plan(&entry, &assembly, &options, &resolver(&[("zsh", "/bin/zsh")]))?;
    assert_eq!(
        plan.argv,
        [
            "/bin/zsh",
            source.to_string_lossy().as_ref(),
            "--fast"
        ]
    );
    assert_eq!(plan.cwd, root.path());
    assert_eq!(plan.env_overlay["MODE"], "prod");
    Ok(())
}

#[test]
fn powershell_keeps_file_prefix_between_runtime_and_script()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let entry_dir = root.path().join("data/scripts/demo");
    fs::create_dir_all(&entry_dir)?;
    fs::write(entry_dir.join("script.ps1"), "Write-Host ok\n")?;
    let source = root.path().join("origin.ps1");
    let entry = entry(root.path(), "powershell", "copy", &source, "invoke", "")?;
    let options = LaunchOptions::new(Platform::Windows, root.path());

    let plan = build_launch_plan(
        &entry,
        &Assembly::default(),
        &options,
        &resolver(&[("pwsh", "C:/pwsh.exe")]),
    )?;
    assert_eq!(plan.argv[0], "C:/pwsh.exe");
    assert_eq!(plan.argv[1], "-File");
    assert!(plan.argv[2].ends_with("script.ps1"));
    Ok(())
}

#[test]
fn javascript_runtime_order_and_invocation_match_python_contract()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let entry_dir = root.path().join("data/scripts/demo");
    fs::create_dir_all(&entry_dir)?;
    fs::write(entry_dir.join("script.js"), "console.log(1)\n")?;
    let source = root.path().join("origin.js");
    let entry = entry(root.path(), "js", "copy", &source, "invoke", "")?;
    let options = LaunchOptions::new(Platform::Linux, root.path());

    let deno = build_launch_plan(
        &entry,
        &Assembly::default(),
        &options,
        &resolver(&[("deno", "/d"), ("bun", "/b"), ("node", "/n")]),
    )?;
    assert_eq!(deno.argv[0..3], ["/d", "run", "--allow-all"]);

    let bun = build_launch_plan(
        &entry,
        &Assembly::default(),
        &options,
        &resolver(&[("bun", "/b"), ("node", "/n")]),
    )?;
    assert_eq!(bun.argv[0..2], ["/b", "run"]);

    let node = build_launch_plan(
        &entry,
        &Assembly::default(),
        &options,
        &resolver(&[("node", "/n")]),
    )?;
    assert_eq!(node.argv[0], "/n");
    assert!(node.argv[1].ends_with("script.js"));
    Ok(())
}

#[test]
fn javascript_override_is_strict_and_does_not_fall_back() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let entry_dir = root.path().join("data/scripts/demo");
    fs::create_dir_all(&entry_dir)?;
    fs::write(entry_dir.join("script.js"), "console.log(1)\n")?;
    let source = root.path().join("origin.js");
    let mut entry = entry(root.path(), "js", "copy", &source, "invoke", "")?;
    entry.meta.interpreter = "bun".to_owned();
    let options = LaunchOptions::new(Platform::Linux, root.path());

    let result = build_launch_plan(
        &entry,
        &Assembly::default(),
        &options,
        &resolver(&[("deno", "/d"), ("node", "/n")]),
    );
    assert!(matches!(
        result,
        Err(LaunchPlanError::MissingJavaScriptRuntime(names)) if names == ["bun"]
    ));
    Ok(())
}

#[test]
fn windows_shell_can_use_configured_bash_when_path_lookup_misses()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let entry_dir = root.path().join("data/scripts/demo");
    fs::create_dir_all(&entry_dir)?;
    fs::write(entry_dir.join("script.sh"), "echo ok\n")?;
    let source = root.path().join("origin.sh");
    let entry = entry(root.path(), "shell", "copy", &source, "invoke", "")?;
    let bash = root.path().join("bash.exe");
    fs::write(&bash, b"")?;
    let mut options = LaunchOptions::new(Platform::Windows, root.path());
    options.windows_bash = Some(bash.clone());

    let plan = build_launch_plan(&entry, &Assembly::default(), &options, &resolver(&[]))?;
    assert_eq!(plan.argv[0], bash.to_string_lossy());
    Ok(())
}

#[test]
fn missing_declared_needs_refuse_before_launch_snapshot_is_returned()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let entry_dir = root.path().join("data/scripts/demo");
    fs::create_dir_all(&entry_dir)?;
    fs::write(entry_dir.join("script.sh"), "echo ok\n")?;
    let source = root.path().join("origin.sh");
    let mut entry = entry(root.path(), "shell", "copy", &source, "invoke", "")?;
    entry.meta.needs = Some(vec!["jq".to_owned(), "ffmpeg".to_owned()]);
    let options = LaunchOptions::new(Platform::Linux, root.path());

    let result = build_launch_plan(
        &entry,
        &Assembly::default(),
        &options,
        &resolver(&[("bash", "/bin/bash"), ("jq", "/usr/bin/jq")]),
    );
    assert!(matches!(
        result,
        Err(LaunchPlanError::MissingNeeds(names)) if names == ["ffmpeg"]
    ));
    Ok(())
}

#[test]
fn vanished_copy_origin_does_not_block_legacy_origin_workdir()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let entry_dir = root.path().join("data/scripts/demo");
    fs::create_dir_all(&entry_dir)?;
    fs::write(entry_dir.join("script.sh"), "echo ok\n")?;
    let vanished_source = root.path().join("gone/origin.sh");
    let entry = entry(
        root.path(),
        "shell",
        "copy",
        &vanished_source,
        "origin",
        "",
    )?;
    let options = LaunchOptions::new(Platform::Linux, root.path());

    let plan = build_launch_plan(
        &entry,
        &Assembly::default(),
        &options,
        &resolver(&[("bash", "/bin/bash")]),
    )?;
    assert_eq!(plan.cwd, root.path());
    Ok(())
}

#[test]
fn custom_missing_workdir_is_a_named_refusal() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let entry_dir = root.path().join("data/scripts/demo");
    fs::create_dir_all(&entry_dir)?;
    fs::write(entry_dir.join("script.sh"), "echo ok\n")?;
    let source = root.path().join("origin.sh");
    let missing = root.path().join("missing-workdir");
    let entry = entry(
        root.path(),
        "shell",
        "copy",
        &source,
        missing.to_string_lossy().as_ref(),
        "",
    )?;
    let options = LaunchOptions::new(Platform::Linux, root.path());

    let result = build_launch_plan(
        &entry,
        &Assembly::default(),
        &options,
        &resolver(&[("bash", "/bin/bash")]),
    );
    assert!(matches!(
        result,
        Err(LaunchPlanError::WorkingDirectoryMissing(path)) if path == missing
    ));
    Ok(())
}

#[cfg(unix)]
#[test]
fn direct_executable_checks_the_posix_execute_bit() -> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::PermissionsExt;

    let root = tempdir()?;
    let source = root.path().join("tool");
    fs::write(&source, "#!/bin/sh\necho ok\n")?;
    fs::set_permissions(&source, fs::Permissions::from_mode(0o644))?;
    let entry = entry(root.path(), "exe", "reference", &source, "origin", "")?;
    let options = LaunchOptions::new(Platform::Linux, root.path());
    let result = build_launch_plan(&entry, &Assembly::default(), &options, &resolver(&[]));
    assert!(matches!(result, Err(LaunchPlanError::NotRunnable(path)) if path == source));
    Ok(())
}

#[test]
fn deep_kinds_are_explicitly_refused_instead_of_partially_planned()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let source = root.path().join("body");
    fs::write(&source, "echo {x}\n")?;
    let entry = entry(root.path(), "command", "copy", &source, "invoke", "")?;
    let options = LaunchOptions::new(Platform::Linux, root.path());
    let result = build_launch_plan(&entry, &Assembly::default(), &options, &resolver(&[]));
    assert!(matches!(
        result,
        Err(LaunchPlanError::UnsupportedKind(kind)) if kind == "command"
    ));
    Ok(())
}
