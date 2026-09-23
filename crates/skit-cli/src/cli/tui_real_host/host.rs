//! The seeded production host that the walker drives.

use std::{
    cell::{Cell, RefCell},
    fs, io,
    path::{Path, PathBuf},
    rc::Rc,
    sync::Arc,
};

use serde_json::{Value, json};
use skit_application::{
    LibraryService, form_state::FormStateService, prompt_selection::PromptSelectionService,
};
use skit_i18n::{Locale, Localize, Message, requested_locale};
use skit_store::{FileConfigStore, FileFormStateStore, FilePromptSelectionStore, FileStore};
use skit_tui_walker_support::{
    engine::CheckpointCauseProjection,
    sandbox::{SafeProfileId, SandboxMetadata},
};
use skit_ui::{Action, Effect, LibraryState};

use super::{
    adapters::{AllocationMaximums, FixedClock, LaunchScript, RecordingAdapters},
    observation::{
        HostObservation, LeakOracleFacts, ObservationModeProvenance, PendingHostObservation,
        ensure_system_temp_empty, snapshot_tree, sorted_tui_drafts,
    },
    projection::PathMap,
    seed::{
        DirectorySeedIo, SystemDirectorySeedIo, WalkerCreateClock, WalkerFilePickerTree,
        WalkerSeedSpec, persisted_form_json, read_seed_snapshot, seed_external_world, seed_profile,
        seed_profile_directories, validate_external_spec,
    },
    stable_namespace::{
        SandboxError, SandboxFaults, StableHostError, StableHostPrimaryError,
        StableSandboxNamespace,
    },
    stable_sandbox::{SandboxOwner, StableSandbox},
};
use crate::{
    cli::tui_host::{ProductRoots, TuiHost},
    library::library_surface_at,
    run::RunClock,
};

#[derive(Debug)]
pub(crate) struct SeededTuiHost {
    pub(super) _sandbox: SandboxOwner,
    roots: ProductRoots,
    pub(super) external_root: PathBuf,
    pub(super) file_picker_tree: WalkerFilePickerTree,
    pub(super) service: LibraryService<FileStore>,
    locale: Cell<Locale>,
    clock: FixedClock,
    pub(super) adapters: RecordingAdapters,
    pub(super) path_map: PathMap,
    pub(super) observation_modes: RefCell<ObservationModeProvenance>,
    pub(super) profile: String,
}

pub(crate) type RealWalkerHost = SeededTuiHost;

impl SeededTuiHost {
    pub(crate) fn spawn(spec: WalkerSeedSpec) -> Result<Self, String> {
        Self::spawn_with_allocation_maximums(spec, AllocationMaximums::default())
    }

    pub(crate) fn spawn_stable_in(
        spec: WalkerSeedSpec,
        review_profile: SafeProfileId,
        namespace: StableSandboxNamespace,
    ) -> Result<Self, StableHostError> {
        Self::spawn_stable_in_with_faults(spec, review_profile, namespace, SandboxFaults::default())
    }

    pub(super) fn spawn_stable_in_with_faults(
        spec: WalkerSeedSpec,
        review_profile: SafeProfileId,
        namespace: StableSandboxNamespace,
        faults: SandboxFaults,
    ) -> Result<Self, StableHostError> {
        Self::spawn_stable_in_with_faults_and_directory_seed_io(
            spec,
            review_profile,
            namespace,
            faults,
            &SystemDirectorySeedIo,
        )
    }

    #[cfg(test)]
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    pub(super) fn spawn_stable_in_with_directory_seed_io(
        spec: WalkerSeedSpec,
        review_profile: SafeProfileId,
        namespace: StableSandboxNamespace,
        directory_seed_io: &dyn DirectorySeedIo,
    ) -> Result<Self, StableHostError> {
        Self::spawn_stable_in_with_faults_and_directory_seed_io(
            spec,
            review_profile,
            namespace,
            SandboxFaults::default(),
            directory_seed_io,
        )
    }

    pub(super) fn spawn_stable_in_with_faults_and_directory_seed_io(
        spec: WalkerSeedSpec,
        review_profile: SafeProfileId,
        namespace: StableSandboxNamespace,
        faults: SandboxFaults,
        directory_seed_io: &dyn DirectorySeedIo,
    ) -> Result<Self, StableHostError> {
        validate_external_spec(&spec).map_err(|primary| StableHostError::Primary {
            primary: StableHostPrimaryError::Seed(primary),
            cleanup: None,
        })?;
        let sandbox = SandboxOwner::Stable(Box::new(StableSandbox::acquire(
            namespace,
            review_profile,
            faults,
        )?));
        match Self::spawn_in_sandbox(
            spec,
            sandbox,
            AllocationMaximums::default(),
            directory_seed_io,
        ) {
            Ok(host) => Ok(host),
            Err((sandbox, primary)) => {
                let cleanup = sandbox.close().err();
                Err(StableHostError::Primary {
                    primary: StableHostPrimaryError::Seed(primary),
                    cleanup,
                })
            }
        }
    }

