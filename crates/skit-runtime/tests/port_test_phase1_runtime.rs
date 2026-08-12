//! Runtime-plan and private-uv ports from Python `tests/test_phase1.py` at `main@206f9ef`.
//! All uv tests are hermetic: asset construction is pure, the platform test only resolves the build
//! target, and the installer test pre-creates the managed binary so the network path must not run.

use std::{collections::BTreeMap, fs, path::{Path, PathBuf}};

use skit_application::delivery::Assembly;
use skit_domain::{Entry, EntryKind, EntryMeta, EntrySettings, Slug};
use skit_runtime::{
    LaunchError, LaunchPaths, ProgramProbe, UV_VERSION, UvTarget, build_launch_preview,
    ensure_managed_uv, managed_uv_path, uv_asset,
};
use tempfile::TempDir;

#[derive(Clone, Copy, Debug)]
struct PreviewFs;

impl ProgramProbe for PreviewFs {
    fn find_program(&self, name: &str) -> Option<PathBuf> {
        Some(PathBuf::from(name))
    }

    fn is_file(&self, _path: &Path) -> bool {
        true
    }

    fn is_dir(&self, _path: &Path) -> bool {
        true
    }

    fn exists(&self, _path: &Path) -> bool {
        true
    }

    fn is_executable(&self, _path: &Path) -> bool {
        true
    }
}

fn entry(kind: &str, name: &str, settings: EntrySettings) -> Entry {
    let kind = EntryKind::parse(kind).unwrap();
    let mut meta = EntryMeta::minimal(name, kind);
    meta.workdir = "invoke".to_owned();
    settings.write_to_meta(&mut meta);
    Entry {
        slug: Slug::from_display_name(name),
        meta,
    }
}

fn paths(script: &str) -> LaunchPaths {
    LaunchPaths {
        script: PathBuf::from(script),
        entry_dir: PathBuf::from("entry"),
        invoke_cwd: PathBuf::from("invoke"),
    }
}

fn command_text(plan: &skit_runtime::LaunchPlan) -> &str {
    plan.args
        .last()
        .expect("command launch passes the filled command as the final shell argument")
}

#[test]
fn test_build_command_reference_deps() {
    let settings = EntrySettings {
        dependencies: vec!["requests".to_owned(), "rich".to_owned()],
        requires_python: ">=3.11".to_owned(),
        ..EntrySettings::default()
    };
    let entry = entry("python", "ref", settings);

    let plan = build_launch_preview(
        &entry,
        &paths("/source/s.py"),
        &Assembly::default(),
        None,
        None,
        None,
        &PreviewFs,
    )
    .unwrap();

    assert_eq!(plan.program, PathBuf::from("uv"));
    assert_eq!(&plan.args[..4], ["run", "--no-project", "--python", ">=3.11"]);
    assert_eq!(plan.args.iter().filter(|arg| *arg == "--with").count(), 2);
    assert!(plan.args.windows(2).any(|pair| pair == ["--with", "requests"]));
    assert!(plan.args.windows(2).any(|pair| pair == ["--with", "rich"]));
    assert!(plan.args.iter().any(|arg| arg == "--script"));
    assert!(plan.args.iter().any(|arg| arg == "/source/s.py"));
}

#[test]
fn test_command_params_fill_and_escape() {
    let settings = EntrySettings {
        template: "convert {src} to {dst} keep {{braces}}".to_owned(),
        params: vec!["src".to_owned(), "dst".to_owned()],
        ..EntrySettings::default()
    };
    let entry = entry("command", "conv", settings);
    let values = BTreeMap::from([
        ("src".to_owned(), "a.png".to_owned()),
        ("dst".to_owned(), "b.jpg".to_owned()),
    ]);
    let assembly = Assembly {
        command_values: values.clone(),
        masked_command_values: values,
        ..Assembly::default()
    };

    let plan = build_launch_preview(
        &entry,
        &paths(""),
        &assembly,
        None,
        None,
        None,
        &PreviewFs,
    )
    .unwrap();

    assert_eq!(command_text(&plan), "convert a.png to b.jpg keep {braces}");
}

#[test]
fn test_command_missing_values_raises() {
    let settings = EntrySettings {
        template: "echo {x}".to_owned(),
        params: vec!["x".to_owned()],
        ..EntrySettings::default()
    };
    let entry = entry("command", "e", settings);

    let error = build_launch_preview(
        &entry,
        &paths(""),
        &Assembly::default(),
        None,
        None,
        None,
        &PreviewFs,
    )
    .unwrap_err();

    assert!(matches!(
        error,
        LaunchError::MissingTemplateValue { name } if name == "x"
    ));
}

#[test]
fn test_uv_download_url_shape() {
    let linux = UvTarget::from_parts("x86_64", "linux", false).unwrap();
    let linux_asset = uv_asset(&linux, None);
    assert!(linux_asset.url.starts_with("https://github.com/astral-sh/uv/releases/download/"));
    assert!(linux_asset.url.contains(UV_VERSION));
    assert!(linux_asset.url.ends_with("uv-x86_64-unknown-linux-gnu.tar.gz"));

    let windows = UvTarget::from_parts("x86_64", "windows", false).unwrap();
    let windows_asset = uv_asset(&windows, None);
    assert!(windows_asset.url.ends_with("uv-x86_64-pc-windows-msvc.zip"));
    assert_eq!(windows_asset.executable_name, "uv.exe");
}

#[test]
fn test_uv_triple_current_platform() {
    let triple = UvTarget::current().unwrap().triple().to_owned();
    assert!(
        ["linux", "darwin", "windows"].iter().any(|token| triple.contains(token)),
        "current uv target is not a supported OS triple: {triple}"
    );
}

#[test]
fn test_ensure_uv_downloaded_skips_when_present() {
    let data = TempDir::new().unwrap();
    let managed = managed_uv_path(data.path());
    fs::create_dir_all(managed.parent().unwrap()).unwrap();
    fs::write(&managed, b"already installed\n").unwrap();
    let before = fs::read(&managed).unwrap();

    let resolved = ensure_managed_uv(data.path(), None).unwrap();

    assert_eq!(resolved, managed);
    assert_eq!(fs::read(&resolved).unwrap(), before, "existing private uv was rewritten");
}
