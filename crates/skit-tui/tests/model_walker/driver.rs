use std::{
    any::Any,
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    fs,
    panic::{AssertUnwindSafe, catch_unwind},
    path::{Path, PathBuf},
    time::Duration,
};

use proptest::{
    collection,
    test_runner::{
        Config, FileFailurePersistence, RngAlgorithm, TestCaseError, TestError, TestRunner,
    },
};
use ratatui_core::{
    backend::TestBackend,
    layout::{Rect, Size},
    terminal::Terminal,
};
use ratatui_crossterm::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use skit_domain::parameters::{ParamDecl, ParameterType, ParameterValue};
use skit_i18n::{Locale, detect_locale};
use skit_tui::{
    AddControlId, EventHandling, HitTarget, LocalActionOutcome, LocalActionTarget,
    LocalAdvertisedAction, TuiSession, ViewGeometry, map_event, render_with_session,
};
use skit_ui::{
    Action, AddAction, AddStage, Effect, LibraryState, ModalState, PreferencesAction,
    PreferencesControlId, RunFormView, RunTokenOption, Screen, UiBinding, UiCommand, UiKey,
    command_specs,
};

use super::{
    asciicast::AsciicastRecorder,
    fake_host::FakeHost,
    invariants,
    strategy::{
        MouseKind, ResolvedOperation, WalkerOperation, operation_strategy, resolve,
        resolve_advertised_command, resolve_public_hit,
    },
};

const HOST_EFFECT_LIMIT: usize = 64;
const FRAME_INTERVAL: Duration = Duration::from_millis(100);
const ARTIFACT_DIR: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../target/ui-walker-artifacts"
);
const REGRESSION_FILE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../target/ui-walker-artifacts/regressions.txt"
);
trait WalkerHost {
    fn initial_state(&self) -> LibraryState;
    fn serve(&mut self, effect: Effect) -> Result<Action, String>;

    fn validate_effect(&self, _effect: &Effect) -> Result<(), String> {
        Ok(())
    }

    fn try_fork(&self) -> Option<Self>
    where
        Self: Sized,
    {
        None
    }

    fn file_picker_tree(&self) -> Option<(PathBuf, BTreeSet<PathBuf>, BTreeSet<PathBuf>)> {
        None
    }
}

impl WalkerHost for FakeHost {
    fn initial_state(&self) -> LibraryState {
        self.initial_state()
    }

    fn serve(&mut self, effect: Effect) -> Result<Action, String> {
        self.serve(effect).map_err(|error| error.to_string())
    }

    fn validate_effect(&self, effect: &Effect) -> Result<(), String> {
        self.validate_effect_sanity(effect)
            .map_err(|error| error.to_string())
    }

    fn try_fork(&self) -> Option<Self> {
        Some(self.clone())
    }

