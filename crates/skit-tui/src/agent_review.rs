//! Canonical terminal-session state for agent-review artifacts.

use std::{
    collections::BTreeMap,
    ffi::OsStr,
    hash::Hash,
    path::{Path, PathBuf},
};

use ratatui_core::{layout::Rect, style::Color};
use ratatui_interact::{
    components::{
        ButtonState, CheckBoxState, ListPickerState, ScrollableContentState, SelectState,
    },
    state::FocusManager,
    traits::ClickRegionRegistry,
};
use ratatui_textarea::TextArea as RichTextArea;
use serde::Serialize;

use crate::local_action::LocalKey;
use crate::local_action::{
    LocalActionInventory, LocalActionOutcome, LocalActionTarget, LocalAdvertisedAction,
};
use crate::screens::add::add_action_snapshot;

/// `ratatui-textarea` crate name recorded beside opaque editor state.
pub const RATATUI_TEXTAREA_CRATE: &str = "ratatui-textarea";
/// Locked `ratatui-textarea` version whose `Debug` representation is recorded.
pub const RATATUI_TEXTAREA_VERSION: &str = "0.9.2";
/// Schema version for the canonical terminal-session snapshot.
pub const AGENT_REVIEW_SNAPSHOT_VERSION: u32 = 1;

/// A deterministic, typed node in the canonical terminal-session snapshot.
#[derive(Clone, Debug, Serialize)]
pub(crate) struct AgentReviewNode {
    kind: &'static str,
    fields: BTreeMap<&'static str, serde_json::Value>,
}

/// Canonical snapshot of every `TuiSession` field.
#[derive(Clone, Debug, Serialize)]
pub struct AgentReviewSnapshot {
    schema_version: u32,
    quit_armed_at: AgentReviewNode,
    quit_toast: AgentReviewNode,
    search: AgentReviewNode,
    library: AgentReviewNode,
    help: AgentReviewNode,
    confirm_remove: AgentReviewNode,
    run: AgentReviewNode,
    path_suggestions: AgentReviewNode,
    run_modal: AgentReviewNode,
    preferences: AgentReviewNode,
    report: AgentReviewNode,
    settings: AgentReviewNode,
    settings_geometry: AgentReviewNode,
    settings_prompt_overlay: Option<AgentReviewNode>,
    add: AgentReviewNode,
    add_geometry: AgentReviewNode,
    add_overlay: Option<AgentReviewNode>,
    file_picker_source: Option<AgentReviewNode>,
    health: AgentReviewNode,
    runners: AgentReviewNode,
    runner_editor: AgentReviewNode,
    form: AgentReviewNode,
    footer: AgentReviewNode,
    clicks: AgentReviewNode,
    top_level_click: AgentReviewNode,
    local_actions: AgentReviewNode,
}

impl AgentReviewSnapshot {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        quit_armed_at: AgentReviewNode,
        quit_toast: AgentReviewNode,
        search: AgentReviewNode,
        library: AgentReviewNode,
        help: AgentReviewNode,
        confirm_remove: AgentReviewNode,
        run: AgentReviewNode,
        path_suggestions: AgentReviewNode,
        run_modal: AgentReviewNode,
        preferences: AgentReviewNode,
        report: AgentReviewNode,
        settings: AgentReviewNode,
        settings_geometry: AgentReviewNode,
        settings_prompt_overlay: Option<AgentReviewNode>,
        add: AgentReviewNode,
        add_geometry: AgentReviewNode,
        add_overlay: Option<AgentReviewNode>,
        file_picker_source: Option<AgentReviewNode>,
        health: AgentReviewNode,
        runners: AgentReviewNode,
        runner_editor: AgentReviewNode,
        form: AgentReviewNode,
        footer: AgentReviewNode,
        clicks: AgentReviewNode,
        top_level_click: AgentReviewNode,
        local_actions: AgentReviewNode,
    ) -> Self {
        Self {
            schema_version: AGENT_REVIEW_SNAPSHOT_VERSION,
            quit_armed_at,
            quit_toast,
            search,
            library,
            help,
            confirm_remove,
            run,
            path_suggestions,
            run_modal,
            preferences,
            report,
            settings,
            settings_geometry,
            settings_prompt_overlay,
            add,
            add_geometry,
            add_overlay,
            file_picker_source,
            health,
            runners,
            runner_editor,
            form,
            footer,
            clicks,
            top_level_click,
            local_actions,
        }
    }
}

