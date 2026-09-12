//! The real and corpus frontends over the production TUI session.

use ratatui_core::{
    backend::TestBackend,
    layout::{Rect, Size},
    terminal::Terminal,
};
use ratatui_crossterm::crossterm::event::{Event, KeyCode, KeyModifiers};
use serde_json::{Value, json};
use skit_i18n::{Locale, detect_locale};
use skit_tui::{EventHandling, TuiSession, ViewGeometry, render_with_session};
use skit_tui_walker_model::{invariants, model::LiveInventory, parity};
use skit_tui_walker_support::{
    RectSnapshot, StyledFrameSnapshot,
    engine::{DispatchOutcome, EffectRoute, FrontendAdapter, OperationResolution},
    validate_styled_frame,
};
use skit_ui::{Action, Effect, LibraryState};

use super::corpus::{
    CorpusEvent, CorpusOperation, CorpusResolution, FrontendObservation, FrontendParity,
    SmokeEvent, SmokeHandling, SmokeOperation, SmokeResolution, resolve_corpus_operation,
    screen_target_error_message,
};
use crate::cli::tui_real_host::WalkerFilePickerTree;

/// Return the geometry that the parity probe reads.
///
/// The live walk reads the geometry of the frame it just drew. A contract replaces this reader to
/// prove that a broken geometry poisons the engine.
fn live_probe_geometry(geometry: &ViewGeometry) -> ViewGeometry {
    geometry.clone()
}

// `SchemaThreeSink` is visible to the random walk, and it records the smoke checkpoint of this
// frontend. The `private_interfaces` lint needs the same visibility here.
pub(crate) struct RealFrontend {
    pub(super) state: LibraryState,
    pub(super) locale: Locale,
    pub(super) session: TuiSession,
    pub(super) terminal: Terminal<TestBackend>,
    pub(super) geometry: ViewGeometry,
    pub(super) probe_geometry: fn(&ViewGeometry) -> ViewGeometry,
}

impl RealFrontend {
    pub(super) fn new(state: LibraryState, locale: Locale, size: Size) -> Result<Self, String> {
        if size.width == 0 || size.height == 0 {
            return Err("the real walker smoke viewport must be positive".to_owned());
        }
        Ok(Self {
            state,
            locale,
            session: TuiSession::default(),
            terminal: Terminal::new(TestBackend::new(size.width, size.height))
                .map_err(|error| error.to_string())?,
            geometry: ViewGeometry::default(),
            probe_geometry: live_probe_geometry,
        })
    }

    pub(super) fn session_value(&self) -> Result<Value, String> {
        serde_json::to_value(
            self.session
                .agent_review_snapshot()
                .map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())
    }

    fn geometry_value(geometry: &ViewGeometry) -> Value {
        json!({
            "rows": RectSnapshot::from(geometry.rows),
            "first_visible": geometry.first_visible,
            "hits": geometry.hits.iter().map(|hit| json!({
                "rect": RectSnapshot::from(hit.rect),
                "action": format!("{:?}", hit.action),
            })).collect::<Vec<_>>(),
            "detail_pane_visible": geometry.detail_pane_visible,
        })
    }

    fn dispatch_terminal_event(
        &mut self,
        event: Event,
    ) -> Result<DispatchOutcome<Action, SmokeHandling>, String> {
        let handling = self
            .session
            .handle_event(event, &self.state, &self.geometry);
        Ok(match handling {
            EventHandling::Action(action) => DispatchOutcome::Action(action),
            EventHandling::Consumed => DispatchOutcome::Session(SmokeHandling::Consumed),
            EventHandling::Ignored => DispatchOutcome::Session(SmokeHandling::Ignored),
        })
    }

    fn reduce_action(&mut self, action: Action) -> Effect {
        if let Action::PreferencesSaved { locale, .. } = &action {
            self.locale = detect_locale(Some(locale));
        }
        self.state.update(action)
    }

    const fn route_effect(effect: &Effect) -> EffectRoute {
        match effect {
            Effect::None => EffectRoute::Settled,
            Effect::Quit => EffectRoute::Quit,
            _ => EffectRoute::Host,
        }
    }

    pub(super) fn observe_frontend(&mut self) -> Result<FrontendObservation, String> {
        // The legacy walker reads the invariants of the state that produced the frame, and it
        // reports the violation after the frame exists. The failing screen then stays available
        // for the artifact beside the named rule.
        let invariant = invariants::check_state(&self.state);
        let mut geometry = ViewGeometry::default();
        let completed = self
            .terminal
            .draw(|frame| {
                geometry = render_with_session(frame, &self.state, self.locale, &mut self.session);
            })
            .map_err(|error| error.to_string())?;
        self.geometry = geometry;
        let buffer = completed.buffer.clone();
        #[allow(deprecated)]
        let styled_frame = StyledFrameSnapshot::from_buffer(
            &buffer,
            self.terminal.backend().cursor_position(),
            self.terminal.backend().cursor_visible(),
        );
        validate_styled_frame(&styled_frame).map_err(|error| error.to_string())?;
        invariant.map_err(|violation| format!("the walker state broke one rule: {violation}"))?;
        parity::check_public_hit_parity(
            &self.state,
            &(self.probe_geometry)(&self.geometry),
            buffer.area.as_size(),
            &self.session,
            self.locale,
        )
        .map_err(|violation| format!("the walker frame broke one rule: {violation}"))?;
        Ok(FrontendObservation {
            state: self.state.clone(),
            session: self.session_value()?,
            styled_frame,
            geometry: Self::geometry_value(&self.geometry),
            locale: self.locale,
        })
    }

