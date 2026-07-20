"""Library-screen behavior: pure helpers, list/detail/footer rendering, empty state.

Behaviour is asserted (widget content, store/argstate mutations, returned values) —
never lines executed for their own sake. Logic-level behavior (plans, prefill,
assembly) lives in test_flows.py; these tests cover the presentation glue.
"""

from __future__ import annotations

import pytest
from textual.widgets import DataTable, Input, Static

from conftest import footer_text
from skit import argstate, store, tui
from skit.langs.python import metawriter
from skit.params import ParamDecl


@pytest.fixture(autouse=True)
def tmp_store(tmp_path, monkeypatch):
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path / "config"))
    monkeypatch.setenv("SKIT_LANG", "en")


def _py(tmp_path, body: str, name: str = "job.py"):
    p = tmp_path / name
    p.write_text(body, encoding="utf-8")
    return p


def _static_text(app, selector) -> str:
    return footer_text(app.query_one(selector, Static))


# ---------------------------------------------------------------------------
# Pure helpers
# ---------------------------------------------------------------------------


def test_fuzzy_match_subsequence():
    assert tui._fuzzy_match("cta", "Create Task")
    assert tui._fuzzy_match("", "anything")  # empty query matches everything
    assert not tui._fuzzy_match("xyz", "Create Task")
    assert tui._fuzzy_match("CT", "create task")  # case-insensitive


def test_activity_sort_puts_latest_activity_first(tmp_path):
    old = store.add_python(_py(tmp_path, "print(1)\n", "old.py"), name="old")
    store.add_python(_py(tmp_path, "print(2)\n", "fresh.py"), name="fresh")
    # A run on the OLD entry is newer activity than the fresh add.
    argstate.record_run(old.slug, 0, at="2099-01-01T00:00:00+00:00")
    entries = sorted(store.list_entries(), key=tui._activity_key, reverse=True)
    assert [e.meta.name for e in entries] == ["old", "fresh"]


def test_relative_time_buckets():
    from datetime import UTC, datetime, timedelta

    now = datetime.now(UTC)
    assert tui._relative_time(now.isoformat()) == "just now"
    assert "min ago" in tui._relative_time((now - timedelta(minutes=10)).isoformat())
    assert "h ago" in tui._relative_time((now - timedelta(hours=5)).isoformat())
    assert "d ago" in tui._relative_time((now - timedelta(days=3)).isoformat())
    assert tui._relative_time("not-a-date") == "not-a-date"  # degrade, don't crash


# ---------------------------------------------------------------------------
# Library rendering
# ---------------------------------------------------------------------------


async def test_empty_state_welcomes_and_points_at_add():
    app = tui.MenuApp()
    async with app.run_test():
        detail = _static_text(app, "#detail-body")
        assert "Your entries will appear here." in detail
        assert "Press a to add the first one," in detail
        assert "skit add <path>" in detail


async def test_list_shows_kind_badges_and_missing_glyph(tmp_path):
    store.add_python(_py(tmp_path, "print(1)\n"), name="pyjob")
    gone = store.add_python(_py(tmp_path, "print(2)\n", "gone.py"), name="gonejob")
    gone.script_path.unlink()
    store.add_command("echo hi", name="cmdjob")
    app = tui.MenuApp()
    async with app.run_test():
        table = app.query_one(DataTable)
        rows = [[str(cell) for cell in table.get_row_at(i)] for i in range(table.row_count)]
        flat = "\n".join(",".join(r) for r in rows)
        assert "⬡ Python" in flat
        assert "$ Command" in flat
        gone_row = next(r for r in rows if r[0] == "gonejob")
        assert gone_row[2] == "⚠"


async def test_search_filters_and_status_counts(tmp_path):
    store.add_python(_py(tmp_path, "print(1)\n", "a.py"), name="alpha")
    store.add_python(_py(tmp_path, "print(2)\n", "b.py"), name="beta")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.query_one("#search", Input).value = "alp"
        await pilot.pause()
        table = app.query_one(DataTable)
        assert table.row_count == 1
        assert "1/2" in _static_text(app, "#status")