    fn file_picker_tree(&self) -> Option<(PathBuf, BTreeSet<PathBuf>, BTreeSet<PathBuf>)> {
        Some(self.file_picker_tree())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Boundary {
    Initial,
    Session,
    UserAction,
    HostAction,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Checkpoint {
    boundary: Boundary,
    pending_effect: bool,
    add_stage: Option<AddStage>,
    add_delete_candidate: bool,
    size: Size,
}

struct Walker<H: WalkerHost> {
    host: H,
    state: LibraryState,
    locale: Locale,
    session: TuiSession,
    terminal: Terminal<TestBackend>,
    geometry: ViewGeometry,
    recorder: AsciicastRecorder,
    checkpoints: Vec<Checkpoint>,
    quit: bool,
    probe_checks: bool,
    liveness_checks: usize,
}

impl<H: WalkerHost> Walker<H> {
    fn new(host: H, locale: Locale, size: Size) -> Result<Self, String> {
        Self::with_canvas(host, locale, size, size)
    }

    fn with_canvas(host: H, locale: Locale, size: Size, canvas: Size) -> Result<Self, String> {
        let mut walker = Self::unstarted(host, locale, size, canvas)?;
        walker.checkpoint(Boundary::Initial, None)?;
        Ok(walker)
    }

    fn unstarted(host: H, locale: Locale, size: Size, canvas: Size) -> Result<Self, String> {
        if size.width == 0 || size.height == 0 {
            return Err("the walker viewport must not be empty".to_owned());
        }
        if canvas.width < size.width || canvas.height < size.height {
            return Err(format!(
                "the cast canvas {canvas:?} is smaller than the initial viewport {size:?}"
            ));
        }
        let state = host.initial_state();
        let terminal = Terminal::new(TestBackend::new(size.width, size.height))
            .map_err(|error| error.to_string())?;
        let recorder = AsciicastRecorder::new(canvas.width, canvas.height)
            .map_err(|error| error.to_string())?;
        let session = host
            .file_picker_tree()
            .map_or_else(TuiSession::default, |tree| {
                TuiSession::with_file_picker_tree(tree.0, tree.1, tree.2)
            });
        Ok(Self {
            host,
            state,
            locale,
            session,
            terminal,
            geometry: ViewGeometry::default(),
            recorder,
            checkpoints: Vec::new(),
            quit: false,
            probe_checks: true,
            liveness_checks: 0,
        })
    }

    fn step(&mut self, operation: &WalkerOperation) -> Result<(), String> {
        if self.quit {
            return Ok(());
        }
        let resolved = resolve(
            operation,
            &self.state,
            &self.geometry,
            self.terminal_size(),
            self.session.local_action_inventory(),
        );
        match resolved {
            ResolvedOperation::Event(event) => self.dispatch_event(event),
            ResolvedOperation::LocalEvent { event, advertised } => {
                self.dispatch_local_event(event, &advertised)
            }
            ResolvedOperation::Resize { width, height } => {
                self.terminal.backend_mut().resize(width, height);
                self.terminal
                    .resize(Rect::new(0, 0, width, height))
                    .map_err(|error| error.to_string())?;
                self.dispatch_event(Event::Resize(width, height))
            }
            ResolvedOperation::Noop => self.checkpoint(Boundary::Session, None),
        }
    }

    fn dispatch_event(&mut self, event: Event) -> Result<(), String> {
        let handling = self
            .session
            .handle_event(event, &self.state, &self.geometry);
        self.finish_handling(handling)
    }

    fn dispatch_local_event(
        &mut self,
        event: Event,
        advertised: &LocalAdvertisedAction,
    ) -> Result<(), String> {
        match &event {
            Event::Key(key)
                if !advertised.keys.iter().any(|binding| {
                    let expected = binding.event();
                    expected.code == key.code
                        && expected.modifiers == key.modifiers
                        && expected.kind == key.kind
                }) =>
            {
                return Err(format!(
                    "LOCAL_ACTION_KEY target={:?} event={event:?} keys={:?}",
                    advertised.target, advertised.keys,
                ));
            }
            Event::Mouse(mouse)
                if !advertised
                    .hit
                    .is_some_and(|rect| rect.contains((mouse.column, mouse.row).into())) =>
            {
                return Err(format!(
                    "LOCAL_ACTION_RECT target={:?} event={event:?} hit={:?}",
                    advertised.target, advertised.hit,
                ));
            }
            Event::Key(_) | Event::Mouse(_) => {}
            Event::FocusGained | Event::FocusLost | Event::Paste(_) | Event::Resize(_, _) => {
                return Err(format!(
                    "LOCAL_ACTION_EVENT target={:?} event={event:?}",
                    advertised.target,
                ));
            }
        }
        let handling = self
            .session
            .handle_event(event.clone(), &self.state, &self.geometry);
        let matches = match (&advertised.outcome, &handling) {
            (LocalActionOutcome::Action(expected), EventHandling::Action(actual)) => {
                expected == actual
            }
            (LocalActionOutcome::Consumed, EventHandling::Consumed) => true,
            (LocalActionOutcome::Action(_), EventHandling::Consumed | EventHandling::Ignored)
            | (LocalActionOutcome::Consumed, EventHandling::Action(_) | EventHandling::Ignored) => {
                false
            }
        };
        if !matches {
            return Err(format!(
                "LOCAL_ACTION_ENDPOINT target={:?} event={event:?} hit={:?} expected={:?} actual={handling:?}",
                advertised.target, advertised.hit, advertised.outcome,
            ));
        }
        self.finish_handling(handling)
    }

    fn finish_handling(&mut self, handling: EventHandling) -> Result<(), String> {
        match handling {
            EventHandling::Action(action) => self.apply_action(action),
            EventHandling::Consumed | EventHandling::Ignored => {
                self.checkpoint(Boundary::Session, None)
            }
        }
    }

    fn apply_action(&mut self, action: Action) -> Result<(), String> {
        let effect = self.state.update(action);
        self.checkpoint(Boundary::UserAction, Some(&effect))?;
        self.drain_host_effects(effect)
    }

    fn drain_host_effects(&mut self, mut effect: Effect) -> Result<(), String> {
        for _ in 0..HOST_EFFECT_LIMIT {
            match effect {
                Effect::None => return Ok(()),
                Effect::Quit => {
                    self.quit = true;
                    return Ok(());
                }
                current => {
                    self.host.validate_effect(&current)?;
                    let action = self.host.serve(current)?;
                    if let Some((root, directories, files)) = self.host.file_picker_tree() {
                        self.session.set_file_picker_tree(root, directories, files);
                    }
                    if let Action::PreferencesSaved { locale, .. } = &action {
                        self.locale = detect_locale(Some(locale));
                    }
                    effect = self.state.update(action);
                    self.checkpoint(Boundary::HostAction, Some(&effect))?;
                }
            }
        }
        match effect {
            Effect::None => Ok(()),
            Effect::Quit => {
                self.quit = true;
                Ok(())
            }
            current => Err(format!(
                "the model host effect chain exceeded {HOST_EFFECT_LIMIT} actions: {current:?}"
            )),
        }
    }

    fn checkpoint(
        &mut self,
        boundary: Boundary,
        pending_effect: Option<&Effect>,
    ) -> Result<(), String> {
        let invariant = invariants::check_state(&self.state);
        let mut geometry = ViewGeometry::default();
        let render = self
            .terminal
            .draw(|frame| {
                geometry = render_with_session(frame, &self.state, self.locale, &mut self.session);
            })
            .map_err(|error| error.to_string());
        if let Err(render_error) = render {
            return match invariant {
                Ok(()) => Err(render_error),
                Err(invariant_error) => Err(format!(
                    "{invariant_error}; failing-state render also failed: {render_error}"
                )),
            };
        }
        self.geometry = geometry;
        self.recorder
            .record_frame(FRAME_INTERVAL, self.terminal.backend())
            .map_err(|error| error.to_string())?;
        let (add_stage, add_delete_candidate) = match self.state.screen() {
            Screen::Add(add) => (Some(add.stage()), add.delete_candidate().is_some()),
            Screen::Library
            | Screen::Run(_)
            | Screen::Preferences(_)
            | Screen::Health(_)
            | Screen::Runners(_)
            | Screen::Settings(_)
            | Screen::Form(_)
            | Screen::Report(_) => (None, false),
        };
        self.checkpoints.push(Checkpoint {
            boundary,
            pending_effect: pending_effect.is_some_and(host_effect_pending),
            add_stage,
            add_delete_candidate,
            size: self.terminal_size(),
        });
        invariant?;
        if self.probe_checks {
            check_public_hit_parity(
                &self.state,
                &self.geometry,
                self.terminal_size(),
                &self.session,
                self.locale,
            )?;
            if let Some(mut probe) = self.fork_probe()? {
                if let Some(effect) = pending_effect.filter(|effect| host_effect_pending(effect)) {
                    probe.drain_host_effects(effect.clone())?;
                }
                probe.assert_liveness()?;
                self.liveness_checks = self.liveness_checks.saturating_add(1);
            }
        }
        Ok(())
    }

    fn fork_probe(&self) -> Result<Option<Self>, String> {
        let Some(host) = self.host.try_fork() else {
            return Ok(None);
        };
        let session = self
            .session
            .try_fork()
            .ok_or("the persistent TUI session has an asynchronous worker and cannot fork")?;
        let size = self.terminal_size();
        let terminal = Terminal::new(TestBackend::new(size.width, size.height))
            .map_err(|error| error.to_string())?;
        let recorder =
            AsciicastRecorder::new(size.width, size.height).map_err(|error| error.to_string())?;
        Ok(Some(Self {
            host,
            state: self.state.clone(),
            locale: self.locale,
            session,
            terminal,
            geometry: self.geometry.clone(),
            recorder,
            checkpoints: Vec::new(),
            quit: self.quit,
            probe_checks: false,
            liveness_checks: 0,
        }))
    }

    fn assert_liveness(&mut self) -> Result<(), String> {
        for _ in 0..32 {
            if self.quit
                || (matches!(self.state.screen(), Screen::Library) && self.state.modal().is_none())
            {
                return Ok(());
            }
            let code = if matches!(self.state.modal(), Some(ModalState::ConfirmDiscardChanges)) {
                KeyCode::Char('y')
            } else {
                KeyCode::Esc
            };
            self.dispatch_event(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)))?;
        }
        Err(format!(
            "the UI did not return to the library or quit after 32 leave events: screen={:?} modal={:?}",
            self.state.screen(),
            self.state.modal()
        ))
    }

    fn state(&self) -> &LibraryState {
        &self.state
    }

    fn checkpoints(&self) -> &[Checkpoint] {
        &self.checkpoints
    }

    fn terminal_size(&self) -> Size {
        self.terminal.backend().buffer().area.as_size()
    }

    fn output_event_count(&self) -> usize {
        self.recorder
            .as_bytes()
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .count()
            .saturating_sub(1)
    }

    fn cast_bytes(&self) -> &[u8] {
        self.recorder.as_bytes()
    }
}

fn host_effect_pending(effect: &Effect) -> bool {
    !matches!(effect, Effect::None | Effect::Quit)
}

fn check_public_hit_parity(
    state: &LibraryState,
    geometry: &ViewGeometry,
    size: Size,
    session: &TuiSession,
    locale: Locale,
) -> Result<(), String> {
    for hit in &geometry.hits {
        if hit.rect.width == 0 || hit.rect.height == 0 {
            // Responsive layouts can retain a clipped footer item as a zero-area geometry
            // record. It is not visible and `resolve(PublicHit)` excludes it from mouse input.
            continue;
        }
        if hit.rect.right() > size.width || hit.rect.bottom() > size.height {
            return Err(format!(
                "PUBLIC_HIT_BOUNDS hit={hit:?} viewport={}x{}",
                size.width, size.height
            ));
        }
        let click = Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: hit.rect.x.saturating_add(hit.rect.width / 2),
            row: hit.rect.y.saturating_add(hit.rect.height / 2),
            modifiers: KeyModifiers::NONE,
        });
        let expected = expected_hit_action(hit.action, geometry, state)?;
        let mapped = map_event(click, state, geometry)
            .ok_or_else(|| format!("PUBLIC_HIT_MAP hit={hit:?}"))?;
        if mapped != expected {
            return Err(format!(
                "PUBLIC_HIT_ACTION hit={hit:?} actual={mapped:?} expected={expected:?}"
            ));
        }
    }

    // Fork the exact persistent widget state from this frame. Mouse and keyboard probes must see
    // the same cursor, scroll, dropdown, overlay, and private click registries as the real walk.
    for hit in &geometry.hits {
        if hit.rect.width == 0 || hit.rect.height == 0 {
            continue;
        }
        let click = Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: hit.rect.x.saturating_add(hit.rect.width / 2),
            row: hit.rect.y.saturating_add(hit.rect.height / 2),
            modifiers: KeyModifiers::NONE,
        });
        let expected = expected_hit_action(hit.action, geometry, state)?;
        let mut mouse_session = session
            .try_fork()
            .ok_or("the persistent TUI session cannot fork for mouse parity")?;
        let mouse_handling = mouse_session.handle_event(click, state, geometry);
        if matches!(
            hit.action,
            HitTarget::FocusField(_)
                | HitTarget::ToggleField(_)
                | HitTarget::SelectFieldOption { .. }
        ) {
            check_field_hit_parity(
                FieldParityContext {
                    session,
                    state,
                    geometry,
                    locale,
                    size,
                },
                hit.action,
                mouse_session,
                mouse_handling,
            )?;
            continue;
        }
        let click_action = match mouse_handling {
            EventHandling::Action(action) => action,
            handling => {
                return Err(format!(
                    "PUBLIC_SESSION_HIT hit={hit:?} handling={handling:?} expected={expected:?}"
                ));
            }
        };
        if click_action != expected
            && !matches!(
                hit.action,
                HitTarget::Command(UiCommand::FocusNext | UiCommand::FocusPrevious)
            )
        {
            return Err(format!(
                "PUBLIC_SESSION_HIT hit={hit:?} actual={click_action:?} expected={expected:?}"
            ));
        }
        match hit.action {
            HitTarget::Command(command) => {
                let bindings = public_command_bindings(session, state, command)?;
                let _ = command_key_action(
                    session,
                    state,
                    geometry,
                    command,
                    &bindings,
                    &click_action,
                    0,
                )?;
            }
            HitTarget::RunFieldCommand { field, command } => {
                let mut key_state = state.clone();
                let focus_effect = key_state.update(Action::FocusField(field));
                if host_effect_pending(&focus_effect) {
                    return Err(format!(
                        "RUN_FIELD_FOCUS_EFFECT field={field} effect={focus_effect:?}"
                    ));
                }
                // A field chip can focus a field in one click. Its keyboard path can use a
                // bounded Tab/BackTab prefix before it invokes the advertised command. Compare
                // the command from that shared focused state instead of treating focus itself as
                // a semantic difference.
                let mut mouse_state = key_state.clone();
                let mouse_effect = mouse_state.update(click_action.clone());
                if command == UiCommand::BrowsePath {
                    let key_action = browse_keyboard_action(session, &key_state, geometry, field)?;
                    if key_action != click_action {
                        return Err(format!(
                            "RUN_BROWSE_PARITY field={field} mouse={click_action:?} key={key_action:?}"
                        ));
                    }
                    let key_effect = key_state.update(key_action);
                    if mouse_state != key_state || mouse_effect != key_effect {
                        return Err(format!(
                            "RUN_BROWSE_ENDPOINT field={field} mouse_effect={mouse_effect:?} key_effect={key_effect:?}"
                        ));
                    }
                } else {
                    let bindings = command_bindings(&key_state, command)?;
                    let _ = command_key_action(
                        session,
                        &key_state,
                        geometry,
                        command,
                        &bindings,
                        &click_action,
                        64,
                    )?;
                }
            }
            HitTarget::FocusField(field) => {
                return Err(format!(
                    "FOCUS_PARITY_INTERNAL field={field}; the typed field branch did not run"
                ));
            }
            HitTarget::ToggleField(_) | HitTarget::SelectFieldOption { .. } => {
                return Err("FIELD_PARITY_INTERNAL; the typed field branch did not run".to_owned());
            }
        }
    }
    Ok(())
}

fn public_command_bindings(
    session: &TuiSession,
    state: &LibraryState,
    command: UiCommand,
) -> Result<Vec<UiBinding>, String> {
    let footer = session.advertised_command_bindings(state, command);
    if !footer.is_empty() {
        return Ok(footer);
    }
    command_specs(state.command_context())
        .find(|spec| !spec.footer && spec.command == command && state.command_enabled(command))
        .map(|spec| spec.bindings.to_vec())
        .ok_or_else(|| format!("visible command {command:?} has no printed key binding"))
}

#[derive(Debug, PartialEq)]
struct ProbeEndpoint {
    state: LibraryState,
    geometry: ViewGeometry,
    backend: TestBackend,
}

#[derive(Clone, Copy)]
struct FieldParityContext<'a> {
    session: &'a TuiSession,
    state: &'a LibraryState,
    geometry: &'a ViewGeometry,
    locale: Locale,
    size: Size,
}

