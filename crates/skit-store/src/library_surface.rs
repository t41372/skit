//! Project one complete Library refresh from the stored library.
//!
//! Version 0.4 reads the same facts for the same screen: the list sorts on recency and marks a
//! missing target (`src/skit/tui.py:394` and `:414`), and the detail pane reports parameters,
//! presets, dependencies, the last run, and drift (`src/skit/tui.py:531-604`). One projection keeps
//! both faces on one read, so they cannot disagree.
//!
//! This lives beside the storage adapters rather than in a composition root because every frontend
//! host needs the same answer: the CLI workbench and the headless benchmark probe both call it, and
//! a second copy would drift from this one.

use std::path::{Path, PathBuf};

use skit_application::{
    EntryRepository as _, LibraryScan, RepositoryError,
    form_state::{FormStateService, LastRunState, prefill},
    library_detail::{
        LibraryEntryDetail, LibraryLastRun, LibraryParameterDetail, LibraryPromptRunner,
        LibraryRunAge, LibrarySurface,
    },
};
use skit_domain::{Entry, EntrySettings, StorageMode};
use skit_form::{FormPlan, form_plan};
use skit_language::{LosslessSource, UvMetadata, effective_uv_metadata_bytes};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::{
    FileConfigStore, FileFormStateStore, FileStore, PromptRunner, paths::stored_filenames,
};

/// Build the Library list rows and every detail fact in one pass.
///
/// A configuration this read cannot parse must not hide the library, so an unreadable runner list
/// only means "no runner is configured" for the pin check.
pub fn library_surface(
    store: &FileStore,
    state_dir: &Path,
    config_dir: &Path,
) -> Result<LibrarySurface, RepositoryError> {
    let scan = store.scan()?;
    library_surface_at(
        store,
        state_dir,
        config_dir,
        scan,
        OffsetDateTime::now_utc(),
    )
}

/// Build the projection against one explicit clock reading.
///
/// The relative age of a run is resolved once per refresh, so every row on one screen agrees about
/// what "now" was.
pub fn library_surface_at(
    store: &FileStore,
    state_dir: &Path,
    config_dir: &Path,
    scan: LibraryScan,
    now: OffsetDateTime,
) -> Result<LibrarySurface, RepositoryError> {
    let form_state = FormStateService::new(FileFormStateStore::new(state_dir));
    let runners = FileConfigStore::new(config_dir.to_path_buf())
        .runners()
        .unwrap_or_default();
    // One directory pass, exactly like version 0.4's `store.list_entries()` (`src/skit/store.py:857`).
    // Resolving each slug instead would re-read the registry once per entry.
    let details = store
        .scan_entries()?
        .into_iter()
        .map(|entry| {
            let detail = entry_detail(store, &form_state, &runners, now, &entry);
            (entry.slug, detail)
        })
        .collect();
    Ok(LibrarySurface { scan, details })
}

/// Project every Library detail fact for one entry.
fn entry_detail(
    store: &FileStore,
    form_state: &FormStateService<FileFormStateStore>,
    runners: &[PromptRunner],
    now: OffsetDateTime,
    entry: &Entry,
) -> LibraryEntryDetail {
    let kind = entry.meta.kind.as_str();
    let settings = effective_settings(store, entry);
    let persisted = form_state.load(&entry.slug);
    // Version 0.4 builds one plan per entry and reads both the parameter summary and the drift
    // notice from it (`src/skit/tui.py:561` and `:500`), so the two faces cannot disagree and the
    // source is parsed once.
    let plan = detail_form_plan(store, entry);
    let declarations = plan.declarations();
    // The pane shows the value a run would start from: the definition default under the last used
    // value, secrets excluded (`src/skit/tui.py:566-569`).
    let values = prefill(&declarations, &persisted.values, None);
    let parameters = declarations
        .iter()
        .map(|declaration| LibraryParameterDetail {
            key: declaration.name.clone(),
            value: values.get(&declaration.name).cloned().unwrap_or_default(),
            secret: declaration.secret,
        })
        .collect();
    let target = launch_target(store, entry);
    LibraryEntryDetail {
        added_at: entry.meta.added_at.clone(),
        // Only the command-template family shows its launch material here
        // (`src/skit/tui.py:544-545`).
        template: (kind == "command" && !settings.template.is_empty())
            .then(|| settings.template.clone()),
        prompt_runner: (kind == "prompt").then(|| prompt_runner(&settings.runner, runners)),
        parameters,
        presets: persisted.presets.keys().cloned().collect(),
        dependencies: settings.dependencies.clone(),
        last_run: last_run(now, &persisted.last_run),
        missing_target: target
            .filter(|path| !path.exists())
            .map(|path| path.display().to_string()),
        drifted: !plan.drift.is_empty(),
        original_file_preserved: original_file_preserved(entry),
    }
}

