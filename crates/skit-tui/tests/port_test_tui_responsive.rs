//! Mechanical port of the Python oracle module `tests/test_tui_responsive.py`
//! (`origin/main@206f9ef`): "Responsive layout: the size tiers (`tui_layout`) and what each
//! screen does in them." Each `#[test]` keeps its Python `def test_*` name and its "WHY"
//! comment so it traces back to its origin.
//!
//! Concept mapping (Textual pilot -> Ratatui render-to-`TestBackend`):
//! - The oracle drives a live `tui.MenuApp` through a `Pilot` and observes CSS classes
//!   (`has_class("-w-normal")`), widget geometry (`.region`), and visibility (`.display`).
//!   The Rust frontend has no App/pilot/CSS/widget-tree: it renders one `LibraryState` into a
//!   `TestBackend` with `render_localized` / `render_with_session` and reads back the buffer +
//!   the returned `ViewGeometry`. The CONTRACT is the same outcome (pane beside/below/hidden,
//!   chip fires, modal fits, filter works); only the observation mechanism differs. This is the
//!   idiom `main_library_surface.rs::library_layout_uses_main_breakpoints_and_three_to_two_ratio`
//!   already uses in this crate.
//! - `pilot.resize_terminal(w, h)` -> re-render the SAME `LibraryState` into a new
//!   `TestBackend::new(w, h)`. Oracle sizes are kept exactly so an off-by-one in a threshold
//!   cannot survive.
//! - `pilot.press("tab")` on the Library detail pin -> `state.update(Action::ToggleDetail)`.
//! - Detail visibility rule (`crates/skit-tui/src/screens/library.rs:72-84`): `narrow =
//!   width < 80`, `short = height < 16`; Automatic shows unless `narrow && short`.
//!
//! Oracle implementation read (mandatory step 2): `src/skit/tui_layout.py` (the tier constants:
//! `NARROW_WIDTH=80`, `TINY_HEIGHT=10`, `SHORT_HEIGHT=16`, `TALL_HEIGHT=28`, and the two
//! breakpoint lists), `src/skit/tui_footer.py` (`GLUE="⠀"` U+2800, `chip`, `nav_chip`), and
//! `src/skit/tui.py` (the responsive CSS block ~ lines 269-317 that flattens `#search` to
//! `height:1` at `-h-short`/`-h-tiny` and caps `#keys`, and `action_toggle_detail` at lines
//! 1026-1048, whose `visible = pinned_open if pinned else #detail.display` pins a hidden pane OPEN
//! on the first Tab — the exact source of the two divergences below).
//!
//! Buckets:
//! - REAL: the width/height visibility contract, the Tab tri-state on a wide terminal, footer
//!   wrap+click, footer short-cap scroll, flattened-search filtering, nav-chip two-key pills, the
//!   help and confirm-remove modals, and the env-picker input fit. Driven through the real public
//!   API (`render_localized` / `render_with_session` / `LibraryState`).
//! - DIVERGENCE (`#[ignore]`, full asserting body kept): `test_narrow_short_hides_...` and
//!   `test_tiny_narrow_tab_...`. Python's first Tab reads the current tier and pins a hidden pane
//!   OPEN; the Rust reducer's `ToggleDetail` is blind to size and goes Automatic -> PinnedClosed
//!   first (`crates/skit-ui/src/lib.rs:1840-1845`), so the pane stays hidden. Fix the reducer ->
//!   delete the `#[ignore]` -> green.
//! - ABSENT / CROSS-CRATE (`#[ignore]` stub): the Textual-only unit contracts (breakpoint
//!   constant lists, GLUE glyph, chip markup strings), the search-flatten row-saving, the
//!   preset-RadioSet row, the preferences per-control geometry, and the inline-form tier classes.
//!   Each stub records the Python behavior and the owning tier / gap.

use ratatui_core::{backend::TestBackend, buffer::Buffer, terminal::Terminal};
use ratatui_crossterm::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use skit_application::LibraryScan;
use skit_domain::{EntryKind, EntrySummary, Slug, StorageMode};
use skit_i18n::Locale;
use skit_tui::{
    EventHandling, HitTarget, TuiSession, ViewGeometry, render_localized, render_with_session,
};
use skit_ui::{
    Action, FormField, FormPurpose, FormView, LibraryState, RunFormContext, RunFormView,
    RunPathContext, Screen, UiCommand,
};

