use ratatui_core::{backend::TestBackend, layout::Size, terminal::Terminal};
use ratatui_crossterm::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use skit_i18n::Locale;
use skit_tui::{
    EventHandling, HitRegion, HitTarget, LocalActionOutcome, LocalAdvertisedAction,
    RunFieldCommand, TuiSession, ViewGeometry, render_with_session,
};
use skit_ui::{
    Action, Effect, FormControl, LibraryState, ModalState, RunTokenOption, UiBinding, UiCommand,
    UiKey, command_specs,
};

/// One complete rendered endpoint.
#[derive(Clone, Debug, PartialEq)]
pub struct ProbeEndpoint {
    /// Reducer state that the frame presents.
    pub state: LibraryState,
    /// Geometry that the frame publishes.
    pub geometry: ViewGeometry,
    /// Backend that holds the drawn cells and the cursor position.
    pub backend: TestBackend,
}

/// The rendered frame that one probe reads.
#[derive(Clone, Copy, Debug)]
pub struct FieldParityContext<'a> {
    /// Persistent session of the frame.
    pub session: &'a TuiSession,
    /// Reducer state of the frame.
    pub state: &'a LibraryState,
    /// Geometry that the frame published.
    pub geometry: &'a ViewGeometry,
    /// Presentation locale of the frame.
    pub locale: Locale,
    /// Viewport of the frame.
    pub size: Size,
}

/// Check the pointer and keyboard parity of every hit on one frame.
///
/// Each visible hit must stay inside the viewport and must name an action that this state can
/// produce. A forked session then receives one primary click on the hit, and the endpoint of that
/// click must equal the endpoint of the advertised keyboard path.
pub fn check_public_hit_parity(
    state: &LibraryState,
    geometry: &ViewGeometry,
    size: Size,
    session: &TuiSession,
    locale: Locale,
) -> Result<(), String> {
    for hit in &geometry.hits {
        if is_clipped(hit) {
            continue;
        }
        check_hit_bounds(hit, size)?;
        let _ = expected_hit_action(hit.action, geometry, state)?;
    }

    let context = FieldParityContext {
        session,
        state,
        geometry,
        locale,
        size,
    };
    for hit in &geometry.hits {
        if is_clipped(hit) {
            continue;
        }
        check_hit_parity(context, hit)?;
    }
    Ok(())
}

/// Report a hit that the layout clipped away.
///
/// A responsive layout keeps a clipped footer item as a zero-area record. The item is not visible
/// and it takes no mouse input.
#[must_use]
pub fn is_clipped(hit: &HitRegion) -> bool {
    hit.rect.width == 0 || hit.rect.height == 0
}

/// Report whether one frame publishes a visible hit for `target`.
#[must_use]
pub fn publishes_target(geometry: &ViewGeometry, target: HitTarget) -> bool {
    geometry
        .hits
        .iter()
        .any(|hit| !is_clipped(hit) && hit.action == target)
}

/// Refuse a visible hit that reaches outside the viewport.
pub fn check_hit_bounds(hit: &HitRegion, size: Size) -> Result<(), String> {
    if hit.rect.right() > size.width || hit.rect.bottom() > size.height {
        return Err(format!(
            "PUBLIC_HIT_BOUNDS hit={hit:?} viewport={}x{}",
            size.width, size.height
        ));
    }
    Ok(())
}

/// Compare the pointer and keyboard endpoints of one visible hit.
fn check_hit_parity(context: FieldParityContext<'_>, hit: &HitRegion) -> Result<(), String> {
    let expected = expected_hit_action(hit.action, context.geometry, context.state)?;
    // Fork the exact persistent widget state from this frame. Mouse and keyboard probes must see
    // the same cursor, scroll, dropdown, overlay, and private click registries as the real walk.
    let forked = context.session.try_fork();
    // `try_fork` returns `None` only while a path-completion worker owns a channel. A walker
    // session never starts that worker.
    let mut mouse_session =
        forked.ok_or("the persistent TUI session cannot fork for mouse parity")?;
    let (column, row) = public_hit_probe_point(hit);
    let mouse_handling = session_primary_click(
        &mut mouse_session,
        context.state,
        context.geometry,
        column,
        row,
    )?;
    if matches!(
        hit.action,
        HitTarget::FocusField(_) | HitTarget::ToggleField(_) | HitTarget::SelectFieldOption { .. }
    ) {
        return check_field_hit_parity(context, hit.action, mouse_session, mouse_handling);
    }
    check_command_hit_parity(context, hit, &expected, mouse_session, mouse_handling)
}