fn check_field_hit_parity(
    context: FieldParityContext<'_>,
    target: HitTarget,
    mut mouse_session: TuiSession,
    mouse_handling: EventHandling,
) -> Result<(), String> {
    let FieldParityContext {
        session: base_session,
        state,
        geometry,
        locale,
        size,
    } = context;
    let field = match target {
        HitTarget::FocusField(field)
        | HitTarget::ToggleField(field)
        | HitTarget::SelectFieldOption { field, .. } => field,
        HitTarget::Command(_) | HitTarget::RunFieldCommand { .. } => {
            return Err(format!("FIELD_PARITY_TARGET target={target:?}"));
        }
    };
    if matches!(
        target,
        HitTarget::ToggleField(_) | HitTarget::SelectFieldOption { .. }
    ) {
        let expected = expected_hit_action(target, geometry, state)?;
        if mouse_handling != EventHandling::Action(expected.clone()) {
            return Err(format!(
                "FIELD_SESSION_ACTION target={target:?} actual={mouse_handling:?} expected={expected:?}"
            ));
        }
    }
    let mouse_is_plain_focus = matches!(
        &mouse_handling,
        EventHandling::Action(Action::FocusField(actual)) if *actual == field
    );
    let mut mouse_state = state.clone();
    apply_probe_handling(
        &mut mouse_state,
        mouse_handling,
        &format!("mouse target={target:?}"),
    )?;
    let mouse_endpoint = render_probe_endpoint(&mut mouse_session, &mouse_state, locale, size)?;
    let field_count = state.form().map_or_else(
        || state.run_form().map_or(0, |form| form.fields().len()),
        |form| form.fields.len(),
    );
    if field_count == 0 || field >= field_count {
        return Err(format!(
            "FIELD_PARITY_OWNER target={target:?} fields={field_count}"
        ));
    }
    let mut focus_session = base_session
        .try_fork()
        .ok_or("the persistent TUI session cannot fork for direct focus parity")?;
    let mut focus_state = state.clone();
    let focus_effect = focus_state.update(Action::FocusField(field));
    if host_effect_pending(&focus_effect) {
        return Err(format!(
            "FIELD_FOCUS_EFFECT field={field} effect={focus_effect:?}"
        ));
    }
    let focus_endpoint = render_probe_endpoint(&mut focus_session, &focus_state, locale, size)?;
    if target == HitTarget::FocusField(field)
        && mouse_is_plain_focus
        && is_plain_focus_field(state, field)
        && focus_endpoint == mouse_endpoint
    {
        return check_keyboard_focus_path(
            context,
            target,
            field,
            field_count,
            &mouse_endpoint.state,
        );
    }
    if state.focused_form_field() != Some(field) {
        check_keyboard_focus_path(context, target, field, field_count, &focus_endpoint.state)?;
    }

    let activations = field_activation_sequences(state, target);
    let mut failures = Vec::new();
    for activation in &activations {
        let mut candidate_session = focus_session
            .try_fork()
            .ok_or("the focused TUI session cannot fork for field activation")?;
        let mut candidate_state = focus_endpoint.state.clone();
        let mut candidate_geometry = focus_endpoint.geometry.clone();
        let mut activation_failed = false;
        for key in activation {
            let handling = candidate_session.handle_event(
                Event::Key(*key),
                &candidate_state,
                &candidate_geometry,
            );
            if let Err(error) = apply_probe_handling(
                &mut candidate_state,
                handling,
                &format!("activation target={target:?} key={key:?}"),
            ) {
                failures.push(error);
                activation_failed = true;
                break;
            }
            candidate_geometry =
                render_probe_endpoint(&mut candidate_session, &candidate_state, locale, size)?
                    .geometry;
        }
        if activation_failed {
            continue;
        }
        let candidate =
            render_probe_endpoint(&mut candidate_session, &candidate_state, locale, size)?;
        if candidate == mouse_endpoint {
            return Ok(());
        }
        failures.push(format!(
            "endpoint target={target:?} activation={activation:?} state_equal={} geometry_equal={} buffer_equal={} cursor_equal={}",
            candidate.state == mouse_endpoint.state,
            candidate.geometry == mouse_endpoint.geometry,
            candidate.backend.buffer() == mouse_endpoint.backend.buffer(),
            candidate.backend.cursor_position() == mouse_endpoint.backend.cursor_position(),
        ));
    }
    Err(format!(
        "FIELD_SESSION_PARITY target={target:?} mouse_focus={:?} failures={failures:?}",
        mouse_endpoint.state.focused_form_field(),
    ))
}

fn is_plain_focus_field(state: &LibraryState, field: usize) -> bool {
    if let Some(form) = state.run_form() {
        return form
            .fields()
            .get(field)
            .is_some_and(|field| matches!(&field.control, skit_ui::FormControl::Text(_)));
    }
    state.form().is_some_and(|form| field < form.fields.len())
}

fn check_keyboard_focus_path(
    context: FieldParityContext<'_>,
    target: HitTarget,
    field: usize,
    field_count: usize,
    mouse_state: &LibraryState,
) -> Result<(), String> {
    let mut failures = Vec::new();
    for navigation in [
        KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
        KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT),
    ] {
        let mut key_session = context
            .session
            .try_fork()
            .ok_or("the persistent TUI session cannot fork for direct focus parity")?;
        let mut key_state = context.state.clone();
        let mut key_geometry = context.geometry.clone();
        for step in 1..=field_count {
            let handling =
                key_session.handle_event(Event::Key(navigation), &key_state, &key_geometry);
            apply_probe_handling(
                &mut key_state,
                handling,
                &format!("direct focus field={field} step={step}"),
            )?;
            key_geometry =
                render_probe_endpoint(&mut key_session, &key_state, context.locale, context.size)?
                    .geometry;
            let visible = key_geometry
                .hits
                .iter()
                .any(|hit| hit.rect.width > 0 && hit.rect.height > 0 && hit.action == target);
            // One advertised keyboard path is sufficient. Mouse focus can jump directly to a
            // field, while Tab and BackTab must traverse the active focus ring.
            if key_state == *mouse_state && visible {
                return Ok(());
            }
            failures.push(format!(
                "navigation={navigation:?} step={step} focus={:?} state_equal={} visible={visible}",
                key_state.focused_form_field(),
                key_state == *mouse_state,
            ));
        }
    }
    Err(format!(
        "FIELD_FOCUS_KEY_PATH field={field} fields={field_count} mouse_focus={:?} failures={failures:?}",
        mouse_state.focused_form_field(),
    ))
}

fn field_activation_sequences(state: &LibraryState, target: HitTarget) -> Vec<Vec<KeyEvent>> {
    let key = |code| KeyEvent::new(code, KeyModifiers::NONE);
    match target {
        HitTarget::ToggleField(_) => vec![vec![key(KeyCode::Char(' '))]],
        HitTarget::SelectFieldOption { field, .. } => {
            let options = state
                .run_form()
                .and_then(|form| form.fields().get(field))
                .and_then(|field| match &field.control {
                    skit_ui::FormControl::Choice(control) => Some(control.options.len()),
                    skit_ui::FormControl::Text(_) | skit_ui::FormControl::Checkbox { .. } => None,
                })
                .unwrap_or(0);
            (1..=options.max(1))
                .flat_map(|count| {
                    [
                        vec![key(KeyCode::Right); count],
                        vec![key(KeyCode::Left); count],
                    ]
                })
                .chain([
                    vec![key(KeyCode::Right), key(KeyCode::Left)],
                    vec![key(KeyCode::Left), key(KeyCode::Right)],
                ])
                .collect()
        }
        HitTarget::FocusField(_) => vec![
            vec![key(KeyCode::Esc)],
            vec![key(KeyCode::Enter)],
            vec![key(KeyCode::Char(' '))],
            vec![key(KeyCode::Down)],
            vec![key(KeyCode::Up)],
            vec![key(KeyCode::Right)],
            vec![key(KeyCode::Left)],
        ],
        HitTarget::Command(_) | HitTarget::RunFieldCommand { .. } => Vec::new(),
    }
}

fn apply_probe_handling(
    state: &mut LibraryState,
    handling: EventHandling,
    context: &str,
) -> Result<(), String> {
    match handling {
        EventHandling::Action(action) => {
            let effect = state.update(action);
            if host_effect_pending(&effect) {
                return Err(format!("FIELD_SESSION_EFFECT {context} effect={effect:?}"));
            }
            Ok(())
        }
        EventHandling::Consumed => Ok(()),
        EventHandling::Ignored => Err(format!("FIELD_SESSION_IGNORED {context}")),
    }
}

fn render_probe_endpoint(
    session: &mut TuiSession,
    state: &LibraryState,
    locale: Locale,
    size: Size,
) -> Result<ProbeEndpoint, String> {
    let mut terminal = Terminal::new(TestBackend::new(size.width, size.height))
        .map_err(|error| error.to_string())?;
    let mut geometry = ViewGeometry::default();
    terminal
        .draw(|frame| {
            geometry = render_with_session(frame, state, locale, session);
        })
        .map_err(|error| error.to_string())?;
    Ok(ProbeEndpoint {
        state: state.clone(),
        geometry,
        backend: terminal.backend().clone(),
    })
}

fn render_probe_session(
    state: &LibraryState,
    locale: Locale,
    size: Size,
) -> Result<(TuiSession, ViewGeometry), String> {
    let mut session = TuiSession::default();
    let mut terminal = Terminal::new(TestBackend::new(size.width, size.height))
        .map_err(|error| error.to_string())?;
    let mut geometry = ViewGeometry::default();
    terminal
        .draw(|frame| {
            geometry = render_with_session(frame, state, locale, &mut session);
        })
        .map_err(|error| error.to_string())?;
    Ok((session, geometry))
}

