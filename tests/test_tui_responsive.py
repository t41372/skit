"""Responsive layout: the size tiers (tui_layout) and what each screen does in them.

The policy under test: all size adaptation is CSS keyed off the -w-*/-h-* classes the
App-level breakpoints put on every screen — the Library's detail pane stacks below
the list on portrait shapes (narrow + normal/tall) and collapses only when narrow AND
short (Tab and its footer chip pin it either way), the search box flattens when
short, footer pills wrap as whole clickable units inside a scrollable KeysBar whose
tier caps trim VISIBILITY only (uncapped when tall, everything wheel-reachable
always), horizontal option rows stack when narrow, and no modal ever exceeds the
screen. Tier boundaries are asserted at N-1/N so an off-by-one in the thresholds
(or a renamed class) cannot survive.
"""

from __future__ import annotations

from textual.containers import VerticalScroll
from textual.widgets import Input, RadioButton, Static

from skit import argstate, store, tui, tui_footer, tui_layout
from skit.tui_form import EnvPickerModal, RunFormScreen
from skit.tui_prefs import PreferencesScreen


def _py(tmp_path, body: str, name: str = "job.py"):
    p = tmp_path / name
    p.write_text(body, encoding="utf-8")
    return p


# ---------------------------------------------------------------------------
# the tier contract and the pill glue (unit level)
# ---------------------------------------------------------------------------


def test_breakpoint_tiers_are_the_documented_contract():
    """The tier names are load-bearing: every responsive CSS rule selects on them
    literally, so a renamed class or a shifted threshold silently disables rules
    everywhere. Pin the whole contract (values hardcoded on purpose)."""
    assert tui_layout.HORIZONTAL_BREAKPOINTS == [(0, "-w-narrow"), (80, "-w-normal")]
    assert tui_layout.VERTICAL_BREAKPOINTS == [
        (0, "-h-tiny"),
        (10, "-h-short"),
        (16, "-h-normal"),
        (28, "-h-tall"),
    ]


def test_chip_glues_every_blank_so_the_pill_is_one_word():
    """Every blank inside a pill must be U+2800 (not a space, not U+00A0 — both are
    \\s to the wrapper): one plain space anywhere lets a footer wrap split the pill
    mid-label. Expected strings hardcode the glyph so a mutated GLUE can't hide."""
    glue = "⠀"
    assert glue == tui_footer.GLUE
    assert tui_footer.chip("app.run", "Enter", "Save as preset") == (
        f"[on #2A211C @click=app.run]{glue}[bold $accent]Enter[/]{glue}"
        f"Save{glue}as{glue}preset{glue}[/]"
    )
    # A key with a blank glues too, and an empty label yields a key-only pill.
    assert tui_footer.chip("app.x", "Ctrl S", "") == (
        f"[on #2A211C @click=app.x]{glue}[bold $accent]Ctrl{glue}S[/]{glue}[/]"
    )
    # EVERY regex-\s blank glues, not just ASCII space: a translated label with an
    # NBSP (French "Aide ?") or an ideographic space would otherwise snap its pill.
    assert tui_footer.chip("app.x", "?", "Aide\u00a0?") == (
        f"[on #2A211C @click=app.x]{glue}[bold $accent]?[/]{glue}Aide{glue}?{glue}[/]"
    )
    assert "\u3000" not in tui_footer.chip("app.x", "k", "全角\u3000空格")


def test_nav_chip_is_exactly_the_two_key_only_pills():
    """nav_chip's two pills are the movement contract of EVERY form footer: pin the
    exact markup (actions, keys, key-only shape) so no drift — a reworded key, a lost
    direction, a label sneaking into the tight footers — can pass unnoticed."""
    glue = "⠀"
    assert tui_footer.nav_chip() == (
        f"[on #2A211C @click=app.focus_next]{glue}[bold $accent]Tab/↓[/]{glue}[/]"
        "  "
        f"[on #2A211C @click=app.focus_previous]{glue}[bold $accent]Shift+Tab/↑[/]{glue}[/]"
    )