/// A terminal session cannot produce a deterministic canonical snapshot.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum AgentReviewSnapshotError {
    /// The two-step quit chord contains a live monotonic and wall-clock deadline.
    #[error("the session has active clock-dependent quit state")]
    ClockActive,
    /// A path-completion channel has schedule-dependent queued state.
    #[error("the session has active asynchronous path completion")]
    AsyncPathCompletionActive,
    /// An active picker reads the host filesystem rather than the deterministic memory tree.
    #[error("the session has an active real-filesystem picker")]
    RealFilesystemPicker,
    /// One typed field unexpectedly failed serde conversion.
    #[error("could not serialize canonical field {field}: {message}")]
    Serialization {
        field: &'static str,
        message: String,
    },
}

pub(crate) fn node<const N: usize>(
    kind: &'static str,
    fields: [(&'static str, serde_json::Value); N],
) -> AgentReviewNode {
    AgentReviewNode {
        kind,
        fields: fields.into_iter().collect(),
    }
}

pub(crate) fn value<T: Serialize + ?Sized>(
    field: &'static str,
    value: &T,
) -> Result<serde_json::Value, AgentReviewSnapshotError> {
    serde_json::to_value(value).map_err(|error| AgentReviewSnapshotError::Serialization {
        field,
        message: error.to_string(),
    })
}

pub(crate) fn rect(rect: Rect) -> serde_json::Value {
    serde_json::json!({
        "x": rect.x,
        "y": rect.y,
        "width": rect.width,
        "height": rect.height,
    })
}

/// Return one exact native path without asking serde to interpret an `OsStr` as UTF-8.
pub(crate) fn path_value(path: &Path) -> serde_json::Value {
    if let Some(raw) = path.to_str() {
        return serde_json::json!(raw);
    }
    non_utf8_os_value(path.as_os_str())
}

pub(crate) fn optional_path_value(path: Option<&PathBuf>) -> serde_json::Value {
    path.map_or(serde_json::Value::Null, |path| path_value(path))
}

pub(crate) fn path_values<'a>(paths: impl IntoIterator<Item = &'a PathBuf>) -> serde_json::Value {
    let mut paths = paths
        .into_iter()
        .map(|path| (native_sort_key(path), path_value(path)))
        .collect::<Vec<_>>();
    paths.sort_by(|left, right| left.0.cmp(&right.0));
    serde_json::Value::Array(paths.into_iter().map(|(_, value)| value).collect())
}

#[cfg(unix)]
fn non_utf8_os_value(value: &OsStr) -> serde_json::Value {
    use std::os::unix::ffi::OsStrExt as _;

    serde_json::json!({
        "unix_bytes": value.as_bytes(),
    })
}

#[cfg(windows)]
fn non_utf8_os_value(value: &OsStr) -> serde_json::Value {
    use std::os::windows::ffi::OsStrExt as _;

    serde_json::json!({
        "windows_wide": value.encode_wide().collect::<Vec<_>>(),
    })
}

#[cfg(unix)]
fn native_sort_key(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt as _;

    path.as_os_str().as_bytes().to_vec()
}

#[cfg(windows)]
fn native_sort_key(path: &Path) -> Vec<u8> {
    use std::os::windows::ffi::OsStrExt as _;

    path.as_os_str()
        .encode_wide()
        .flat_map(u16::to_be_bytes)
        .collect()
}

pub(crate) fn color(color: Color) -> serde_json::Value {
    let name = match color {
        Color::Reset => "reset",
        Color::Black => "black",
        Color::Red => "red",
        Color::Green => "green",
        Color::Yellow => "yellow",
        Color::Blue => "blue",
        Color::Magenta => "magenta",
        Color::Cyan => "cyan",
        Color::Gray => "gray",
        Color::DarkGray => "dark_gray",
        Color::LightRed => "light_red",
        Color::LightGreen => "light_green",
        Color::LightYellow => "light_yellow",
        Color::LightBlue => "light_blue",
        Color::LightMagenta => "light_magenta",
        Color::LightCyan => "light_cyan",
        Color::White => "white",
        Color::Indexed(index) => return serde_json::json!({"indexed": index}),
        Color::Rgb(red, green, blue) => {
            return serde_json::json!({"rgb": [red, green, blue]});
        }
    };
    serde_json::json!(name)
}

pub(crate) fn scroll(scroll: &ScrollableContentState) -> serde_json::Value {
    serde_json::json!({
        "line_count": scroll.line_count(),
        "scroll_offset": scroll.scroll_offset(),
        "focused": scroll.is_focused(),
        "fullscreen": scroll.is_fullscreen(),
        "title": scroll.title(),
    })
}

pub(crate) fn focus<T>(
    field: &'static str,
    focus: &FocusManager<T>,
) -> Result<serde_json::Value, AgentReviewSnapshotError>
where
    T: Clone + Eq + Hash + Serialize,
{
    value(
        field,
        &serde_json::json!({
            "elements": value(field, focus.elements())?,
            "current_index": focus.current_index(),
        }),
    )
}

pub(crate) fn clicks<T>(
    field: &'static str,
    clicks: &ClickRegionRegistry<T>,
) -> Result<serde_json::Value, AgentReviewSnapshotError>
where
    T: Clone + Serialize,
{
    clicks
        .regions()
        .iter()
        .map(|region| {
            Ok(serde_json::json!({
                "area": rect(region.area),
                "data": value(field, &region.data)?,
            }))
        })
        .collect::<Result<Vec<_>, AgentReviewSnapshotError>>()
        .map(serde_json::Value::Array)
}

pub(crate) fn list_picker(state: &ListPickerState) -> serde_json::Value {
    serde_json::json!({
        "selected_index": state.selected_index,
        "scroll": state.scroll,
        "total_items": state.total_items,
    })
}

pub(crate) fn select(state: &SelectState) -> serde_json::Value {
    serde_json::json!({
        "selected_index": state.selected_index,
        "is_open": state.is_open,
        "focused": state.focused,
        "enabled": state.enabled,
        "highlighted_index": state.highlighted_index,
        "scroll_offset": state.scroll_offset,
        "total_options": state.total_options,
    })
}

pub(crate) fn button(state: &ButtonState) -> serde_json::Value {
    serde_json::json!({
        "focused": state.focused,
        "pressed": state.pressed,
        "enabled": state.enabled,
        "toggled": state.toggled,
    })
}

pub(crate) fn checkbox(state: &CheckBoxState) -> serde_json::Value {
    serde_json::json!({
        "checked": state.checked,
        "focused": state.focused,
        "enabled": state.enabled,
    })
}

pub(crate) fn textarea(state: &RichTextArea<'_>) -> serde_json::Value {
    serde_json::json!({
        "kind": "opaque_debug",
        "crate": RATATUI_TEXTAREA_CRATE,
        "version": RATATUI_TEXTAREA_VERSION,
        "debug": format!("{state:?}"),
    })
}

pub(crate) const fn local_key(key: LocalKey) -> &'static str {
    match key {
        LocalKey::Enter => "enter",
        LocalKey::Escape => "escape",
        LocalKey::Space => "space",
        LocalKey::Character(_) => "character",
        LocalKey::Control(_) => "control",
        LocalKey::Tab => "tab",
        LocalKey::BackTab => "back_tab",
        LocalKey::NextField => "next_field",
        LocalKey::PreviousField => "previous_field",
    }
}

