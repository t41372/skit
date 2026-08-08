use std::fs;
use std::path::Path;

use skit_core::{LibraryRoots, load_launch_config};
use tempfile::tempdir;

fn roots(root: &Path) -> LibraryRoots {
    LibraryRoots::new(root.join("data"), root.join("state"), root.join("config"))
}

#[test]
fn launch_config_reads_existing_js_and_shell_sections() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let roots = roots(root.path());
    fs::create_dir_all(roots.config_dir())?;
    fs::write(
        roots.config_dir().join("config.toml"),
        r#"language = "zh_TW"

[js]
runner = "node"

[shell]
bash_path = "C:/Git/bin/bash.exe"

[future]
kept = true
"#,
    )?;

    let config = load_launch_config(&roots);
    assert_eq!(config.js_runner.as_deref(), Some("node"));
    assert_eq!(
        config
            .windows_bash
            .as_ref()
            .map(|path| path.to_string_lossy()),
        Some("C:/Git/bin/bash.exe".into())
    );
    Ok(())
}

#[test]
fn missing_corrupt_and_wrong_shaped_config_are_total_and_read_only()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let roots = roots(root.path());
    assert_eq!(load_launch_config(&roots), Default::default());

    fs::create_dir_all(roots.config_dir())?;
    let path = roots.config_dir().join("config.toml");
    fs::write(&path, "[[ broken")?;
    let corrupt = fs::read(&path)?;
    assert_eq!(load_launch_config(&roots), Default::default());
    assert_eq!(fs::read(&path)?, corrupt);

    fs::write(&path, "js = 5\nshell = \"nope\"\n")?;
    let wrong_shape = fs::read(&path)?;
    assert_eq!(load_launch_config(&roots), Default::default());
    assert_eq!(fs::read(&path)?, wrong_shape);
    Ok(())
}

#[test]
fn empty_and_non_string_values_do_not_create_runtime_overrides()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let roots = roots(root.path());
    fs::create_dir_all(roots.config_dir())?;
    fs::write(
        roots.config_dir().join("config.toml"),
        "[js]\nrunner = \"\"\n[shell]\nbash_path = 7\n",
    )?;
    assert_eq!(load_launch_config(&roots), Default::default());
    Ok(())
}