    fn frontend_parity(&self) -> Result<FrontendParity, String> {
        Ok(FrontendParity {
            state: self.state.clone(),
            session: self.session_value()?,
            locale: self.locale,
        })
    }
}

impl FrontendAdapter for RealFrontend {
    type Operation = SmokeOperation;
    type Resolution = SmokeResolution;
    type Event = SmokeEvent;
    type Action = Action;
    type Effect = Effect;
    type Handling = SmokeHandling;
    type Observation = FrontendObservation;
    type ParitySnapshot = FrontendParity;

    fn resolve(
        &self,
        operation: &Self::Operation,
    ) -> Result<OperationResolution<Self::Event, Self::Resolution>, String> {
        let event = match operation {
            SmokeOperation::OpenRun => SmokeEvent {
                key: KeyCode::Enter,
                modifiers: KeyModifiers::NONE,
            },
            SmokeOperation::OpenPreferences => SmokeEvent {
                key: KeyCode::Char(','),
                modifiers: KeyModifiers::NONE,
            },
            SmokeOperation::OpenLanguagePicker | SmokeOperation::ChooseLanguage => SmokeEvent {
                key: KeyCode::Enter,
                modifiers: KeyModifiers::NONE,
            },
            SmokeOperation::NextLanguage => SmokeEvent {
                key: KeyCode::Down,
                modifiers: KeyModifiers::NONE,
            },
            SmokeOperation::SavePreferences => SmokeEvent {
                key: KeyCode::Char('s'),
                modifiers: KeyModifiers::CONTROL,
            },
            SmokeOperation::FinalLiveness => SmokeEvent {
                key: KeyCode::Esc,
                modifiers: KeyModifiers::NONE,
            },
        };
        Ok(OperationResolution::Events {
            resolved: SmokeResolution { event },
            events: vec![event],
        })
    }

    fn dispatch(
        &mut self,
        event: Self::Event,
    ) -> Result<DispatchOutcome<Self::Action, Self::Handling>, String> {
        self.dispatch_terminal_event(event.terminal_event())
    }

    fn reduce(&mut self, action: Self::Action) -> Result<Self::Effect, String> {
        Ok(self.reduce_action(action))
    }

    fn effect_route(&self, effect: &Self::Effect) -> EffectRoute {
        Self::route_effect(effect)
    }

    fn observe(&mut self) -> Result<Self::Observation, String> {
        self.observe_frontend()
    }

    fn parity_snapshot(&self) -> Result<Self::ParitySnapshot, String> {
        self.frontend_parity()
    }
}

pub(crate) struct CorpusFrontend {
    pub(super) inner: RealFrontend,
}

impl CorpusFrontend {
    pub(super) fn new(
        state: LibraryState,
        locale: Locale,
        size: Size,
        file_picker_tree: WalkerFilePickerTree,
    ) -> Result<Self, String> {
        let inner = RealFrontend::new(state, locale, size)?;
        Ok(Self::from_real(inner, file_picker_tree))
    }

    /// Give one real frontend the deterministic file-picker tree of its host.
    pub(super) fn from_real(
        mut inner: RealFrontend,
        file_picker_tree: WalkerFilePickerTree,
    ) -> Self {
        inner.session = TuiSession::with_file_picker_tree(
            file_picker_tree.root,
            file_picker_tree.directories,
            file_picker_tree.files,
        );
        Self { inner }
    }

    fn resize(&mut self, width: u16, height: u16) -> Result<(), String> {
        self.inner.terminal.backend_mut().resize(width, height);
        self.inner
            .terminal
            .resize(Rect::new(0, 0, width, height))
            .map_err(|error| error.to_string())
    }

    /// Collect everything the current frame offers to the random operation binder.
    pub(crate) fn live_inventory(&self) -> Result<LiveInventory, String> {
        let screen_targets = self
            .inner
            .session
            .screen_target_inventory(&self.inner.state)
            .map_err(screen_target_error_message)?;
        Ok(LiveInventory::new(
            &self.inner.state,
            &self.inner.geometry,
            self.inner.session.local_action_inventory(),
            Some(&screen_targets),
            self.inner.terminal.backend().buffer().area.as_size(),
        ))
    }
}

impl FrontendAdapter for CorpusFrontend {
    type Operation = CorpusOperation;
    type Resolution = CorpusResolution;
    type Event = CorpusEvent;
    type Action = Action;
    type Effect = Effect;
    type Handling = SmokeHandling;
    type Observation = FrontendObservation;
    type ParitySnapshot = FrontendParity;

    fn resolve(
        &self,
        operation: &Self::Operation,
    ) -> Result<OperationResolution<Self::Event, Self::Resolution>, String> {
        resolve_corpus_operation(&self.inner, operation)
    }

    fn dispatch(
        &mut self,
        event: Self::Event,
    ) -> Result<DispatchOutcome<Self::Action, Self::Handling>, String> {
        if let CorpusEvent::Resize { width, height } = event {
            self.resize(width, height)?;
            return self
                .inner
                .dispatch_terminal_event(Event::Resize(width, height));
        }
        self.inner.dispatch_terminal_event(event.terminal())
    }

    fn reduce(&mut self, action: Self::Action) -> Result<Self::Effect, String> {
        Ok(self.inner.reduce_action(action))
    }

    fn effect_route(&self, effect: &Self::Effect) -> EffectRoute {
        RealFrontend::route_effect(effect)
    }

    fn observe(&mut self) -> Result<Self::Observation, String> {
        self.inner.observe_frontend()
    }

    fn parity_snapshot(&self) -> Result<Self::ParitySnapshot, String> {
        self.inner.frontend_parity()
    }
}