pub(crate) fn local_key_value(key: LocalKey) -> serde_json::Value {
    match key {
        LocalKey::Character(character) | LocalKey::Control(character) => {
            serde_json::json!({"kind": local_key(key), "character": character})
        }
        LocalKey::Enter
        | LocalKey::Escape
        | LocalKey::Space
        | LocalKey::Tab
        | LocalKey::BackTab
        | LocalKey::NextField
        | LocalKey::PreviousField => serde_json::json!({"kind": local_key(key)}),
    }
}

pub(crate) fn local_action_inventory(
    inventory: &LocalActionInventory,
) -> Result<AgentReviewNode, AgentReviewSnapshotError> {
    let LocalActionInventory { actions } = inventory;
    let actions = actions
        .iter()
        .map(|advertised| {
            let LocalAdvertisedAction {
                target,
                keys,
                hit,
                outcome,
            } = advertised;
            let target_value = match target {
                LocalActionTarget::Add(target) => serde_json::json!({
                    "add": value("local_action.target.add", target)?,
                }),
                LocalActionTarget::Health(action) => serde_json::json!({
                    "health": value("local_action.target.health", action)?,
                }),
                LocalActionTarget::Runners(action) => serde_json::json!({
                    "runners": value("local_action.target.runners", action)?,
                }),
                LocalActionTarget::RunnerEditor(action) => serde_json::json!({
                    "runner_editor": value("local_action.target.runner_editor", action)?,
                }),
            };
            let outcome = match outcome {
                LocalActionOutcome::Action(action) => {
                    serde_json::json!({"action": action_snapshot(action)?})
                }
                LocalActionOutcome::Consumed => serde_json::json!("consumed"),
            };
            let keys = keys
                .iter()
                .map(|binding| value("local_action.key", &binding.event()))
                .collect::<Result<Vec<_>, AgentReviewSnapshotError>>()?;
            Ok(serde_json::json!({
                "target": target_value,
                "keys": keys,
                "hit": hit.map(rect),
                "outcome": outcome,
            }))
        })
        .collect::<Result<Vec<_>, AgentReviewSnapshotError>>()?;
    Ok(node(
        "local_actions",
        [("actions", serde_json::json!(actions))],
    ))
}

