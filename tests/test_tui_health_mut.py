"""Exact-behavior tests for the Health check screen (D).

Pins the observable output of the two acting surfaces the mutation survivors touch:
`on_mount`'s panel title and `action_rebuild`'s report line (singular/plural wording and
the newline join). All message-content assertions force the English catalog so a mutated
msgid diverges from the pinned English source.
"""

from __future__ import annotations

from pathlib import Path

import pytest
from textual.widgets import Static

from skit import i18n, store, tui
from skit.paths import scripts_dir
from skit.tui_health import HealthScreen


@pytest.fixture(autouse=True)
def tmp_store(tmp_path, monkeypatch):
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path / "config"))
    monkeypatch.setenv("SKIT_LANG", "en")
    i18n.init("en")  # message-content assertions read the pinned English source


def _py(tmp_path: Path, body: str, name: str) -> Path:
    p = tmp_path / name
    p.write_text(body, encoding="utf-8")
    return p


# ---------------------------------------------------------------------------
# on_mount: the panel border title
# ---------------------------------------------------------------------------


async def test_health_panel_border_title_is_health_check(tmp_path):
    """on_mount stamps the "Health check" title onto the #hc-body panel border. A dropped,
    emptied, or re-cased msgid (or a None title) leaves a different border title."""
    store.add_python(_py(tmp_path, "print(1)\n", "a.py"), name="a")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = HealthScreen()
        app.push_screen(screen)
        await pilot.pause()
        assert screen.query_one("#hc-body").border_title == "Health check"


# ---------------------------------------------------------------------------
# action_rebuild: the rebuilt-index report line
# ---------------------------------------------------------------------------


async def test_rebuild_reports_singular_entry_count(tmp_path):
    """With exactly one registered script the report uses the singular msgid, verbatim."""
    store.add_python(_py(tmp_path, "print(1)\n", "only.py"), name="only")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = HealthScreen()
        app.push_screen(screen)
        await pilot.pause()
        screen.action_rebuild()
        await pilot.pause()
        report = str(screen.query_one("#hc-rebuilt", Static).render())
        assert report.splitlines()[0] == "Index rebuilt: 1 entry"


async def test_rebuild_reports_plural_count_and_joins_problem_lines(tmp_path):
    """Two scripts -> the plural msgid; a stray entry dir with no meta.toml -> a problem
    line. The count line and the problem line are joined by a real newline (not a marker),
    so the report reads as two separate lines with the exact English plural wording."""
    store.add_python(_py(tmp_path, "print(1)\n", "one.py"), name="one")
    store.add_python(_py(tmp_path, "print(2)\n", "two.py"), name="two")
    (scripts_dir() / "orphan").mkdir(parents=True)  # no meta.toml -> doctor reports it
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = HealthScreen()
        app.push_screen(screen)
        await pilot.pause()
        screen.action_rebuild()
        await pilot.pause()
        report = str(screen.query_one("#hc-rebuilt", Static).render())
        lines = report.splitlines()
        assert lines[0] == "Index rebuilt: 2 entries"
        assert lines[1] == "orphan: meta.toml is missing; skipped"
        assert "XX" not in report  # lines joined by "\n", never an "XX\nXX" marker