# ---------------------------------------------------------------------------
# width tiers: side-by-side vs stacked at the 79/80 boundary, Tab pin wins both ways
# ---------------------------------------------------------------------------


async def test_width_tier_boundary_flips_side_by_side_to_stacked(tmp_path):
    """At >= 80 cols the detail pane sits beside the list; one column narrower flips
    #main to the portrait layout — the pane moves BELOW the list at full row width
    (the terminal is tall enough to spare the rows), it does not disappear."""
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    app = tui.MenuApp()
    async with app.run_test(size=(80, 24)) as pilot:
        await pilot.pause()
        assert app.screen.has_class("-w-normal")
        table = app.query_one("#entry-table")
        detail = app.query_one("#detail")
        assert detail.display
        assert detail.region.y == table.region.y  # beside the list
        assert detail.region.x > table.region.x
        await pilot.resize_terminal(79, 24)
        await pilot.pause()
        assert app.screen.has_class("-w-narrow")
        assert detail.display  # stacked, not hidden
        assert detail.region.y > table.region.y  # below the list
        assert detail.region.x == table.region.x  # at full row width


async def test_narrow_short_hides_detail_and_tab_pin_survives_resizes(tmp_path):
    """Narrow AND short is the shape with no room for the pane in either direction:
    only there does it auto-hide. Tab pins it OPEN and the pin holds through a wide
    resize AND back; further Tabs alternate via the pinned-closed / pinned-open
    branches (each state asserted, so no branch can invert silently)."""
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    app = tui.MenuApp()
    async with app.run_test(size=(70, 12)) as pilot:
        await pilot.pause()
        detail = app.query_one("#detail")
        assert not detail.display  # narrow + short → auto-hidden
        await pilot.press("tab")  # pin open
        await pilot.pause()
        assert detail.display
        await pilot.resize_terminal(120, 12)
        await pilot.pause()
        assert detail.display
        await pilot.resize_terminal(70, 12)
        await pilot.pause()
        assert detail.display  # the pin beats the narrow+short tier
        await pilot.press("tab")  # pinned-open → pinned-closed
        await pilot.pause()
        assert not detail.display
        await pilot.press("tab")  # pinned-closed → pinned-open
        await pilot.pause()
        assert detail.display


async def test_tiny_narrow_tab_still_brings_the_pane_back(tmp_path):
    """Even at the degradation floor the pane is reachable: the first Tab on a
    tiny+narrow screen must read the tier as hidden and pin the pane OPEN."""
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    app = tui.MenuApp()
    async with app.run_test(size=(46, 9)) as pilot:
        await pilot.pause()
        detail = app.query_one("#detail")
        assert not detail.display  # tiny + narrow → auto-hidden
        await pilot.press("tab")
        await pilot.pause()
        assert detail.display  # pinned open, not re-hidden


async def test_tab_walks_the_pin_states_on_a_wide_terminal_too(tmp_path):
    """The closed→open flip must read the PIN, not the width tier: on a wide terminal
    the tier alone would already say "visible", so a toggle that ignored the
    pinned-closed class would re-hide the pane instead of reopening it. (The narrow
    twin above can't catch that — there the tier and the pin agree.)"""
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    app = tui.MenuApp()
    async with app.run_test(size=(120, 24)) as pilot:
        await pilot.pause()
        detail = app.query_one("#detail")
        assert detail.display  # wide → auto-shown
        await pilot.press("tab")  # auto → pinned-closed
        await pilot.pause()
        assert not detail.display
        await pilot.press("tab")  # pinned-closed → pinned-open, while wide
        await pilot.pause()
        assert detail.display
        await pilot.press("tab")  # pinned-open → pinned-closed again
        await pilot.pause()
        assert not detail.display


# ---------------------------------------------------------------------------
# height tiers: search flattens, key rows stop wrapping, global row yields
# ---------------------------------------------------------------------------


