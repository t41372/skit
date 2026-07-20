"""Exact-behavior tests for the Library's render/data glue (tui.py chunk 8).

Pins observable behaviour of MenuApp._refresh_status (empty-state copy + count
pluralization), _reload (recency ordering + honouring the live search filter),
_retranslate_chrome (window title, search placeholder, column headers, pane border
titles), the static _run_banner outcome copy, and _select_slug (refreshing the footer
for the selected entry after finding it).
"""

from __future__ import annotations

from pathlib import Path

from textual.widgets import DataTable, Input, Static

from conftest import footer_text
from skit import argstate, flows, store, tui


def _py(tmp_path: Path, body: str = "print(1)\n", name: str = "job.py") -> Path:
    p = tmp_path / name
    p.write_text(body, encoding="utf-8")
    return p


# ---------------------------------------------------------------------------
# _refresh_status
# ---------------------------------------------------------------------------


async def test_refresh_status_shows_placeholder_when_empty() -> None:
    """An empty library shows the onboarding copy verbatim (no count line yet)."""
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        assert not store.list_entries()
        status = str(app.query_one("#status", Static).render())
        assert status == "Your entries will appear here."


async def test_refresh_status_counts_singular(tmp_path: Path) -> None:
    """One entry uses the singular ngettext form: '1/1 entry'."""
    store.add_python(_py(tmp_path, name="a.py"), name="alpha")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        status = str(app.query_one("#status", Static).render())
        assert status == "1/1 entry"


async def test_refresh_status_counts_plural(tmp_path: Path) -> None:
    """Two entries use the plural ngettext form: '2/2 entries'."""
    store.add_python(_py(tmp_path, name="a.py"), name="alpha")
    store.add_python(_py(tmp_path, name="b.py"), name="beta")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        status = str(app.query_one("#status", Static).render())
        assert status == "2/2 entries"


# ---------------------------------------------------------------------------
# _reload
# ---------------------------------------------------------------------------


async def test_reload_orders_by_recent_activity_desc(tmp_path: Path) -> None:
    """Rows sort by activity, most-recent first (reverse=True): a script whose last
    run is in the future must surface above a never-run one."""
    store.add_python(_py(tmp_path, name="a.py"), name="alpha")  # never run -> added_at key
    recent = store.add_python(_py(tmp_path, name="b.py"), name="beta")
    argstate.record_run(recent.slug, 0, at="2030-01-01T00:00:00+00:00")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        # newest activity first; reverse=False/None/omitted would flip this to [alpha, beta]
        assert [e.meta.name for e in app._visible] == ["beta", "alpha"]


async def test_reload_applies_current_search_filter(tmp_path: Path) -> None:
    """_reload re-applies whatever is typed in the search box (not an empty/None
    filter): with 'alph' typed, only alpha survives the reload.

    The assertion is read synchronously right after _reload() — the Input.Changed
    handler would otherwise re-filter on the next message pump and mask a mutant that
    drops the search argument."""
    store.add_python(_py(tmp_path, name="a.py"), name="alpha")
    store.add_python(_py(tmp_path, name="b.py"), name="beta")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        app.query_one("#search", Input).value = "alph"
        app._reload()  # reads the live "alph" value and filters on it
        assert [e.meta.name for e in app._visible] == ["alpha"]


# ---------------------------------------------------------------------------
# _retranslate_chrome
# ---------------------------------------------------------------------------


async def test_retranslate_updates_title(tmp_path: Path) -> None:
    store.add_python(_py(tmp_path, name="a.py"), name="alpha")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        app._retranslate_chrome()
        assert app.title == "skit · Library"


async def test_retranslate_updates_search_placeholder(tmp_path: Path) -> None:
    store.add_python(_py(tmp_path, name="a.py"), name="alpha")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        app._retranslate_chrome()
        assert app.query_one("#search", Input).placeholder == "/ to search names and descriptions…"


async def test_retranslate_updates_column_headers(tmp_path: Path) -> None:
    store.add_python(_py(tmp_path, name="a.py"), name="alpha")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        app._retranslate_chrome()
        labels = [str(c.label) for c in app.query_one(DataTable).ordered_columns]
        assert labels == ["Name", "Kind", " "]


async def test_retranslate_updates_border_titles(tmp_path: Path) -> None:
    store.add_python(_py(tmp_path, name="a.py"), name="alpha")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        app._retranslate_chrome()
        assert app.query_one(DataTable).border_title == "Library"
        assert app.query_one("#detail").border_title == "Detail pane"


# ---------------------------------------------------------------------------
# _run_banner  (pure staticmethod)
# ---------------------------------------------------------------------------


def test_run_banner_finished() -> None:
    assert tui.MenuApp._run_banner(flows.RunOutcome(0)) == "✓ finished"


def test_run_banner_failed_carries_exit_code() -> None:
    assert tui.MenuApp._run_banner(flows.RunOutcome(3)) == "✗ failed (code 3)"


def test_run_banner_couldnt_launch() -> None:
    assert tui.MenuApp._run_banner(flows.RunOutcome(None)) == "✗ couldn't launch"


# ---------------------------------------------------------------------------
# _select_slug
# ---------------------------------------------------------------------------


async def test_select_slug_refreshes_footer_after_state_change(tmp_path: Path) -> None:
    """_select_slug refreshes the footer for the selected entry even when the cursor
    does not move. A run is recorded for the only (already-highlighted) script without
    a repaint, so the Rerun chip is still absent; _select_slug must re-render and reveal
    it. A `return` that fires before the refresh (instead of `break` after it) would
    leave the stale, Rerun-less footer in place — no RowHighlighted event repaints it
    because the cursor row is unchanged."""
    alpha = store.add_python(_py(tmp_path, name="a.py"), name="alpha")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        keys = app.query_one("#keys-local", Static)
        assert "Rerun" not in footer_text(keys)  # never run yet
        argstate.record_run(alpha.slug, 0, at="2030-01-01T00:00:00+00:00")  # no repaint
        assert "Rerun" not in footer_text(keys)  # footer still stale
        app._select_slug(alpha.slug)  # same row -> only the post-loop refresh can update it
        await pilot.pause()
        assert "Rerun" in footer_text(keys)