fn action_snapshot(
    action: &skit_ui::Action,
) -> Result<serde_json::Value, AgentReviewSnapshotError> {
    match action {
        skit_ui::Action::Add(action) => {
            Ok(serde_json::json!({"add": add_action_snapshot(action)?}))
        }
        complete => value("local_action.outcome.action", complete),
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use ratatui_core::{layout::Rect, style::Color};
    use ratatui_interact::{
        components::{ButtonState, CheckBoxState, ListPickerState, SelectState},
        traits::ClickRegionRegistry,
    };
    use serde::ser::Error as _;
    use skit_ui::{Action, AddAction, HealthAction, RunnerEditorAction, RunnerManagerAction};

    use crate::TuiSession;
    use crate::local_action::LocalKey;
    use crate::{
        AddControlId, LocalActionInventory, LocalActionOutcome, LocalActionTarget,
        LocalAdvertisedAction,
    };

    use super::{RATATUI_TEXTAREA_CRATE, RATATUI_TEXTAREA_VERSION};

    #[test]
    fn utf8_paths_keep_exact_native_spelling() {
        let value = super::path_value(Path::new("alpha/../beta"));
        assert_eq!(value, "alpha/../beta");
    }

    #[test]
    fn utf8_path_snapshot_keeps_exact_separator_dot_and_trailing_spelling() {
        let spelled = super::path_value(Path::new("a//./b/"));
        let normalized = super::path_value(Path::new("a/b"));
        assert_eq!(spelled, "a//./b/");
        assert_eq!(normalized, "a/b");
        assert_ne!(spelled, normalized);
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_unix_paths_keep_exact_native_bytes() {
        use std::{ffi::OsStr, os::unix::ffi::OsStrExt};

        let path = Path::new(OsStr::from_bytes(b"bad-\xff-path"));
        let value = super::path_value(path);
        assert_eq!(
            value["unix_bytes"],
            serde_json::json!([98, 97, 100, 45, 255, 45, 112, 97, 116, 104])
        );
    }

    #[cfg(windows)]
    #[test]
    fn non_utf8_windows_paths_keep_exact_native_wide_units() {
        use std::{ffi::OsString, os::windows::ffi::OsStringExt as _};

        let path = PathBuf::from(OsString::from_wide(&[b'x' as u16, 0xD800]));
        let value = super::path_value(&path);
        assert_eq!(
            value["windows_wide"],
            serde_json::json!([b'x' as u16, 0xD800])
        );
    }

    #[test]
    fn path_collections_and_optional_paths_are_stable_and_total() {
        let first = PathBuf::from("zeta");
        let second = PathBuf::from("alpha");
        let values = super::path_values([&first, &second]);
        assert_eq!(values[0], "alpha");
        assert_eq!(super::optional_path_value(Some(&first)), "zeta");
        assert_eq!(super::optional_path_value(None), serde_json::Value::Null);
    }

    #[test]
    fn fallible_serde_and_click_paths_return_typed_results() {
        struct Refuses;

        impl serde::Serialize for Refuses {
            fn serialize<S>(&self, _serializer: S) -> Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
            {
                Err(S::Error::custom("refused"))
            }
        }

        assert!(matches!(
            super::value("refuses", &Refuses),
            Err(super::AgentReviewSnapshotError::Serialization {
                field: "refuses",
                ..
            })
        ));
        let mut clicks = ClickRegionRegistry::new();
        clicks.register(Rect::new(1, 2, 3, 4), "target".to_owned());
        let clicks = super::clicks("clicks", &clicks).unwrap();
        assert_eq!(clicks[0]["data"], "target");
    }

    #[test]
    fn primitive_widget_mappers_cover_nondefault_state() {
        let mut list = ListPickerState::new(3);
        list.select(2);
        list.scroll = 1;
        assert_eq!(super::list_picker(&list)["selected_index"], 2);

        let mut select = SelectState::with_selected(3, 1);
        select.open();
        select.focused = true;
        select.scroll_offset = 1;
        assert_eq!(super::select(&select)["is_open"], true);

        let mut button = ButtonState::toggled(true);
        button.set_focused(true);
        button.set_pressed(true);
        assert_eq!(super::button(&button)["pressed"], true);

        let mut checkbox = CheckBoxState::new(true);
        checkbox.set_focused(true);
        assert_eq!(super::checkbox(&checkbox)["checked"], true);
    }

    #[test]
    fn color_mapper_covers_every_color_shape() {
        let colors = [
            Color::Reset,
            Color::Black,
            Color::Red,
            Color::Green,
            Color::Yellow,
            Color::Blue,
            Color::Magenta,
            Color::Cyan,
            Color::Gray,
            Color::DarkGray,
            Color::LightRed,
            Color::LightGreen,
            Color::LightYellow,
            Color::LightBlue,
            Color::LightMagenta,
            Color::LightCyan,
            Color::White,
            Color::Indexed(7),
            Color::Rgb(1, 2, 3),
        ];
        let values = colors.map(super::color);
        assert_eq!(values[0], "reset");
        assert_eq!(values[17]["indexed"], 7);
        assert_eq!(values[18]["rgb"], serde_json::json!([1, 2, 3]));
    }

    #[test]
    fn local_action_mapper_covers_every_owned_key_target_and_outcome() {
        let keys = [
            LocalKey::Enter,
            LocalKey::Escape,
            LocalKey::Space,
            LocalKey::Character('x'),
            LocalKey::Control('x'),
            LocalKey::Tab,
            LocalKey::BackTab,
            LocalKey::NextField,
            LocalKey::PreviousField,
        ];
        let targets = [
            LocalActionTarget::Add(AddControlId::Cancel),
            LocalActionTarget::Health(HealthAction::Back),
            LocalActionTarget::Runners(RunnerManagerAction::Previous),
            LocalActionTarget::RunnerEditor(RunnerEditorAction::Cancel),
        ];
        let mut actions = keys
            .into_iter()
            .enumerate()
            .map(|(index, key)| LocalAdvertisedAction {
                target: targets[index % targets.len()].clone(),
                keys: key.bindings(),
                hit: Some(Rect::new(index as u16, 0, 1, 1)),
                outcome: LocalActionOutcome::Consumed,
            })
            .collect::<Vec<_>>();
        actions.push(LocalAdvertisedAction {
            target: LocalActionTarget::Add(AddControlId::Save),
            keys: Vec::new(),
            hit: None,
            outcome: LocalActionOutcome::Action(Action::Add(AddAction::Cancel)),
        });
        actions.push(LocalAdvertisedAction {
            target: LocalActionTarget::Health(HealthAction::Back),
            keys: Vec::new(),
            hit: None,
            outcome: LocalActionOutcome::Action(Action::Health(HealthAction::Back)),
        });
        actions.push(LocalAdvertisedAction {
            target: LocalActionTarget::Runners(RunnerManagerAction::Previous),
            keys: Vec::new(),
            hit: None,
            outcome: LocalActionOutcome::Action(Action::Runners(RunnerManagerAction::Previous)),
        });
        actions.push(LocalAdvertisedAction {
            target: LocalActionTarget::RunnerEditor(RunnerEditorAction::Cancel),
            keys: Vec::new(),
            hit: None,
            outcome: LocalActionOutcome::Action(Action::RunnerEditor(RunnerEditorAction::Cancel)),
        });
        let snapshot = super::local_action_inventory(&LocalActionInventory { actions }).unwrap();
        let json = serde_json::to_string(&snapshot).unwrap();
        for expected in [
            "add",
            "health",
            "runners",
            "runner_editor",
            "consumed",
            "action",
        ] {
            assert!(json.contains(expected), "missing {expected}");
        }
        for key in keys {
            let _ = super::local_key_value(key);
        }

        let cross_domain = LocalActionInventory {
            actions: vec![LocalAdvertisedAction {
                target: LocalActionTarget::Add(AddControlId::Cancel),
                keys: Vec::new(),
                hit: None,
                outcome: LocalActionOutcome::Action(Action::OpenAddRunnerEditor),
            }],
        };
        let cross_domain = super::local_action_inventory(&cross_domain).unwrap();
        assert!(
            serde_json::to_string(&cross_domain)
                .unwrap()
                .contains("open_add_runner_editor")
        );
    }

    #[test]
    fn default_session_has_a_canonical_agent_review_snapshot() {
        let first =
            serde_json::to_vec(&TuiSession::default().agent_review_snapshot().unwrap()).unwrap();
        let second =
            serde_json::to_vec(&TuiSession::default().agent_review_snapshot().unwrap()).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn canonical_snapshot_names_every_top_level_session_field() {
        let value =
            serde_json::to_value(TuiSession::default().agent_review_snapshot().unwrap()).unwrap();
        let mut keys = value
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        keys.sort();
        assert_eq!(
            keys,
            [
                "add",
                "add_geometry",
                "add_overlay",
                "clicks",
                "confirm_remove",
                "file_picker_source",
                "footer",
                "form",
                "health",
                "help",
                "library",
                "local_actions",
                "path_suggestions",
                "preferences",
                "quit_armed_at",
                "quit_toast",
                "report",
                "run",
                "run_modal",
                "runner_editor",
                "runners",
                "schema_version",
                "search",
                "settings",
                "settings_geometry",
                "settings_prompt_overlay",
                "top_level_click",
            ]
        );
    }

    #[test]
    fn opaque_textarea_contract_pins_the_locked_dependency() {
        assert_eq!(RATATUI_TEXTAREA_CRATE, "ratatui-textarea");
        assert_eq!(RATATUI_TEXTAREA_VERSION, "0.9.2");
        assert!(
            include_str!("../../../Cargo.lock")
                .contains("name = \"ratatui-textarea\"\nversion = \"0.9.2\"")
        );
    }
}