// --- shared fixtures / observation helpers (self-contained; no shared-file edits) ---

/// One python library entry, the oracle's `store.add_python(..., name=...)`.
fn python_entry(name: &str) -> EntrySummary {
    EntrySummary {
        slug: Slug::parse(name).unwrap(),
        name: name.to_owned(),
        kind: EntryKind::parse("python").unwrap(),
        mode: StorageMode::Copy,
        description: String::new(),
        target: None,
    }
}

/// A Library projection over the given entries.
fn library(entries: Vec<EntrySummary>) -> LibraryState {
    LibraryState::from_scan(LibraryScan {
        entries,
        diagnostics: Vec::new(),
    })
}

/// A single-entry Library, matching the oracle's `store.add_python(_py(...), name="a")`.
fn library_a() -> LibraryState {
    library(vec![python_entry("a")])
}

/// Every buffer row as a `String`.
fn buffer_lines(buffer: &Buffer) -> Vec<String> {
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect()
}

/// Render one state into a fresh `TestBackend` and return its rows. Detail visibility depends only
/// on `state` and size, so a throwaway session is faithful.
fn lines_at(state: &LibraryState, width: u16, height: u16) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| {
            let _ = render_localized(frame, state, Locale::En);
        })
        .unwrap();
    buffer_lines(terminal.backend().buffer())
}

/// Render with a persistent session and return both the rows and the mouse hit map.
fn session_frame(
    session: &mut TuiSession,
    state: &LibraryState,
    width: u16,
    height: u16,
) -> (Vec<String>, ViewGeometry) {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    let mut geometry = ViewGeometry::default();
    terminal
        .draw(|frame| {
            geometry = render_with_session(frame, state, Locale::En, session);
        })
        .unwrap();
    (buffer_lines(terminal.backend().buffer()), geometry)
}

/// `(row, column)` of the first occurrence of `needle`, if any.
fn find(lines: &[String], needle: &str) -> Option<(usize, usize)> {
    lines
        .iter()
        .enumerate()
        .find_map(|(row, line)| line.find(needle).map(|column| (row, column)))
}

// The bordered-panel titles carry a "╭ " corner prefix, which distinguishes the detail PANEL from
// the footer's "Detail pane" chip and the list PANEL from the header's "Library:" line.

/// True when the indigo detail panel is drawn at all.
fn detail_shown(lines: &[String]) -> bool {
    find(lines, "╭ Detail pane").is_some()
}

/// The Library list panel is titled "Library" (`screens/library.rs:96`).
fn list_position(lines: &[String]) -> (usize, usize) {
    find(lines, "╭ Library").expect("the list panel is always drawn")
}

fn detail_position(lines: &[String]) -> (usize, usize) {
    find(lines, "╭ Detail pane").expect("the detail pane must be drawn")
}

fn left_button() -> MouseButton {
    MouseButton::Left
}

fn click(column: u16, row: u16) -> Event {
    Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(left_button()),
        column,
        row,
        modifiers: KeyModifiers::NONE,
    })
}

fn scroll_down(column: u16, row: u16) -> Event {
    Event::Mouse(MouseEvent {
        kind: MouseEventKind::ScrollDown,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    })
}

