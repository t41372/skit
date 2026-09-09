use proptest::{prelude::*, prop_oneof};
use ratatui_core::layout::Size;
use ratatui_crossterm::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use serde::{Deserialize, Serialize};
use skit_i18n::Locale;
use skit_tui::{
    HitRegion, HitTarget, LocalActionInventory, LocalActionTarget, LocalAdvertisedAction,
    ScreenTarget, ScreenTargetHit, ScreenTargetInventory, ViewGeometry,
};
use skit_ui::{LibraryState, UiBinding, UiCommand, command_specs};

use crate::parity::binding_event;

/// Viewport shapes that one walk resizes to.
///
/// The list holds the degenerate one-column and one-row tiers, the compact tier, the responsive
/// tier, the default terminal, and one oversize terminal.
pub const RESIZE_CASES: &[(u16, u16)] = &[
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

/// Paste payloads that one walk sends.
///
/// The list holds the empty payload, one wide character, one combining sequence, one astral
/// character, one newline, one tab, and one NUL.
pub const PASTE_CASES: &[&str] = &["", "界", "e\u{301}", "🙂", "one\ntwo", "a\tb", "\0"];

const COMPLETE_PROFILES: &[(Locale, Size)] = &[
    (Locale::En, Size::new(1, 1)),
    (Locale::En, Size::new(24, 6)),
    (Locale::En, Size::new(80, 24)),
    (Locale::En, Size::new(120, 30)),
    (Locale::ZhCn, Size::new(1, 1)),
    (Locale::ZhCn, Size::new(24, 6)),
    (Locale::ZhCn, Size::new(120, 30)),
    (Locale::ZhTw, Size::new(1, 1)),
    (Locale::ZhTw, Size::new(24, 6)),
    (Locale::ZhTw, Size::new(40, 40)),
    (Locale::ZhTw, Size::new(120, 30)),
    (Locale::Pseudo, Size::new(1, 1)),
    (Locale::Pseudo, Size::new(24, 6)),
    (Locale::Pseudo, Size::new(120, 12)),
    (Locale::Pseudo, Size::new(120, 30)),
];

/// Every locale and viewport shape that one complete walk replays each trace against.
#[must_use]
pub const fn random_walk_profiles() -> &'static [(Locale, Size)] {
    COMPLETE_PROFILES
}

/// One weighted family of the random operation model.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationFamily {
    /// A shared command reached by its advertised chord.
    AdvertisedKey,
    /// A public hit target from the rendered geometry.
    PublicHit,
    /// A screen-local action reached by its advertised chord.
    LocalAdvertisedKey,
    /// A screen-local action reached by a click.
    LocalHit,
    /// One arbitrary cell of the viewport.
    MouseCell,
    /// One viewport shape from [`RESIZE_CASES`].
    Resize,
    /// One paste payload.
    Paste,
    /// One raw key chord.
    RawKey,
    /// One terminal focus change.
    Focus,
}

impl OperationFamily {
    /// Every family that [`operation_strategy`] draws from.
    pub const ALL: &'static [Self] = &[
        Self::AdvertisedKey,
        Self::PublicHit,
        Self::LocalAdvertisedKey,
        Self::LocalHit,
        Self::MouseCell,
        Self::Resize,
        Self::Paste,
        Self::RawKey,
        Self::Focus,
    ];
}

/// One drawn operation with late-bound ordinals.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "operation")]
pub enum RandomOperation {
    /// Reach one enabled shared command through the command registry.
    AdvertisedKey {
        /// Ordinal into the enabled command registry.
        command: u8,
        /// Ordinal into the chords of the selected command.
        binding: u8,
    },
    /// Click one public hit target of the rendered geometry.
    PublicHit {
        /// Ordinal into the visible hit targets.
        ordinal: u8,
    },
    /// Reach one screen-local target through its advertised chord.
    LocalAdvertisedKey {
        /// Ordinal into the screen-local keyboard targets.
        action: u8,
        /// Ordinal into the chords of the selected target.
        binding: u8,
    },
    /// Click one screen-local target.
    LocalHit {
        /// Ordinal into the screen-local click targets.
        action: u8,
    },
    /// Send one pointer event to an arbitrary cell of the viewport.
    MouseCell {
        /// Horizontal position as a fraction of the viewport width.
        x_fraction: u8,
        /// Vertical position as a fraction of the viewport height.
        y_fraction: u8,
        /// Pointer event kind.
        kind: MouseKind,
    },
    /// Resize the terminal.
    Resize {
        /// New viewport width.
        width: u16,
        /// New viewport height.
        height: u16,
    },
    /// Paste one payload.
    Paste {
        /// Pasted text.
        value: String,
    },
    /// Send one raw key chord.
    RawKey {
        /// Key chord from [`RawKey::ALL`].
        key: RawKey,
        /// Key event kind.
        kind: KeyKind,
    },
    /// Change the terminal focus.
    Focus {
        /// The terminal gained focus.
        gained: bool,
    },
}