async def test_detail_shows_copy_promise_and_masked_secret(tmp_path):
    text = metawriter.write_params(
        'CITY = "x"\nAPI_KEY = "s"\nprint(CITY, API_KEY)\n',
        [
            ParamDecl(name="CITY", binding="const", type="str", default="x"),
            ParamDecl(name="API_KEY", binding="const", type="str", secret=True),
        ],
    )
    store.add_python(_py(tmp_path, text), name="widget")
    app = tui.MenuApp()
    async with app.run_test():
        detail = _static_text(app, "#detail-body")
        assert "your original file is never modified" in detail
        assert "CITY=x" in detail
        assert "API_KEY=•••🔒" in detail
        assert "Not run yet" in detail


async def test_detail_shows_presets_deps_and_last_run(tmp_path):
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="j")
    store.update_dependencies(entry.slug, ["rich"])
    argstate.save_preset(entry.slug, "web", {"X": "1"})
    argstate.record_run(entry.slug, 0, at="2026-07-09T00:00:00+00:00")
    app = tui.MenuApp()
    async with app.run_test():
        detail = _static_text(app, "#detail-body")
        assert "web" in detail
        assert "rich" in detail
        assert "finished" in detail


async def test_detail_command_entry_shows_template(tmp_path):
    store.add_command("echo {msg}", name="e")
    app = tui.MenuApp()
    async with app.run_test():
        assert "echo {msg}" in _static_text(app, "#detail-body")


async def test_footer_rerun_key_is_contextual(tmp_path):
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="j")
    app = tui.MenuApp()
    async with app.run_test():
        assert "Rerun" not in _static_text(app, "#keys-local")  # never run yet
    argstate.record_run(entry.slug, 0, at="2026-07-09T00:00:00+00:00")
    app2 = tui.MenuApp()
    async with app2.run_test():
        assert "Rerun" in _static_text(app2, "#keys-local")


async def test_footer_global_row_lists_every_surface(tmp_path):
    store.add_python(_py(tmp_path, "print(1)\n"), name="j")
    app = tui.MenuApp()
    async with app.run_test():
        row = _static_text(app, "#keys-global")
        for label in (
            "Add entry",
            "Presets",
            "Search",
            "Detail pane",
            "Preferences",
            "Health check",
            "Help",
        ):
            assert label in row


async def test_up_down_forwarded_from_search(tmp_path):
    store.add_python(_py(tmp_path, "print(1)\n", "a.py"), name="alpha")
    store.add_python(_py(tmp_path, "print(2)\n", "b.py"), name="beta")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        table = app.query_one(DataTable)
        assert table.cursor_row == 0
        await pilot.press("down")
        assert table.cursor_row == 1
        await pilot.press("up")
        assert table.cursor_row == 0


async def test_letters_type_as_text_inside_the_search_box(tmp_path, monkeypatch):
    # Focus model: `/` enters the search box, where every letter (including action
    # letters like r) is text; Esc returns to the table.
    store.add_python(_py(tmp_path, "print(1)\n"), name="runner")
    fired: list[str] = []
    monkeypatch.setattr(tui.MenuApp, "action_rerun", lambda self: fired.append("r"))
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.press("slash", "r", "u", "n")
        assert fired == []
        assert app.query_one("#search", Input).value == "run"
        await pilot.press("escape")  # back to the table, not quit
        assert app.focused is app.query_one(DataTable)


# ---------------------------------------------------------------------------
# Every advertised footer key must actually fire from the default focus. The old
# always-focused search box ate all of them, while negative-direction tests alone
# could not prove that the advertised actions were reachable.
# ---------------------------------------------------------------------------