fn expected_hit_action(
    target: HitTarget,
    geometry: &ViewGeometry,
    state: &LibraryState,
) -> Result<Action, String> {
    match target {
        HitTarget::Command(UiCommand::ToggleDetail) => Ok(Action::ToggleDetail {
            currently_visible: geometry.detail_pane_visible,
        }),
        HitTarget::Command(command) => command
            .direct_action()
            .ok_or_else(|| format!("PUBLIC_HIT_CONTEXT command={command:?}")),
        HitTarget::RunFieldCommand {
            field,
            command: UiCommand::BrowsePath,
        } => Ok(Action::OpenRunFilePicker(field)),
        HitTarget::RunFieldCommand {
            field,
            command: UiCommand::InsertValue,
        } => Ok(Action::OpenRunTokenMenuFor(field)),
        HitTarget::RunFieldCommand {
            field,
            command: UiCommand::ResetDefault,
        } => Ok(Action::ResetRunField(field)),
        HitTarget::RunFieldCommand { command, .. } => command
            .direct_action()
            .ok_or_else(|| format!("RUN_FIELD_HIT_CONTEXT command={command:?}")),
        HitTarget::FocusField(field) => Ok(Action::FocusField(field)),
        HitTarget::ToggleField(field) => Ok(Action::ToggleField(field)),
        HitTarget::SelectFieldOption { field, option } => state
            .run_form()
            .and_then(|form| form.fields().get(field))
            .and_then(|field| match &field.control {
                skit_ui::FormControl::Choice(control) => control.options.get(option),
                skit_ui::FormControl::Text(_) | skit_ui::FormControl::Checkbox { .. } => None,
            })
            .cloned()
            .map(|value| Action::SelectFieldOption { field, value })
            .ok_or_else(|| format!("RUN_OPTION_HIT field={field} option={option}")),
    }
}

fn session_action(
    session: &mut TuiSession,
    event: Event,
    state: &LibraryState,
    geometry: &ViewGeometry,
) -> Result<Action, String> {
    let event_debug = event.clone();
    match session.handle_event(event, state, geometry) {
        EventHandling::Action(action) => Ok(action),
        handling => Err(format!(
            "SESSION_ACTION state={:?} event={event_debug:?} handling={handling:?}",
            state.command_context(),
        )),
    }
}

fn browse_keyboard_action(
    base_session: &TuiSession,
    state: &LibraryState,
    geometry: &ViewGeometry,
    field: usize,
) -> Result<Action, String> {
    if !state.command_enabled(UiCommand::BrowsePath) {
        return Err(format!("RUN_BROWSE_CAPABILITY field={field}"));
    }
    let mut keyboard_state = state.clone();
    let mut session = base_session
        .try_fork()
        .ok_or("the persistent TUI session cannot fork for browse parity")?;
    let open_tokens = command_key_action_with_session(
        &mut session,
        &keyboard_state,
        geometry,
        UiCommand::InsertValue,
    )?;
    let effect = keyboard_state.update(open_tokens);
    if host_effect_pending(&effect) {
        return Err(format!("RUN_TOKEN_EFFECT field={field} effect={effect:?}"));
    }
    let options = match keyboard_state.modal() {
        Some(ModalState::RunTokenMenu {
            field: target,
            options,
        }) if *target == field => options,
        modal => {
            return Err(format!("RUN_TOKEN_OWNER field={field} modal={modal:?}"));
        }
    };
    let file_index = options
        .iter()
        .position(|option| matches!(option, RunTokenOption::FileOrFolder))
        .ok_or_else(|| format!("RUN_TOKEN_FILE_OPTION field={field} options={options:?}"))?;
    if file_index > 0 {
        if file_index + 1 != options.len() {
            return Err(format!(
                "RUN_TOKEN_FILE_POSITION field={field} index={file_index} options={}",
                options.len()
            ));
        }
        match session.handle_event(
            Event::Key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE)),
            &keyboard_state,
            geometry,
        ) {
            EventHandling::Consumed => {}
            handling => {
                return Err(format!("RUN_TOKEN_END field={field} handling={handling:?}"));
            }
        }
    }
    session_action(
        &mut session,
        Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        &keyboard_state,
        geometry,
    )
}

fn command_key_action(
    base_session: &TuiSession,
    state: &LibraryState,
    geometry: &ViewGeometry,
    command: UiCommand,
    bindings: &[UiBinding],
    expected: &Action,
    max_prefix: usize,
) -> Result<Action, String> {
    if bindings.is_empty() {
        return Err(format!("visible command {command:?} has no key binding"));
    }
    let mut failures = Vec::new();
    let mut last_action = None;
    for binding in bindings.iter().copied() {
        let mut binding_action = None;
        for prefix in 0..=max_prefix {
            let mut probe_state = state.clone();
            let mut session = base_session
                .try_fork()
                .ok_or("the persistent TUI session cannot fork for key parity")?;
            let mut prefix_failed = None;
            for step in 0..prefix {
                match session.handle_event(
                    Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
                    &probe_state,
                    geometry,
                ) {
                    EventHandling::Action(action) => {
                        let effect = probe_state.update(action);
                        if host_effect_pending(&effect) {
                            prefix_failed =
                                Some(format!("focus_effect step={step} effect={effect:?}"));
                            break;
                        }
                    }
                    EventHandling::Consumed => {}
                    EventHandling::Ignored => {
                        prefix_failed = Some(format!("focus_ignored step={step}"));
                        break;
                    }
                }
            }
            if let Some(reason) = prefix_failed {
                failures.push(format!("binding={binding:?} prefix={prefix} {reason}"));
                break;
            }
            match session.handle_event(Event::Key(binding_event(binding)), &probe_state, geometry) {
                EventHandling::Action(action) => {
                    let mut expected_state = probe_state.clone();
                    let expected_effect = expected_state.update(expected.clone());
                    let effect = probe_state.update(action.clone());
                    if probe_state == expected_state && effect == expected_effect {
                        binding_action = Some(action);
                        break;
                    }
                    failures.push(format!(
                        "binding={binding:?} prefix={prefix} action={action:?} effect={effect:?}"
                    ));
                }
                handling => failures.push(format!(
                    "binding={binding:?} prefix={prefix} handling={handling:?}"
                )),
            }
        }
        let Some(action) = binding_action else {
            return Err(format!(
                "COMMAND_KEY_PATH command={command:?} binding={binding:?} expected={expected:?} failures={failures:?}"
            ));
        };
        last_action = Some(action);
    }
    last_action.ok_or_else(|| format!("visible command {command:?} has no key binding"))
}

fn command_key_action_with_session(
    session: &mut TuiSession,
    state: &LibraryState,
    geometry: &ViewGeometry,
    command: UiCommand,
) -> Result<Action, String> {
    let binding = command_binding(state, command)?;
    session_action(session, Event::Key(binding_event(binding)), state, geometry)
}

fn command_binding(state: &LibraryState, command: UiCommand) -> Result<UiBinding, String> {
    command_bindings(state, command)?
        .first()
        .copied()
        .ok_or_else(|| format!("visible command {command:?} has no key binding"))
}

fn command_bindings(state: &LibraryState, command: UiCommand) -> Result<Vec<UiBinding>, String> {
    let spec = command_specs(state.command_context())
        .find(|spec| spec.command == command && state.command_enabled(command))
        .ok_or_else(|| format!("visible command {command:?} is not advertised in this context"))?;
    if spec.bindings.is_empty() {
        return Err(format!("visible command {command:?} has no key binding"));
    }
    Ok(spec.bindings.to_vec())
}

fn binding_event(binding: UiBinding) -> KeyEvent {
    let code = match binding.key {
        UiKey::Character(character) => KeyCode::Char(character),
        UiKey::Enter => KeyCode::Enter,
        UiKey::Escape => KeyCode::Esc,
        UiKey::Delete => KeyCode::Delete,
        UiKey::Backspace => KeyCode::Backspace,
        UiKey::Tab => KeyCode::Tab,
        UiKey::BackTab => KeyCode::BackTab,
        UiKey::Up => KeyCode::Up,
        UiKey::Down => KeyCode::Down,
        UiKey::PageUp => KeyCode::PageUp,
        UiKey::PageDown => KeyCode::PageDown,
        UiKey::Home => KeyCode::Home,
        UiKey::End => KeyCode::End,
        UiKey::Function(number) => KeyCode::F(number),
    };
    let mut modifiers = KeyModifiers::NONE;
    modifiers.set(KeyModifiers::CONTROL, binding.modifiers.control);
    modifiers.set(KeyModifiers::ALT, binding.modifiers.alt);
    modifiers.set(KeyModifiers::SHIFT, binding.modifiers.shift);
    KeyEvent::new(code, modifiers)
}

fn canvas_size(initial: Size, operations: &[WalkerOperation]) -> Size {
    operations.iter().fold(initial, |canvas, operation| {
        if let WalkerOperation::Resize { width, height } = operation {
            Size::new(canvas.width.max(*width), canvas.height.max(*height))
        } else {
            canvas
        }
    })
}

#[derive(Debug)]
struct CaseFailure {
    error: String,
    cast: Vec<u8>,
}

#[derive(Debug)]
struct CaseSuccess {
    cast: Vec<u8>,
}

fn evaluate_case(
    operations: &[WalkerOperation],
    locale: Locale,
    initial: Size,
) -> Result<CaseSuccess, CaseFailure> {
    evaluate_case_with_host(FakeHost::new(), operations, locale, initial)
}

fn evaluate_case_with_host<H: WalkerHost>(
    host: H,
    operations: &[WalkerOperation],
    locale: Locale,
    initial: Size,
) -> Result<CaseSuccess, CaseFailure> {
    let canvas = canvas_size(initial, operations);
    let construction = catch_unwind(AssertUnwindSafe(|| {
        Walker::unstarted(host, locale, initial, canvas)
    }));
    let mut walker = match construction {
        Ok(Ok(walker)) => walker,
        Ok(Err(error)) => {
            return Err(CaseFailure {
                error,
                cast: Vec::new(),
            });
        }
        Err(payload) => {
            let cast = AsciicastRecorder::new(canvas.width, canvas.height)
                .map_or_else(|_| Vec::new(), |recorder| recorder.as_bytes().to_vec());
            return Err(CaseFailure {
                error: format!(
                    "walker initialization panic: {}",
                    panic_message(payload.as_ref())
                ),
                cast,
            });
        }
    };
    let outcome = catch_unwind(AssertUnwindSafe(|| -> Result<(), String> {
        walker.checkpoint(Boundary::Initial, None)?;
        for operation in operations {
            walker.step(operation)?;
            if walker.quit {
                break;
            }
        }
        walker.assert_liveness()
    }));
    match outcome {
        Ok(Ok(())) => Ok(CaseSuccess {
            cast: walker.cast_bytes().to_vec(),
        }),
        Ok(Err(error)) => Err(CaseFailure {
            error,
            cast: walker.cast_bytes().to_vec(),
        }),
        Err(payload) => Err(CaseFailure {
            error: format!("walker panic: {}", panic_message(payload.as_ref())),
            cast: walker.cast_bytes().to_vec(),
        }),
    }
}

