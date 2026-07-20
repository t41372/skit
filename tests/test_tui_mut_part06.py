"""Exact-behavior tests for MenuApp._refresh_detail's rendered detail pane.

Pins the Library detail pane's two rendering branches:
  * the empty-library placeholder (its exact help copy and line layout), and
  * the per-entry detail (its fields, each on its own line).

Both are driven end-to-end through a real ``MenuApp`` and read back off the live
``#detail-body`` Static, so the assertions catch any drift in the newline joining
or the user-visible strings. The autouse conftest fixtures pin SKIT_* dirs to a
tmp dir and the locale to English, so gettext returns the msgids verbatim.
"""

from __future__ import annotations

from pathlib import Path

from textual.widgets import Static

from skit import store, tui


def _py(tmp_path: Path, body: str, name: str = "job.py") -> Path:
    p = tmp_path / name
    p.write_text(body, encoding="utf-8")
    return p


def _detail(app: tui.MenuApp) -> str:
    return str(app.query_one("#detail-body", Static).render())


async def test_empty_library_detail_shows_exact_placeholder() -> None:
    """With no scripts added, the detail pane shows the onboarding placeholder:
    a bold headline, a blank spacer line, then the two ways to add a first script
    — each segment on its own newline-separated line, verbatim English copy."""
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        lines = _detail(app).split("\n")

    assert lines == [
        "Your entries will appear here.",
        "",
        "Press a to add the first one,",
        "or run: skit add <path> in a terminal.",
    ]


async def test_selected_entry_detail_joins_lines_by_newline(tmp_path: Path) -> None:
    """A selected entry's detail is one field per line: the name heads the pane on
    its own line, the kind badge is its own line, and the run-state footer ("Not run
    yet") is its own line. This pins the newline joining of ``_detail_lines`` — a glued
    separator would fuse the fields into a single run instead of discrete lines."""
    store.add_python(_py(tmp_path, "print(1)\n"), name="alpha")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        rendered = _detail(app)

    lines = rendered.split("\n")
    assert lines[0] == "alpha"  # the name is its own leading line
    assert "⬡ Python" in lines  # the kind badge stands alone
    assert lines[-1] == "Not run yet"  # the run-state footer is its own trailing line
    # And there is more than one line — the fields are not fused into a single run.
    assert len(lines) > 3
