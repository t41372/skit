//! Host boundaries, walker factories, and the corpus engine.

use ratatui_core::layout::Size;
use skit_i18n::Locale;
use skit_tui_walker_support::{
    engine::{
        CheckpointCapture, CheckpointCauseProjection, HostAdapter, ReplayFactory, WalkerEngine,
    },
    sandbox::{SafeProfileId, SandboxMetadata},
};
use skit_ui::{Action, Effect, LibraryState};

use super::{
    corpus::{EFFECT_LIMIT, FrontendObservation},
    frontend::{CorpusFrontend, RealFrontend},
    recording::{canonical_corpus_seed, close_after_primary},
    trace::SchemaThreeSink,
};
use crate::cli::tui_real_host::{
    HostObservation, LeakOracleFacts, RealWalkerHost, SandboxError, WalkerSeedSpec,
};

pub(crate) struct RealHostBoundary {
    pub(super) host: RealWalkerHost,
}

impl RealHostBoundary {
    pub(super) fn sandbox_metadata(
        &self,
        review_profile: &SafeProfileId,
    ) -> Result<SandboxMetadata, SandboxError> {
        self.host.sandbox_metadata(review_profile)
    }

    pub(super) fn leak_oracle_facts(&self) -> LeakOracleFacts {
        self.host.leak_oracle_facts()
    }

    pub(super) fn close(self) -> Result<(), SandboxError> {
        self.host.close()
    }

    fn serve_effect(&mut self, effect: Effect) -> Result<Action, String> {
        self.host
            .dispatch(effect)
            .map_err(|error| error.to_string())
    }

    fn capture_frontend(
        &mut self,
        mut frontend: FrontendObservation,
        cause: CheckpointCauseProjection<'_, Action, Effect>,
    ) -> Result<CheckpointCapture<FrontendObservation, HostObservation>, String> {
        let host = self.host.capture_checkpoint_parts(
            &mut frontend.state,
            &mut frontend.session,
            cause,
        )?;
        Ok(CheckpointCapture { frontend, host })
    }
}

impl HostAdapter<RealFrontend> for RealHostBoundary {
    type Observation = HostObservation;

    fn serve(&mut self, effect: Effect) -> Result<Action, String> {
        self.serve_effect(effect)
    }

    fn capture_checkpoint(
        &mut self,
        frontend: FrontendObservation,
        cause: CheckpointCauseProjection<'_, Action, Effect>,
    ) -> Result<CheckpointCapture<FrontendObservation, Self::Observation>, String> {
        self.capture_frontend(frontend, cause)
    }
}

pub(crate) struct CorpusHostBoundary {
    pub(super) inner: RealHostBoundary,
    pub(super) leak_sandbox: Option<SandboxMetadata>,
}

/// Facts for one projected checkpoint. They never enter the stored host observation.
pub(super) struct CheckpointLeakContext {
    pub(super) sandbox: SandboxMetadata,
    pub(super) facts: LeakOracleFacts,
}

pub(crate) struct CorpusHostObservation {
    pub(super) observation: HostObservation,
    pub(super) leak_context: Option<CheckpointLeakContext>,
}

impl CorpusHostBoundary {
    pub(super) fn close(self) -> Result<(), SandboxError> {
        self.inner.close()
    }
}

impl HostAdapter<CorpusFrontend> for CorpusHostBoundary {
    type Observation = CorpusHostObservation;

    fn serve(&mut self, effect: Effect) -> Result<Action, String> {
        self.inner.serve_effect(effect)
    }

    fn capture_checkpoint(
        &mut self,
        frontend: FrontendObservation,
        cause: CheckpointCauseProjection<'_, Action, Effect>,
    ) -> Result<CheckpointCapture<FrontendObservation, Self::Observation>, String> {
        let CheckpointCapture { frontend, host } = self.inner.capture_frontend(frontend, cause)?;
        let leak_context = self
            .leak_sandbox
            .as_ref()
            .map(|sandbox| CheckpointLeakContext {
                sandbox: sandbox.clone(),
                facts: self.inner.leak_oracle_facts(),
            });
        Ok(CheckpointCapture {
            frontend,
            host: CorpusHostObservation {
                observation: host,
                leak_context,
            },
        })
    }
}

#[derive(Clone, Debug)]
pub(super) struct RealWalkerFactory {
    pub(super) seed: WalkerSeedSpec,
    pub(super) review_profile: SafeProfileId,
    pub(super) locale: Locale,
    pub(super) size: Size,
}

