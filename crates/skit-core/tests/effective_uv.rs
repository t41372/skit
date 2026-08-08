use std::fs;
use std::path::{Path, PathBuf};

use skit_core::{Entry, ScriptMeta, effective_uv_metadata};
use tempfile::tempdir;

fn entry(
    root: &Path,
    mode: &str,
    meta_deps: Option<Vec<&str>>,
    meta_python: &str,
) -> Result<Entry, Box<dyn std::error::Error>> {
    let mut meta = ScriptMeta {
        schema: 1,
        name: "demo".to_owned(),
        kind: "python".to_owned(),
        mode: mode.to_owned(),
        source: root.join("origin.py").to_string_lossy().into_owned(),
        source_hash: String::new(),
        added_at: String::new(),
        workdir: "invoke".to_owned(),
        description: String::new(),
        template: String::new(),
        dependencies: meta_deps.map(|items| items.into_iter().map(str::to_owned).collect()),
        requires_python: meta_python.to_owned(),
        params: None,
        interpreter: String::new(),
        runner: String::new(),
        interpolate: true,
        needs: None,
        parameters: None,
        extra: Default::default(),
    };
    if mode == "reference" {
        fs::write(&meta.source, "print(1)\n")?;
    }
    Ok(Entry {
        slug: "demo".to_owned(),
        meta,
        dir: root.join("data/scripts/demo"),
    })
}

fn write_copy(entry: &Entry, body: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    fs::create_dir_all(&entry.dir)?;
    let path = entry.dir.join("script.py");
    fs::write(&path, body)?;
    Ok(path)
}

#[test]
fn copy_mode_falls_back_per_axis_to_stored_pep723() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let entry = entry(root.path(), "copy", None, "")?;
    write_copy(
        &entry,
        "# /// script\n# requires-python = \">=3.12\"\n# dependencies = [\"rich\"]\n# ///\nprint(1)\n",
    )?;
    assert_eq!(
        effective_uv_metadata(&entry),
        (vec!["rich".to_owned()], ">=3.12".to_owned())
    );
    Ok(())
}

#[test]
fn meta_wins_independently_per_axis() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let entry = entry(root.path(), "copy", Some(vec!["requests>=2"]), "")?;
    write_copy(
        &entry,
        "# /// script\n# requires-python = \">=3.13\"\n# dependencies = [\"rich\"]\n# ///\nprint(1)\n",
    )?;
    assert_eq!(
        effective_uv_metadata(&entry),
        (vec!["requests>=2".to_owned()], ">=3.13".to_owned())
    );

    let entry = entry(root.path(), "copy", None, ">=3.11")?;
    write_copy(
        &entry,
        "# /// script\n# requires-python = \">=3.13\"\n# dependencies = [\"rich\"]\n# ///\nprint(1)\n",
    )?;
    assert_eq!(
        effective_uv_metadata(&entry),
        (vec!["rich".to_owned()], ">=3.11".to_owned())
    );
    Ok(())
}

#[test]
fn reference_mode_never_reads_original_pep723_for_effective_axes()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let entry = entry(root.path(), "reference", None, "")?;
    fs::write(
        &entry.meta.source,
        "# /// script\n# requires-python = \">=3.13\"\n# dependencies = [\"rich\"]\n# ///\nprint(1)\n",
    )?;
    assert_eq!(effective_uv_metadata(&entry), (Vec::new(), String::new()));
    Ok(())
}

#[test]
fn unreadable_or_malformed_copy_keeps_meta_without_crashing()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let entry = entry(root.path(), "copy", Some(vec!["requests"]), "")?;
    assert_eq!(
        effective_uv_metadata(&entry),
        (vec!["requests".to_owned()], String::new())
    );

    write_copy(&entry, "# /// script\n# dependencies = [ broken\n# ///\n")?;
    assert_eq!(
        effective_uv_metadata(&entry),
        (vec!["requests".to_owned()], String::new())
    );
    Ok(())
}

#[test]
fn non_python_kind_never_consults_pep723() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let mut entry = entry(root.path(), "copy", None, "")?;
    entry.meta.kind = "shell".to_owned();
    fs::create_dir_all(&entry.dir)?;
    fs::write(
        entry.dir.join("script.sh"),
        "# /// script\n# dependencies = [\"rich\"]\n# ///\necho ok\n",
    )?;
    assert_eq!(effective_uv_metadata(&entry), (Vec::new(), String::new()));
    Ok(())
}