fn panic_message(payload: &(dyn Any + Send)) -> String {
    payload.downcast_ref::<String>().map_or_else(
        || {
            payload.downcast_ref::<&str>().map_or_else(
                || "non-text panic".to_owned(),
                |message| (*message).to_owned(),
            )
        },
        Clone::clone,
    )
}

fn write_failure_artifacts(
    directory: &Path,
    operations: &[WalkerOperation],
    locale: Locale,
    initial: Size,
    error: &str,
    cast: &[u8],
) -> Result<std::path::PathBuf, String> {
    fs::create_dir_all(directory).map_err(|failure| failure.to_string())?;
    let staged = tempfile::Builder::new()
        .prefix(".failure-")
        .tempdir_in(directory)
        .map_err(|failure| failure.to_string())?;
    let repro = serde_json::to_vec_pretty(&serde_json::json!({
        "version": 1,
        "locale": locale.tag(),
        "initial_size": {"cols": initial.width, "rows": initial.height},
        "error": error,
        "regression_file": "../regressions.txt",
        "cast": (!cast.is_empty()).then_some("failure.cast"),
        "operations": operations,
    }))
    .map_err(|failure| failure.to_string())?;
    if !cast.is_empty() {
        fs::write(staged.path().join("failure.cast"), cast)
            .map_err(|failure| failure.to_string())?;
    }
    fs::write(staged.path().join("repro.json"), repro).map_err(|failure| failure.to_string())?;
    let staged_path = staged.keep();
    let bundle_name = staged_path
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_prefix('.'))
        .ok_or_else(|| format!("invalid staged artifact path: {}", staged_path.display()))?;
    let final_path = directory.join(bundle_name);
    fs::rename(staged_path, &final_path).map_err(|failure| failure.to_string())?;
    Ok(final_path)
}

fn write_success_artifacts(
    directory: &Path,
    operations: &[WalkerOperation],
    locale: Locale,
    initial: Size,
    cast: &[u8],
) -> Result<std::path::PathBuf, String> {
    fs::create_dir_all(directory).map_err(|failure| failure.to_string())?;
    let staged = tempfile::Builder::new()
        .prefix(".success-")
        .tempdir_in(directory)
        .map_err(|failure| failure.to_string())?;
    let repro = serde_json::to_vec_pretty(&serde_json::json!({
        "version": 1,
        "locale": locale.tag(),
        "initial_size": {"cols": initial.width, "rows": initial.height},
        "result": "passed",
        "cast": "success.cast",
        "operations": operations,
    }))
    .map_err(|failure| failure.to_string())?;
    fs::write(staged.path().join("success.cast"), cast).map_err(|failure| failure.to_string())?;
    fs::write(staged.path().join("repro.json"), repro).map_err(|failure| failure.to_string())?;
    let staged_path = staged.keep();
    let bundle_name = staged_path
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_prefix('.'))
        .ok_or_else(|| format!("invalid staged artifact path: {}", staged_path.display()))?;
    let final_path = directory.join(bundle_name);
    fs::rename(staged_path, &final_path).map_err(|failure| failure.to_string())?;
    Ok(final_path)
}

fn run_property_profile(
    test_name: &'static str,
    cases: u32,
    steps: usize,
    locales: &[Locale],
    initial_sizes: &[Size],
    record_success: bool,
) {
    fs::create_dir_all(ARTIFACT_DIR).expect("the walker artifact directory must be writable");
    let config = Config {
        cases,
        max_shrink_iters: 512,
        failure_persistence: Some(Box::new(FileFailurePersistence::Direct(REGRESSION_FILE))),
        source_file: Some(file!()),
        test_name: Some(test_name),
        rng_algorithm: RngAlgorithm::ChaCha,
        ..Config::default()
    };
    let strategy = collection::vec(operation_strategy(), 0..=steps);
    let requested_success = RefCell::new(None);
    let result = TestRunner::new(config).run(&strategy, |operations| {
        for locale in locales {
            for initial in initial_sizes {
                match evaluate_case(&operations, *locale, *initial) {
                    Ok(success) if record_success => {
                        *requested_success.borrow_mut() =
                            Some((operations.clone(), *locale, *initial, success.cast));
                    }
                    Ok(_) => {}
                    Err(failure) => {
                        return Err(TestCaseError::fail(format!(
                            "locale={} initial={}x{}: {}",
                            locale.tag(),
                            initial.width,
                            initial.height,
                            failure.error,
                        )));
                    }
                }
            }
        }
        Ok(())
    });
    match result {
        Ok(()) => {
            if record_success {
                let (operations, locale, initial, cast) = requested_success
                    .into_inner()
                    .expect("a requested successful recording must retain the final case");
                let artifact_path = write_success_artifacts(
                    Path::new(ARTIFACT_DIR),
                    &operations,
                    locale,
                    initial,
                    &cast,
                )
                .unwrap_or_else(|error| panic!("cannot write the requested cast: {error}"));
                eprintln!(
                    "the requested successful UI walk is in {}",
                    artifact_path.display()
                );
            }
        }
        Err(TestError::Fail(reason, minimal)) => {
            let replay = locales.iter().find_map(|locale| {
                initial_sizes.iter().find_map(|initial| {
                    evaluate_case(&minimal, *locale, *initial)
                        .err()
                        .map(|failure| (*locale, *initial, failure))
                })
            });
            let (locale, initial, failure) = replay.unwrap_or_else(|| {
                panic!(
                    "the minimal trace no longer reproduces the property failure: {reason}; trace={minimal:#?}"
                )
            });
            let artifact_path = write_failure_artifacts(
                Path::new(ARTIFACT_DIR),
                &minimal,
                locale,
                initial,
                &failure.error,
                &failure.cast,
            )
            .unwrap_or_else(|artifact_error| {
                panic!(
                    "the model walker failed and artifact capture also failed: property={reason}; artifact={artifact_error}; trace={minimal:#?}"
                )
            });
            panic!(
                "the model walker found a minimal failure: {reason}; trace={minimal:#?}; artifacts={}; seed is in {REGRESSION_FILE}",
                artifact_path.display()
            );
        }
        Err(TestError::Abort(reason)) => {
            panic!("the model walker aborted before it produced a case: {reason}");
        }
    }
}

#[test]
fn bounded_smoke_walk_checks_every_transition_boundary() {
    run_property_profile(
        "bounded_smoke_walk_checks_every_transition_boundary",
        8,
        40,
        &[Locale::En],
        &[Size::new(80, 24)],
        false,
    );
}

#[test]
#[ignore = "the nightly and label workflow owns the complete model walk"]
fn nightly_model_walk() {
    let cases = std::env::var("SKIT_WALKER_CASES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(16);
    let steps = std::env::var("SKIT_WALKER_STEPS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(100);
    assert!(cases > 0, "SKIT_WALKER_CASES must be greater than zero");
    assert!(steps > 0, "SKIT_WALKER_STEPS must be greater than zero");
    let record_success = match std::env::var("SKIT_WALKER_RECORD_SUCCESS").as_deref() {
        Ok("1") => true,
        Ok("0") | Err(std::env::VarError::NotPresent) => false,
        Ok(value) => panic!("SKIT_WALKER_RECORD_SUCCESS must be 0 or 1, got {value}"),
        Err(error) => panic!("cannot read SKIT_WALKER_RECORD_SUCCESS: {error}"),
    };
    run_property_profile(
        "nightly_model_walk",
        cases,
        steps,
        &[Locale::En, Locale::ZhCn, Locale::ZhTw, Locale::Pseudo],
        &[Size::new(1, 1), Size::new(24, 6), Size::new(120, 30)],
        record_success,
    );
}

#[derive(serde::Deserialize)]
struct SavedRepro {
    locale: String,
    initial_size: SavedSize,
    operations: Vec<WalkerOperation>,
}

#[derive(serde::Deserialize)]
struct SavedSize {
    cols: u16,
    rows: u16,
}

#[test]
#[ignore = "set SKIT_WALKER_REPRO to replay one saved minimal trace"]
fn replay_saved_model_walk() {
    let path = std::path::PathBuf::from(
        std::env::var("SKIT_WALKER_REPRO")
            .expect("SKIT_WALKER_REPRO must name one saved repro.json"),
    );
    let repro: SavedRepro = serde_json::from_slice(
        &fs::read(&path).unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display())),
    )
    .unwrap_or_else(|error| panic!("cannot decode {}: {error}", path.display()));
    evaluate_case(
        &repro.operations,
        detect_locale(Some(&repro.locale)),
        Size::new(repro.initial_size.cols, repro.initial_size.rows),
    )
    .unwrap_or_else(|failure| panic!("saved trace reproduced: {}", failure.error));
}

#[test]
fn initial_state_is_checked_and_rendered_before_the_first_event() {
    let walker = Walker::new(FakeHost::new(), Locale::En, Size::new(80, 24)).unwrap();
    assert_eq!(walker.checkpoints().len(), 1);
    assert_eq!(walker.checkpoints()[0].boundary, Boundary::Initial);
    assert_eq!(walker.output_event_count(), 1);
}