impl RandomOperation {
    /// Return the family that produced this operation.
    #[must_use]
    pub const fn family(&self) -> OperationFamily {
        match self {
            Self::AdvertisedKey { .. } => OperationFamily::AdvertisedKey,
            Self::PublicHit { .. } => OperationFamily::PublicHit,
            Self::LocalAdvertisedKey { .. } => OperationFamily::LocalAdvertisedKey,
            Self::LocalHit { .. } => OperationFamily::LocalHit,
            Self::MouseCell { .. } => OperationFamily::MouseCell,
            Self::Resize { .. } => OperationFamily::Resize,
            Self::Paste { .. } => OperationFamily::Paste,
            Self::RawKey { .. } => OperationFamily::RawKey,
            Self::Focus { .. } => OperationFamily::Focus,
        }
    }
}

/// One pointer event kind that the model sends.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MouseKind {
    /// Primary button press.
    LeftDown,
    /// Secondary button press.
    RightDown,
    /// Middle button press.
    MiddleDown,
    /// Primary button release.
    LeftUp,
    /// Primary button drag.
    LeftDrag,
    /// Pointer motion.
    Move,
    /// Wheel up.
    ScrollUp,
    /// Wheel down.
    ScrollDown,
}

impl MouseKind {
    /// Every pointer event kind that the model sends.
    pub const ALL: &'static [Self] = &[
        Self::LeftDown,
        Self::RightDown,
        Self::MiddleDown,
        Self::LeftUp,
        Self::LeftDrag,
        Self::Move,
        Self::ScrollUp,
        Self::ScrollDown,
    ];
}

/// One raw key chord that the model sends.
///
/// The inventory keeps the chords whose meaning is stable across one walk.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RawKey {
    /// One printable character.
    Character,
    /// Enter.
    Enter,
    /// Escape.
    Escape,
    /// Forward delete.
    Delete,
    /// Backward delete.
    Backspace,
    /// Tab.
    Tab,
    /// Reverse Tab.
    BackTab,
    /// Up arrow.
    Up,
    /// Down arrow.
    Down,
    /// Left arrow.
    Left,
    /// Right arrow.
    Right,
    /// Page Up.
    PageUp,
    /// Page Down.
    PageDown,
    /// Home.
    Home,
    /// End.
    End,
    /// Function key 2.
    Function2,
    /// Control and N.
    ControlN,
    /// Control and R.
    ControlR,
    /// Alt and X.
    AltX,
}

impl RawKey {
    /// Every raw key chord that the model sends.
    pub const ALL: &'static [Self] = &[
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

    fn event(self, kind: KeyKind) -> KeyEvent {
        let (code, modifiers) = self.chord();
        KeyEvent::new_with_kind(code, modifiers, kind.event_kind())
    }
}

/// One key event kind that the model sends.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyKind {
    /// First report of the chord.
    Press,
    /// Repeated report of the chord.
    Repeat,
    /// Release report of the chord.
    Release,
}

impl KeyKind {
    /// Every key event kind that the model sends.
    pub const ALL: &'static [Self] = &[Self::Press, Self::Repeat, Self::Release];

    const fn event_kind(self) -> KeyEventKind {
        match self {
            Self::Press => KeyEventKind::Press,
            Self::Repeat => KeyEventKind::Repeat,
            Self::Release => KeyEventKind::Release,
        }
    }
}

/// One enabled shared command and every chord that reaches it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdvertisedCommand {
    /// Stable command identity.
    pub command: UiCommand,
    /// Every advertised chord of the command, in registry order.
    pub bindings: Vec<UiBinding>,
}

/// Everything one rendered frame offers to the late binder.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LiveInventory {
    /// Enabled shared commands of the active surface with their chords.
    pub commands: Vec<AdvertisedCommand>,
    /// Public hit targets of the rendered geometry.
    pub hits: Vec<HitRegion>,
    /// Screen-local advertised actions of the rendered frame.
    pub local_actions: Vec<LocalAdvertisedAction>,
    /// Keyboard focus order of the active screen.
    pub screen_focus: Vec<ScreenTarget>,
    /// Screen target rectangles of the rendered frame.
    pub screen_hits: Vec<ScreenTargetHit>,
    /// Current viewport size.
    pub size: Size,
}