    pub(super) fn spawn_with_allocation_maximums(
        spec: WalkerSeedSpec,
        allocation_maximums: AllocationMaximums,
    ) -> Result<Self, String> {
        Self::spawn_with_allocation_maximums_and_directory_seed_io(
            spec,
            allocation_maximums,
            &SystemDirectorySeedIo,
        )
    }

    #[cfg(test)]
    pub(super) fn spawn_with_directory_seed_io(
        spec: WalkerSeedSpec,
        directory_seed_io: &dyn DirectorySeedIo,
    ) -> Result<Self, String> {
        Self::spawn_with_allocation_maximums_and_directory_seed_io(
            spec,
            AllocationMaximums::default(),
            directory_seed_io,
        )
    }

    fn spawn_with_allocation_maximums_and_directory_seed_io(
        spec: WalkerSeedSpec,
        allocation_maximums: AllocationMaximums,
        directory_seed_io: &dyn DirectorySeedIo,
    ) -> Result<Self, String> {
        validate_external_spec(&spec)?;
        let sandbox = SandboxOwner::random().map_err(|error| error.to_string())?;
        match Self::spawn_in_sandbox(spec, sandbox, allocation_maximums, directory_seed_io) {
            Ok(host) => Ok(host),
            Err((sandbox, primary)) => Self::failed_random_spawn(sandbox, primary),
        }
    }

    pub(super) fn failed_random_spawn(
        sandbox: SandboxOwner,
        primary: String,
    ) -> Result<Self, String> {
        match sandbox.close() {
            Ok(()) => Err(primary),
            Err(cleanup) => Err(format!(
                "{primary}; random sandbox cleanup also failed: {cleanup}"
            )),
        }
    }

    pub(super) fn spawn_in_sandbox(
        spec: WalkerSeedSpec,
        sandbox: SandboxOwner,
        allocation_maximums: AllocationMaximums,
        directory_seed_io: &dyn DirectorySeedIo,
    ) -> Result<Self, (SandboxOwner, String)> {
        let built = (|| -> Result<_, String> {
            let roots = ProductRoots::new(
                sandbox.path().join("data"),
                sandbox.path().join("state"),
                sandbox.path().join("config"),
                Some(sandbox.path().join("home")),
                sandbox.path().join("cwd"),
            );
            let external_root = sandbox.path().join("external");
            let system_temp = sandbox.path().join("system-temp");
            for root in [
                &roots.data,
                &roots.state,
                &roots.config,
                roots.home.as_ref().expect("the walker profile has a home"),
                &roots.cwd,
                &external_root,
                &system_temp,
            ] {
                if let Some(stable) = sandbox.stable() {
                    stable
                        .validate_namespace()
                        .map_err(|error| error.to_string())?;
                    stable.validate_lease().map_err(|error| error.to_string())?;
                    stable
                        .validate_live_sandbox()
                        .map_err(|error| error.to_string())?;
                } else {
                    fs::create_dir_all(root).map_err(|error| error.to_string())?;
                }
            }
            let seeded_directories =
                seed_profile_directories(&roots, &spec.directories, directory_seed_io)?;
            let file_picker_tree = seed_external_world(&external_root, &spec.external)?;

            let create_clock = Arc::new(WalkerCreateClock::default());
            let (expected, seeded_copy_paths) = {
                let service = LibraryService::new(FileStore::with_create_clock(
                    &roots.data,
                    create_clock.clone(),
                ));
                let seeded_copy_paths = seed_profile(&service, &roots, &external_root, &spec)?;
                (read_seed_snapshot(&service, &roots)?, seeded_copy_paths)
            };
            let service =
                LibraryService::new(FileStore::with_create_clock(&roots.data, create_clock));
            let reopened = read_seed_snapshot(&service, &roots)?;
            (reopened == expected)
                .then_some(())
                .ok_or("the production stores changed after the seed was reopened".to_owned())?;
            let configured = FileConfigStore::new(&roots.config)
                .get("lang")
                .map_err(|error| error.to_string())?;
            let locale = requested_locale(Some(&configured)).unwrap_or(Locale::En);
            let transcript = Rc::new(RefCell::new(Vec::new()));
            let profile = if spec.profile.is_empty() {
                "default".to_owned()
            } else {
                spec.profile.clone()
            };
            let path_map =
                PathMap::new(&profile, sandbox.path(), &service, &file_picker_tree.files)?;
            let observation_modes = ObservationModeProvenance::from_seed(
                &sandbox,
                &roots,
                &external_root,
                &system_temp,
                &seeded_directories,
                &spec.external,
                &seeded_copy_paths,
            )?;
            Ok((
                roots,
                external_root,
                file_picker_tree,
                service,
                locale,
                transcript,
                system_temp,
                path_map,
                observation_modes,
                profile,
            ))
        })();
        let (
            roots,
            external_root,
            file_picker_tree,
            service,
            locale,
            transcript,
            system_temp,
            path_map,
            observation_modes,
            profile,
        ) = match built {
            Ok(built) => built,
            Err(error) => return Err((sandbox, error)),
        };
        Ok(Self {
            _sandbox: sandbox,
            roots,
            external_root,
            file_picker_tree,
            service,
            locale: Cell::new(locale),
            clock: FixedClock::new(Rc::clone(&transcript)),
            adapters: RecordingAdapters::new_with_maximums_and_editor_writes(
                transcript,
                system_temp,
                allocation_maximums,
                spec.editor_writes,
            ),
            path_map,
            observation_modes: RefCell::new(observation_modes),
            profile,
        })
    }