/// Compare the pointer and keyboard endpoints of one command hit.
///
/// The click must produce the expected typed action. Focus commands are exempt because a pointer
/// path can name the focus target that the keyboard path reaches by one step.
pub fn check_command_hit_parity(
    context: FieldParityContext<'_>,
    hit: &HitRegion,
    expected: &Action,
    mouse_session: TuiSession,
    mouse_handling: EventHandling,
) -> Result<(), String> {
    let click_action = match mouse_handling {
        EventHandling::Action(action) => action,
        handling => {
            return Err(format!(
                "PUBLIC_SESSION_HIT hit={hit:?} handling={handling:?} expected={expected:?}"
            ));
        }
    };
    if click_action != *expected
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
            check_command_hit(context, command, &click_action, mouse_session)
        }
        HitTarget::RunFieldCommand { field, command } => {
            check_run_field_hit(context, field, command, &click_action)
        }
        HitTarget::FocusField(field) => Err(format!(
            "FOCUS_PARITY_INTERNAL field={field}; the typed field branch did not run"
        )),
        HitTarget::ToggleField(_) | HitTarget::SelectFieldOption { .. } => {
            Err("FIELD_PARITY_INTERNAL; the typed field branch did not run".to_owned())
        }
    }
}

/// Compare the keyboard path of one shared command with its pointer path.
fn check_command_hit(
    context: FieldParityContext<'_>,
    command: UiCommand,
    click_action: &Action,
    mouse_session: TuiSession,
) -> Result<(), String> {
    let bindings = public_command_bindings(context.session, context.state, command)?;
    if matches!(command, UiCommand::FocusNext | UiCommand::FocusPrevious) {
        return check_command_key_endpoint(context, &bindings, click_action, mouse_session);
    }
    let _ = command_key_action(
        context.session,
        context.state,
        context.geometry,
        command,
        &bindings,
        click_action,
        0,
    )?;
    Ok(())
}

/// Compare the keyboard path of one launch-field chip with its pointer path.
///
/// The chip focuses its field in one click. The keyboard twin focuses the same field first, so
/// the probe compares the command from that shared focused state.
pub fn check_run_field_hit(
    context: FieldParityContext<'_>,
    field: usize,
    command: RunFieldCommand,
    click_action: &Action,
) -> Result<(), String> {
    let ui_command = run_field_ui_command(command);
    let mut key_state = context.state.clone();
    let focus_effect = key_state.update(Action::FocusField(field));
    check_probe_effect("RUN_FIELD_FOCUS_EFFECT", field, &focus_effect)?;
    // A field chip can focus a field in one click. Its keyboard path can use a bounded
    // Tab/BackTab prefix before it invokes the advertised command. Compare the command from that
    // shared focused state instead of treating focus itself as a semantic difference.
    let mut mouse_state = key_state.clone();
    let mouse_effect = mouse_state.update(click_action.clone());
    if command == RunFieldCommand::BrowsePath {
        let key_action =
            browse_keyboard_action(context.session, &key_state, context.geometry, field)?;
        if key_action != *click_action {
            return Err(format!(
                "RUN_BROWSE_PARITY field={field} mouse={click_action:?} key={key_action:?}"
            ));
        }
        let key_effect = key_state.update(key_action);
        return check_browse_endpoint(
            field,
            (&mouse_state, &mouse_effect),
            (&key_state, &key_effect),
        );
    }
    let bindings = command_bindings(&key_state, ui_command)?;
    let _ = command_key_action(
        context.session,
        &key_state,
        context.geometry,
        ui_command,
        &bindings,
        click_action,
        64,
    )?;
    Ok(())
}

/// Refuse a browse path whose keyboard endpoint differs from its pointer endpoint.
pub fn check_browse_endpoint(
    field: usize,
    mouse: (&LibraryState, &Effect),
    key: (&LibraryState, &Effect),
) -> Result<(), String> {
    if mouse.0 != key.0 || mouse.1 != key.1 {
        return Err(format!(
            "RUN_BROWSE_ENDPOINT field={field} mouse_effect={:?} key_effect={:?}",
            mouse.1, key.1
        ));
    }
    Ok(())
}

