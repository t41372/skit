//! One fresh-process headless TUI probe.

use std::{fs, path::Path, time::Instant};

use ratatui_core::{backend::TestBackend, terminal::Terminal};
use ratatui_crossterm::crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use serde::{Deserialize, Serialize};
use skit_application::form_state::FormStateService;
use skit_store::{FileFormStateStore, FileStore};
use skit_tui::{EventHandling, TuiSession, ViewGeometry};
use skit_ui::{Action, LibraryState};
use thiserror::Error;

/// Measured spans and the raw process status used for peak RSS.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ProbeResult {
    /// Store scan, reducer construction, and initial render.
    pub first_idle_ms: f64,
    /// One selection move and repaint. Empty below two entries.
    pub select_ms: Option<f64>,
    /// One character filter and repaint.
    pub search_ms: f64,
    /// Linux process status captured after rendering.
    pub status_text: Option<String>,
}

impl ProbeResult {
    fn new(
        first_idle_ms: f64,
        select_ms: Option<f64>,
        search_ms: f64,
        status_text: Option<String>,
    ) -> Result<Self, TuiProbeError> {
        if !first_idle_ms.is_finite()
            || first_idle_ms < 0.0
            || !search_ms.is_finite()
            || search_ms < 0.0
            || select_ms.is_some_and(|value| !value.is_finite() || value < 0.0)
        {
            return Err(TuiProbeError::NonFinite);
        }
        Ok(Self {
            first_idle_ms,
            select_ms,
            search_ms,
            status_text,
        })
    }

    /// Validate a decoded child payload before it becomes a measurement.
    pub fn validate(&self) -> Result<(), TuiProbeError> {
        Self::new(
            self.first_idle_ms,
            self.select_ms,
            self.search_ms,
            self.status_text.clone(),
        )
        .map(|_| ())
    }
}