    pub(crate) fn sandbox_root(&self) -> &Path {
        self._sandbox.path()
    }

    pub(crate) fn sandbox_metadata(
        &self,
        review_profile: &SafeProfileId,
    ) -> Result<SandboxMetadata, SandboxError> {
        self._sandbox.metadata(review_profile)
    }

    pub(crate) fn leak_oracle_facts(&self) -> LeakOracleFacts {
        self.path_map.leak_oracle_facts()
    }

    pub(crate) fn close(self) -> Result<(), SandboxError> {
        let Self {
            _sandbox,
            roots,
            external_root,
            file_picker_tree,
            service,
            locale,
            clock,
            adapters,
            path_map,
            observation_modes,
            profile,
        } = self;
        drop((
            profile,
            observation_modes,
            path_map,
            adapters,
            clock,
            locale,
            service,
            file_picker_tree,
            external_root,
            roots,
        ));
        _sandbox.close()
    }

    pub(crate) fn roots(&self) -> &ProductRoots {
        &self.roots
    }

    pub(crate) fn file_picker_tree(&self) -> WalkerFilePickerTree {
        self.file_picker_tree.clone()
    }

    pub(crate) fn initial_state(&self) -> Result<LibraryState, crate::cli::CliError> {
        self.with_host(|host| host.initial_state())
    }

    pub(crate) fn preflight(&self, effect: &Effect) -> Result<(), crate::cli::CliError> {
        self.with_host(|host| host.preflight(effect))
    }

    fn serve_after_preflight(&self, effect: Effect) -> Result<Action, crate::cli::CliError> {
        self.with_host(|host| host.serve(effect))
    }

    pub(crate) fn dispatch(&self, effect: Effect) -> Result<Action, crate::cli::CliError> {
        match self.preflight(&effect) {
            Ok(()) => {
                let action = self.serve_after_preflight(effect)?;
                self.observation_modes
                    .borrow_mut()
                    .record_created_copy(&self.service, &action);
                Ok(action)
            }
            Err(error) => Ok(Action::SetStatus(
                Message::new("Error: {}")
                    .nested(error.message())
                    .localize(self.locale.get()),
            )),
        }
    }

