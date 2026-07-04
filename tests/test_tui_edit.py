"""TUI parameter editor (ctrl+e → EditParams): Textual pilot end-to-end tests.

Behaviour is asserted, not locale copy (convention). Every test is isolated from the real home
directory by tmp_store. Uses the same fixture script as test_edit.py: CITY/RETRIES (two const
candidates), one input candidate, and GONE as a "defined but absent from the script" drift item.
"""

from __future__ import annotations

import pytest

from skit import metawriter, store, tui
from skit.metawriter import ParamSpec

SCRIPT = 'CITY = "Taipei"\nRETRIES = 3\nwho = input("Name: ")\nprint(CITY, RETRIES, who)\n'


@pytest.fixture(autouse=True)
def tmp_store(tmp_path, monkeypatch):
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    monkeypatch.setenv("SKIT_LANG", "en")


def spec(name, kind="const", type="str", order=-1, secret=False, prompt=""):
    return ParamSpec(name=name, kind=kind, type=type, order=order, secret=secret, prompt=prompt)


@pytest.fixture
def entry(tmp_path):
    script = tmp_path / "job.py"
    text = metawriter.write_params(
        SCRIPT, [spec("CITY"), spec("RETRIES", type="int"), spec("GONE")]
    )
    script.write_text(text, encoding="utf-8")
    return store.add_python(script, mode="copy")


def _read_back(entry) -> list[ParamSpec]:
    return metawriter.read_params((entry.dir / "script.py").read_text(encoding="utf-8"))


async def _open_editor(pilot) -> None:
    await pilot.pause()
    await pilot.press("ctrl+e")
    await pilot.pause()


async def test_open_edit_screen_lists_managed_and_new(entry):
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await _open_editor(pilot)
        assert isinstance(app.screen, tui.EditParams)
        # Managed rows (including the drifted GONE) appear first; unmanaged input-1 is appended.
        names = [r.name for r in app.screen._rows]
        assert names == ["CITY", "RETRIES", "GONE", "input-1"]
        assert [r.managed for r in app.screen._rows] == [True, True, True, False]


async def test_toggle_secret_and_save_persists(entry):
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await _open_editor(pilot)
        # Cursor is on the first row (CITY): mark as secret, then save.
        await pilot.press("s")
        await pilot.press("ctrl+s")
        await pilot.pause()
        assert not isinstance(app.screen, tui.EditParams)
    by_name = {s.name: s for s in _read_back(entry)}
    assert by_name["CITY"].secret is True
    assert set(by_name) == {"CITY", "RETRIES", "GONE"}  # no resync, GONE is preserved


async def test_unmanage_removes_definition(entry):
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await _open_editor(pilot)
        await pilot.press("space")  # toggle CITY off
        await pilot.press("ctrl+s")
        await pilot.pause()
    names = [s.name for s in _read_back(entry)]
    assert "CITY" not in names


async def test_manage_new_candidate(entry):
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await _open_editor(pilot)
        for _ in range(3):  # navigate to input-1 (row index 3)
            await pilot.press("down")
        await pilot.press("space")
        await pilot.press("ctrl+s")
        await pilot.pause()
    by_name = {s.name: s for s in _read_back(entry)}
    assert "input-1" in by_name
    assert by_name["input-1"].kind == "input"


async def test_resync_prunes_drifted(entry):
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await _open_editor(pilot)
        await pilot.press("r")  # enable resync
        await pilot.press("ctrl+s")
        await pilot.pause()
    names = {s.name for s in _read_back(entry)}
    assert names == {"CITY", "RETRIES"}  # GONE pruned


async def test_edit_prompt_via_input(entry):
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await _open_editor(pilot)
        await pilot.press("p")
        await pilot.pause()
        for ch in "Where?":
            await pilot.press(ch if ch != "?" else "question_mark")
        await pilot.press("enter")
        await pilot.press("ctrl+s")
        await pilot.pause()
    by_name = {s.name: s for s in _read_back(entry)}
    assert by_name["CITY"].prompt == "Where?"


async def test_escape_cancels_without_writing(entry):
    before = (entry.dir / "script.py").read_text(encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await _open_editor(pilot)
        await pilot.press("s")  # mutate state but don't save
        await pilot.press("escape")
        await pilot.pause()
        assert not isinstance(app.screen, tui.EditParams)
    assert (entry.dir / "script.py").read_text(encoding="utf-8") == before


async def test_command_entry_not_editable():
    store.add_command("echo {x}", name="ec")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await _open_editor(pilot)
        # The edit screen must not open; the status bar shows a hint instead.
        assert not isinstance(app.screen, tui.EditParams)


async def test_reference_entry_not_editable(tmp_path):
    script = tmp_path / "ref.py"
    script.write_text(SCRIPT, encoding="utf-8")
    store.add_python(script, mode="reference")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await _open_editor(pilot)
        assert not isinstance(app.screen, tui.EditParams)
    # The original file must never be modified (A7)
    assert script.read_text(encoding="utf-8") == SCRIPT


# ---------- double Ctrl+C quit (the standard quit gesture) ----------


async def test_ctrl_c_twice_quits(entry):
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        await pilot.press("ctrl+c")
        await pilot.pause()
        assert app.return_value is None  # still running after a single press
        await pilot.press("ctrl+c")
        await pilot.pause()
    assert app.return_value == 0


async def test_ctrl_c_expired_press_does_not_quit(entry):
    """A second Ctrl+C outside the time window counts as a new first press."""
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        await pilot.press("ctrl+c")
        await pilot.pause()
        # Simulate the window having expired instead of sleeping for real.
        app._ctrl_c_at -= app.CTRL_C_WINDOW + 1.0
        await pilot.press("ctrl+c")
        await pilot.pause()
        assert app.return_value is None  # still running
        app.exit(0)


async def test_ctrl_c_twice_quits_while_search_focused(entry):
    """The priority binding must win even when the search Input has focus."""
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        assert app.focused is app.query_one("#search")
        await pilot.press("ctrl+c")
        await pilot.press("ctrl+c")
        await pilot.pause()
    assert app.return_value == 0