async def test_height_tier_boundaries_flatten_search_then_drop_the_global_row(tmp_path):
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 28)) as pilot:
        await pilot.pause()
        assert app.screen.has_class("-h-tall")
        await pilot.resize_terminal(100, 27)
        await pilot.pause()
        assert app.screen.has_class("-h-normal")
        await pilot.resize_terminal(100, 16)
        await pilot.pause()
        assert app.screen.has_class("-h-normal")
        assert app.query_one("#search").region.height == 3  # bordered
        await pilot.resize_terminal(100, 15)
        await pilot.pause()
        assert app.screen.has_class("-h-short")
        assert app.query_one("#search").region.height == 1  # flattened
        assert app.query_one("#keys").region.height == 2  # short: one line per row
        await pilot.resize_terminal(100, 10)
        await pilot.pause()
        assert app.screen.has_class("-h-short")
        await pilot.resize_terminal(100, 9)
        await pilot.pause()
        assert app.screen.has_class("-h-tiny")
        assert app.query_one("#keys").region.height == 1  # one visible line total…
        assert app.query_one("#keys-global").display  # …but nothing is dropped
        assert app.query_one("#status").display  # the feedback channel stays


async def test_flattened_search_still_filters(tmp_path):
    """The short-tier search box is chrome-less, not feature-less: / focuses it and
    typing still filters the table."""
    store.add_python(_py(tmp_path, "print(1)\n", "a.py"), name="alpha")
    store.add_python(_py(tmp_path, "print(2)\n", "b.py"), name="beta")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 12)) as pilot:
        await pilot.pause()
        await pilot.press("slash")
        await pilot.press("b", "e")
        await pilot.pause()
        assert [e.meta.name for e in app._visible] == ["beta"]


# ---------------------------------------------------------------------------
# footer pills: wrap between chips, wrapped chips stay clickable, short caps rows
# ---------------------------------------------------------------------------


async def test_footer_wraps_between_pills_and_wrapped_chips_stay_clickable(tmp_path):
    """At 44 cols the global row wraps after the / Search pill, so the Tab pill opens
    line two and the tail lands on line three — every chip still visible (the normal
    tier caps at three lines), and clicking a wrapped chip must still fire its
    action: a wrapped chip is a real button, not decoration. The Tab chip doubles as
    the proof — its click must toggle the detail pane, the advertised mouse path to
    a pane a tier hid."""
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    app = tui.MenuApp()
    async with app.run_test(size=(44, 24)) as pilot:
        await pilot.pause()
        keys_global = app.query_one("#keys-global", Static)
        assert keys_global.region.height == 3  # wrapped, whole pills on each line
        detail = app.query_one("#detail")
        assert detail.display  # narrow + normal → stacked below the list
        await pilot.click("#keys-global", offset=(3, 1))  # the Tab pill, on line 2
        await pilot.pause()
        assert not detail.display  # the wrapped chip fired toggle_detail
        await pilot.click("#keys-global", offset=(3, 1))
        await pilot.pause()
        assert detail.display  # and back


async def test_portrait_stacks_the_detail_pane_and_uncaps_the_footer(tmp_path):
    """The portrait shape (narrow + tall): the detail pane stacks below the list at
    full width instead of vanishing, and the footer wraps without a cap so every
    chip stays visible even on a sliver-narrow window. Tab still hides the stacked
    pane — the pin's !important rules beat the portrait display rule."""
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    app = tui.MenuApp()
    async with app.run_test(size=(26, 44)) as pilot:
        await pilot.pause()
        assert app.screen.has_class("-w-narrow")
        assert app.screen.has_class("-h-tall")
        table = app.query_one("#entry-table")
        detail = app.query_one("#detail")
        assert detail.display
        assert detail.region.y > table.region.y  # below the list
        assert detail.region.width == table.region.width  # at full row width
        assert app.query_one("#keys-global").region.height > 3  # uncapped: all chips
        await pilot.press("tab")
        await pilot.pause()
        assert not detail.display  # the pin beats the portrait stack rule
        await pilot.press("tab")
        await pilot.pause()
        assert detail.display