/// Refuse a probe step that asks the host for work.
pub fn check_probe_effect(code: &str, field: usize, effect: &Effect) -> Result<(), String> {
    if host_effect_pending(effect) {
        return Err(format!("{code} field={field} effect={effect:?}"));
    }
    Ok(())
}

/// Report whether the host owes the frontend an answer for this effect.
#[must_use]
pub const fn host_effect_pending(effect: &Effect) -> bool {
    !matches!(effect, Effect::None | Effect::Quit)
}

/// Return every printed key binding of one visible command.
///
/// The footer bindings come first. A command that the footer does not print still needs one
/// advertised chord from the shared command registry.
pub fn public_command_bindings(
    session: &TuiSession,
    state: &LibraryState,
    command: UiCommand,
) -> Result<Vec<UiBinding>, String> {
    let footer = session.advertised_command_bindings(state, command);
    if !footer.is_empty() {
        return Ok(footer);
    }
    command_specs(state.command_context())
        .find(|spec| spec.command == command && state.command_enabled(command))
        .map(|spec| spec.bindings.to_vec())
        .ok_or_else(|| format!("visible command {command:?} has no printed key binding"))
}

/// Return the cell that the pointer probe presses for one hit.
#[must_use]
pub fn public_hit_probe_point(hit: &HitRegion) -> (u16, u16) {
    if matches!(hit.action, HitTarget::FocusField(_)) {
        // A field border is a real focus target but does not prescribe an arbitrary caret column.
        // This gives the keyboard twin one exact, reproducible endpoint.
        (hit.rect.x, hit.rect.y)
    } else {
        (
            hit.rect.x.saturating_add(hit.rect.width / 2),
            hit.rect.y.saturating_add(hit.rect.height / 2),
        )
    }
}