#[test]
fn consumed_and_ignored_events_still_cross_a_render_boundary() {
    let mut walker = Walker::new(FakeHost::new(), Locale::En, Size::new(80, 24)).unwrap();
    let before = walker.checkpoints().len();
    walker
        .step(&WalkerOperation::Focus { gained: true })
        .unwrap();
    assert_eq!(walker.checkpoints().len(), before + 1);
    assert_eq!(walker.liveness_checks, walker.checkpoints().len());
    assert_eq!(
        walker.checkpoints().last().unwrap().boundary,
        Boundary::Session
    );

    walker.apply_action(Action::OpenAdd).unwrap();
    assert!(
        !walker.session.local_action_inventory().actions.is_empty(),
        "the rendered Add footer must advertise its path picker"
    );
    let before_overlay = walker.output_event_count();
    walker
        .dispatch_event(Event::Key(KeyEvent::new(
            KeyCode::Char('o'),
            KeyModifiers::CONTROL,
        )))
        .unwrap();
    assert_eq!(
        walker.checkpoints().last().unwrap().boundary,
        Boundary::Session
    );
    assert!(
        walker.session.local_action_inventory().actions.is_empty(),
        "the consumed event must refresh the inventory for the open local overlay"
    );
    assert!(
        walker.output_event_count() > before_overlay,
        "the consumed session mutation must emit its changed frame"
    );
    let cast = String::from_utf8_lossy(walker.cast_bytes());
    assert!(cast.contains("/fixtures"));
    assert!(
        !cast.contains(env!("CARGO_MANIFEST_DIR")),
        "the deterministic cast must not expose its checkout path"
    );
    assert_eq!(walker.liveness_checks, walker.checkpoints().len());
}

#[test]
fn filtered_quit_operations_do_not_discard_the_trace_suffix() {
    let mut walker = Walker::new(FakeHost::new(), Locale::En, Size::new(80, 24)).unwrap();
    let before = walker.checkpoints().len();
    walker
        .step(&WalkerOperation::RawKey {
            key: super::strategy::RawKey::Escape,
            kind: super::strategy::KeyKind::Press,
        })
        .unwrap();
    assert!(!walker.quit);
    walker
        .step(&WalkerOperation::Focus { gained: true })
        .unwrap();
    assert_eq!(walker.checkpoints().len(), before + 2);
}

#[test]
fn local_dispatch_checks_the_persistent_session_once_against_its_descriptor() {
    let mut walker = Walker::new(FakeHost::new(), Locale::En, Size::new(80, 24)).unwrap();
    walker.apply_action(Action::OpenAdd).unwrap();
    let browse = walker
        .session
        .local_action_inventory()
        .actions
        .iter()
        .find(|advertised| advertised.target == LocalActionTarget::Add(AddControlId::BrowseSource))
        .cloned()
        .expect("Add source picker is advertised");
    let before = walker.checkpoints().len();
    walker
        .dispatch_local_event(Event::Key(browse.keys[0].event()), &browse)
        .unwrap();
    assert_eq!(walker.checkpoints().len(), before + 1);
    assert!(walker.session.local_action_inventory().actions.is_empty());

    let mut mismatch = Walker::new(FakeHost::new(), Locale::En, Size::new(80, 24)).unwrap();
    mismatch.apply_action(Action::OpenAdd).unwrap();
    let mut false_descriptor = mismatch
        .session
        .local_action_inventory()
        .actions
        .iter()
        .find(|advertised| advertised.target == LocalActionTarget::Add(AddControlId::BrowseSource))
        .cloned()
        .unwrap();
    false_descriptor.outcome = LocalActionOutcome::Action(Action::Quit);
    let error = mismatch
        .dispatch_local_event(
            Event::Key(false_descriptor.keys[0].event()),
            &false_descriptor,
        )
        .unwrap_err();
    assert!(error.contains("LOCAL_ACTION_ENDPOINT"), "{error}");
    assert!(!mismatch.quit);
}

#[test]
fn in_flight_effect_state_is_checked_before_the_fake_host_answers() {
    let mut walker = Walker::new(FakeHost::new(), Locale::En, Size::new(80, 24)).unwrap();
    walker.apply_action(Action::OpenAdd).unwrap();
    walker
        .apply_action(Action::Add(AddAction::HighlightDraft(0)))
        .unwrap();
    walker
        .apply_action(Action::Add(AddAction::DeleteSelectedDraft))
        .unwrap();
    walker
        .apply_action(Action::Add(AddAction::ConfirmDraftDelete(true)))
        .unwrap();

    let checkpoints = walker.checkpoints();
    let pending = checkpoints
        .iter()
        .rfind(|checkpoint| checkpoint.pending_effect)
        .expect("the pending host boundary was not checked");
    assert_eq!(pending.boundary, Boundary::UserAction);
    assert_eq!(pending.add_stage, Some(AddStage::ConfirmDraftDelete));
    assert!(pending.add_delete_candidate);
    let settled = checkpoints.last().unwrap();
    assert_eq!(settled.boundary, Boundary::HostAction);
    assert_eq!(settled.add_stage, Some(AddStage::Source));
}

#[test]
fn resize_changes_the_backend_before_the_resize_event_is_rendered() {
    let mut walker = Walker::new(FakeHost::new(), Locale::En, Size::new(80, 24)).unwrap();
    walker
        .step(&WalkerOperation::Resize {
            width: 24,
            height: 6,
        })
        .unwrap();
    assert_eq!(walker.terminal_size(), Size::new(24, 6));
    assert_eq!(walker.checkpoints().last().unwrap().size, Size::new(24, 6));
}

#[test]
fn run_footer_previous_focus_mouse_matches_the_keyboard_endpoint() {
    let mut host = FakeHost::new();
    let mut state = host.initial_state();
    let effect = state.update(Action::OpenRun);
    let action = host.serve(effect).unwrap();
    let effect = state.update(action);
    assert_eq!(effect, Effect::None);
    let (mut mouse_session, geometry) =
        render_probe_session(&state, Locale::En, Size::new(80, 24)).unwrap();
    let key_session = mouse_session.try_fork().unwrap();
    let hit = geometry
        .hits
        .iter()
        .find(|hit| hit.action == HitTarget::Command(UiCommand::FocusPrevious))
        .unwrap();
    let mouse = session_action(
        &mut mouse_session,
        Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: hit.rect.x.saturating_add(hit.rect.width / 2),
            row: hit.rect.y,
            modifiers: KeyModifiers::NONE,
        }),
        &state,
        &geometry,
    )
    .unwrap();
    let bindings = key_session.advertised_command_bindings(&state, UiCommand::FocusPrevious);
    let _ = command_key_action(
        &key_session,
        &state,
        &geometry,
        UiCommand::FocusPrevious,
        &bindings,
        &mouse,
        0,
    )
    .unwrap();
}

#[test]
fn typed_run_controls_compare_complete_mouse_and_keyboard_endpoints() {
    let mut enabled = ParamDecl::new("ENABLED");
    enabled.parameter_type = ParameterType::Bool;
    enabled.default = Some(ParameterValue::Bool(false));
    let mut format = ParamDecl::new("FORMAT");
    format.parameter_type = ParameterType::Choice;
    format.choices = vec!["json".to_owned(), "yaml".to_owned(), "toml".to_owned()];
    format.default = Some(ParameterValue::String("yaml".to_owned()));
    let form = RunFormView::from_declarations(
        "typed-controls",
        "Typed controls",
        &[enabled, format],
        &BTreeMap::new(),
        &["codex".to_owned(), "claude".to_owned()],
        "codex",
        &BTreeMap::new(),
        "",
    );
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Run(Box::new(form))));
    let size = Size::new(100, 28);
    let (session, geometry) = render_probe_session(&state, Locale::En, size).unwrap();

    assert!(
        geometry
            .hits
            .iter()
            .any(|hit| matches!(hit.action, HitTarget::ToggleField(_)))
    );
    assert!(geometry.hits.iter().any(|hit| {
        matches!(
            hit.action,
            HitTarget::SelectFieldOption {
                field: _,
                option: 1
            }
        )
    }));
    check_public_hit_parity(&state, &geometry, size, &session, Locale::En).unwrap();

    let runner = state
        .run_form()
        .unwrap()
        .fields()
        .iter()
        .position(|field| matches!(field.role, skit_ui::RunFieldRole::Runner))
        .unwrap();
    let runner_hit = geometry
        .hits
        .iter()
        .find(|hit| hit.action == HitTarget::FocusField(runner))
        .expect("the non-current runner picker must be clickable");
    let mut noncurrent_mouse_session = session.try_fork().unwrap();
    let mut noncurrent_mouse_state = state.clone();
    let handling = noncurrent_mouse_session.handle_event(
        Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: runner_hit.rect.x.saturating_add(runner_hit.rect.width / 2),
            row: runner_hit.rect.y.saturating_add(runner_hit.rect.height / 2),
            modifiers: KeyModifiers::NONE,
        }),
        &noncurrent_mouse_state,
        &geometry,
    );
    assert_eq!(handling, EventHandling::Action(Action::FocusField(runner)));
    apply_probe_handling(
        &mut noncurrent_mouse_state,
        handling,
        "non-current picker click",
    )
    .unwrap();
    let noncurrent_mouse = render_probe_endpoint(
        &mut noncurrent_mouse_session,
        &noncurrent_mouse_state,
        Locale::En,
        size,
    )
    .unwrap();
    let mut focus_only_session = session.try_fork().unwrap();
    let mut focus_only_state = state.clone();
    assert_eq!(
        focus_only_state.update(Action::FocusField(runner)),
        Effect::None
    );
    let focus_only =
        render_probe_endpoint(&mut focus_only_session, &focus_only_state, Locale::En, size)
            .unwrap();
    assert_ne!(
        noncurrent_mouse, focus_only,
        "the first picker click must focus and open the dropdown"
    );

    state.update(Action::FocusField(runner));
    let (session, geometry) = render_probe_session(&state, Locale::En, size).unwrap();
    let picker = geometry
        .hits
        .iter()
        .find(|hit| hit.action == HitTarget::FocusField(runner))
        .expect("the focused runner picker must keep its typed activation hit");
    let mut closed_session = session.try_fork().unwrap();
    let closed = render_probe_endpoint(&mut closed_session, &state, Locale::En, size).unwrap();
    let mut mouse_session = session.try_fork().unwrap();
    assert_eq!(
        mouse_session.handle_event(
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: picker.rect.x.saturating_add(picker.rect.width / 2),
                row: picker.rect.y.saturating_add(picker.rect.height / 2),
                modifiers: KeyModifiers::NONE,
            }),
            &state,
            &geometry,
        ),
        EventHandling::Consumed
    );
    let mouse = render_probe_endpoint(&mut mouse_session, &state, Locale::En, size).unwrap();
    let mut key_session = session.try_fork().unwrap();
    assert_eq!(
        key_session.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            &state,
            &geometry,
        ),
        EventHandling::Consumed
    );
    let key = render_probe_endpoint(&mut key_session, &state, Locale::En, size).unwrap();
    assert_ne!(mouse, closed, "the picker click must open its dropdown");
    assert_eq!(
        mouse, key,
        "the advertised Enter key must open the same dropdown"
    );
    let open_hit = mouse
        .geometry
        .hits
        .iter()
        .find(|hit| hit.action == HitTarget::FocusField(runner))
        .expect("the open runner picker must keep its control hit");
    let mut close_mouse_session = mouse_session.try_fork().unwrap();
    let mut close_key_session = mouse_session.try_fork().unwrap();
    assert_eq!(
        close_mouse_session.handle_event(
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: open_hit.rect.x.saturating_add(open_hit.rect.width / 2),
                row: open_hit.rect.y.saturating_add(open_hit.rect.height / 2),
                modifiers: KeyModifiers::NONE,
            }),
            &state,
            &mouse.geometry,
        ),
        EventHandling::Consumed
    );
    assert_eq!(
        close_key_session.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            &state,
            &mouse.geometry,
        ),
        EventHandling::Consumed
    );
    let close_mouse =
        render_probe_endpoint(&mut close_mouse_session, &state, Locale::En, size).unwrap();
    let close_key =
        render_probe_endpoint(&mut close_key_session, &state, Locale::En, size).unwrap();
    assert_ne!(close_mouse, mouse, "the second picker click must close");
    assert_eq!(
        close_mouse, close_key,
        "Escape must close the same dropdown as the picker click"
    );
    check_public_hit_parity(&state, &geometry, size, &session, Locale::En).unwrap();
}

