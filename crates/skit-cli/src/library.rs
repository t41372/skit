//! Compose the application-owned Library surface from concrete adapters.

use std::path::Path;

use skit_application::{
    RepositoryError,
    library_detail::{LibrarySurface, LibrarySurfaceService},
};
use skit_form::FormLibraryProjector;
use skit_language::effective_entry_settings;
use skit_store::{FileConfigStore, FileFormStateStore, FileStore};
use time::OffsetDateTime;

/// Build one complete Library refresh from the configured product roots.
pub fn library_surface(
    store: &FileStore,
    state_dir: &Path,
    config_dir: &Path,
) -> Result<LibrarySurface, RepositoryError> {
    let form_state = FileFormStateStore::new(state_dir);
    let form_projector = FormLibraryProjector;
    let configured_runners = FileConfigStore::new(config_dir.to_path_buf())
        .runners()
        .unwrap_or_default()
        .into_iter()
        .map(|runner| runner.name)
        .collect::<Vec<_>>();
    LibrarySurfaceService::new(
        store,
        &form_state,
        &form_projector,
        effective_entry_settings,
    )
    .load_at(&configured_runners, OffsetDateTime::now_utc())
}