/// Compare the pointer and keyboard endpoints of one form-field hit.
///
/// The pointer path applies `mouse_handling` on a forked session. The keyboard path focuses the
/// same field and then uses one bounded activation sequence. Both paths must render one identical
/// endpoint.
pub fn check_field_hit_parity(
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
    let mouse_is_plain_focus = is_plain_focus_handling(&mouse_handling, field);
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
    let forked = base_session.try_fork();
    // `try_fork` returns `None` only while a path-completion worker owns a channel. A walker
    // session never starts that worker.
    let mut focus_session = forked.ok_or("the persistent TUI session cannot fork for focus")?;
    let mut focus_state = state.clone();
    let focus_effect = focus_state.update(Action::FocusField(field));
    check_probe_effect("FIELD_FOCUS_EFFECT", field, &focus_effect)?;
    let focus_endpoint = render_probe_endpoint(&mut focus_session, &focus_state, locale, size)?;
    if target == HitTarget::FocusField(field) && is_plain_focus_field(state, field) {
        if mouse_endpoint.state != focus_endpoint.state {
            return Err(format!(
                "FIELD_TEXT_FOCUS_STATE field={field} mouse_focus={:?} direct_focus={:?}",
                mouse_endpoint.state.focused_form_field(),
                focus_endpoint.state.focused_form_field(),
            ));
        }
        if !publishes_target(&mouse_endpoint.geometry, target) {
            return Err(format!("FIELD_TEXT_FOCUS_HIDDEN field={field}"));
        }
        if mouse_is_plain_focus {
            check_keyboard_focus_path(context, target, field, field_count, &mouse_endpoint.state)?;
        }
        // A direct text click also chooses a caret column and can use a different valid scroll
        // history. Those pointer-only coordinates do not have one canonical focus-key endpoint.
        return Ok(());
    }
    if state.focused_form_field() != Some(field) {
        check_keyboard_focus_path(context, target, field, field_count, &focus_endpoint.state)?;
    }

    let activations = field_activation_sequences(state, target);
    let mut failures = Vec::new();
    for activation in &activations {
        let forked = focus_session.try_fork();
        // `try_fork` returns `None` only while a path-completion worker owns a channel. A walker
        // session never starts that worker.
        let mut candidate_session = forked.ok_or("the focused TUI session cannot fork")?;
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

/// Report whether one pointer result is exactly one focus move to `field`.
///
/// A direct text click also chooses a caret column, so only a plain focus move has one canonical
/// keyboard twin.
#[must_use]
pub fn is_plain_focus_handling(handling: &EventHandling, field: usize) -> bool {
    matches!(handling, EventHandling::Action(Action::FocusField(actual)) if *actual == field)
}

/// Report whether one field takes focus without any other typed effect.
#[must_use]
pub fn is_plain_focus_field(state: &LibraryState, field: usize) -> bool {
    if let Some(form) = state.run_form() {
        return form
            .fields()
            .get(field)
            .is_some_and(|field| matches!(&field.control, FormControl::Text(_)));
    }
    state.form().is_some_and(|form| field < form.fields.len())
}

/// Reach one field target through the advertised focus keys.
///
/// Tab and BackTab traverse the active focus ring. One advertised path is sufficient, so the
/// probe accepts the first step that reaches the pointer state with the target still visible.
pub fn check_keyboard_focus_path(
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
        let forked = context.session.try_fork();
        // `try_fork` returns `None` only while a path-completion worker owns a channel. A walker
        // session never starts that worker.
        let mut key_session = forked.ok_or("the persistent TUI session cannot fork for focus")?;
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
            let visible = publishes_target(&key_geometry, target);
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

/// Return every bounded key sequence that activates one field target.
#[must_use]
pub fn field_activation_sequences(state: &LibraryState, target: HitTarget) -> Vec<Vec<KeyEvent>> {
    let key = |code| KeyEvent::new(code, KeyModifiers::NONE);
    match target {
        HitTarget::ToggleField(_) => vec![vec![key(KeyCode::Char(' '))]],
        HitTarget::SelectFieldOption { field, .. } => {
            let options = state
                .run_form()
                .and_then(|form| form.fields().get(field))
                .and_then(|field| match &field.control {
                    FormControl::Choice(control) => Some(control.options.len()),
                    FormControl::Text(_) | FormControl::Checkbox { .. } => None,
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

/// Apply one probe result to a probe state.
///
/// A probe stays inside the frontend. An action that asks the host, and an ignored event, both
/// end the probe.
pub fn apply_probe_handling(
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

/// Render one probe endpoint: the state, the published geometry and the drawn frame.
pub fn render_probe_endpoint(
    session: &mut TuiSession,
    state: &LibraryState,
    locale: Locale,
    size: Size,
) -> Result<ProbeEndpoint, String> {
    let backend = TestBackend::new(size.width, size.height);
    let mut terminal = Terminal::new(backend).map_err(|error| error.to_string())?;
    let mut geometry = ViewGeometry::default();
    let drawn = terminal.draw(|frame| {
        geometry = render_with_session(frame, state, locale, session);
    });
    drawn.map_err(|error| error.to_string())?;
    Ok(ProbeEndpoint {
        state: state.clone(),
        geometry,
        backend: terminal.backend().clone(),
    })
}

/// Render one state on a fresh session and return that session with its geometry.
pub fn render_probe_session(
    state: &LibraryState,
    locale: Locale,
    size: Size,
) -> Result<(TuiSession, ViewGeometry), String> {
    let mut session = TuiSession::default();
    let backend = TestBackend::new(size.width, size.height);
    let mut terminal = Terminal::new(backend).map_err(|error| error.to_string())?;
    let mut geometry = ViewGeometry::default();
    let drawn = terminal.draw(|frame| {
        geometry = render_with_session(frame, state, locale, &mut session);
    });
    drawn.map_err(|error| error.to_string())?;
    Ok((session, geometry))
}

/// Return the action that one published hit must produce.
///
/// This is the public hit contract. A command hit yields its command action, a field hit yields
/// the field action, and a launch-token hit yields its option payload.
pub fn expected_hit_action(
    target: HitTarget,
    geometry: &ViewGeometry,
    state: &LibraryState,
) -> Result<Action, String> {
    match target {
        HitTarget::Command(UiCommand::ToggleDetail) => Ok(Action::ToggleDetail {
            currently_visible: geometry.detail_pane_visible,
        }),
        HitTarget::Command(command) => command
            .action_for_context(state.command_context())
            .ok_or_else(|| format!("PUBLIC_HIT_CONTEXT command={command:?}")),
        HitTarget::RunFieldCommand { field, command } => Ok(run_field_action(field, command)),
        HitTarget::FocusField(field) => Ok(Action::FocusField(field)),
        HitTarget::ToggleField(field) => Ok(Action::ToggleField(field)),
        HitTarget::SelectFieldOption { field, option } => state
            .run_form()
            .and_then(|form| form.fields().get(field))
            .and_then(|field| match &field.control {
                FormControl::Choice(control) => control.options.get(option),
                FormControl::Text(_) | FormControl::Checkbox { .. } => None,
            })
            .cloned()
            .map(|value| Action::SelectFieldOption { field, value })
            .ok_or_else(|| format!("RUN_OPTION_HIT field={field} option={option}")),
    }
}

/// Return the shared command that one launch-field chip invokes.
#[must_use]
pub const fn run_field_ui_command(command: RunFieldCommand) -> UiCommand {
    match command {
        RunFieldCommand::BrowsePath => UiCommand::BrowsePath,
        RunFieldCommand::InsertValue => UiCommand::InsertValue,
        RunFieldCommand::ResetDefault => UiCommand::ResetDefault,
    }
}

/// Return the action that one launch-field chip must produce.
#[must_use]
pub const fn run_field_action(field: usize, command: RunFieldCommand) -> Action {
    match command {
        RunFieldCommand::BrowsePath => Action::OpenRunFilePicker(field),
        RunFieldCommand::InsertValue => Action::OpenRunTokenMenuFor(field),
        RunFieldCommand::ResetDefault => Action::ResetRunField(field),
    }
}

/// Send one event and require a typed action.
pub fn session_action(
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

/// Send one complete primary click and return the result of the release.
///
/// The press must arm the target without activating it.
pub fn session_primary_click(
    session: &mut TuiSession,
    state: &LibraryState,
    geometry: &ViewGeometry,
    column: u16,
    row: u16,
) -> Result<EventHandling, String> {
    let down = Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column,
        row,
        modifiers: KeyModifiers::NONE,
    });
    let down_handling = session.handle_event(down, state, geometry);
    if down_handling != EventHandling::Consumed {
        return Err(format!(
            "SESSION_CLICK_PRESS state={:?} point=({column},{row}) handling={down_handling:?}",
            state.command_context(),
        ));
    }
    Ok(session.handle_event(
        Event::Mouse(MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }),
        state,
        geometry,
    ))
}

/// Reach the file picker of one launch field through the advertised keys.
///
/// The keyboard path opens the token menu of the field and selects its file entry.
pub fn browse_keyboard_action(
    base_session: &TuiSession,
    state: &LibraryState,
    geometry: &ViewGeometry,
    field: usize,
) -> Result<Action, String> {
    if !state.command_enabled(UiCommand::BrowsePath) {
        return Err(format!("RUN_BROWSE_CAPABILITY field={field}"));
    }
    let mut keyboard_state = state.clone();
    let forked = base_session.try_fork();
    // `try_fork` returns `None` only while a path-completion worker owns a channel. A walker
    // session never starts that worker.
    let mut session = forked.ok_or("the persistent TUI session cannot fork for browse parity")?;
    let open_tokens = command_key_action_with_session(
        &mut session,
        &keyboard_state,
        geometry,
        UiCommand::InsertValue,
    )?;
    let effect = keyboard_state.update(open_tokens);
    check_probe_effect("RUN_TOKEN_EFFECT", field, &effect)?;
    let options = match keyboard_state.modal() {
        Some(ModalState::RunTokenMenu {
            field: target,
            options,
        }) if *target == field => options.clone(),
        modal => {
            return Err(format!("RUN_TOKEN_OWNER field={field} modal={modal:?}"));
        }
    };
    if token_file_index(field, &options)? > 0 {
        select_last_token(&mut session, &keyboard_state, geometry, field)?;
    }
    session_action(
        &mut session,
        Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        &keyboard_state,
        geometry,
    )
}

/// Return the position of the file entry in one token menu.
///
/// The entry is first for a path field and last for every other field.
pub fn token_file_index(field: usize, options: &[RunTokenOption]) -> Result<usize, String> {
    let file_index = options
        .iter()
        .position(|option| matches!(option, RunTokenOption::FileOrFolder))
        .ok_or_else(|| format!("RUN_TOKEN_FILE_OPTION field={field} options={options:?}"))?;
    if file_index > 0 && file_index + 1 != options.len() {
        return Err(format!(
            "RUN_TOKEN_FILE_POSITION field={field} index={file_index} options={}",
            options.len()
        ));
    }
    Ok(file_index)
}

/// Move the token-menu selection to the last entry.
pub fn select_last_token(
    session: &mut TuiSession,
    state: &LibraryState,
    geometry: &ViewGeometry,
    field: usize,
) -> Result<(), String> {
    let handling = session.handle_event(
        Event::Key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE)),
        state,
        geometry,
    );
    if handling != EventHandling::Consumed {
        return Err(format!("RUN_TOKEN_END field={field} handling={handling:?}"));
    }
    Ok(())
}

/// Apply one focus step of a key prefix and name the reason that ends the prefix.
///
/// A focus step must stay inside the frontend and must move the focus ring.
pub fn focus_prefix_failure(
    state: &mut LibraryState,
    handling: EventHandling,
    step: usize,
) -> Option<String> {
    match handling {
        EventHandling::Action(action) => {
            let effect = state.update(action);
            host_effect_pending(&effect)
                .then(|| format!("focus_effect step={step} effect={effect:?}"))
        }
        EventHandling::Consumed => None,
        EventHandling::Ignored => Some(format!("focus_ignored step={step}")),
    }
}

/// Reach `expected` from every advertised binding of one command.
///
/// A binding can need a bounded prefix of `Tab` focus moves. The probe accepts the first prefix
/// that reaches the expected state and effect, and `max_prefix` is the focus budget.
pub fn command_key_action(
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
            let forked = base_session.try_fork();
            // `try_fork` returns `None` only while a path-completion worker owns a channel. A
            // walker session never starts that worker.
            let mut session = forked.ok_or("the persistent TUI session cannot fork for keys")?;
            let mut prefix_failed = None;
            for step in 0..prefix {
                let handling = session.handle_event(
                    Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
                    &probe_state,
                    geometry,
                );
                prefix_failed = focus_prefix_failure(&mut probe_state, handling, step);
                if prefix_failed.is_some() {
                    break;
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

/// Compare the complete rendered endpoint of one shared command on both input paths.
pub fn check_command_key_endpoint(
    context: FieldParityContext<'_>,
    bindings: &[UiBinding],
    mouse_action: &Action,
    mut mouse_session: TuiSession,
) -> Result<(), String> {
    let mut mouse_state = context.state.clone();
    let mouse_effect = mouse_state.update(mouse_action.clone());
    if host_effect_pending(&mouse_effect) {
        return Err(format!("COMMAND_MOUSE_EFFECT effect={mouse_effect:?}"));
    }
    let mouse_endpoint = render_probe_endpoint(
        &mut mouse_session,
        &mouse_state,
        context.locale,
        context.size,
    )?;
    for binding in bindings.iter().copied() {
        let forked = context.session.try_fork();
        // `try_fork` returns `None` only while a path-completion worker owns a channel. A walker
        // session never starts that worker.
        let mut key_session =
            forked.ok_or("the persistent TUI session cannot fork for commands")?;
        let mut key_state = context.state.clone();
        let key_start =
            render_probe_endpoint(&mut key_session, &key_state, context.locale, context.size)?;
        let handling = key_session.handle_event(
            Event::Key(binding_event(binding)),
            &key_state,
            &key_start.geometry,
        );
        apply_probe_handling(
            &mut key_state,
            handling,
            &format!("shared command binding={binding:?}"),
        )?;
        let key_endpoint =
            render_probe_endpoint(&mut key_session, &key_state, context.locale, context.size)?;
        if key_endpoint != mouse_endpoint {
            return Err(format!(
                "COMMAND_ENDPOINT binding={binding:?} mouse_action={mouse_action:?} mouse_focus={:?} key_focus={:?} state_equal={} geometry_equal={} buffer_equal={} cursor_equal={}",
                mouse_endpoint.state.focused_form_field(),
                key_endpoint.state.focused_form_field(),
                key_endpoint.state == mouse_endpoint.state,
                key_endpoint.geometry == mouse_endpoint.geometry,
                key_endpoint.backend.buffer() == mouse_endpoint.backend.buffer(),
                key_endpoint.backend.cursor_position() == mouse_endpoint.backend.cursor_position(),
            ));
        }
    }
    Ok(())
}

/// Invoke one command on a live session through its first advertised binding.
pub fn command_key_action_with_session(
    session: &mut TuiSession,
    state: &LibraryState,
    geometry: &ViewGeometry,
    command: UiCommand,
) -> Result<Action, String> {
    let binding = command_binding(state, command)?;
    session_action(session, Event::Key(binding_event(binding)), state, geometry)
}

/// Return the first advertised binding of one command.
pub fn command_binding(state: &LibraryState, command: UiCommand) -> Result<UiBinding, String> {
    command_bindings(state, command)?
        .first()
        .copied()
        .ok_or_else(|| format!("visible command {command:?} has no key binding"))
}

/// Return every advertised binding of one command in this context.
pub fn command_bindings(
    state: &LibraryState,
    command: UiCommand,
) -> Result<Vec<UiBinding>, String> {
    command_specs(state.command_context())
        .find(|spec| spec.command == command && state.command_enabled(command))
        .map(|spec| spec.bindings.to_vec())
        .ok_or_else(|| format!("visible command {command:?} is not advertised in this context"))
}

/// Return the terminal key event of one advertised binding.
#[must_use]
pub fn binding_event(binding: UiBinding) -> KeyEvent {
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

/// Return the release event that completes one primary press.
#[must_use]
pub fn matching_primary_release(event: &Event) -> Option<Event> {
    let Event::Mouse(mouse) = event else {
        return None;
    };
    matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)).then(|| {
        Event::Mouse(MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
            column: mouse.column,
            row: mouse.row,
            modifiers: mouse.modifiers,
        })
    })
}

/// Return the press and the release of one semantic click.
pub fn primary_click_events(down: Event) -> Result<(Event, Event), String> {
    let up = matching_primary_release(&down)
        .ok_or_else(|| format!("SEMANTIC_CLICK_EVENT event={down:?}"))?;
    Ok((down, up))
}

/// Refuse an event that one advertised local action does not carry.
pub fn check_local_event(event: &Event, advertised: &LocalAdvertisedAction) -> Result<(), String> {
    match event {
        Event::Key(key)
            if !advertised.keys.iter().any(|binding| {
                let expected = binding.event();
                expected.code == key.code
                    && expected.modifiers == key.modifiers
                    && expected.kind == key.kind
            }) =>
        {
            Err(format!(
                "LOCAL_ACTION_KEY target={:?} event={event:?} keys={:?}",
                advertised.target, advertised.keys,
            ))
        }
        Event::Mouse(mouse)
            if !advertised
                .hit
                .is_some_and(|rect| rect.contains((mouse.column, mouse.row).into())) =>
        {
            Err(format!(
                "LOCAL_ACTION_RECT target={:?} event={event:?} hit={:?}",
                advertised.target, advertised.hit,
            ))
        }
        Event::Key(_) | Event::Mouse(_) => Ok(()),
        Event::FocusGained | Event::FocusLost | Event::Paste(_) | Event::Resize(_, _) => {
            Err(format!(
                "LOCAL_ACTION_EVENT target={:?} event={event:?}",
                advertised.target,
            ))
        }
    }
}

/// Refuse a local press that the session did not consume.
pub fn check_local_press(
    event: &Event,
    advertised: &LocalAdvertisedAction,
    handling: &EventHandling,
) -> Result<(), String> {
    if *handling != EventHandling::Consumed {
        return Err(format!(
            "LOCAL_ACTION_PRESS target={:?} event={event:?} hit={:?} actual={handling:?}",
            advertised.target, advertised.hit,
        ));
    }
    Ok(())
}

/// Refuse a live local endpoint that differs from the advertised outcome.
pub fn check_local_endpoint(
    event: &Event,
    advertised: &LocalAdvertisedAction,
    handling: &EventHandling,
) -> Result<(), String> {
    let matches = match (&advertised.outcome, handling) {
        (LocalActionOutcome::Action(expected), EventHandling::Action(actual)) => expected == actual,
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
    Ok(())
}