#[test]
fn direct_text_focus_accepts_a_keyboard_path_with_different_scroll_history() {
    let mut walker = Walker::new(FakeHost::new(), Locale::En, Size::new(24, 6)).unwrap();
    assert_eq!(
        resolve_advertised_command(walker.state(), 126, 0).map(|(command, _)| command),
        Some(UiCommand::Run)
    );
    walker
        .step(&WalkerOperation::AdvertisedKey {
            command: 126,
            binding: 0,
        })
        .unwrap();
    assert!(matches!(walker.state().screen(), Screen::Run(_)));
    assert_eq!(walker.state().focused_form_field(), Some(1));

    assert_eq!(
        resolve_advertised_command(walker.state(), 63, 0).map(|(command, _)| command),
        Some(UiCommand::Submit)
    );
    walker
        .step(&WalkerOperation::AdvertisedKey {
            command: 63,
            binding: 0,
        })
        .unwrap();
    assert!(matches!(walker.state().screen(), Screen::Library));

    assert_eq!(
        resolve_public_hit(&walker.geometry, 0).map(|hit| hit.action),
        Some(HitTarget::Command(UiCommand::Run))
    );
    walker
        .step(&WalkerOperation::PublicHit { ordinal: 0 })
        .unwrap();
    assert!(matches!(walker.state().screen(), Screen::Run(_)));
    assert_eq!(walker.state().focused_form_field(), Some(1));

    walker
        .step(&WalkerOperation::Resize {
            width: 46,
            height: 12,
        })
        .unwrap();
    assert_eq!(walker.terminal_size(), Size::new(46, 12));
    assert!(walker.geometry.hits.iter().any(|hit| {
        hit.rect.width > 0 && hit.rect.height > 0 && hit.action == HitTarget::FocusField(2)
    }));

    assert_eq!(
        resolve_advertised_command(walker.state(), 215, 0).map(|(command, _)| command),
        Some(UiCommand::FocusNext)
    );
    walker
        .step(&WalkerOperation::AdvertisedKey {
            command: 215,
            binding: 0,
        })
        .unwrap();

    assert_eq!(walker.state().focused_form_field(), Some(2));
    assert_eq!(walker.geometry.first_visible, 11);
    assert!(walker.geometry.hits.iter().any(|hit| {
        hit.rect.width > 0 && hit.rect.height > 0 && hit.action == HitTarget::FocusField(2)
    }));
}

#[test]
fn picker_focus_parity_uses_one_post_navigation_session() {
    let mut walker = Walker::new(FakeHost::new(), Locale::Pseudo, Size::new(120, 30)).unwrap();
    assert_eq!(
        resolve_advertised_command(walker.state(), 238, 0).map(|(command, _)| command),
        Some(UiCommand::Search)
    );
    walker
        .step(&WalkerOperation::AdvertisedKey {
            command: 238,
            binding: 0,
        })
        .unwrap();
    assert!(matches!(walker.state().screen(), Screen::Library));
    assert!(
        walker
            .geometry
            .hits
            .iter()
            .any(|hit| hit.action == HitTarget::Command(UiCommand::Run))
    );

    assert_eq!(
        resolve_public_hit(&walker.geometry, 2).map(|hit| hit.action),
        Some(HitTarget::Command(UiCommand::Run))
    );
    walker
        .step(&WalkerOperation::PublicHit { ordinal: 2 })
        .unwrap();
    assert!(matches!(walker.state().screen(), Screen::Run(_)));
    assert_eq!(walker.state().focused_form_field(), Some(1));

    assert_eq!(
        resolve_public_hit(&walker.geometry, 125).map(|hit| hit.action),
        Some(HitTarget::Command(UiCommand::FocusPrevious))
    );
    walker
        .step(&WalkerOperation::PublicHit { ordinal: 125 })
        .unwrap();
    assert_eq!(walker.state().focused_form_field(), Some(0));

    walker
        .step(&WalkerOperation::MouseCell {
            x_fraction: 3,
            y_fraction: 9,
            kind: MouseKind::ScrollDown,
        })
        .unwrap();

    assert_eq!(walker.state().focused_form_field(), Some(0));
    assert_eq!(walker.geometry.first_visible, 1);
    assert!(walker.geometry.hits.iter().any(|hit| {
        hit.rect.width > 0 && hit.rect.height > 0 && hit.action == HitTarget::FocusField(0)
    }));
}

#[test]
fn runner_editor_modal_does_not_publish_blocked_base_screen_hits() {
    let mut walker = Walker::new(FakeHost::new(), Locale::En, Size::new(80, 24)).unwrap();
    for operation in [
        WalkerOperation::MouseCell {
            x_fraction: 0,
            y_fraction: 0,
            kind: MouseKind::ScrollDown,
        },
        WalkerOperation::PublicHit { ordinal: 0 },
        WalkerOperation::AdvertisedKey {
            command: 151,
            binding: 0,
        },
    ] {
        walker.step(&operation).unwrap();
    }

    assert!(matches!(
        walker.state().modal(),
        Some(ModalState::RunnerEditor { .. })
    ));
    assert!(
        walker.geometry.hits.is_empty(),
        "the modal must not expose blocked hits from its Run screen"
    );
    assert!(
        !walker.session.local_action_inventory().actions.is_empty(),
        "the modal must keep its own typed local actions"
    );
}

#[test]
fn run_dropdown_does_not_publish_occluded_field_command_hits() {
    let mut walker = Walker::new(FakeHost::new(), Locale::En, Size::new(80, 24)).unwrap();
    let operations = [
        WalkerOperation::AdvertisedKey {
            command: 147,
            binding: 0,
        },
        WalkerOperation::AdvertisedKey {
            command: 0,
            binding: 0,
        },
        WalkerOperation::PublicHit { ordinal: 135 },
        WalkerOperation::PublicHit { ordinal: 45 },
    ];
    for operation in operations {
        walker.step(&operation).unwrap();
    }
    assert_eq!(walker.liveness_checks, walker.checkpoints().len());
}

#[test]
fn preferences_agent_picker_does_not_publish_blocked_footer_hits() {
    let mut walker = Walker::new(FakeHost::new(), Locale::En, Size::new(80, 24)).unwrap();
    for operation in [
        WalkerOperation::AdvertisedKey {
            command: 30,
            binding: 0,
        },
        WalkerOperation::PublicHit { ordinal: 99 },
    ] {
        walker.step(&operation).unwrap();
    }

    let Screen::Preferences(view) = walker.state().screen() else {
        panic!("the shrunk trace did not open Preferences");
    };
    assert!(view.agent_skill_install().is_some());
    assert!(
        walker.geometry.hits.iter().all(|hit| {
            hit.action != HitTarget::Command(UiCommand::SavePreferences)
                && hit.action != HitTarget::Command(UiCommand::ClosePreferences)
        }),
        "the picker must not expose blocked Preferences footer actions"
    );
}

#[test]
fn first_preferences_frame_advertises_only_keys_the_focused_widget_releases() {
    let mut host = FakeHost::new();
    let mut state = host.initial_state();
    let action = host.serve(state.update(Action::OpenPreferences)).unwrap();
    assert_eq!(state.update(action), Effect::None);

    let render = |state: &LibraryState| {
        let mut session = TuiSession::default();
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        let mut geometry = ViewGeometry::default();
        terminal
            .draw(|frame| {
                geometry = render_with_session(frame, state, Locale::En, &mut session);
            })
            .unwrap();
        (session, terminal, geometry)
    };

    let (session, terminal, _) = render(&state);
    assert_eq!(
        session
            .advertised_command_bindings(&state, UiCommand::FocusNext)
            .iter()
            .map(|binding| binding.key)
            .collect::<Vec<_>>(),
        [UiKey::Tab]
    );
    let text = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(!text.contains("Tab/↓"));

    assert_eq!(
        state.update(Action::Preferences(PreferencesAction::Focus(
            PreferencesControlId::Editor,
        ))),
        Effect::None
    );
    let (session, terminal, geometry) = render(&state);
    for command in [UiCommand::ManageAgents, UiCommand::InstallAgentSkill] {
        assert!(
            session
                .advertised_command_bindings(&state, command)
                .is_empty()
        );
        assert!(
            geometry
                .hits
                .iter()
                .all(|hit| hit.action != HitTarget::Command(command))
        );
    }
    let text = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(!text.contains("Manage agents"));
    assert!(!text.contains("Teach an AI agent skit"));

    let blocked = command_bindings(&state, UiCommand::InstallAgentSkill).unwrap();
    let expected = Action::Preferences(PreferencesAction::InstallAgentSkill);
    assert!(
        command_key_action(
            &session,
            &state,
            &geometry,
            UiCommand::InstallAgentSkill,
            &blocked,
            &expected,
            0,
        )
        .is_err(),
        "a shared footer key must not use a Tab prefix to hide immediate widget ownership"
    );
    assert!(
        command_key_action(
            &session,
            &state,
            &geometry,
            UiCommand::InstallAgentSkill,
            &blocked,
            &expected,
            64,
        )
        .is_ok(),
        "the same chord must remain reachable after a deliberate focus move"
    );
}