    fn pending_observation(&self) -> Result<PendingHostObservation, String> {
        let surface = library_surface_at(
            self.service.repository(),
            &self.roots.state,
            &self.roots.config,
            self.clock.now_utc(),
        )
        .map_err(|error| error.to_string())?;
        let scan = self.service.list().map_err(|error| error.to_string())?;
        let forms = FormStateService::new(FileFormStateStore::new(&self.roots.state));
        let form_state = scan
            .entries
            .iter()
            .map(|entry| {
                (
                    entry.slug.as_str().to_owned(),
                    persisted_form_json(&forms.load(&entry.slug)),
                )
            })
            .collect();
        let config = FileConfigStore::new(&self.roots.config);
        let runner_rows = config
            .runner_rows()
            .map_err(|error| error.to_string())?
            .into_iter()
            .map(|runner| {
                json!({
                    "index": runner.index,
                    "name": runner.name,
                    "argv": runner.argv,
                    "reason": runner.reason,
                    "descriptor": runner.descriptor,
                    "snapshot": runner.snapshot_token(),
                })
            })
            .collect::<Vec<_>>();
        let mirror = config.mirror().map_err(|error| error.to_string())?;
        let config_path = self.roots.config.join("config.toml");
        let config = json!({
            "settings": config.settings().map_err(|error| error.to_string())?,
            "runner_rows": runner_rows,
            "mirror": {
                "enabled": mirror.enabled,
                "pypi": mirror.pypi,
                "python_install": mirror.python_install,
                "uv_binary": mirror.uv_binary,
                "npm": mirror.npm,
            },
            "document_ref": config_path.is_file().then_some("tree:config/config.toml"),
        });
        let prompt_runner =
            PromptSelectionService::new(FilePromptSelectionStore::new(&self.roots.state))
                .last_runner();
        let drafts = sorted_tui_drafts(&self.roots.data);
        let drafts = serde_json::to_value(drafts).map_err(|error| error.to_string())?;
        let surface = serde_json::to_value(surface).map_err(|error| error.to_string())?;
        Ok(PendingHostObservation {
            surface,
            config,
            form_state,
            prompt_runner,
            drafts,
        })
    }

    pub(crate) fn capture_checkpoint_parts(
        &mut self,
        state: &mut LibraryState,
        session: &mut Value,
        cause: CheckpointCauseProjection<'_, Action, Effect>,
    ) -> Result<HostObservation, String> {
        self.capture_checkpoint_parts_internal(state, Some(session), cause)
    }

    fn capture_checkpoint_parts_internal(
        &mut self,
        state: &mut LibraryState,
        session: Option<&mut Value>,
        cause: CheckpointCauseProjection<'_, Action, Effect>,
    ) -> Result<HostObservation, String> {
        let pending = self.pending_observation()?;
        let events = self.adapters.pending_events();

        self.path_map.refresh(&self.service)?;
        let observation_modes = self.observation_modes.get_mut();
        observation_modes.refresh_live_sources(&self.service)?;
        for event in &events {
            self.path_map.register_event_artifacts(event);
            observation_modes.register_event(event, &self.roots)?;
        }
        self.path_map.project_checkpoint_cause(cause)?;
        self.path_map.project_typed_library_state(state)?;
        let state = serde_json::to_value(&*state).map_err(|error| error.to_string())?;
        if let Some(session) = session {
            self.path_map.project_session_value(session)?;
        }

        let surface = self.path_map.normalize_library_json(pending.surface)?;
        let drafts = self.path_map.normalize_draft_list(pending.drafts)?;
        ensure_system_temp_empty(&self.adapters.system_temp, &self.path_map)?;
        let tree = snapshot_tree(
            &self.roots,
            &self.external_root,
            &mut self.path_map,
            observation_modes,
        )?;
        let transcript = RecordingAdapters::render_transcript(&events, &mut self.path_map);
        let observation = HostObservation {
            surface,
            state,
            config: pending.config,
            form_state: pending.form_state,
            prompt_runner: pending.prompt_runner,
            drafts,
            tree,
            transcript,
        };
        self.adapters.drain_event_prefix(events.len())?;
        Ok(observation)
    }

    pub(crate) fn observe(&mut self, state: &LibraryState) -> Result<HostObservation, String> {
        let mut state = state.clone();
        self.capture_checkpoint_parts_internal(
            &mut state,
            None,
            CheckpointCauseProjection::Observation,
        )
    }

    pub(crate) fn canonical_action(&mut self, action: &Action) -> Result<Value, String> {
        self.path_map.refresh(&self.service)?;
        let mut action = action.clone();
        self.path_map.project_typed_action(&mut action)?;
        serde_json::to_value(action).map_err(|error| error.to_string())
    }

    pub(crate) fn clear_transcript(&self) {
        self.adapters.events.borrow_mut().clear();
    }

    pub(crate) fn fail_next_launch(&self, kind: io::ErrorKind, reason: &str) {
        self.adapters.launch_script.replace(LaunchScript::Failure {
            kind,
            reason: reason.to_owned(),
        });
    }

    fn with_host<T>(
        &self,
        operation: impl FnOnce(&TuiHost<'_, FixedClock>) -> Result<T, crate::cli::CliError>,
    ) -> Result<T, crate::cli::CliError> {
        let host = TuiHost::new(
            &self.service,
            self.roots.clone(),
            self.locale.get(),
            &self.clock,
            self.adapters.platform(),
        )
        .expect("the walker service and roots share the same data directory");
        let result = operation(&host);
        self.locale.set(host.locale());
        result
    }
}