async def test_short_tier_caps_visible_lines_but_keeps_chips_scroll_reachable(tmp_path):
    """Narrow AND short: wrapping would spend the rows the tier just reclaimed, so the
    KeysBar shows two lines — but the cap trims visibility only: the wrapped rows
    behind it stay wheel-reachable, so every chip keeps a mouse path (the mouse-alone
    policy holds at every size, not just comfortable ones)."""
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    app = tui.MenuApp()
    async with app.run_test(size=(46, 12)) as pilot:
        await pilot.pause()
        keys = app.query_one("#keys", tui_footer.KeysBar)
        assert keys.region.height == 2  # the Library's short budget: 2 visible lines
        assert keys.virtual_size.height > keys.region.height  # more rows exist…
        keys.scroll_end(animate=False)
        await pilot.pause()
        assert keys.scroll_y > 0  # …and the wheel path to them is real
        # The last chip of the global row is inside the scrollable content.
        assert "Help" in str(app.query_one("#keys-global", Static).render())


# ---------------------------------------------------------------------------
# form screens: preset row and option sets stack when narrow, footer caps when short
# ---------------------------------------------------------------------------


def _choice_entry(tmp_path):
    entry = store.add_python(
        _py(
            tmp_path,
            "import argparse\n"
            "p = argparse.ArgumentParser()\n"
            'p.add_argument("--mode", choices=["alpha", "beta"])\n'
            "p.parse_args()\n",
            "choices.py",
        ),
        name="choices",
    )
    argstate.save_preset(entry.slug, "web", {"mode": "alpha"})
    return entry


async def test_run_form_stacks_preset_row_and_choices_when_narrow(tmp_path):
    _choice_entry(tmp_path)
    app = tui.MenuApp()
    async with app.run_test(size=(120, 30)) as pilot:
        app.action_run()
        await pilot.pause()
        assert isinstance(app.screen, RunFormScreen)
        caption, radio_set = app.screen.query_one("#preset-row").children[:2]
        assert caption.region.y == radio_set.region.y  # side by side when wide
        alpha, beta = app.screen.query_one("#fr-mode").query(RadioButton)
        assert alpha.region.y == beta.region.y
        await pilot.resize_terminal(46, 30)
        await pilot.pause()
        assert caption.region.y < radio_set.region.y  # stacked when narrow
        alpha, beta = app.screen.query_one("#fr-mode").query(RadioButton)
        assert alpha.region.y < beta.region.y
        await pilot.resize_terminal(46, 12)
        await pilot.pause()
        # The cap lives on the KeysBar container; the Static inside keeps its full
        # wrapped height and stays scroll-reachable behind the one visible line.
        assert app.screen.query_one(tui_footer.KeysBar).region.height == 1


async def test_prefs_mirror_rows_are_horizontal_until_narrow_and_sentences_always_stack(tmp_path):
    app = tui.MenuApp()
    async with app.run_test(size=(120, 40)) as pilot:
        app.push_screen(PreferencesScreen())
        await pilot.pause()
        # All three axis rows share the .pf-mirror-row layout contract.
        for row in ("#pf-mirror-pypi", "#pf-mirror-github", "#pf-mirror-npm"):
            buttons = list(app.screen.query_one(row).query(RadioButton))
            assert buttons[0].region.y == buttons[1].region.y, row  # side by side
        form = list(app.screen.query_one("#pf-form").query(RadioButton))
        assert form[0].region.y < form[1].region.y  # sentence options always stack
        await pilot.resize_terminal(60, 40)
        await pilot.pause()
        mirror = list(app.screen.query_one("#pf-mirror-pypi").query(RadioButton))
        assert mirror[0].region.y < mirror[1].region.y  # narrow stacks the mirror rows too


# ---------------------------------------------------------------------------
# modals: never exceed the screen; tall content scrolls; pickers keep their input
# ---------------------------------------------------------------------------