impl RealWalkerFactory {
    fn frontend_for_host(
        &self,
        host: RealWalkerHost,
    ) -> Result<(RealFrontend, RealHostBoundary), String> {
        self.frontend_for_host_with_initial_state(host, |host| {
            host.initial_state().map_err(|error| error.to_string())
        })
    }

    pub(super) fn frontend_for_host_with_initial_state(
        &self,
        host: RealWalkerHost,
        initial_state: impl FnOnce(&RealWalkerHost) -> Result<LibraryState, String>,
    ) -> Result<(RealFrontend, RealHostBoundary), String> {
        let state = match initial_state(&host) {
            Ok(state) => state,
            Err(error) => return Err(close_after_primary(host, error)),
        };
        let frontend = match RealFrontend::new(state, self.locale, self.size) {
            Ok(frontend) => frontend,
            Err(error) => return Err(close_after_primary(host, error)),
        };
        Ok((frontend, RealHostBoundary { host }))
    }
}

impl ReplayFactory for RealWalkerFactory {
    type Frontend = RealFrontend;
    type Host = RealHostBoundary;

    fn create(&self) -> Result<(Self::Frontend, Self::Host), String> {
        let host = RealWalkerHost::spawn(self.seed.clone())?;
        self.frontend_for_host(host)
    }

    fn effect_limit(&self) -> usize {
        EFFECT_LIMIT
    }
}

/// One engine that runs corpus operations against a real host.
pub(crate) type CorpusEngine = WalkerEngine<CorpusFrontend, CorpusHostBoundary>;

/// A factory that spawns one fresh random-mode host for the corpus adapter.
#[derive(Clone, Debug)]
pub(crate) struct CorpusWalkerFactory {
    inner: RealWalkerFactory,
}

impl CorpusWalkerFactory {
    /// Create one corpus frontend and host pair from one caller-supplied initial state.
    pub(crate) fn create_with_initial_state(
        &self,
        initial_state: impl FnOnce(&RealWalkerHost) -> Result<LibraryState, String>,
    ) -> Result<(CorpusFrontend, CorpusHostBoundary), String> {
        let host = RealWalkerHost::spawn(self.inner.seed.clone())?;
        let file_picker_tree = host.file_picker_tree();
        let (frontend, inner) = self
            .inner
            .frontend_for_host_with_initial_state(host, initial_state)?;
        Ok((
            CorpusFrontend::from_real(frontend, file_picker_tree),
            CorpusHostBoundary {
                inner,
                leak_sandbox: None,
            },
        ))
    }

    /// Create one unstarted engine from one caller-supplied initial state.
    pub(crate) fn engine_with_initial_state(
        &self,
        initial_state: impl FnOnce(&RealWalkerHost) -> Result<LibraryState, String>,
    ) -> Result<CorpusEngine, String> {
        let (frontend, host) = self.create_with_initial_state(initial_state)?;
        Ok(WalkerEngine::new(frontend, host, EFFECT_LIMIT)
            .expect("the corpus effect limit is positive"))
    }

    /// Create one recorder for a walk that resizes into `canvas`.
    pub(crate) fn walk_sink(&self, canvas: Size) -> Result<SchemaThreeSink, String> {
        SchemaThreeSink::new(
            self.inner.review_profile.clone(),
            self.inner.locale,
            self.inner.size,
            canvas,
        )
    }
}

impl ReplayFactory for CorpusWalkerFactory {
    type Frontend = CorpusFrontend;
    type Host = CorpusHostBoundary;

    fn create(&self) -> Result<(Self::Frontend, Self::Host), String> {
        self.create_with_initial_state(real_initial_state)
    }

    fn effect_limit(&self) -> usize {
        EFFECT_LIMIT
    }
}

/// Read the initial state of one real host.
pub(crate) fn real_initial_state(host: &RealWalkerHost) -> Result<LibraryState, String> {
    host.initial_state().map_err(|error| error.to_string())
}

/// Build one random-mode corpus factory for one locale and viewport.
pub(crate) fn random_walk_factory(locale: Locale, size: Size) -> CorpusWalkerFactory {
    let profile = format!(
        "random-{}-{}x{}",
        locale.tag().to_ascii_lowercase(),
        size.width,
        size.height
    );
    CorpusWalkerFactory {
        inner: RealWalkerFactory {
            seed: canonical_corpus_seed(locale),
            review_profile: SafeProfileId::try_from(profile)
                .expect("the random walk profile identifier is valid"),
            locale,
            size,
        },
    }
}

/// Close the real host of one corpus engine.
pub(crate) fn close_corpus_engine(engine: CorpusEngine) -> Result<(), String> {
    let (frontend, host) = engine.into_parts();
    drop(frontend);
    host.close().map_err(|error| error.to_string())
}