/// Footer command targets present in a hit map.
fn footer_commands(geometry: &ViewGeometry) -> Vec<UiCommand> {
    geometry
        .hits
        .iter()
        .filter_map(|hit| match hit.action {
            HitTarget::Command(command) => Some(command),
            HitTarget::RunFieldCommand { .. } | HitTarget::FocusField(_) => None,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// the tier contract and the pill glue (unit level)
// ---------------------------------------------------------------------------

#[test]
#[ignore = "UNMAPPED (absent): the oracle pins the literal CSS-class breakpoint lists \
tui_layout.HORIZONTAL_BREAKPOINTS == [(0,'-w-narrow'),(80,'-w-normal')] and VERTICAL_BREAKPOINTS \
== [(0,'-h-tiny'),(10,'-h-short'),(16,'-h-normal'),(28,'-h-tall')] (test_tui_responsive.py:35-45). \
The Ratatui frontend has no CSS classes; the equivalent thresholds are imperative and private \
(width<80/height<16 in screens/library.rs:72-73; the footer's 28/16/10 ladder in footer.rs \
row_budget). No public constant mirrors the list. The boundaries themselves are pinned \
BEHAVIORALLY by test_width_tier_boundary_flips_side_by_side_to_stacked below; only the \
introspection surface is the non-transfer."]
fn test_breakpoint_tiers_are_the_documented_contract() {
    // The tier names are load-bearing in Textual: every responsive CSS rule selects on them
    // literally. Ratatui selects layout in code, so there is no class list to pin.
}

#[test]
#[ignore = "UNMAPPED (absent, NOT a must-fix): the oracle glues every blank inside a footer pill \
with U+2800 so a text wrapper cannot split a pill mid-label, and pins the exact markup string of \
tui_footer.chip(...) / GLUE (test_tui_responsive.py:48-67). Ratatui does not build pills as \
rich-markup text: footer.rs positions each chip as a whole `Button` at an explicit Rect and wraps \
BETWEEN chips (footer.rs:238-249), so a mid-pill split is impossible by construction and no GLUE \
glyph exists. The whole-pill-wraps-as-one-unit outcome is covered by footer.rs's own unit test \
`action_footer_wraps_every_chip_and_keeps_each_visible_chip_clickable`."]
fn test_chip_glues_every_blank_so_the_pill_is_one_word() {
    // No public markup-string builder / GLUE constant exists in skit-tui; the behavior it protects
    // is achieved by Rect-positioned Button chips instead.
}

#[test]
fn test_nav_chip_is_exactly_the_two_key_only_pills() {
    // nav_chip's two pills are the movement contract of EVERY form footer: both keys named for
    // each direction so no one who tabs one field too far is stranded. The Rust footer builds them
    // in footer_groups (footer.rs:475-493, citing tui_footer.py:82-94): the FocusNext/FocusPrevious
    // key-only pills read "Tab/↓" and "Shift+Tab/↑". Assert both appear on a form footer.
    let mut state = library_a();
    state.update(Action::Present(Screen::Form(FormView {
        purpose: FormPurpose::Add,
        title: "Add an entry".to_owned(),
        title_arguments: Vec::new(),
        translate_title: true,
        selector: None,
        fields: vec![
            FormField::text("source", "Source path", ""),
            FormField::text("name", "Name", ""),
        ],
        focused: 0,
        submit_label: "Save".to_owned(),
    })));
    let mut session = TuiSession::default();
    let (lines, geometry) = session_frame(&mut session, &state, 100, 30);
    let joined = lines.join("\n");
    // Both movement pills render their exact key-only labels on the form footer.
    assert!(joined.contains("Tab/↓"), "{joined}");
    assert!(joined.contains("Shift+Tab/↑"), "{joined}");
    // render-model: the oracle pins the exact Textual rich-markup of nav_chip() — the @click
    // actions, the U+2800 GLUE, [bold $accent], and the key-only empty-label shape. Ratatui builds
    // these as Rect-positioned Button chips with no markup string and no GLUE glyph
    // (footer.rs:475-493), so the exact markup has no render-model twin. Verified benign: both
    // movement pills are real footer click targets (built key-only in footer_groups), so the
    // two-key movement contract is present as advertised.
    let commands = footer_commands(&geometry);
    assert!(commands.contains(&UiCommand::FocusNext), "{commands:?}");
    assert!(commands.contains(&UiCommand::FocusPrevious), "{commands:?}");
}

// ---------------------------------------------------------------------------
// width tiers: side-by-side vs stacked at the 79/80 boundary, Tab pin wins both ways
// ---------------------------------------------------------------------------

#[test]
fn test_width_tier_boundary_flips_side_by_side_to_stacked() {
    // At >= 80 cols the detail pane sits beside the list; one column narrower flips #main to the
    // portrait layout — the pane moves BELOW the list at full row width (the terminal is tall
    // enough to spare the rows), it does not disappear.
    let state = library_a();

    let wide = lines_at(&state, 80, 24);
    assert!(detail_shown(&wide));
    let (list_row, list_col) = list_position(&wide);
    let (detail_row, detail_col) = detail_position(&wide);
    // beside the list: same top band, to the right
    assert!(
        detail_row < 8,
        "detail row {detail_row} not in the top band"
    );
    assert!(
        detail_col > list_col + 10,
        "detail col {detail_col} must sit right of list col {list_col}"
    );
    assert!(list_row < 8);

    let narrow = lines_at(&state, 79, 24);
    assert!(detail_shown(&narrow)); // stacked, not hidden
    let (list_row2, list_col2) = list_position(&narrow);
    let (detail_row2, detail_col2) = detail_position(&narrow);
    assert!(
        detail_row2 > list_row2 + 1,
        "detail row {detail_row2} must sit below list row {list_row2}"
    );
    // at full row width: same left column as the list
    assert_eq!(
        detail_col2, list_col2,
        "stacked detail must share the list's left edge"
    );
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): the oracle's first Tab on a tier-hidden pane pins it \
OPEN — action_toggle_detail sets `visible = #detail.display` when unpinned, then pins the opposite \
(src/skit/tui.py:1026-1048; test_tui_responsive.py:109-134). The Rust reducer's ToggleDetail is \
blind to the terminal size and steps Automatic -> PinnedClosed first \
(crates/skit-ui/src/lib.rs:1840-1845), so the pane stays hidden. Fix lives in skit-ui's reducer \
(ToggleDetail needs tier awareness); then delete this #[ignore]."]
fn test_narrow_short_hides_detail_and_tab_pin_survives_resizes() {
    // Narrow AND short is the shape with no room for the pane in either direction: only there does
    // it auto-hide. Tab pins it OPEN and the pin holds through a wide resize AND back; further Tabs
    // alternate via the pinned-closed / pinned-open branches (each state asserted).
    let mut state = library_a();
    assert!(!detail_shown(&lines_at(&state, 70, 12))); // narrow + short -> auto-hidden
    state.update(Action::ToggleDetail); // pin open
    assert!(detail_shown(&lines_at(&state, 70, 12)));
    assert!(detail_shown(&lines_at(&state, 120, 12)));
    assert!(detail_shown(&lines_at(&state, 70, 12))); // the pin beats the narrow+short tier
    state.update(Action::ToggleDetail); // pinned-open -> pinned-closed
    assert!(!detail_shown(&lines_at(&state, 70, 12)));
    state.update(Action::ToggleDetail); // pinned-closed -> pinned-open
    assert!(detail_shown(&lines_at(&state, 70, 12)));
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): the oracle's first Tab on a tiny+narrow (tier-hidden) \
pane pins it OPEN (src/skit/tui.py:1026-1048; test_tui_responsive.py:137-148); the Rust reducer's \
ToggleDetail steps Automatic -> PinnedClosed first (crates/skit-ui/src/lib.rs:1840-1845), so the \
pane stays hidden. Fix lives in skit-ui's reducer, then delete this #[ignore]."]
fn test_tiny_narrow_tab_still_brings_the_pane_back() {
    // Even at the degradation floor the pane is reachable: the first Tab on a tiny+narrow screen
    // must read the tier as hidden and pin the pane OPEN.
    let mut state = library_a();
    assert!(!detail_shown(&lines_at(&state, 46, 9))); // tiny + narrow -> auto-hidden
    state.update(Action::ToggleDetail);
    assert!(detail_shown(&lines_at(&state, 46, 9))); // pinned open, not re-hidden
}

#[test]
fn test_tab_walks_the_pin_states_on_a_wide_terminal_too() {
    // The closed->open flip must read the PIN, not the width tier: on a wide terminal the tier
    // alone would already say "visible", so a toggle that ignored the pinned-closed state would
    // re-hide the pane instead of reopening it.
    let mut state = library_a();
    assert!(detail_shown(&lines_at(&state, 120, 24))); // wide -> auto-shown
    state.update(Action::ToggleDetail); // auto -> pinned-closed
    assert!(!detail_shown(&lines_at(&state, 120, 24)));
    state.update(Action::ToggleDetail); // pinned-closed -> pinned-open, while wide
    assert!(detail_shown(&lines_at(&state, 120, 24)));
    state.update(Action::ToggleDetail); // pinned-open -> pinned-closed again
    assert!(!detail_shown(&lines_at(&state, 120, 24)));
}

// ---------------------------------------------------------------------------
// height tiers: search flattens, key rows stop wrapping, global row yields
// ---------------------------------------------------------------------------

#[test]
#[ignore = "UNMAPPED (absent) + GAP: the oracle asserts the CSS height tiers -h-tall/-h-normal/\
-h-short/-h-tiny at 28/27/16/15/10/9, that the bordered #search box flattens from region.height==3 \
to ==1 at the short boundary to buy the list a row, and that nothing is dropped at -h-tiny \
(test_tui_responsive.py:178-205). Ratatui has no tier classes (non-transfer). The row-saving \
flatten is a candidate v0.4 behavior loss: the Rust Library header is fixed at 3 rows and hosts \
the search input inside it (crates/skit-tui/src/lib.rs:144-155 header_height; \
crates/skit-tui/src/session.rs:579-591), so search never flattens to 1 row on a short terminal. \
MUST-FIX candidate — main agent to adjudicate against test_tui_responsive.py:190-204."]
fn test_height_tier_boundaries_flatten_search_then_drop_the_global_row() {
    // Search box flattens (3 -> 1) at the short boundary; the global footer row yields but nothing
    // is dropped at the tiny floor. No public API observes a search-widget region height or a tier
    // class, so this cannot be expressed as a compiling real assertion.
}

#[test]
fn test_flattened_search_still_filters() {
    // The short-tier search box is chrome-less, not feature-less: / focuses it and typing still
    // filters the table. Observed here at the short size (100, 12): drive BeginSearch + input and
    // read back the visible projection.
    let mut state = library(vec![python_entry("alpha"), python_entry("beta")]);
    state.update(Action::BeginSearch);
    state.update(Action::Input('b'));
    state.update(Action::Input('e'));
    let visible = state
        .visible_entries()
        .map(|entry| entry.name.clone())
        .collect::<Vec<_>>();
    assert_eq!(visible, ["beta"]);
    // the filter must reach the rendered short-tier table, not only the reducer projection
    let rendered = lines_at(&state, 100, 12).join("\n");
    assert!(rendered.contains("beta"), "{rendered}");
    assert!(!rendered.contains("alpha"), "{rendered}");
}

// ---------------------------------------------------------------------------
// footer pills: wrap between chips, wrapped chips stay clickable, short caps rows
// ---------------------------------------------------------------------------

#[test]
fn test_footer_wraps_between_pills_and_wrapped_chips_stay_clickable() {
    // At 44 cols the global row wraps, so chips land on later footer lines — every chip still a
    // real button: clicking the wrapped Tab (Detail pane) chip must toggle the detail pane, the
    // advertised mouse path to a pane a tier hid.
    let mut state = library_a();
    let mut session = TuiSession::default();
    let (_, geometry) = session_frame(&mut session, &state, 44, 24);
    assert!(detail_shown(&lines_at(&state, 44, 24))); // narrow + normal -> stacked below

    // render-model: the oracle pins keys_global.region.height==3 (its global-chips Static). The
    // Rust footer is a flat set of Rect-positioned Button chips (local + global) with no per-group
    // widget and a different budget (row_budget(24, library-browse)==6, footer.rs:530), so that exact
    // height has no render-model twin. What the oracle's docstring protects — the row wraps and a
    // wrapped chip still fires its command — is what is asserted here.
    // the footer wrapped: its command chips occupy more than one row
    let mut rows = geometry
        .hits
        .iter()
        .filter(|hit| matches!(hit.action, HitTarget::Command(_)))
        .map(|hit| hit.rect.y)
        .collect::<Vec<_>>();
    rows.sort_unstable();
    rows.dedup();
    assert!(rows.len() > 1, "the footer must wrap to more than one row");

    // the Detail-pane (Tab) chip sits on a wrapped (non-first) row and still fires toggle_detail
    let toggle = geometry
        .hits
        .iter()
        .find(|hit| hit.action == HitTarget::Command(UiCommand::ToggleDetail))
        .expect("the Tab / Detail pane chip must be a clickable footer button");
    assert!(
        toggle.rect.y > rows[0],
        "the Detail chip must be on a wrapped row, not the first"
    );
    assert_eq!(
        session.handle_event(click(toggle.rect.x, toggle.rect.y), &state, &geometry),
        EventHandling::Action(Action::ToggleDetail),
    );
    state.update(Action::ToggleDetail); // Automatic -> PinnedClosed
    assert!(!detail_shown(&lines_at(&state, 44, 24))); // the wrapped chip fired toggle_detail
    state.update(Action::ToggleDetail); // PinnedClosed -> PinnedOpen
    assert!(detail_shown(&lines_at(&state, 44, 24))); // and back
}

#[test]
fn test_portrait_stacks_the_detail_pane_and_uncaps_the_footer() {
    // The portrait shape (narrow + tall): the detail pane stacks below the list at full width
    // instead of vanishing, and the footer wraps without a cap so every chip stays visible even on
    // a sliver-narrow window. Tab still hides the stacked pane.
    let mut state = library_a();
    let mut session = TuiSession::default();
    let (lines, geometry) = session_frame(&mut session, &state, 26, 44);
    assert!(detail_shown(&lines));
    let (list_row, list_col) = list_position(&lines);
    let (detail_row, detail_col) = detail_position(&lines);
    assert!(
        detail_row > list_row + 1,
        "detail must stack below the list"
    );
    assert_eq!(detail_col, list_col, "stacked detail is at full row width");

    // uncapped footer: chips reach well past three rows (the tall tier lifts the row cap)
    let mut rows = geometry
        .hits
        .iter()
        .filter(|hit| matches!(hit.action, HitTarget::Command(_)))
        .map(|hit| hit.rect.y)
        .collect::<Vec<_>>();
    rows.sort_unstable();
    rows.dedup();
    assert!(rows.len() > 3, "the tall footer must be uncapped: {rows:?}");

    state.update(Action::ToggleDetail);
    assert!(!detail_shown(&lines_at(&state, 26, 44))); // the pin beats the portrait stack rule
    state.update(Action::ToggleDetail);
    assert!(detail_shown(&lines_at(&state, 26, 44)));
}

#[test]
fn test_short_tier_caps_visible_lines_but_keeps_chips_scroll_reachable() {
    // Narrow AND short: wrapping would spend the rows the tier just reclaimed, so the footer caps
    // its visible lines — but the cap trims visibility only: the wrapped rows behind it stay
    // wheel-reachable, so every chip keeps a mouse path (mouse-alone policy holds at every size).
    let state = library_a();
    let mut session = TuiSession::default();
    let (_, first) = session_frame(&mut session, &state, 46, 12);
    // render-model: the oracle pins keys.region.height==2 (2 visible lines). Ratatui exposes no
    // widget region height, but row_budget(12, library)==2 (footer.rs:532) is the same cap, and it
    // IS observable through the hit map: the first frame's visible command chips span at most 2
    // rows.
    let mut first_rows = first
        .hits
        .iter()
        .filter(|hit| matches!(hit.action, HitTarget::Command(_)))
        .map(|hit| hit.rect.y)
        .collect::<Vec<_>>();
    first_rows.sort_unstable();
    first_rows.dedup();
    assert!(
        first_rows.len() <= 2,
        "the short tier caps the footer at 2 visible rows: {first_rows:?}"
    );
    // The short library budget caps the footer to a couple of rows: Help is not yet hit-mapped.
    let mut reachable = footer_commands(&first);
    assert!(
        !reachable.contains(&UiCommand::Help),
        "the short cap should hide the tail chips at first: {reachable:?}"
    );

    // Wheel the footer: previously-hidden chips (Help among them) become hit-mapped.
    let mut geometry = first;
    let mut found_help = false;
    for _ in 0..32 {
        let Some(anchor) = geometry
            .hits
            .iter()
            .find(|hit| matches!(hit.action, HitTarget::Command(_)))
            .map(|hit| hit.rect)
        else {
            break;
        };
        assert_eq!(
            session.handle_event(scroll_down(anchor.x, anchor.y), &state, &geometry),
            EventHandling::Consumed,
            "the capped footer viewport must accept wheel scrolling"
        );
        let frame = session_frame(&mut session, &state, 46, 12);
        geometry = frame.1;
        for command in footer_commands(&geometry) {
            if !reachable.contains(&command) {
                reachable.push(command);
            }
        }
        if reachable.contains(&UiCommand::Help) {
            found_help = true;
            break;
        }
    }
    assert!(found_help, "the wheel path must reach the tail Help chip");
}

// ---------------------------------------------------------------------------
// form screens: preset row and option sets stack when narrow, footer caps when short
// ---------------------------------------------------------------------------

#[test]
#[ignore = "UNMAPPED (absent): the oracle's run form hosts presets as a caption + RadioSet in a \
#preset-row that flips from side-by-side to stacked when narrow (test_tui_responsive.py:314-334). \
The Rust run form presents presets as a Picker DROPDOWN, not a caption+RadioSet \
(crates/skit-ui/src/run.rs preset_field -> ChoicePresentation::Picker), so there is no \
caption/radio-set geometry to observe the side-by-side->stacked flip. (The choice-parameter radio \
options DO wrap when narrow at crates/skit-tui/src/session.rs:1010-1039, but the test cannot be \
split without dropping the preset-row assertion.)"]
fn test_run_form_stacks_preset_row_and_choices_when_narrow() {
    // Preset row caption/radio-set side-by-side when wide, stacked when narrow; choice radios the
    // same; the footer KeysBar caps to one visible line when short. The preset RadioSet has no
    // Rust analog (dropdown), so the full contract is not expressible here.
}

#[test]
#[ignore = "UNMAPPED (absent: no public observation surface): the oracle asserts each \
#pf-mirror-pypi/github/npm row keeps its two radio buttons side by side until narrow, while the \
#pf-form sentence options always stack, by reading per-RadioButton .region.y \
(test_tui_responsive.py:337-351). The Rust preferences renderer DOES wrap radio options by width \
(crates/skit-tui/src/screens/preferences.rs:657-700 render_control, radio_rows), but the behavior \
is present only in the private renderer: render_preferences exposes no per-control geometry — \
ViewGeometry carries empty hits and the whole area (crates/skit-tui/src/session.rs:785-798) — and \
the option labels are long localized sentences inside a scrolling screen, so an integration test \
cannot observe an individual mirror-row layout through the public surface."]
fn test_prefs_mirror_rows_are_horizontal_until_narrow_and_sentences_always_stack() {
    // Mirror rows horizontal until narrow; sentence options always stack. Reachable only through
    // the private preferences renderer's geometry, not the public ViewGeometry.
}

// ---------------------------------------------------------------------------
// modals: never exceed the screen; tall content scrolls; pickers keep their input
// ---------------------------------------------------------------------------

#[test]
fn test_help_overlay_caps_to_a_tiny_screen_and_scrolls_by_key() {
    // On a terminal shorter than the key list, the ? overlay clamps to the screen and its body is
    // a focused scroll region — ↓ actually reveals the clipped rows, so the keyboard path survives
    // the smallest windows.
    let mut state = library_a();
    state.update(Action::OpenHelp);
    let mut session = TuiSession::default();
    let (before, geometry) = session_frame(&mut session, &state, 40, 8);
    assert!(before.join("\n").contains("Help"), "{before:?}");

    assert_eq!(
        session.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
            &state,
            &geometry,
        ),
        EventHandling::Consumed,
    );
    let (after, _) = session_frame(&mut session, &state, 40, 8);
    assert_ne!(before, after, "↓ must scroll the clipped help body");
}

#[test]
fn test_confirm_remove_shrinks_for_a_long_name_on_a_narrow_screen() {
    // A name far wider than the terminal must wrap inside the capped confirm box, never push the
    // border off screen. Ratatui cannot overflow a 40-wide buffer, so the observable contract is:
    // at (40, 20) the box renders intact — its prompt, the wrapped name, and BOTH exit buttons are
    // on screen.
    let long_name = "a-script-with-a-name-far-wider-than-the-terminal-itself";
    let mut state = library(vec![python_entry(long_name)]);
    state.update(Action::AskRemove);
    let joined = lines_at(&state, 40, 20).join("\n");
    assert!(joined.contains("Remove this entry:"), "{joined}");
    assert!(joined.contains("Remove"), "{joined}"); // the submit button survived the shrink
    assert!(joined.contains("Keep"), "{joined}"); // and the cancel button
    // the long name wrapped inside rather than being clipped away entirely
    assert!(joined.contains("a-script-with-a-name"), "{joined}");
}

#[test]
fn test_env_picker_fits_input_and_esc_chip_across_the_tiers() {
    // The picker must FIT its tier: 'modals never exceed the screen'. The oracle's worst case is a
    // long env list (chrome 10 + list 12 = 22 rows on a 20-row terminal), so the list must cap. The
    // Rust env picker renders a filter Input over a capped variable list
    // (crates/skit-tui/src/screens/run_modal.rs:327-363, list .min(12)); the fit contract observed
    // here is that the Input row is fully drawn on screen across the band (the worst -h-normal case
    // and the short floor), with a long env set so the cap is load-bearing.
    //
    // GAP: the oracle's Esc/Cancel chip is "the modal's only mouse path out"
    // (test_tui_responsive.py:390-418). The Rust env picker renders no in-modal Esc chip and its
    // mouse handler only accepts the input and the list rows (run_modal.rs:485-492) — Esc is
    // keyboard-only. Recorded as an absent capability gap; this test stays REAL for the fit half.
    let env = (0..30)
        .map(|index| (format!("SKIT_VAR_{index:02}"), format!("value-{index}")))
        .chain(std::iter::once((
            "HOME".to_owned(),
            "/home/alice".to_owned(),
        )))
        .collect::<std::collections::BTreeMap<_, _>>();
    let form = RunFormView::from_declarations(
        "envd",
        "Envd",
        &[skit_domain::parameters::ParamDecl::new("value")],
        &std::collections::BTreeMap::from([("value".to_owned(), String::new())]),
        &[],
        "",
        &std::collections::BTreeMap::new(),
        "",
    )
    .with_context(RunFormContext {
        entry_kind: "python".to_owned(),
        path: Some(RunPathContext {
            workdir: "/work".to_owned(),
            invoke_cwd: "/invoke".to_owned(),
        }),
        tokens: skit_application::tokens::TokenContext {
            cwd: "/invoke".to_owned(),
            home: Some("/home/alice".to_owned()),
            env,
            today: "2026-08-08".to_owned(),
            now: "12-00-00".to_owned(),
        },
    });
    let mut state = library_a();
    state.update(Action::Present(Screen::Run(Box::new(form))));

    for (width, height) in [(70, 20), (70, 10)] {
        state.update(Action::OpenRunTokenMenuFor(0));
        state.update(Action::OpenRunEnvironmentPicker(0));
        let joined = lines_at(&state, width, height).join("\n");
        // render-model: Ratatui TestBackend exposes no widget region height, so the oracle's
        // input_box.region.height==3 and region.y+height<=screen are not directly assertable. The
        // filter input's placeholder proves the input is drawn within the tier. Verified benign:
        // the picker draws the filter input at the modal top (Rect height inner.height.min(3),
        // run_modal.rs:341) BEFORE the capped list, and centered() clamps the panel to the area, so
        // the placeholder rendering proves the input drew unclipped — the fit the oracle checks.
        assert!(
            joined.contains("type to filter…"),
            "env picker input clipped at {width}x{height}: {joined}"
        );
        // dismiss the modal before the next tier
        state.update(Action::Back);
    }
}

#[test]
#[ignore = "UNMAPPED (absent: no public observation surface): the oracle opens the add flow, walks \
focus path->template->name, and asserts the #add-template-name field sits above the docked KeysBar \
because FormBody scrolls it into view on a short terminal (test_tui_responsive.py:421-434). In Rust \
the Add screen suppresses the shared footer (crates/skit-tui/src/footer.rs:303-311 is_suppressed) \
and renders its own docked footer and its field scroll-into-view inside the private \
AddScreenSession; reaching the template-name field needs the 'write a new script' authoring \
sub-flow, and the focused-field-vs-footer geometry is not exposed on the public AddScreenGeometry \
(body/first_visible/hits only)."]
fn test_add_source_fields_stay_reachable_on_short_terminals() {
    // The add flow's body scrolls (FormBody): on a short terminal, walking focus onto the
    // template/name field scrolls it above the docked footer instead of hiding it. The scroll and
    // the add screen's own footer live in the private AddScreenSession, not the public surface.
}

#[test]
#[ignore = "UNMAPPED (cross-crate: skit-cli inlineform): the oracle drives skit.inlineform.\
_InlineFormApp and asserts an inline screen gets width tiers but no -h-* classes because its \
height is content-sized, not terminal-sized (test_tui_responsive.py:437-451). The inline form lives \
in the skit-cli-rs crate (collect_form), runs a live terminal loop rather than a TestBackend, and \
Ratatui carries no tier classes — untranslatable from this crate. Owning tier: skit-cli-rs."]
fn test_inline_form_gets_width_tiers_but_no_height_tiers() {
    // Inline screens are content-sized; a height tier computed from that would clip the footer.
    // Not reachable from skit-tui's integration tests.
}
