//! Seed specifications and the profile seeding of one real walker host.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use serde_json::{Value, json};
use skit_application::{
    CreateEntry, LibraryService,
    form_state::{FormStateService, PersistedFormState},
    prompt_selection::PromptSelectionService,
};
use skit_domain::Slug;
use skit_store::{
    EntryCreateClock, FileConfigStore, FileFormStateStore, FilePromptSelectionStore, FileStore,
    PromptRunner,
};
use time::OffsetDateTime;

use super::{
    adapters::fixed_instant,
    observation::{has_store_owned_copy_payload, portable_mode},
};
use crate::cli::tui_host::{ProductRoots, set_private_directory_mode};

#[derive(Clone, Debug, Default)]
pub(crate) struct WalkerSeedSpec {
    pub(crate) profile: String,
    pub(crate) entries: Vec<CreateEntry>,
    pub(crate) settings: BTreeMap<String, String>,
    pub(crate) runners: Vec<PromptRunner>,
    pub(crate) forms: Vec<WalkerFormSeed>,
    pub(crate) prompt_runner: String,
    pub(crate) external: Vec<WalkerExternalSeed>,
    pub(crate) external_references: Vec<WalkerExternalReferenceSeed>,
    pub(crate) directories: Vec<WalkerDirectorySeed>,
    pub(crate) editor_writes: Vec<Vec<u8>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WalkerDirectoryRoot {
    Home,
    Cwd,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WalkerDirectorySeed {
    pub(crate) root: WalkerDirectoryRoot,
    pub(crate) path: PathBuf,
}

pub(super) trait DirectorySeedIo {
    fn create_dir(&self, path: &Path) -> io::Result<()>;
    fn symlink_metadata(&self, path: &Path) -> io::Result<fs::Metadata>;
    fn set_private_directory_mode(&self, path: &Path) -> io::Result<()>;
}

#[derive(Debug)]
pub(super) struct SystemDirectorySeedIo;

impl DirectorySeedIo for SystemDirectorySeedIo {
    fn create_dir(&self, path: &Path) -> io::Result<()> {
        fs::create_dir(path)
    }

    fn symlink_metadata(&self, path: &Path) -> io::Result<fs::Metadata> {
        fs::symlink_metadata(path)
    }

    fn set_private_directory_mode(&self, path: &Path) -> io::Result<()> {
        set_private_directory_mode(path)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum WalkerExternalSeed {
    File {
        path: PathBuf,
        bytes: Vec<u8>,
        readonly: bool,
        unix_mode: u32,
    },
    Symlink {
        path: PathBuf,
        target: PathBuf,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WalkerFilePickerTree {
    pub(crate) root: PathBuf,
    pub(crate) directories: BTreeSet<PathBuf>,
    pub(crate) files: BTreeSet<PathBuf>,
}

#[derive(Clone, Debug)]
pub(crate) struct WalkerExternalReferenceSeed {
    pub(crate) request: CreateEntry,
    pub(crate) source: PathBuf,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct WalkerFormSeed {
    pub(crate) selector: String,
    pub(crate) values: BTreeMap<String, String>,
    pub(crate) extra_args: Vec<String>,
    pub(crate) extra_args_raw: bool,
    pub(crate) preset: Option<String>,
    pub(crate) last_run: Option<WalkerLastRunSeed>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WalkerLastRunSeed {
    pub(crate) exit: i64,
    pub(crate) at: String,
    pub(crate) values: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Default)]
pub(super) struct WalkerCreateClock(AtomicU64);

impl EntryCreateClock for WalkerCreateClock {
    fn now_utc(&self) -> OffsetDateTime {
        let seconds = self.0.fetch_add(1, Ordering::Relaxed);
        let seconds = i64::try_from(seconds).unwrap_or(i64::MAX);
        fixed_instant() + time::Duration::seconds(seconds)
    }
}

pub(super) fn seed_profile(
    service: &LibraryService<FileStore>,
    roots: &ProductRoots,
    external_root: &Path,
    spec: &WalkerSeedSpec,
) -> Result<Vec<PathBuf>, String> {
    let mut copy_paths = Vec::new();
    for request in &spec.entries {
        let entry = service
            .add(request.clone())
            .map_err(|error| error.to_string())?;
        if has_store_owned_copy_payload(&entry) {
            let path = service
                .repository()
                .payload_path(&entry)
                .map_err(|error| error.to_string())?;
            copy_paths.push(path);
        }
    }
    for seed in &spec.external_references {
        let mut request = seed.request.clone();
        request.source = external_root.join(&seed.source).display().to_string();
        let entry = service.add(request).map_err(|error| error.to_string())?;
        if has_store_owned_copy_payload(&entry) {
            let path = service
                .repository()
                .payload_path(&entry)
                .map_err(|error| error.to_string())?;
            copy_paths.push(path);
        }
    }
    let config = FileConfigStore::new(&roots.config);
    config
        .set_many(&spec.settings)
        .map_err(|error| error.to_string())?;
    for runner in &spec.runners {
        config
            .set_runner(runner.clone(), true)
            .map_err(|error| error.to_string())?;
    }
    let forms = FormStateService::new(FileFormStateStore::new(&roots.state));
    for seed in &spec.forms {
        let entry = service
            .show(&seed.selector)
            .map_err(|error| error.to_string())?;
        let declarations = crate::cli::entry_parameters(service.repository(), &entry);
        forms
            .save_last(
                &entry.slug,
                &declarations,
                Some(&seed.values),
                Some(seed.extra_args.clone()),
                seed.extra_args_raw,
            )
            .map_err(|error| error.to_string())?;
        if let Some(preset) = &seed.preset {
            forms
                .save_preset(&entry.slug, preset, &declarations, &seed.values)
                .map_err(|error| error.to_string())?;
        }
        if let Some(last_run) = &seed.last_run {
            forms
                .record_run(
                    &entry.slug,
                    last_run.exit,
                    &last_run.at,
                    &declarations,
                    last_run.values.as_ref(),
                )
                .map_err(|error| error.to_string())?;
        }
    }
    if !spec.prompt_runner.is_empty() {
        PromptSelectionService::new(FilePromptSelectionStore::new(&roots.state))
            .remember_runner(&spec.prompt_runner)
            .map_err(|error| error.to_string())?;
    }
    Ok(copy_paths)
}

pub(super) fn validate_external_spec(spec: &WalkerSeedSpec) -> Result<(), String> {
    for directory in &spec.directories {
        validate_directory_seed_path(&directory.path)?;
    }
    let _ = validated_unique_external_seeds(&spec.external)?;
    for reference in &spec.external_references {
        validate_fixture_path(&reference.source)?;
    }
    Ok(())
}

fn validated_unique_external_seeds(
    seeds: &[WalkerExternalSeed],
) -> Result<Vec<&WalkerExternalSeed>, String> {
    let mut unique: Vec<&WalkerExternalSeed> = Vec::with_capacity(seeds.len());
    'seeds: for seed in seeds {
        let path = match seed {
            WalkerExternalSeed::File { path, .. } => {
                validate_fixture_path(path)?;
                path.as_path()
            }
            WalkerExternalSeed::Symlink { path, target } => {
                validate_fixture_path(path)?;
                validate_fixture_path(target)?;
                path.as_path()
            }
        };
        for previous in &unique {
            let previous_path = match previous {
                WalkerExternalSeed::File { path, .. }
                | WalkerExternalSeed::Symlink { path, .. } => path,
            };
            if path == previous_path {
                if seed == *previous {
                    continue 'seeds;
                }
                let same_kind = matches!(
                    (seed, *previous),
                    (
                        WalkerExternalSeed::File { .. },
                        WalkerExternalSeed::File { .. }
                    ) | (
                        WalkerExternalSeed::Symlink { .. },
                        WalkerExternalSeed::Symlink { .. }
                    )
                );
                if same_kind {
                    return Err(format!(
                        "walker external fixture seed payloads conflict at {}",
                        path.display()
                    ));
                }
            }
            if path == previous_path
                || path.starts_with(previous_path)
                || previous_path.starts_with(path)
            {
                let mut paths = [
                    previous_path.display().to_string(),
                    path.display().to_string(),
                ];
                paths.sort();
                return Err(format!(
                    "walker external fixture leaf paths overlap: {} and {}",
                    paths[0], paths[1]
                ));
            }
        }
        unique.push(seed);
    }
    Ok(unique)
}

fn validate_directory_seed_path(path: &Path) -> Result<(), String> {
    let mut components = path.components();
    if path.as_os_str().is_empty()
        || !components.all(|component| matches!(component, std::path::Component::Normal(_)))
        || !directory_seed_spelling_is_normal(path)
    {
        return Err(format!(
            "walker profile directory seed paths must be relative descendants: {}",
            path.display()
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn directory_seed_spelling_is_normal(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt as _;
    let bytes = path.as_os_str().as_bytes();
    !bytes
        .split(|byte| *byte == b'/')
        .any(|segment| segment.is_empty() || segment == b"." || segment == b"..")
}

#[cfg(windows)]
fn directory_seed_spelling_is_normal(path: &Path) -> bool {
    use std::os::windows::ffi::OsStrExt as _;
    let units = path.as_os_str().encode_wide().collect::<Vec<_>>();
    !units
        .split(|unit| *unit == u16::from(b'/') || *unit == u16::from(b'\\'))
        .any(|segment| {
            segment.is_empty()
                || segment == [u16::from(b'.')]
                || segment == [u16::from(b'.'), u16::from(b'.')]
        })
}

#[cfg(not(any(unix, windows)))]
fn directory_seed_spelling_is_normal(path: &Path) -> bool {
    let path = path.to_string_lossy();
    !path
        .split(std::path::MAIN_SEPARATOR)
        .any(|segment| segment.is_empty() || matches!(segment, "." | ".."))
}

fn validate_fixture_path(path: &Path) -> Result<(), String> {
    let mut components = path.components();
    if path.as_os_str().is_empty()
        || !components.all(|component| matches!(component, std::path::Component::Normal(_)))
    {
        return Err(format!(
            "walker external fixture paths must be relative descendants: {}",
            path.display()
        ));
    }
    Ok(())
}

pub(super) fn seed_profile_directories(
    roots: &ProductRoots,
    seeds: &[WalkerDirectorySeed],
    directory_seed_io: &dyn DirectorySeedIo,
) -> Result<Vec<PathBuf>, String> {
    let mut seeded = BTreeSet::new();
    for seed in seeds {
        let root = match seed.root {
            WalkerDirectoryRoot::Home => roots
                .home
                .as_deref()
                .expect("the walker profile has a home"),
            WalkerDirectoryRoot::Cwd => roots.cwd.as_path(),
        };
        let mut path = root.to_path_buf();
        for name in &seed.path {
            path.push(name);
            match directory_seed_io.create_dir(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    let metadata = directory_seed_io.symlink_metadata(&path).map_err(|error| {
                        format!(
                            "could not inspect walker profile directory seed {}: {error}",
                            path.display()
                        )
                    })?;
                    if metadata.file_type().is_symlink() || !metadata.is_dir() {
                        return Err(format!(
                            "walker profile directory seed is not a directory: {}",
                            path.display()
                        ));
                    }
                }
                Err(error) => {
                    return Err(format!(
                        "could not create walker profile directory seed {}: {error}",
                        path.display()
                    ));
                }
            }
            directory_seed_io
                .set_private_directory_mode(&path)
                .map_err(|error| {
                    format!(
                        "could not set private mode on walker profile directory seed {}: {error}",
                        path.display()
                    )
                })?;
            seeded.insert(path.clone());
        }
    }
    Ok(seeded.into_iter().collect())
}

pub(super) fn seed_external_world(
    root: &Path,
    seeds: &[WalkerExternalSeed],
) -> Result<WalkerFilePickerTree, String> {
    let mut tree = WalkerFilePickerTree {
        root: root.to_path_buf(),
        directories: BTreeSet::from([root.to_path_buf()]),
        files: BTreeSet::new(),
    };
    for seed in validated_unique_external_seeds(seeds)? {
        let seeded_path = match seed {
            WalkerExternalSeed::File {
                path,
                bytes,
                readonly,
                unix_mode,
            } => {
                let path = root.join(path);
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
                }
                fs::write(&path, bytes).map_err(|error| error.to_string())?;
                let mut permissions = fs::metadata(&path)
                    .map_err(|error| error.to_string())?
                    .permissions();
                permissions.set_readonly(*readonly);
                fs::set_permissions(&path, permissions).map_err(|error| error.to_string())?;
                set_unix_mode(&path, *unix_mode)?;
                tree.files.insert(path.clone());
                path
            }
            WalkerExternalSeed::Symlink { path, target } => {
                let path = root.join(path);
                create_fixture_symlink(&root.join(target), &path)?;
                path
            }
        };
        tree.directories.extend(
            seeded_path
                .ancestors()
                .skip(1)
                .take_while(|directory| directory.starts_with(root))
                .map(Path::to_path_buf),
        );
    }
    Ok(tree)
}

#[cfg(unix)]
pub(super) fn set_unix_mode(path: &Path, mode: u32) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).map_err(|error| error.to_string())
}

#[cfg(not(unix))]
pub(super) fn set_unix_mode(_path: &Path, _mode: u32) -> Result<(), String> {
    Ok(())
}

#[cfg(unix)]
fn create_fixture_symlink(target: &Path, link: &Path) -> Result<(), String> {
    if let Some(parent) = link.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    std::os::unix::fs::symlink(target, link).map_err(|error| error.to_string())
}

#[cfg(windows)]
fn create_fixture_symlink(target: &Path, link: &Path) -> Result<(), String> {
    if let Some(parent) = link.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    if target.is_dir() {
        std::os::windows::fs::symlink_dir(target, link).map_err(|error| error.to_string())
    } else {
        std::os::windows::fs::symlink_file(target, link).map_err(|error| error.to_string())
    }
}

#[cfg(not(any(unix, windows)))]
fn create_fixture_symlink(_target: &Path, _link: &Path) -> Result<(), String> {
    Err("symbolic-link fixtures are not supported on this platform".to_owned())
}

#[derive(Debug, PartialEq)]
pub(super) struct SeedSnapshot {
    scan: skit_application::LibraryScan,
    entries: Vec<skit_domain::Entry>,
    payloads: BTreeMap<Slug, SeedPayloadSnapshot>,
    settings: BTreeMap<String, String>,
    runners: Vec<PromptRunner>,
    runner_rows: Vec<skit_store::PromptRunnerRow>,
    mirror: skit_store::MirrorSettings,
    config_bytes: Option<Vec<u8>>,
    forms: BTreeMap<Slug, PersistedFormState>,
    prompt_runner: String,
}

#[derive(Debug, Eq, PartialEq)]
enum SeedPayloadSnapshot {
    NoPayload,
    Present {
        path: PathBuf,
        bytes: Vec<u8>,
        readonly: bool,
        mode: Option<u32>,
        symlink_target: Option<PathBuf>,
    },
}

pub(super) fn read_seed_snapshot(
    service: &LibraryService<FileStore>,
    roots: &ProductRoots,
) -> Result<SeedSnapshot, String> {
    let scan = service.list().map_err(|error| error.to_string())?;
    let config = FileConfigStore::new(&roots.config);
    let forms = FormStateService::new(FileFormStateStore::new(&roots.state));
    let mut entries = service
        .repository()
        .scan_entries()
        .map_err(|error| error.to_string())?;
    entries.sort_by(|left, right| left.slug.cmp(&right.slug));
    let payloads: BTreeMap<Slug, SeedPayloadSnapshot> = entries
        .iter()
        .map(|entry| {
            let payload = if entry.meta.kind.as_str() == "command" {
                SeedPayloadSnapshot::NoPayload
            } else {
                let path = service
                    .repository()
                    .payload_path(entry)
                    .map_err(|error| error.to_string())?;
                let metadata = fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
                let symlink_target = if metadata.file_type().is_symlink() {
                    Some(fs::read_link(&path).map_err(|error| error.to_string())?)
                } else {
                    None
                };
                SeedPayloadSnapshot::Present {
                    bytes: fs::read(&path).map_err(|error| error.to_string())?,
                    readonly: metadata.permissions().readonly(),
                    mode: portable_mode(&metadata),
                    symlink_target,
                    path,
                }
            };
            Ok((entry.slug.clone(), payload))
        })
        .collect::<Result<_, String>>()?;
    let form_state = scan
        .entries
        .iter()
        .map(|entry| (entry.slug.clone(), forms.load(&entry.slug)))
        .collect();
    Ok(SeedSnapshot {
        scan,
        entries,
        payloads,
        settings: config.settings().map_err(|error| error.to_string())?,
        runners: config.runners().map_err(|error| error.to_string())?,
        runner_rows: config.runner_rows().map_err(|error| error.to_string())?,
        mirror: config.mirror().map_err(|error| error.to_string())?,
        config_bytes: fs::read(roots.config.join("config.toml")).ok(),
        forms: form_state,
        prompt_runner: PromptSelectionService::new(FilePromptSelectionStore::new(&roots.state))
            .last_runner(),
    })
}

pub(super) fn persisted_form_json(state: &PersistedFormState) -> Value {
    json!({
        "values": state.values,
        "extra_args": state.extra_args,
        "extra_args_raw": state.extra_args_raw,
        "presets": state.presets,
        "last_run": {
            "at": state.last_run.at,
            "exit": state.last_run.exit,
            "values": state.last_run.values,
        },
    })
}