#[test]
fn persistent_preferences_controls_do_not_occlude_the_shared_footer() {
    let mut walker = Walker::new(FakeHost::new(), Locale::En, Size::new(80, 24)).unwrap();
    for operation in [
        WalkerOperation::AdvertisedKey {
            command: 156,
            binding: 0,
        },
        WalkerOperation::AdvertisedKey {
            command: 130,
            binding: 29,
        },
    ] {
        walker.step(&operation).unwrap();
    }

    assert!(matches!(walker.state().screen(), Screen::Preferences(_)));
    assert_eq!(walker.liveness_checks, walker.checkpoints().len());
}

#[test]
fn liveness_uses_real_session_events_to_leave_a_dirty_workflow() {
    let mut walker = Walker::new(FakeHost::new(), Locale::En, Size::new(80, 24)).unwrap();
    walker.apply_action(Action::OpenPreferences).unwrap();
    walker
        .apply_action(Action::Preferences(skit_ui::PreferencesAction::SetEditor(
            "micro".to_owned(),
        )))
        .unwrap();
    walker.assert_liveness().unwrap();
    assert!(matches!(walker.state().screen(), Screen::Library));
    assert!(walker.state().modal().is_none());
}

#[test]
fn host_effect_cycles_fail_after_every_intermediate_boundary_is_checked() {
    struct LoopHost;

    impl WalkerHost for LoopHost {
        fn initial_state(&self) -> LibraryState {
            LibraryState::default()
        }

        fn serve(&mut self, effect: Effect) -> Result<Action, String> {
            assert!(matches!(effect, Effect::Reload));
            Ok(Action::Reload)
        }
    }

    let mut walker = Walker::new(LoopHost, Locale::En, Size::new(24, 6)).unwrap();
    let error = walker.drain_host_effects(Effect::Reload).unwrap_err();
    assert!(error.contains("exceeded 64 actions"));
    assert_eq!(
        walker
            .checkpoints()
            .iter()
            .filter(|checkpoint| checkpoint.boundary == Boundary::HostAction)
            .count(),
        HOST_EFFECT_LIMIT
    );
    assert!(
        walker
            .checkpoints()
            .iter()
            .filter(|checkpoint| checkpoint.boundary == Boundary::HostAction)
            .all(|checkpoint| checkpoint.pending_effect)
    );
}

#[test]
fn exactly_the_host_effect_limit_may_settle_on_the_final_action() {
    struct FiniteHost(usize);

    impl WalkerHost for FiniteHost {
        fn initial_state(&self) -> LibraryState {
            LibraryState::default()
        }

        fn serve(&mut self, effect: Effect) -> Result<Action, String> {
            assert!(matches!(effect, Effect::Reload));
            self.0 = self.0.saturating_add(1);
            Ok(if self.0 == HOST_EFFECT_LIMIT {
                Action::ClearStatus
            } else {
                Action::Reload
            })
        }
    }

    let mut walker = Walker::new(FiniteHost(0), Locale::En, Size::new(24, 6)).unwrap();
    walker.drain_host_effects(Effect::Reload).unwrap();
    assert_eq!(walker.host.0, HOST_EFFECT_LIMIT);
}

#[test]
fn missing_effect_selectors_fail_before_the_fake_host_can_turn_them_into_status() {
    let mut walker = Walker::new(FakeHost::new(), Locale::En, Size::new(80, 24)).unwrap();
    let error = walker
        .drain_host_effects(Effect::Rerun {
            selector: "missing-entry".to_owned(),
        })
        .unwrap_err();
    assert!(error.contains("effect selector is absent from the fake model: missing-entry"));
}

#[test]
fn quit_is_a_terminal_effect_and_never_reaches_the_host() {
    struct NoQuitHost;

    impl WalkerHost for NoQuitHost {
        fn initial_state(&self) -> LibraryState {
            LibraryState::default()
        }

        fn serve(&mut self, effect: Effect) -> Result<Action, String> {
            panic!("the terminal effect reached the host: {effect:?}");
        }
    }

    let mut walker = Walker::new(NoQuitHost, Locale::En, Size::new(24, 6)).unwrap();
    walker.apply_action(Action::Quit).unwrap();
    assert!(walker.quit);
    assert_eq!(
        walker.checkpoints().last().unwrap().boundary,
        Boundary::UserAction
    );
}

#[test]
fn cast_header_uses_the_maximum_replay_canvas() {
    let walker = Walker::with_canvas(
        FakeHost::new(),
        Locale::En,
        Size::new(24, 6),
        Size::new(300, 100),
    )
    .unwrap();
    let header: serde_json::Value = serde_json::from_slice(
        walker
            .cast_bytes()
            .split(|byte| *byte == b'\n')
            .next()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(header["term"]["cols"], 300);
    assert_eq!(header["term"]["rows"], 100);
}

#[test]
fn failure_artifacts_keep_each_complete_bundle_separate() {
    let directory = tempfile::tempdir().unwrap();
    let first = write_failure_artifacts(
        directory.path(),
        &[WalkerOperation::Focus { gained: true }],
        Locale::En,
        Size::new(24, 6),
        "first failure",
        b"first cast",
    )
    .unwrap();
    let shrunk = write_failure_artifacts(
        directory.path(),
        &[WalkerOperation::Focus { gained: false }],
        Locale::ZhTw,
        Size::new(1, 1),
        "shrunk failure",
        b"shrunk cast",
    )
    .unwrap();

    assert_ne!(first, shrunk);
    assert_eq!(fs::read(first.join("failure.cast")).unwrap(), b"first cast");
    assert_eq!(
        fs::read(shrunk.join("failure.cast")).unwrap(),
        b"shrunk cast"
    );
    let repro: serde_json::Value =
        serde_json::from_slice(&fs::read(shrunk.join("repro.json")).unwrap()).unwrap();
    assert_eq!(repro["locale"], "zh-TW");
    assert_eq!(repro["error"], "shrunk failure");
    assert_eq!(repro["operations"][0]["gained"], false);
}

#[test]
fn requested_success_artifact_contains_the_replay_and_cast_in_one_bundle() {
    let directory = tempfile::tempdir().unwrap();
    let bundle = write_success_artifacts(
        directory.path(),
        &[WalkerOperation::Resize {
            width: 46,
            height: 12,
        }],
        Locale::Pseudo,
        Size::new(24, 6),
        b"successful cast",
    )
    .unwrap();

    assert!(
        bundle
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("success-")
    );
    assert_eq!(
        fs::read(bundle.join("success.cast")).unwrap(),
        b"successful cast"
    );
    let repro: serde_json::Value =
        serde_json::from_slice(&fs::read(bundle.join("repro.json")).unwrap()).unwrap();
    assert_eq!(repro["locale"], "x-pseudo");
    assert_eq!(repro["result"], "passed");
    assert_eq!(repro["cast"], "success.cast");
    assert_eq!(repro["operations"][0]["width"], 46);
}

#[test]
fn initial_invariant_failure_keeps_a_replay_header_and_subject() {
    struct InvalidInitialHost(LibraryState);

    impl WalkerHost for InvalidInitialHost {
        fn initial_state(&self) -> LibraryState {
            self.0.clone()
        }

        fn serve(&mut self, effect: Effect) -> Result<Action, String> {
            panic!("an invalid initial state reached the host: {effect:?}");
        }
    }

    let state = FakeHost::new().initial_state();
    let mut value = serde_json::to_value(state).unwrap();
    value["visible"] = serde_json::json!([1, 0]);
    value["selected"] = serde_json::json!(0);
    let invalid = serde_json::from_value(value).unwrap();
    let failure = evaluate_case_with_host(
        InvalidInitialHost(invalid),
        &[],
        Locale::En,
        Size::new(1, 1),
    )
    .unwrap_err();

    assert!(failure.error.contains("LIBRARY_VISIBLE_ORDER"));
    let header: serde_json::Value =
        serde_json::from_slice(failure.cast.split(|byte| *byte == b'\n').next().unwrap()).unwrap();
    assert_eq!(header["version"], 3);
    assert_eq!(header["term"]["cols"], 1);
    assert_eq!(header["term"]["rows"], 1);
    let output: serde_json::Value = serde_json::from_slice(
        failure
            .cast
            .split(|byte| *byte == b'\n')
            .nth(1)
            .expect("the invalid state must still produce one diagnostic frame"),
    )
    .unwrap();
    assert_eq!(output[1], "o");
    assert!(!output[2].as_str().unwrap().is_empty());
}

#[test]
fn initial_host_panic_keeps_a_replay_header() {
    struct PanicInitialHost;

    impl WalkerHost for PanicInitialHost {
        fn initial_state(&self) -> LibraryState {
            panic!("model initialization failed");
        }

        fn serve(&mut self, effect: Effect) -> Result<Action, String> {
            panic!("a failed initial host reached an effect: {effect:?}");
        }
    }

    let failure =
        evaluate_case_with_host(PanicInitialHost, &[], Locale::En, Size::new(1, 1)).unwrap_err();
    assert!(
        failure
            .error
            .contains("walker initialization panic: model initialization failed")
    );
    let header: serde_json::Value =
        serde_json::from_slice(failure.cast.split(|byte| *byte == b'\n').next().unwrap()).unwrap();
    assert_eq!(header["version"], 3);
}