impl LiveInventory {
    /// Collect the live inventory of one rendered frame.
    #[must_use]
    pub fn new(
        state: &LibraryState,
        geometry: &ViewGeometry,
        local_actions: &LocalActionInventory,
        screen_targets: Option<&ScreenTargetInventory>,
        size: Size,
    ) -> Self {
        Self {
            commands: command_specs(state.command_context())
                .filter(|spec| state.command_enabled(spec.command))
                .map(|spec| AdvertisedCommand {
                    command: spec.command,
                    bindings: spec.bindings.to_vec(),
                })
                .collect(),
            hits: geometry.hits.clone(),
            local_actions: local_actions.actions.clone(),
            screen_focus: screen_targets
                .and_then(|inventory| inventory.focus.as_ref())
                .map(|focus| focus.order.clone())
                .unwrap_or_default(),
            screen_hits: screen_targets
                .map(|inventory| inventory.hits.clone())
                .unwrap_or_default(),
            size,
        }
    }
}

/// The reason one ordinal binds to nothing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResolveRefusal {
    /// The frame offers no target for the family.
    Unavailable,
    /// The selected target has no visible rectangle.
    Clipped,
    /// The selected target ends the session.
    QuitFiltered,
}

/// One bound input that a frontend can send.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResolvedInput {
    /// Reach one shared command through one of its advertised chords.
    CommandKeyboard {
        /// Shared command that the chord invokes.
        command: UiCommand,
        /// Advertised chord that the binding ordinal selected.
        key: KeyEvent,
    },
    /// Click one public hit target.
    Hit(HitTarget),
    /// Reach one screen-local action through one advertised chord.
    LocalKeyboard {
        /// Screen-local identity that the chord reaches.
        target: LocalActionTarget,
        /// Advertised chord of the target.
        key: KeyEvent,
    },
    /// Click one screen-local action.
    LocalHit(LocalActionTarget),
    /// Move the keyboard focus to one screen target.
    ScreenFocus(ScreenTarget),
    /// Click one screen target.
    ScreenHit(ScreenTarget),
    /// Send one pointer event to one viewport cell.
    RawMouse {
        /// Viewport column.
        column: u16,
        /// Viewport row.
        row: u16,
        /// Pointer event kind.
        kind: MouseKind,
    },
    /// Send one raw key chord.
    RawKey(KeyEvent),
    /// Paste one payload.
    Paste(String),
    /// Change the terminal focus.
    Focus {
        /// The terminal gained focus.
        gained: bool,
    },
    /// Resize the terminal.
    Resize {
        /// New viewport width.
        width: u16,
        /// New viewport height.
        height: u16,
    },
    /// The ordinal binds to nothing on this frame.
    NotApplicable(ResolveRefusal),
}

/// Build the weighted strategy over the nine operation families.
pub fn operation_strategy() -> BoxedStrategy<RandomOperation> {
    let advertised = (any::<u8>(), any::<u8>())
        .prop_map(|(command, binding)| RandomOperation::AdvertisedKey { command, binding });
    let public_hit = any::<u8>().prop_map(|ordinal| RandomOperation::PublicHit { ordinal });
    let local_key = (any::<u8>(), any::<u8>())
        .prop_map(|(action, binding)| RandomOperation::LocalAdvertisedKey { action, binding });
    let local_hit = any::<u8>().prop_map(|action| RandomOperation::LocalHit { action });
    let mouse = (
        any::<u8>(),
        any::<u8>(),
        proptest::sample::select(MouseKind::ALL.to_vec()),
    )
        .prop_map(
            |(x_fraction, y_fraction, kind)| RandomOperation::MouseCell {
                x_fraction,
                y_fraction,
                kind,
            },
        );
    let resize = proptest::sample::select(RESIZE_CASES.to_vec())
        .prop_map(|(width, height)| RandomOperation::Resize { width, height });
    let paste = prop_oneof![
        4 => proptest::sample::select(PASTE_CASES.iter().map(|value| (*value).to_owned()).collect::<Vec<_>>()),
        1 => ".{0,24}",
    ]
    .prop_map(|value| RandomOperation::Paste { value });
    let raw_key = (
        proptest::sample::select(RawKey::ALL.to_vec()),
        proptest::sample::select(KeyKind::ALL.to_vec()),
    )
        .prop_map(|(key, kind)| RandomOperation::RawKey { key, kind });

    prop_oneof![
        12 => advertised,
        10 => public_hit,
        12 => local_key,
        10 => local_hit,
        8 => mouse,
        3 => resize,
        5 => paste,
        8 => raw_key,
        1 => any::<bool>().prop_map(|gained| RandomOperation::Focus { gained }),
    ]
    .boxed()
}

