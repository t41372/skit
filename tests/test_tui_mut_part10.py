"""Mutation-kill tests for the Library TUI chrome (chunk 10 of src/skit/tui.py).

Covers the observable contracts of MenuApp.on_mount (the table's column headers and the
two border titles it sets once at startup), MenuApp.on_key (Up/Down only drive the table
from the app handler while the search box owns focus), and the focus watcher that keeps
the footer in step with who holds the keyboard. The detail-pane toggle's pin/visibility
behaviour is pinned here too, so the equivalents pragma'd in action_toggle_detail stay
honest about what the surviving, killable siblings still guarantee.
"""

from __future__ import annotations

from pathlib import Path

from textual.widgets import DataTable, Input, Static

from conftest import footer_text
from skit import store, tui


def _py(tmp_path: Path, body: str = "print(1)\n", name: str = "job.py") -> Path:
    p = tmp_path / name
    p.write_text(body, encoding="utf-8")
    return p


# ---------------------------------------------------------------------------
# on_mount: the table headers and border titles set once at startup
# ---------------------------------------------------------------------------


async def test_on_mount_sets_the_three_column_headers(tmp_path):
    """The table advertises exactly Name / Kind / (blank health glyph column) — the
    English source strings, capitalised as shown, with the third header a single space."""
    store.add_python(_py(tmp_path), name="a")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 24)) as pilot:
        await pilot.pause()
        table = app.query_one(DataTable)
        labels = [str(c.label) for c in table.columns.values()]
        assert labels == ["Name", "Kind", " "]


async def test_on_mount_sets_the_table_border_title(tmp_path):
    store.add_python(_py(tmp_path), name="a")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 24)) as pilot:
        await pilot.pause()
        assert app.query_one(DataTable).border_title == "Library"


async def test_on_mount_sets_the_detail_border_title(tmp_path):
    store.add_python(_py(tmp_path), name="a")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 24)) as pilot:
        await pilot.pause()
        assert app.query_one("#detail").border_title == "Detail pane"


# ---------------------------------------------------------------------------
# on_key: Up/Down drive the table only while the search box owns the keyboard
# ---------------------------------------------------------------------------


async def test_updown_drive_the_table_while_the_search_box_is_focused(tmp_path):
    """The feature the guard exists for: with the search box focused, Up/Down still move
    the table cursor (browse results while typing). Also pins on_key's `#search` query —
    a wrong selector would raise on the first key and this could never move the cursor."""
    store.add_python(_py(tmp_path, name="a.py"), name="alpha")
    store.add_python(_py(tmp_path, name="b.py"), name="beta")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 24)) as pilot:
        await pilot.pause()
        table = app.query_one(DataTable)
        app.query_one("#search", Input).focus()
        await pilot.pause()
        assert table.cursor_row == 0
        await pilot.press("down")
        await pilot.pause()
        assert table.cursor_row == 1  # search-mode Down reached the table
        await pilot.press("up")
        await pilot.pause()
        assert table.cursor_row == 0


async def test_updown_do_not_drive_the_table_when_search_is_unfocused(tmp_path):
    """The guard is `and self.focused is search`, not `or`: with focus off the search box
    the app handler must NOT hijack Up/Down and move the table itself. With focus cleared,
    the DataTable never receives the key, so the cursor must stay put."""
    store.add_python(_py(tmp_path, name="a.py"), name="alpha")
    store.add_python(_py(tmp_path, name="b.py"), name="beta")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 24)) as pilot:
        await pilot.pause()
        table = app.query_one(DataTable)
        assert table.cursor_row == 0
        app.screen.set_focus(None)
        await pilot.pause()
        await pilot.press("down")
        await pilot.pause()
        assert table.cursor_row == 0  # on_key stayed out of the way


# ---------------------------------------------------------------------------
# the focus watcher: the footer follows whoever holds the keyboard
# ---------------------------------------------------------------------------


async def test_footer_switches_to_search_chips_when_focus_moves_to_search(tmp_path):
    """on_mount wires a watcher on the screen's `focused` reactive so the footer re-renders
    every time the keyboard changes hands. Moving focus into the search box must swap the
    per-row local chips for the two search-mode chips (Run / Back to list); moving back to
    the table restores the row chips. Pins the watcher's object/attribute/callback."""
    store.add_python(_py(tmp_path), name="a")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 24)) as pilot:
        await pilot.pause()
        local = app.query_one("#keys-local", Static)
        assert "Edit source" in footer_text(local)  # table-focus: full row chips
        app.query_one("#search", Input).focus()
        await pilot.pause()
        search_bar = footer_text(local)
        assert "Back to list" in search_bar  # watcher fired: search-mode chips
        assert "Edit source" not in search_bar
        app.query_one(DataTable).focus()
        await pilot.pause()
        assert "Edit source" in footer_text(local)  # watcher fired again on the way back


# ---------------------------------------------------------------------------
# action_toggle_detail: Tab pins the pane's visibility against the size tiers
# ---------------------------------------------------------------------------


async def test_tab_toggles_and_pins_detail_visibility(tmp_path):
    """From a wide terminal (detail auto-shown) Tab hides it and pins it closed; a second
    Tab reads that pin and reopens it. Pins the toggle's real read of `#detail` and the
    visibility flip that the has_class/`or` equivalents leave untouched."""
    store.add_python(_py(tmp_path), name="a")
    app = tui.MenuApp()
    async with app.run_test(size=(120, 24)) as pilot:
        await pilot.pause()
        detail = app.query_one("#detail")
        assert detail.display  # wide → auto-shown
        await pilot.press("tab")
        await pilot.pause()
        assert not detail.display
        assert app.screen.has_class("-detail-pinned-closed")
        assert not app.screen.has_class("-detail-pinned-open")
        await pilot.press("tab")
        await pilot.pause()
        assert detail.display
        assert app.screen.has_class("-detail-pinned-open")
        assert not app.screen.has_class("-detail-pinned-closed")