async def test_help_overlay_caps_to_a_tiny_screen_and_scrolls_by_key(tmp_path):
    """On a terminal shorter than the key list, the ? overlay clamps to the screen and
    its body is a focused scroll region — ↓ actually reveals the clipped rows, so the
    keyboard path survives the smallest windows."""
    app = tui.MenuApp()
    async with app.run_test(size=(40, 8)) as pilot:
        app.action_help()
        await pilot.pause()
        box = app.screen.query_one("#help-box", VerticalScroll)
        assert box.region.width <= 40
        assert box.region.height <= 8
        assert app.focused is box  # "*" auto-focus lands on the scroll body
        assert box.scroll_y == 0
        await pilot.press("down")
        await pilot.pause()
        assert box.scroll_y > 0


async def test_confirm_remove_shrinks_for_a_long_name_on_a_narrow_screen(tmp_path):
    long_name = "a-script-with-a-name-far-wider-than-the-terminal-itself"
    store.add_python(_py(tmp_path, "print(1)\n"), name=long_name)
    app = tui.MenuApp()
    async with app.run_test(size=(40, 20)) as pilot:
        await pilot.pause()
        app.action_remove()
        await pilot.pause()
        box = app.screen.query_one("#confirm-box")
        assert box.region.width <= 40  # capped: wraps inside, never pushes the border off
        assert box.region.height <= 20


def _chip_static(screen) -> Static:
    """The modal's Esc/Cancel chip row — its only mouse path out."""
    return list(screen.query(Static))[-1]


async def test_env_picker_fits_input_and_esc_chip_across_the_tiers(tmp_path):
    """The picker must FIT its tier, chip included: the Esc chip is the modal's only
    mouse path out, so 'modals never exceed the screen' means the chip is on screen
    across the whole band — not just at the band's tallest sizes. The -h-normal band
    needs its own list cap (chrome 10 + list 12 was 22 rows on a 20-row terminal);
    the short band flattens the box padding and chip margin and shrinks the list so
    everything fits even at the band's 10-row floor."""
    app = tui.MenuApp()
    async with app.run_test(size=(70, 20)) as pilot:  # -h-normal, the worst old case
        app.push_screen(EnvPickerModal())
        await pilot.pause()
        chip_row = _chip_static(app.screen)
        assert chip_row.region.y + chip_row.region.height <= 20  # Esc chip on screen
        await pilot.press("escape")
        await pilot.pause()
        await pilot.resize_terminal(70, 10)  # the short band's floor
        await pilot.pause()
        app.push_screen(EnvPickerModal())
        await pilot.pause()
        input_box = app.screen.query_one(Input)
        assert input_box.region.height == 3  # fully visible, not clipped
        assert input_box.region.y + input_box.region.height <= 10
        chip_row = _chip_static(app.screen)
        assert chip_row.region.y + chip_row.region.height <= 10  # chip too


async def test_add_source_fields_stay_reachable_on_short_terminals(tmp_path):
    """The add flow's body scrolls (FormBody): on a short terminal the template/name
    fields must not sit hidden under the docked footer — walking focus onto a field
    scrolls it into view above the KeysBar."""
    app = tui.MenuApp()
    async with app.run_test(size=(80, 12)) as pilot:
        app.action_add()
        await pilot.pause()
        await pilot.press("down", "down")  # path → template → name
        name_box = app.focused
        assert name_box is not None
        assert name_box.id == "add-template-name"
        keys_bar = app.screen.query_one(tui_footer.KeysBar)
        assert name_box.region.y + name_box.region.height <= keys_bar.region.y


async def test_inline_form_gets_width_tiers_but_no_height_tiers(tmp_path):
    """Inline screens are sized to their CONTENT, not the terminal: a height tier
    computed from that would stamp a compact form -h-short on a 50-row terminal and
    clip its own footer. Width still comes from the terminal, so width tiers stay."""
    from skit import flows
    from skit.inlineform import _InlineFormApp

    entry = _choice_entry(tmp_path)
    plan = flows.plan_for_entry(entry)
    app = _InlineFormApp(entry, plan, flows.prefill(plan, entry.slug))
    async with app.run_test(size=(100, 50)) as pilot:
        await pilot.pause()
        assert app.screen.has_class("-w-normal")
        assert not any(cls.startswith("-h-") for cls in app.screen.classes)