/// A headless probe did not exercise the promised interaction.
#[derive(Debug, Error)]
pub enum TuiProbeError {
    /// The dataset environment was absent.
    #[error("SKIT_DATA_DIR is not set; refusing to probe the default library")]
    MissingDataset,
    /// The state directory was absent.
    #[error("SKIT_STATE_DIR is not set; refusing to probe the default state")]
    MissingState,
    /// Product store read failed.
    #[error("could not scan the benchmark library: {0}")]
    Store(String),
    /// The selected dataset and requested cardinality differ.
    #[error("expected {expected} entries, saw {actual}")]
    EntryCount {
        /// Expected count.
        expected: usize,
        /// Actual count.
        actual: usize,
    },
    /// A generated dataset contained diagnostics.
    #[error("benchmark dataset contains {0} scan diagnostics")]
    Diagnostics(usize),
    /// The reducer ignored a promised selection move.
    #[error("selection did not move")]
    SelectionNoop,
    /// The reducer ignored the search input.
    #[error("search input did not reach the reducer")]
    SearchInputNoop,
    /// Search did not remove any row.
    #[error("search did not remove rows")]
    SearchNoop,
    /// Search removed every row from a dataset that guarantees matches.
    #[error("search removed every row")]
    SearchEmpty,
    /// The real terminal input path ignored a required benchmark event.
    #[error("the TUI input path ignored the required {0} event")]
    InputIgnored(&'static str),
    /// A measured span was not finite.
    #[error("TUI probe produced a non-finite span")]
    NonFinite,
}

/// Run the probe against the dataset selected by `SKIT_DATA_DIR`.
pub fn run(entries: usize, probe_char: char) -> Result<ProbeResult, TuiProbeError> {
    run_from_environment(
        std::env::var_os("SKIT_DATA_DIR").map(std::path::PathBuf::from),
        std::env::var_os("SKIT_STATE_DIR").map(std::path::PathBuf::from),
        entries,
        probe_char,
    )
}

fn run_from_environment(
    data: Option<std::path::PathBuf>,
    state: Option<std::path::PathBuf>,
    entries: usize,
    probe_char: char,
) -> Result<ProbeResult, TuiProbeError> {
    let data = data.ok_or(TuiProbeError::MissingDataset)?;
    let state = state.ok_or(TuiProbeError::MissingState)?;
    // The dataset environment always sets a configuration directory
    // (`crates/skit-benchmarks/src/environment.rs:131-133`); an absent one only means no runner is
    // configured, which is what a fresh install looks like.
    let config = std::env::var_os("SKIT_CONFIG_DIR")
        .map_or_else(|| data.join("config"), std::path::PathBuf::from);
    run_for_dirs(&data, &state, &config, entries, probe_char)
}

/// Run the probe against explicit product data and state directories.
pub fn run_for_dirs(
    data: &Path,
    state_dir: &Path,
    config_dir: &Path,
    entries: usize,
    probe_char: char,
) -> Result<ProbeResult, TuiProbeError> {
    let started = Instant::now();
    let store = FileStore::new(data);
    // The product builds this exact projection before it draws its first frame, so the probe must
    // build it too. A scan-only measurement under-reports the work a real start does.
    let surface = skit_store::library_surface(&store, state_dir, config_dir)
        .map_err(|error| TuiProbeError::Store(error.to_string()))?;
    if !surface.scan.diagnostics.is_empty() {
        return Err(TuiProbeError::Diagnostics(surface.scan.diagnostics.len()));
    }
    let form_state = FormStateService::new(FileFormStateStore::new(state_dir));
    let rerunnable = surface
        .scan
        .entries
        .iter()
        .filter_map(|entry| {
            let last_run = form_state.last_run(&entry.slug);
            (last_run.at.is_some() || last_run.exit.is_some() || last_run.values.is_some())
                .then(|| entry.slug.clone())
        })
        .collect();
    let mut state = LibraryState::from_library_surface(surface);
    state.update(Action::ReplaceRerunnable(rerunnable));
    if state.entry_count() != entries {
        return Err(TuiProbeError::EntryCount {
            expected: entries,
            actual: state.entry_count(),
        });
    }
    let backend = TestBackend::new(120, 40);
    let mut terminal = Terminal::new(backend).unwrap_or_else(|never| match never {});
    let mut session = TuiSession::default();
    let mut geometry = draw(&mut terminal, &state, &mut session);
    let first_idle_ms = started.elapsed().as_secs_f64() * 1_000.0;

    let select_ms = if entries >= 2 {
        let before = state.selected_visible_index();
        let started = Instant::now();
        dispatch_key(
            &mut state,
            &mut session,
            &geometry,
            KeyCode::Down,
            "selection",
        )?;
        geometry = draw(&mut terminal, &state, &mut session);
        let elapsed = started.elapsed().as_secs_f64() * 1_000.0;
        validate_selection(before, state.selected_visible_index())?;
        Some(elapsed)
    } else {
        None
    };

    dispatch_key(
        &mut state,
        &mut session,
        &geometry,
        KeyCode::Char('/'),
        "begin-search",
    )?;
    geometry = draw(&mut terminal, &state, &mut session);
    let started = Instant::now();
    dispatch_key(
        &mut state,
        &mut session,
        &geometry,
        KeyCode::Char(probe_char),
        "search-input",
    )?;
    let _ = draw(&mut terminal, &state, &mut session);
    let search_ms = started.elapsed().as_secs_f64() * 1_000.0;
    validate_search(
        entries,
        state.query(),
        state.visible_entry_count(),
        probe_char,
    )?;
    let status = Path::new("/proc/self/status");
    let status_text = status
        .is_file()
        .then(|| fs::read_to_string(status).ok())
        .flatten();
    ProbeResult::new(first_idle_ms, select_ms, search_ms, status_text)
}

fn validate_selection(before: Option<usize>, after: Option<usize>) -> Result<(), TuiProbeError> {
    if after == before {
        Err(TuiProbeError::SelectionNoop)
    } else {
        Ok(())
    }
}

fn validate_search(
    entries: usize,
    query: &str,
    visible_entries: usize,
    probe_char: char,
) -> Result<(), TuiProbeError> {
    if query != probe_char.to_string() {
        return Err(TuiProbeError::SearchInputNoop);
    }
    if entries > 0 && visible_entries >= entries {
        return Err(TuiProbeError::SearchNoop);
    }
    if entries >= 3 && visible_entries == 0 {
        return Err(TuiProbeError::SearchEmpty);
    }
    Ok(())
}

fn draw(
    terminal: &mut Terminal<TestBackend>,
    state: &LibraryState,
    session: &mut TuiSession,
) -> ViewGeometry {
    let mut geometry = ViewGeometry::default();
    terminal
        .draw(|frame| {
            geometry = skit_tui::render_with_session(frame, state, skit_i18n::Locale::En, session);
        })
        .unwrap_or_else(|never| match never {});
    geometry
}

fn dispatch_key(
    state: &mut LibraryState,
    session: &mut TuiSession,
    geometry: &ViewGeometry,
    code: KeyCode,
    label: &'static str,
) -> Result<(), TuiProbeError> {
    let event = Event::Key(KeyEvent::new(code, KeyModifiers::NONE));
    let EventHandling::Action(action) = session.handle_event(event, state, geometry) else {
        return Err(TuiProbeError::InputIgnored(label));
    };
    state.update(action);
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use crate::dataset::{DEFAULT_SEED, DEFAULT_STATE_FRACTION, dataset_dirs, generate};

    #[test]
    fn probe_document_rejects_non_finite_spans() {
        assert!(super::ProbeResult::new(f64::NAN, None, 1.0, None).is_err());
        assert!(super::ProbeResult::new(1.0, Some(f64::INFINITY), 3.0, None).is_err());
        assert!(super::ProbeResult::new(1.0, None, f64::NEG_INFINITY, None).is_err());
        assert!(super::ProbeResult::new(-1.0, None, 1.0, None).is_err());
        assert!(super::ProbeResult::new(1.0, Some(-1.0), 1.0, None).is_err());
        assert!(super::ProbeResult::new(1.0, None, -1.0, None).is_err());
        assert!(super::ProbeResult::new(1.0, Some(2.0), 3.0, None).is_ok());
    }

    #[test]
    fn interaction_postconditions_name_each_broken_span() {
        assert!(super::validate_selection(Some(1), Some(2)).is_ok());
        assert!(matches!(
            super::validate_selection(Some(1), Some(1)),
            Err(super::TuiProbeError::SelectionNoop)
        ));
        assert!(super::validate_search(3, "o", 1, 'o').is_ok());
        assert!(matches!(
            super::validate_search(3, "x", 1, 'o'),
            Err(super::TuiProbeError::SearchInputNoop)
        ));
        assert!(matches!(
            super::validate_search(3, "o", 3, 'o'),
            Err(super::TuiProbeError::SearchNoop)
        ));
        assert!(matches!(
            super::validate_search(3, "o", 0, 'o'),
            Err(super::TuiProbeError::SearchEmpty)
        ));
    }

    #[test]
    fn environment_front_door_requires_both_product_roots() {
        assert!(matches!(
            super::run_from_environment(None, None, 0, 'o'),
            Err(super::TuiProbeError::MissingDataset)
        ));
        assert!(matches!(
            super::run_from_environment(Some("data".into()), None, 0, 'o'),
            Err(super::TuiProbeError::MissingState)
        ));
    }

    #[test]
    fn generated_datasets_drive_the_real_reducer_renderer_and_input_path() {
        let root = TempDir::new().unwrap();
        let manifest = generate(
            &root.path().join("dataset"),
            3,
            DEFAULT_SEED,
            DEFAULT_STATE_FRACTION,
        )
        .unwrap();
        let dirs = dataset_dirs(&manifest.root).unwrap();
        let probe = super::run_for_dirs(&dirs.data, &dirs.state, &dirs.config, 3, 'o').unwrap();
        probe.validate().unwrap();
        assert!(probe.select_ms.is_some());

        let empty = generate(
            &root.path().join("empty"),
            0,
            DEFAULT_SEED,
            DEFAULT_STATE_FRACTION,
        )
        .unwrap();
        let empty_dirs = dataset_dirs(&empty.root).unwrap();
        let empty_probe = super::run_for_dirs(
            &empty_dirs.data,
            &empty_dirs.state,
            &empty_dirs.config,
            0,
            'o',
        )
        .unwrap();
        assert_eq!(empty_probe.select_ms, None);

        assert!(matches!(
            super::run_for_dirs(&dirs.data, &dirs.state, &dirs.config, 2, 'o'),
            Err(super::TuiProbeError::EntryCount {
                expected: 2,
                actual: 3
            })
        ));

        let store = skit_store::FileStore::new(&dirs.data);
        let surface = skit_store::library_surface(&store, &dirs.state, &dirs.config).unwrap();
        let mut state = skit_ui::LibraryState::from_library_surface(surface);
        let backend = ratatui_core::backend::TestBackend::new(120, 40);
        let mut terminal =
            ratatui_core::terminal::Terminal::new(backend).unwrap_or_else(|never| match never {});
        let mut session = skit_tui::TuiSession::default();
        let geometry = super::draw(&mut terminal, &state, &mut session);
        assert!(matches!(
            super::dispatch_key(
                &mut state,
                &mut session,
                &geometry,
                ratatui_crossterm::crossterm::event::KeyCode::F(12),
                "unsupported-key"
            ),
            Err(super::TuiProbeError::InputIgnored("unsupported-key"))
        ));

        fs::write(
            dirs.data
                .join("scripts")
                .join(manifest.slugs[0].as_str())
                .join("meta.toml"),
            "not = [valid",
        )
        .unwrap();
        assert!(matches!(
            super::run_for_dirs(&dirs.data, &dirs.state, &dirs.config, 3, 'o'),
            Err(super::TuiProbeError::Diagnostics(1))
        ));
    }
}
