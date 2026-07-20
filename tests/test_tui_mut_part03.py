"""Mutation-kill tests for a slice of src/skit/tui.py (worklist part 03).

Covers the pure ``_relative_time`` formatter (boundaries, floor division, and the
exact translated copy), the ``ConfirmRemove`` cancel contract, ``MenuApp.__init__``'s
initial state, the ``_apply_filter`` table build (health cell + slug row keys), and
``_detail_lines`` (kind badge, mode reassurance, description placeholder, separators).

All English-catalog (conftest pins SKIT_LANG=en and resets the i18n singleton per test),
so exact-copy assertions are stable. Windows-safe: no real clock, no subprocesses.
"""

from __future__ import annotations

from datetime import UTC, datetime, timedelta
from unittest import mock

from textual.widgets import DataTable

from skit import store, tui


def _py(tmp_path, body: str, name: str = "job.py"):
    p = tmp_path / name
    p.write_text(body, encoding="utf-8")
    return p


# ---------------------------------------------------------------------------
# _relative_time — a pure formatter, driven with a frozen clock so the second
# count landing on each branch boundary is exact (no wall-clock drift).
# ---------------------------------------------------------------------------

_FIXED_NOW = datetime(2026, 6, 1, 12, 0, 0, tzinfo=UTC)


class _FrozenDatetime:
    """Stand-in for tui.datetime: a fixed ``now`` plus the real ``fromisoformat``."""

    @staticmethod
    def now(tz: object = None) -> datetime:
        return _FIXED_NOW

    @staticmethod
    def fromisoformat(value: str) -> datetime:
        return datetime.fromisoformat(value)


def _rel_factory(monkeypatch):
    monkeypatch.setattr(tui, "datetime", _FrozenDatetime)

    def _make(seconds: int) -> str:
        iso = (_FIXED_NOW - timedelta(seconds=seconds)).isoformat()
        return tui._relative_time(iso)

    return _make


def test_relative_time_just_now_ends_below_90s(monkeypatch):
    rel = _rel_factory(monkeypatch)
    assert rel(89) == "just now"
    # Exactly 90s is already the first minute — kills "< 90" -> "<= 90" and "< 91".
    assert rel(90) == "1 min ago"


def test_relative_time_minutes_floor_and_exact_copy(monkeypatch):
    rel = _rel_factory(monkeypatch)
    # Kills "XX...XX" wrap, "// 60" -> "/ 60" (would give "2.0"), and "// 61" (would give "1").
    assert rel(120) == "2 min ago"


def test_relative_time_minutes_end_below_5400s(monkeypatch):
    rel = _rel_factory(monkeypatch)
    assert rel(5399) == "89 min ago"
    # Exactly 5400s is the first hour — kills "< 5400" -> "<= 5400" and "< 5401".
    assert rel(5400) == "1 h ago"


def test_relative_time_hours_floor_and_exact_copy(monkeypatch):
    rel = _rel_factory(monkeypatch)
    # Kills "XX...XX" wrap, "// 3600" -> "/ 3600" ("2.0"), and "// 3601" ("1").
    assert rel(7200) == "2 h ago"


def test_relative_time_hours_end_below_129600s(monkeypatch):
    rel = _rel_factory(monkeypatch)
    assert rel(129599) == "35 h ago"
    # Exactly 129600s is the first day — kills "< 129600" -> "<= 129600" and "< 129601".
    assert rel(129600) == "1 d ago"


def test_relative_time_days_floor_and_exact_copy(monkeypatch):
    rel = _rel_factory(monkeypatch)
    # 259200s == 3d exactly. Kills "XX...XX" wrap, "// 86400" -> "/ 86400" ("3.0"),
    # and "// 86401" (259203 > 259200 so floors to 2).
    assert rel(259200) == "3 d ago"


# ---------------------------------------------------------------------------
# ConfirmRemove.action_cancel — the typed dismiss value (False, not None)
# ---------------------------------------------------------------------------


async def test_confirm_remove_cancel_dismisses_with_false(tmp_path):
    """ConfirmRemove is ModalScreen[bool]: cancelling must hand back the typed False,
    which a bool-typed caller can distinguish from None. Kills dismiss(False)->dismiss(None)."""
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="keepme")
    app = tui.MenuApp()
    results: list[object] = []
    async with app.run_test() as pilot:
        await pilot.pause()
        app.push_screen(tui.ConfirmRemove(entry), results.append)
        await pilot.pause()
        await pilot.press("n")  # the modal's cancel key
        await pilot.pause()
    assert results == [False]
    assert store.list_entries()  # cancel kept the entry