/// Build one form plan for the Library detail pane.
///
/// The stored source is read once and normalized the same way the health check normalizes it, so a
/// line-ending difference is never reported as drift.
fn detail_form_plan(store: &FileStore, entry: &Entry) -> FormPlan {
    let kind = entry.meta.kind.as_str();
    let bytes = store
        .payload_path(entry)
        .ok()
        .and_then(|path| std::fs::read(path).ok());
    // A kind with no stored payload still has a form: a command template and a declared schema
    // both live in the metadata, so an absent source means an empty source, never an empty plan.
    let source = match bytes {
        Some(bytes) if kind == "prompt" => String::from_utf8(bytes).unwrap_or_default(),
        Some(bytes) => LosslessSource::from_bytes(&bytes)
            .normalized_text()
            .to_owned(),
        None => String::new(),
    };
    form_plan(kind, &source, &EntrySettings::from_meta(&entry.meta))
}

/// Read the effective dependency metadata a run would enforce.
fn effective_settings(store: &FileStore, entry: &Entry) -> EntrySettings {
    let mut settings = EntrySettings::from_meta(&entry.meta);
    if entry.meta.kind.as_str() == "python" && entry.meta.mode == StorageMode::Copy {
        let source = store
            .payload_path(entry)
            .ok()
            .and_then(|path| std::fs::read(path).ok());
        let effective = effective_uv_metadata_bytes(
            source.as_deref(),
            &UvMetadata {
                dependencies: settings.dependencies.clone(),
                requires_python: settings.requires_python.clone(),
            },
        );
        settings.dependencies = effective.dependencies;
        settings.requires_python = effective.requires_python;
    }
    settings
}

/// Classify one prompt entry's runner pin.
///
/// Version 0.4 is honest about a pin whose configuration row is gone (`src/skit/tui.py:74-84`).
fn prompt_runner(pin: &str, runners: &[PromptRunner]) -> LibraryPromptRunner {
    if pin.is_empty() {
        return LibraryPromptRunner::PickOnRunForm;
    }
    if runners.iter().any(|runner| runner.name == pin) {
        LibraryPromptRunner::Configured(pin.to_owned())
    } else {
        LibraryPromptRunner::Missing(pin.to_owned())
    }
}

/// Report whether removal can truthfully promise that an original file survives.
///
/// Version 0.4 says so only for a kind that has an original file and only while that file still
/// exists: a drafted entry's "original" was a temporary file, and reassuring the user about a file
/// that is already gone would be a lie (`src/skit/tui.py:149-155`).
fn original_file_preserved(entry: &Entry) -> bool {
    let kind = entry.meta.kind.as_str();
    let has_original_file = !known_entry_kind(kind) || kind != "command";
    has_original_file && !entry.meta.source.is_empty() && Path::new(&entry.meta.source).exists()
}

/// Project the last recorded run with the relative age already resolved.
fn last_run(now: OffsetDateTime, last_run: &LastRunState) -> Option<LibraryLastRun> {
    let at = last_run.at.clone()?;
    // Version 0.4 keeps an unparseable legacy stamp exactly as written
    // (`src/skit/tui.py:105-108`).
    let elapsed = OffsetDateTime::parse(&at, &Rfc3339)
        .ok()
        .map(|then| (now - then).whole_seconds());
    Some(LibraryLastRun {
        age: LibraryRunAge::from_elapsed(at.clone(), elapsed),
        at,
        exit: last_run.exit,
    })
}