async def test_key_a_opens_the_add_panel(tmp_path):
    from skit.tui_add import AddSourceScreen

    store.add_python(_py(tmp_path, "print(1)\n"), name="j")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.press("a")
        assert isinstance(app.screen, AddSourceScreen)


async def test_key_r_triggers_rerun(tmp_path, monkeypatch):
    store.add_python(_py(tmp_path, "print(1)\n"), name="j")
    fired: list[str] = []
    monkeypatch.setattr(tui.MenuApp, "action_rerun", lambda self: fired.append("r"))
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.press("r")
        assert fired == ["r"]


async def test_key_p_and_s_open_script_settings(tmp_path):
    from skit.tui_settings import ScriptSettingsScreen

    store.add_python(_py(tmp_path, "print(1)\n"), name="j")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.press("p")
        assert isinstance(app.screen, ScriptSettingsScreen)
        await pilot.press("escape")
        await pilot.pause()
        await pilot.press("s")
        assert isinstance(app.screen, ScriptSettingsScreen)


async def test_key_comma_opens_preferences(tmp_path):
    from skit.tui_prefs import PreferencesScreen

    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.press("comma")
        assert isinstance(app.screen, PreferencesScreen)


async def test_key_shift_d_opens_health_check(tmp_path):
    from skit.tui_health import HealthScreen

    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.press("D")
        assert isinstance(app.screen, HealthScreen)


async def test_key_question_mark_opens_help(tmp_path):
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.press("question_mark")
        assert isinstance(app.screen, tui.HelpScreen)


async def test_key_e_opens_the_editor(tmp_path, monkeypatch):
    import contextlib

    opened: list[object] = []
    monkeypatch.setattr(tui.editor, "open_in_editor", opened.append)

    @contextlib.contextmanager
    def _noop(self):
        yield

    monkeypatch.setattr(tui.MenuApp, "suspend", _noop)
    store.add_python(_py(tmp_path, "print(1)\n"), name="j")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.press("e")
        await pilot.pause()
        assert len(opened) == 1


async def test_key_e_opens_shell_script_source(tmp_path, monkeypatch):
    """The `e` key opens interpreted (non-python) sources too — a shell entry's stored
    copy is editable, so pressing `e` hands it to the editor."""
    import contextlib

    opened: list[object] = []
    monkeypatch.setattr(tui.editor, "open_in_editor", opened.append)

    @contextlib.contextmanager
    def _noop(self):
        yield

    monkeypatch.setattr(tui.MenuApp, "suspend", _noop)
    sh = tmp_path / "deploy.sh"
    sh.write_text("#!/bin/bash\necho hi\n", encoding="utf-8")
    store.add_script(sh, kind="shell", name="deploy")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.press("e")
        await pilot.pause()
        assert len(opened) == 1
        assert str(opened[0]).endswith("script.sh")  # the stored shell copy


async def test_enter_in_search_runs_top_match_and_refocuses_table(tmp_path, monkeypatch):
    store.add_python(_py(tmp_path, "print(1)\n"), name="alpha")
    fired: list[str] = []
    monkeypatch.setattr(tui.MenuApp, "action_run", lambda self: fired.append("run"))
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.press("slash", "a", "l", "enter")
        assert fired == ["run"]
        assert app.focused is app.query_one(DataTable)


async def test_letter_keys_do_not_leak_into_pushed_screens(tmp_path):
    # `a` bubbling out of another screen's widget must not open the add panel
    # underneath it (check_action gates the Library actions to the Library).
    from skit.tui_health import HealthScreen

    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.press("D")
        assert isinstance(app.screen, HealthScreen)
        await pilot.press("a")
        assert isinstance(app.screen, HealthScreen)  # still here; nothing opened over it


def test_run_menu_returns_int(monkeypatch):
    monkeypatch.setattr(tui.MenuApp, "run", lambda self: 7)
    assert tui.run_menu() == 7
    monkeypatch.setattr(tui.MenuApp, "run", lambda self: None)
    assert tui.run_menu() == 0