# ---------------------------------------------------------------------------
# MenuApp.__init__ — the constructor's initial state
# ---------------------------------------------------------------------------


def test_fresh_menuapp_starts_with_empty_entry_lists():
    """A freshly constructed app must present empty lists (not None) before the first
    _reload runs — the list[Entry] contract the rest of the app relies on. Kills the two
    ``= []`` -> ``= None`` mutants."""
    app = tui.MenuApp()
    assert app._entries == []
    assert app._visible == []


async def test_first_ctrl_c_warns_instead_of_quitting(tmp_path):
    """The 0.0 ctrl-c sentinel means "never pressed": the FIRST Ctrl+C must only warn.
    Frozen just inside the double-press window (now == 2.5, window == 2.0), the correct
    0.0 sentinel gives 2.5 > window (warn), while a 1.0 sentinel gives 1.5 <= window and
    would wrongly quit on the very first press. Kills ``_ctrl_c_at = 0.0`` -> ``1.0``."""
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        with mock.patch.object(tui.time, "monotonic", return_value=app.CTRL_C_WINDOW + 0.5):
            app.action_ctrl_c_quit()
            assert app.return_value is None  # warned, did not exit


# ---------------------------------------------------------------------------
# MenuApp._apply_filter — the table build
# ---------------------------------------------------------------------------


async def test_apply_filter_healthy_entry_has_blank_health_cell(tmp_path):
    """A present target leaves the health column empty (only a missing target shows ⚠).
    Kills the else-branch ``""`` -> ``"XXXX"`` mutant on the health cell."""
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        table = app.screen.query_one(DataTable)
        assert table.get_row_at(0)[2] == ""


async def test_apply_filter_keys_rows_by_slug(tmp_path):
    """Rows are keyed by the entry slug so they are addressable by identity. Kills both
    ``key=e.slug`` -> ``key=None`` and the dropped-``key`` mutant (get_row raises without it)."""
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="alpha")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        table = app.screen.query_one(DataTable)
        row = table.get_row(entry.slug)  # RowDoesNotExist if the row wasn't keyed by slug
        assert row[0] == "alpha"


# ---------------------------------------------------------------------------
# MenuApp._detail_lines — the detail-pane body (called directly; no DOM needed)
# ---------------------------------------------------------------------------


def test_detail_lines_render_the_real_kind_label(tmp_path):
    """The kind badge is built from the entry's actual kind, not None. Kills
    ``_kind_badge(entry.meta.kind)`` -> ``_kind_badge(None)`` (which yields "? None")."""
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="a", description="")
    lines = tui.MenuApp()._detail_lines(entry)
    assert any("Python" in ln for ln in lines)


def test_detail_lines_copy_mode_reassurance_is_verbatim(tmp_path):
    """The copy-mode A5 reassurance line renders exactly. Kills its "XX...XX" wrap and the
    "The" -> "the" case mutant (both fail an exact-line match)."""
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="a", description="")
    lines = tui.MenuApp()._detail_lines(entry)
    assert "[dim]✓ The copy is kept by skit; your original file is never modified.[/dim]" in lines


def test_detail_lines_reference_mode_links_the_original(tmp_path):
    """A reference entry's detail names the linked original. Kills the "XX...XX" wrap on the
    'Linked to the original: %(path)s' msgid (which would push 'XXLinked' to the front)."""
    p = _py(tmp_path, "print(1)\n", "orig.py")
    entry = store.add_python(p, name="linked", mode="reference", description="")
    lines = tui.MenuApp()._detail_lines(entry)
    ref = next(ln for ln in lines if "Linked to the original" in ln)
    assert ref.startswith("[dim]↗ Linked to the original: ")


def test_detail_lines_surround_description_with_blank_separators(tmp_path):
    """The description is flanked by blank separator lines. Kills both ``append("")`` ->
    ``append("XXXX")`` mutants, and ``escape(entry.meta.description)`` -> ``escape(None)``
    (which raises before any line is built)."""
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="a", description="hello world")
    lines = tui.MenuApp()._detail_lines(entry)
    di = lines.index("hello world")
    assert lines[di - 1] == ""
    assert lines[di + 1] == ""


def test_detail_lines_no_description_placeholder_is_verbatim(tmp_path):
    """With no description the placeholder renders exactly. Kills its ``gettext(None)``,
    "XX...XX" wrap, "Entry settings" -> "entry settings", and full-uppercase mutants."""
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="a", description="")
    lines = tui.MenuApp()._detail_lines(entry)
    assert "[dim](no description — add one in Entry settings)[/dim]" in lines