/// Bind one drawn operation to the live inventory of the current frame.
///
/// Quit is never bound by the inventory families. A choice that lands on Quit becomes
/// [`ResolveRefusal::QuitFiltered`]. The caller applies the session-level filter to
/// [`ResolvedInput::RawMouse`] and [`ResolvedInput::RawKey`].
///
/// The two screen-local families address one ordinal space over both screen-local inventories.
/// An ordinal below the local action count reaches one advertised chip. A higher ordinal reaches
/// one screen target of the same frame.
///
/// The command family binds two ordinals. The first selects one enabled command. The second
/// selects one chord of that command, so a command with two chords reaches both of them. A
/// command that advertises no chord refuses as [`ResolveRefusal::Unavailable`].
#[must_use]
pub fn resolve(operation: &RandomOperation, live: &LiveInventory) -> ResolvedInput {
    match operation {
        RandomOperation::AdvertisedKey { command, binding } => {
            resolve_command(live, *command, *binding)
        }
        RandomOperation::PublicHit { ordinal } => resolve_public_hit(live, *ordinal),
        RandomOperation::LocalAdvertisedKey { action, binding } => {
            resolve_local_key(live, *action, *binding)
        }
        RandomOperation::LocalHit { action } => resolve_local_hit(live, *action),
        RandomOperation::MouseCell {
            x_fraction,
            y_fraction,
            kind,
        } => ResolvedInput::RawMouse {
            column: scale(*x_fraction, live.size.width),
            row: scale(*y_fraction, live.size.height),
            kind: *kind,
        },
        RandomOperation::Resize { width, height } => ResolvedInput::Resize {
            width: *width,
            height: *height,
        },
        RandomOperation::Paste { value } => ResolvedInput::Paste(value.clone()),
        RandomOperation::RawKey { key, kind } => ResolvedInput::RawKey(key.event(*kind)),
        RandomOperation::Focus { gained } => ResolvedInput::Focus { gained: *gained },
    }
}

fn resolve_command(live: &LiveInventory, ordinal: u8, binding: u8) -> ResolvedInput {
    let Some(entry) = choose(&live.commands, ordinal) else {
        return ResolvedInput::NotApplicable(ResolveRefusal::Unavailable);
    };
    if entry.command == UiCommand::Quit {
        return ResolvedInput::NotApplicable(ResolveRefusal::QuitFiltered);
    }
    let Some(chord) = choose(&entry.bindings, binding) else {
        return ResolvedInput::NotApplicable(ResolveRefusal::Unavailable);
    };
    ResolvedInput::CommandKeyboard {
        command: entry.command,
        key: binding_event(*chord),
    }
}

fn resolve_public_hit(live: &LiveInventory, ordinal: u8) -> ResolvedInput {
    let visible = live
        .hits
        .iter()
        .filter(|hit| hit.rect.width > 0 && hit.rect.height > 0)
        .collect::<Vec<_>>();
    let Some(hit) = choose(&visible, ordinal) else {
        return ResolvedInput::NotApplicable(ResolveRefusal::Unavailable);
    };
    if hit.action == HitTarget::Command(UiCommand::Quit) {
        return ResolvedInput::NotApplicable(ResolveRefusal::QuitFiltered);
    }
    ResolvedInput::Hit(hit.action)
}

fn resolve_local_key(live: &LiveInventory, ordinal: u8, binding: u8) -> ResolvedInput {
    let local = live.local_actions.len();
    let Some(index) = position(local + live.screen_focus.len(), ordinal) else {
        return ResolvedInput::NotApplicable(ResolveRefusal::Unavailable);
    };
    if index >= local {
        return ResolvedInput::ScreenFocus(live.screen_focus[index - local].clone());
    }
    let action = &live.local_actions[index];
    let Some(key) = choose(&action.keys, binding) else {
        return ResolvedInput::NotApplicable(ResolveRefusal::Unavailable);
    };
    ResolvedInput::LocalKeyboard {
        target: action.target.clone(),
        key: key.event(),
    }
}

fn resolve_local_hit(live: &LiveInventory, ordinal: u8) -> ResolvedInput {
    let local = live.local_actions.len();
    let Some(index) = position(local + live.screen_hits.len(), ordinal) else {
        return ResolvedInput::NotApplicable(ResolveRefusal::Unavailable);
    };
    if index >= local {
        return ResolvedInput::ScreenHit(live.screen_hits[index - local].target.clone());
    }
    let action = &live.local_actions[index];
    if action.hit.is_none() {
        return ResolvedInput::NotApplicable(ResolveRefusal::Clipped);
    }
    ResolvedInput::LocalHit(action.target.clone())
}

fn position(length: usize, ordinal: u8) -> Option<usize> {
    (length > 0).then(|| usize::from(ordinal) % length)
}

fn choose<T>(values: &[T], ordinal: u8) -> Option<&T> {
    position(values.len(), ordinal).map(|index| &values[index])
}

fn scale(fraction: u8, length: u16) -> u16 {
    let last = length.saturating_sub(1);
    let scaled = u32::from(fraction) * u32::from(last) / u32::from(u8::MAX);
    u16::try_from(scaled).unwrap_or(last)
}