/// Resolve the path a launch would use, or `None` when the kind is not file-backed.
fn launch_target(store: &FileStore, entry: &Entry) -> Option<PathBuf> {
    let kind = entry.meta.kind.as_str();
    if !known_entry_kind(kind) || kind == "command" {
        return None;
    }
    if entry.meta.mode == StorageMode::Reference || kind == "exe" {
        return Some(if entry.meta.source.is_empty() {
            PathBuf::from(".")
        } else {
            PathBuf::from(&entry.meta.source)
        });
    }
    let directory = store.entry_dir_path(&entry.slug);
    let names = stored_filenames(kind);
    let canonical = names
        .iter()
        .map(|name| directory.join(name))
        .find(|path| path.exists())
        .or_else(|| names.first().map(|name| directory.join(name)))?;
    if canonical.exists() {
        return Some(canonical);
    }
    store.payload_path(entry).ok().or(Some(canonical))
}

/// Report whether this kind is one the shipped product knows how to launch.
fn known_entry_kind(kind: &str) -> bool {
    matches!(
        kind,
        "python"
            | "shell"
            | "fish"
            | "js"
            | "ts"
            | "powershell"
            | "ruby"
            | "perl"
            | "lua"
            | "r"
            | "exe"
            | "command"
            | "prompt"
    )
}

#[cfg(test)]
mod tests {
    use skit_application::library_detail::LibraryRunAge;

    use super::*;

    fn configured() -> Vec<PromptRunner> {
        vec![PromptRunner {
            name: "claude".to_owned(),
            argv: vec!["claude".to_owned(), "{{prompt}}".to_owned()],
        }]
    }

    /// A pinned prompt runner that no configuration row defines must say so.
    ///
    /// Version 0.4 keeps the detail pane honest about a pin whose row is gone
    /// (`src/skit/tui.py:74-84`).
    #[test]
    fn a_prompt_runner_pin_is_classified_against_the_configuration() {
        let configured = configured();
        assert_eq!(
            prompt_runner("", &configured),
            LibraryPromptRunner::PickOnRunForm
        );
        assert_eq!(
            prompt_runner("claude", &configured),
            LibraryPromptRunner::Configured("claude".to_owned())
        );
        assert_eq!(
            prompt_runner("removed", &configured),
            LibraryPromptRunner::Missing("removed".to_owned())
        );
    }

    /// Version 0.4 keeps an unparseable legacy stamp exactly as written and otherwise reports a
    /// relative age (`src/skit/tui.py:105-116`).
    #[test]
    fn the_last_run_age_is_resolved_once_per_refresh() {
        let now = OffsetDateTime::parse("2026-08-09T12:00:00Z", &Rfc3339).unwrap();
        let at = |value: &str| LastRunState {
            at: Some(value.to_owned()),
            exit: Some(0),
            values: None,
        };
        assert!(last_run(now, &LastRunState::default()).is_none());
        assert_eq!(
            last_run(now, &at("2026-08-09T11:59:30Z")).unwrap().age,
            LibraryRunAge::JustNow
        );
        assert_eq!(
            last_run(now, &at("2026-08-09T11:00:00Z")).unwrap().age,
            LibraryRunAge::Minutes(60)
        );
        assert_eq!(
            last_run(now, &at("2026-08-08T12:00:00Z")).unwrap().age,
            LibraryRunAge::Hours(24)
        );
        assert_eq!(
            last_run(now, &at("2026-07-09T12:00:00Z")).unwrap().age,
            LibraryRunAge::Days(31)
        );
        assert_eq!(
            last_run(now, &at("not a timestamp")).unwrap().age,
            LibraryRunAge::Raw("not a timestamp".to_owned())
        );
    }

    /// Only a kind that keeps an original file, and only while that file exists, earns the
    /// removal reassurance (`src/skit/tui.py:149-155`).
    #[test]
    fn the_removal_reassurance_needs_an_original_that_still_exists() {
        use skit_domain::{EntryKind, EntryMeta, Slug};
        let entry = |kind: &str, source: &str| Entry {
            slug: Slug::parse("demo").unwrap(),
            meta: EntryMeta {
                source: source.to_owned(),
                ..EntryMeta::minimal("Demo", EntryKind::parse(kind).unwrap())
            },
        };
        let existing = std::env::current_exe().unwrap().display().to_string();
        assert!(original_file_preserved(&entry("shell", &existing)));
        // A command template never had an original file.
        assert!(!original_file_preserved(&entry("command", &existing)));
        // A drafted entry's original was a temporary file that is already gone.
        assert!(!original_file_preserved(&entry(
            "shell",
            "/nonexistent/draft.sh"
        )));
        assert!(!original_file_preserved(&entry("shell", "")));
    }
}
