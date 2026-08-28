use proptest::{prelude::*, prop_oneof};
use ratatui_core::layout::{Rect, Size};
use ratatui_crossterm::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use serde::{Deserialize, Serialize};
use skit_tui::{
    HitRegion, HitTarget, LocalActionInventory, LocalAdvertisedAction, ViewGeometry, map_event,
};
use skit_ui::Action;
use skit_ui::{LibraryState, UiBinding, UiCommand, UiKey, command_specs};

pub(super) const RESIZE_CASES: &[(u16, u16)] = &[
    (1, 1),
    (1, 2),
    (2, 1),
    (24, 6),
    (40, 40),
    (46, 12),
    (80, 24),
    (120, 12),
    (120, 30),
    (300, 100),
];

pub(super) const PASTE_CASES: &[&str] = &["", "界", "e\u{301}", "🙂", "one\ntwo", "a\tb", "\0"];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum OperationFamily {
    AdvertisedKey,
    PublicHit,
    LocalAdvertisedKey,
    LocalHit,
    MouseCell,
    Resize,
    Paste,
    RawKey,
    Focus,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "operation")]
pub(super) enum WalkerOperation {
    AdvertisedKey {
        command: u8,
        binding: u8,
    },
    PublicHit {
        ordinal: u8,
    },
    LocalAdvertisedKey {
        action: u8,
        binding: u8,
    },
    LocalHit {
        action: u8,
    },
    MouseCell {
        x_fraction: u8,
        y_fraction: u8,
        kind: MouseKind,
    },
    Resize {
        width: u16,
        height: u16,
    },
    Paste {
        value: String,
    },
    RawKey {
        key: RawKey,
        kind: KeyKind,
    },
    Focus {
        gained: bool,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum MouseKind {
    LeftDown,
    RightDown,
    MiddleDown,
    LeftUp,
    LeftDrag,
    Move,
    ScrollUp,
    ScrollDown,
}

impl MouseKind {
    const ALL: &[Self] = &[
        Self::LeftDown,
        Self::RightDown,
        Self::MiddleDown,
        Self::LeftUp,
        Self::LeftDrag,
        Self::Move,
        Self::ScrollUp,
        Self::ScrollDown,
    ];

    const fn event_kind(self) -> MouseEventKind {
        match self {
            Self::LeftDown => MouseEventKind::Down(MouseButton::Left),
            Self::RightDown => MouseEventKind::Down(MouseButton::Right),
            Self::MiddleDown => MouseEventKind::Down(MouseButton::Middle),
            Self::LeftUp => MouseEventKind::Up(MouseButton::Left),
            Self::LeftDrag => MouseEventKind::Drag(MouseButton::Left),
            Self::Move => MouseEventKind::Moved,
            Self::ScrollUp => MouseEventKind::ScrollUp,
            Self::ScrollDown => MouseEventKind::ScrollDown,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum RawKey {
    Character,
    Enter,
    Escape,
    Delete,
    Backspace,
    Tab,
    BackTab,
    Up,
    Down,
    Left,
    Right,
    PageUp,
    PageDown,
    Home,
    End,
    Function2,
    ControlN,
    ControlR,
    AltX,
}

impl RawKey {
    pub(super) const ALL: &[Self] = &[
        Self::Character,
        Self::Enter,
        Self::Escape,
        Self::Delete,
        Self::Backspace,
        Self::Tab,
        Self::BackTab,
        Self::Up,
        Self::Down,
        Self::Left,
        Self::Right,
        Self::PageUp,
        Self::PageDown,
        Self::Home,
        Self::End,
        Self::Function2,
        Self::ControlN,
        Self::ControlR,
        Self::AltX,
    ];

    fn chord(self) -> (KeyCode, KeyModifiers) {
        match self {
            Self::Character => (KeyCode::Char('x'), KeyModifiers::NONE),
            Self::Enter => (KeyCode::Enter, KeyModifiers::NONE),
            Self::Escape => (KeyCode::Esc, KeyModifiers::NONE),
            Self::Delete => (KeyCode::Delete, KeyModifiers::NONE),
            Self::Backspace => (KeyCode::Backspace, KeyModifiers::NONE),
            Self::Tab => (KeyCode::Tab, KeyModifiers::NONE),
            Self::BackTab => (KeyCode::BackTab, KeyModifiers::SHIFT),
            Self::Up => (KeyCode::Up, KeyModifiers::NONE),
            Self::Down => (KeyCode::Down, KeyModifiers::NONE),
            Self::Left => (KeyCode::Left, KeyModifiers::NONE),
            Self::Right => (KeyCode::Right, KeyModifiers::NONE),
            Self::PageUp => (KeyCode::PageUp, KeyModifiers::NONE),
            Self::PageDown => (KeyCode::PageDown, KeyModifiers::NONE),
            Self::Home => (KeyCode::Home, KeyModifiers::NONE),
            Self::End => (KeyCode::End, KeyModifiers::NONE),
            Self::Function2 => (KeyCode::F(2), KeyModifiers::NONE),
            Self::ControlN => (KeyCode::Char('n'), KeyModifiers::CONTROL),
            Self::ControlR => (KeyCode::Char('r'), KeyModifiers::CONTROL),
            Self::AltX => (KeyCode::Char('x'), KeyModifiers::ALT),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum KeyKind {
    Press,
    Repeat,
    Release,
}

impl KeyKind {
    const ALL: &[Self] = &[Self::Press, Self::Repeat, Self::Release];

    const fn event_kind(self) -> KeyEventKind {
        match self {
            Self::Press => KeyEventKind::Press,
            Self::Repeat => KeyEventKind::Repeat,
            Self::Release => KeyEventKind::Release,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum ResolvedOperation {
    Event(Event),
    LocalEvent {
        event: Event,
        advertised: Box<LocalAdvertisedAction>,
    },
    Resize {
        width: u16,
        height: u16,
    },
    Noop,
}

pub(super) fn operation_strategy() -> BoxedStrategy<WalkerOperation> {
    let advertised = (any::<u8>(), any::<u8>())
        .prop_map(|(command, binding)| WalkerOperation::AdvertisedKey { command, binding });
    let public_hit = any::<u8>().prop_map(|ordinal| WalkerOperation::PublicHit { ordinal });
    let local_key = (any::<u8>(), any::<u8>())
        .prop_map(|(action, binding)| WalkerOperation::LocalAdvertisedKey { action, binding });
    let local_hit = any::<u8>().prop_map(|action| WalkerOperation::LocalHit { action });
    let mouse = (
        any::<u8>(),
        any::<u8>(),
        proptest::sample::select(MouseKind::ALL.to_vec()),
    )
        .prop_map(
            |(x_fraction, y_fraction, kind)| WalkerOperation::MouseCell {
                x_fraction,
                y_fraction,
                kind,
            },
        );
    let resize = proptest::sample::select(RESIZE_CASES.to_vec())
        .prop_map(|(width, height)| WalkerOperation::Resize { width, height });
    let paste = prop_oneof![
        4 => proptest::sample::select(PASTE_CASES.iter().map(|value| (*value).to_owned()).collect::<Vec<_>>()),
        1 => ".{0,24}",
    ]
    .prop_map(|value| WalkerOperation::Paste { value });
    let raw_key = (
        proptest::sample::select(RawKey::ALL.to_vec()),
        proptest::sample::select(KeyKind::ALL.to_vec()),
    )
        .prop_map(|(key, kind)| WalkerOperation::RawKey { key, kind });

    prop_oneof![
        12 => advertised,
        10 => public_hit,
        12 => local_key,
        10 => local_hit,
        8 => mouse,
        3 => resize,
        5 => paste,
        8 => raw_key,
        1 => any::<bool>().prop_map(|gained| WalkerOperation::Focus { gained }),
    ]
    .boxed()
}

pub(super) fn resolve(
    operation: &WalkerOperation,
    state: &LibraryState,
    geometry: &ViewGeometry,
    size: Size,
    local_actions: &LocalActionInventory,
) -> ResolvedOperation {
    match operation {
        WalkerOperation::AdvertisedKey { command, binding } => {
            let Some((_, binding)) = resolve_advertised_command(state, *command, *binding) else {
                return ResolvedOperation::Noop;
            };
            ResolvedOperation::Event(Event::Key(binding_event(binding)))
        }
        WalkerOperation::PublicHit { ordinal } => {
            let Some(hit) = resolve_public_hit(geometry, *ordinal) else {
                return ResolvedOperation::Noop;
            };
            ResolvedOperation::Event(Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: hit.rect.x.saturating_add(hit.rect.width / 2),
                row: hit.rect.y.saturating_add(hit.rect.height / 2),
                modifiers: KeyModifiers::NONE,
            }))
        }
        WalkerOperation::LocalAdvertisedKey { action, binding } => {
            let Some(action) = choose(&local_actions.actions, *action) else {
                return ResolvedOperation::Noop;
            };
            let Some(binding) = choose(&action.keys, *binding) else {
                return ResolvedOperation::Noop;
            };
            ResolvedOperation::LocalEvent {
                event: Event::Key(binding.event()),
                advertised: Box::new(action.clone()),
            }
        }
        WalkerOperation::LocalHit { action } => {
            let Some(action) = choose(&local_actions.actions, *action) else {
                return ResolvedOperation::Noop;
            };
            let Some(rect) = action.hit else {
                return ResolvedOperation::Noop;
            };
            ResolvedOperation::LocalEvent {
                event: Event::Mouse(MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    column: rect.x.saturating_add(rect.width / 2),
                    row: rect.y.saturating_add(rect.height / 2),
                    modifiers: KeyModifiers::NONE,
                }),
                advertised: Box::new(action.clone()),
            }
        }
        WalkerOperation::MouseCell {
            x_fraction,
            y_fraction,
            kind,
        } => no_quit(
            Event::Mouse(MouseEvent {
                kind: kind.event_kind(),
                column: scale(*x_fraction, size.width),
                row: scale(*y_fraction, size.height),
                modifiers: KeyModifiers::NONE,
            }),
            state,
            geometry,
        ),
        WalkerOperation::Resize { width, height } => ResolvedOperation::Resize {
            width: (*width).max(1),
            height: (*height).max(1),
        },
        WalkerOperation::Paste { value } => ResolvedOperation::Event(Event::Paste(value.clone())),
        WalkerOperation::RawKey { key, kind } => {
            let (code, modifiers) = key.chord();
            let event = Event::Key(KeyEvent::new_with_kind(code, modifiers, kind.event_kind()));
            no_quit(event, state, geometry)
        }
        WalkerOperation::Focus { gained } => ResolvedOperation::Event(if *gained {
            Event::FocusGained
        } else {
            Event::FocusLost
        }),
    }
}

pub(super) fn resolve_advertised_command(
    state: &LibraryState,
    command: u8,
    binding: u8,
) -> Option<(UiCommand, UiBinding)> {
    let commands = command_specs(state.command_context())
        .filter(|spec| spec.command != UiCommand::Quit && state.command_enabled(spec.command))
        .collect::<Vec<_>>();
    let spec = choose(&commands, command)?;
    Some((spec.command, *choose(spec.bindings, binding)?))
}

pub(super) fn resolve_public_hit(geometry: &ViewGeometry, ordinal: u8) -> Option<HitRegion> {
    let hits = geometry
        .hits
        .iter()
        .filter(|hit| {
            hit.rect.width > 0
                && hit.rect.height > 0
                && hit.action != HitTarget::Command(UiCommand::Quit)
        })
        .collect::<Vec<_>>();
    choose(&hits, ordinal).copied().copied()
}

fn no_quit(event: Event, state: &LibraryState, geometry: &ViewGeometry) -> ResolvedOperation {
    if map_event(event.clone(), state, geometry) == Some(Action::Quit) {
        ResolvedOperation::Noop
    } else {
        ResolvedOperation::Event(event)
    }
}

fn choose<T>(values: &[T], ordinal: u8) -> Option<&T> {
    (!values.is_empty()).then(|| &values[usize::from(ordinal) % values.len()])
}

fn scale(fraction: u8, length: u16) -> u16 {
    if length <= 1 {
        return 0;
    }
    let last = u32::from(length - 1);
    u16::try_from(u32::from(fraction) * last / u32::from(u8::MAX)).unwrap_or(length - 1)
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

fn geometry(rect: Rect) -> ViewGeometry {
    ViewGeometry {
        hits: vec![HitRegion {
            rect,
            action: HitTarget::Command(UiCommand::Help),
        }],
        ..ViewGeometry::default()
    }
}

#[test]
fn operation_strategy_generates_every_required_event_family() {
    let strategy = operation_strategy();
    let mut runner = proptest::test_runner::TestRunner::deterministic();
    let mut observed = Vec::new();
    for _ in 0..4096 {
        let operation = strategy.new_tree(&mut runner).unwrap().current();
        let family = match operation {
            WalkerOperation::AdvertisedKey { .. } => OperationFamily::AdvertisedKey,
            WalkerOperation::PublicHit { .. } => OperationFamily::PublicHit,
            WalkerOperation::LocalAdvertisedKey { .. } => OperationFamily::LocalAdvertisedKey,
            WalkerOperation::LocalHit { .. } => OperationFamily::LocalHit,
            WalkerOperation::MouseCell { .. } => OperationFamily::MouseCell,
            WalkerOperation::Resize { .. } => OperationFamily::Resize,
            WalkerOperation::Paste { .. } => OperationFamily::Paste,
            WalkerOperation::RawKey { .. } => OperationFamily::RawKey,
            WalkerOperation::Focus { .. } => OperationFamily::Focus,
        };
        if !observed.contains(&family) {
            observed.push(family);
        }
    }
    for expected in [
        OperationFamily::AdvertisedKey,
        OperationFamily::PublicHit,
        OperationFamily::LocalAdvertisedKey,
        OperationFamily::LocalHit,
        OperationFamily::MouseCell,
        OperationFamily::Resize,
        OperationFamily::Paste,
        OperationFamily::RawKey,
        OperationFamily::Focus,
    ] {
        assert!(observed.contains(&expected), "missing {expected:?}");
    }
}

#[test]
fn public_hits_resolve_against_the_latest_geometry() {
    let operation = WalkerOperation::PublicHit { ordinal: 0 };
    let state = LibraryState::default();
    let first = resolve(
        &operation,
        &state,
        &geometry(Rect::new(1, 2, 3, 2)),
        Size::new(20, 10),
        &LocalActionInventory::default(),
    );
    let second = resolve(
        &operation,
        &state,
        &geometry(Rect::new(10, 7, 5, 3)),
        Size::new(20, 10),
        &LocalActionInventory::default(),
    );

    assert_eq!(mouse_position(&first), Some((2, 3)));
    assert_eq!(mouse_position(&second), Some((12, 8)));
}

#[test]
fn arbitrary_mouse_cells_stay_inside_the_current_viewport() {
    let state = LibraryState::default();
    let operation = WalkerOperation::MouseCell {
        x_fraction: u8::MAX,
        y_fraction: u8::MAX,
        kind: MouseKind::LeftDown,
    };
    let resolved = resolve(
        &operation,
        &state,
        &ViewGeometry::default(),
        Size::new(24, 6),
        &LocalActionInventory::default(),
    );
    assert_eq!(mouse_position(&resolved), Some((23, 5)));
}

#[test]
fn resize_cases_include_tiny_responsive_and_large_viewports() {
    for required in [
        (1, 1),
        (1, 2),
        (2, 1),
        (24, 6),
        (40, 40),
        (46, 12),
        (80, 24),
        (120, 12),
        (120, 30),
        (300, 100),
    ] {
        assert!(RESIZE_CASES.contains(&required), "missing {required:?}");
    }
}

#[test]
fn paste_cases_cover_terminal_text_edges() {
    for required in ["", "界", "e\u{301}", "🙂", "one\ntwo", "a\tb", "\0"] {
        assert!(PASTE_CASES.contains(&required), "missing {required:?}");
    }
}

#[test]
fn semantic_traces_have_deterministic_json_replays() {
    let trace = vec![
        WalkerOperation::Resize {
            width: 24,
            height: 6,
        },
        WalkerOperation::Paste {
            value: "界🙂".to_owned(),
        },
        WalkerOperation::RawKey {
            key: RawKey::Escape,
            kind: KeyKind::Press,
        },
    ];
    let bytes = serde_json::to_vec(&trace).unwrap();
    let decoded: Vec<WalkerOperation> = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(decoded, trace);
}

#[test]
fn raw_key_inventory_excludes_the_session_clock_dependent_ctrl_c_chord() {
    let state = LibraryState::default();
    for key in RawKey::ALL {
        let resolved = resolve(
            &WalkerOperation::RawKey {
                key: *key,
                kind: KeyKind::Press,
            },
            &state,
            &ViewGeometry::default(),
            Size::new(80, 24),
            &LocalActionInventory::default(),
        );
        if let ResolvedOperation::Event(Event::Key(key)) = resolved {
            assert_ne!(
                (key.code, key.modifiers),
                (KeyCode::Char('c'), KeyModifiers::CONTROL)
            );
        }
    }
}

#[test]
fn model_operations_do_not_select_quit_from_shared_keys_or_hits() {
    let state = LibraryState::default();
    for command in 0..=u8::MAX {
        let resolved = resolve(
            &WalkerOperation::AdvertisedKey {
                command,
                binding: 0,
            },
            &state,
            &ViewGeometry::default(),
            Size::new(80, 24),
            &LocalActionInventory::default(),
        );
        if let ResolvedOperation::Event(Event::Key(key)) = resolved {
            assert_ne!(
                (key.code, key.modifiers),
                (KeyCode::Char('c'), KeyModifiers::CONTROL),
            );
        }
    }

    let quit = ViewGeometry {
        hits: vec![HitRegion {
            rect: Rect::new(1, 1, 8, 1),
            action: HitTarget::Command(UiCommand::Quit),
        }],
        ..ViewGeometry::default()
    };
    assert_eq!(
        resolve(
            &WalkerOperation::PublicHit { ordinal: 0 },
            &state,
            &quit,
            Size::new(80, 24),
            &LocalActionInventory::default(),
        ),
        ResolvedOperation::Noop,
    );
}

fn mouse_position(operation: &ResolvedOperation) -> Option<(u16, u16)> {
    match operation {
        ResolvedOperation::Event(Event::Mouse(mouse))
        | ResolvedOperation::LocalEvent {
            event: Event::Mouse(mouse),
            ..
        } => Some((mouse.column, mouse.row)),
        ResolvedOperation::Event(_)
        | ResolvedOperation::LocalEvent { .. }
        | ResolvedOperation::Resize { .. }
        | ResolvedOperation::Noop => None,
    }
}
