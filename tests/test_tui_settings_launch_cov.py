"""Script settings' launch section (workdir radio, interpreter pin, editable command
template) and the declared editor's Choices/Help fields.

Every test asserts the persisted meta / ParamDecl or the stay-on-error contract, never a
line executed for its own sake.
"""

from __future__ import annotations

import pytest
from textual.widgets import Input, RadioButton, RadioSet

from skit import store, tui
from skit.params import ParamDecl
from skit.tui_settings import DeclParamRow, ScriptSettingsScreen


@pytest.fixture(autouse=True)
def tmp_store(tmp_path, monkeypatch):
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path / "config"))
    monkeypatch.setenv("SKIT_LANG", "en")


def _shell(tmp_path, name="sh"):
    p = tmp_path / f"{name}.sh"
    p.write_text("#!/usr/bin/env bash\necho hi\n", encoding="utf-8")
    return store.add_script(p, kind="shell", name=name)


def _capture_notify(monkeypatch):
    notes: list[str] = []
    monkeypatch.setattr(
        ScriptSettingsScreen, "notify", lambda self, message, **kw: notes.append(message)
    )
    return notes


# ---------------------------------------------------------------- workdir radio


async def test_workdir_radio_persists_a_literal(tmp_path):
    _shell(tmp_path)  # default workdir = invoke
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(store.resolve("sh"))
        app.push_screen(screen)
        await pilot.pause()
        wd = screen.query_one("#st-workdir", RadioSet)
        assert wd.pressed_index == 2  # "Wherever skit is run from" (invoke)
        assert screen.query_one("#st-workdir-path", Input).display is False
        next(iter(wd.query(RadioButton))).value = True  # "The script's own folder" (origin)
        await pilot.pause()
        screen.action_save()
        await pilot.pause()
    assert store.resolve("sh").meta.workdir == "origin"


async def test_workdir_custom_path_reveals_input_and_saves(tmp_path):
    _shell(tmp_path)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(store.resolve("sh"))
        app.push_screen(screen)
        await pilot.pause()
        list(screen.query_one("#st-workdir", RadioSet).query(RadioButton))[3].value = True
        await pilot.pause()
        path_input = screen.query_one("#st-workdir-path", Input)
        assert path_input.display is True  # the 4th option reveals the path field
        path_input.value = "/opt/data"
        screen.action_save()
        await pilot.pause()
    assert store.resolve("sh").meta.workdir == "/opt/data"


async def test_workdir_custom_empty_keeps_stored(tmp_path):
    _shell(tmp_path)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(store.resolve("sh"))
        app.push_screen(screen)
        await pilot.pause()
        list(screen.query_one("#st-workdir", RadioSet).query(RadioButton))[3].value = True
        await pilot.pause()
        screen.query_one("#st-workdir-path", Input).value = "   "  # nothing typed
        screen.action_save()
        await pilot.pause()
    assert store.resolve("sh").meta.workdir == "invoke"  # kept, not guessed


async def test_workdir_relative_path_notifies_and_stays(tmp_path, monkeypatch):
    notes = _capture_notify(monkeypatch)
    _shell(tmp_path)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(store.resolve("sh"))
        app.push_screen(screen)
        await pilot.pause()
        list(screen.query_one("#st-workdir", RadioSet).query(RadioButton))[3].value = True
        await pilot.pause()
        screen.query_one("#st-workdir-path", Input).value = "rel/ative"
        screen.action_save()
        await pilot.pause()
        assert isinstance(app.screen, ScriptSettingsScreen)  # stayed on the screen
    assert any("origin, store, invoke, or an absolute path" in m for m in notes)
    assert store.resolve("sh").meta.workdir == "invoke"  # nothing persisted


# ---------------------------------------------------------------- interpreter pin


async def test_interpreter_input_sets_then_clears(tmp_path):
    _shell(tmp_path)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(store.resolve("sh"))
        app.push_screen(screen)
        await pilot.pause()
        screen.query_one("#st-interpreter", Input).value = "zsh"
        screen.action_save()
        await pilot.pause()
    assert store.resolve("sh").meta.interpreter == "zsh"

    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(store.resolve("sh"))
        app.push_screen(screen)
        await pilot.pause()
        assert screen.query_one("#st-interpreter", Input).value == "zsh"  # prefilled
        screen.query_one("#st-interpreter", Input).value = ""
        screen.action_save()
        await pilot.pause()
    assert store.resolve("sh").meta.interpreter == ""


# ---------------------------------------------------------------- editable template


async def test_template_input_saves_via_update_template(tmp_path):
    store.add_command("echo {old}", name="cmd")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(store.resolve("cmd"))
        app.push_screen(screen)
        await pilot.pause()
        screen.query_one("#st-template", Input).value = "convert {size} {out}"
        screen.action_save()
        await pilot.pause()
        assert not isinstance(app.screen, ScriptSettingsScreen)  # saved & dismissed
    entry = store.resolve("cmd")
    assert entry.meta.template == "convert {size} {out}"
    assert entry.meta.params == ["size", "out"]  # re-read from the new template


async def test_empty_template_notifies_and_stays(tmp_path, monkeypatch):
    notes = _capture_notify(monkeypatch)
    store.add_command("echo {x}", name="cmd")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(store.resolve("cmd"))
        app.push_screen(screen)
        await pilot.pause()
        screen.query_one("#st-template", Input).value = "   "
        screen.action_save()
        await pilot.pause()
        assert isinstance(app.screen, ScriptSettingsScreen)  # stayed
    assert notes  # the empty-template error was raised
    assert store.resolve("cmd").meta.template == "echo {x}"  # unchanged


# ---------------------------------------------------------------- Choices / Help fields


async def test_declared_choices_and_help_round_trip(tmp_path):
    tool = tmp_path / "prog"
    tool.write_text("#!/bin/sh\necho hi\n", encoding="utf-8")
    entry = store.add_exe(tool, name="prog")
    entry = store.write_parameters(
        entry.slug, [ParamDecl(name="fmt", delivery="flag", flag="--fmt", type="str")]
    )
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(entry)
        app.push_screen(screen)
        await pilot.pause()
        row = screen.query(DeclParamRow).first()
        row.query_one(".d-type", Input).value = "choice"
        row.query_one(".d-choices", Input).value = "png, jpg , webp"  # spaces stripped
        row.query_one(".d-help", Input).value = "the output format"
        await pilot.pause()
        screen.action_save()
        await pilot.pause()
        assert not isinstance(app.screen, ScriptSettingsScreen)  # valid → saved
    d = store.read_parameters(entry.slug)[0]
    assert d.choices == ("png", "jpg", "webp")
    assert d.help == "the output format"


async def test_declared_choice_empty_choices_shows_new_message(tmp_path, monkeypatch):
    notes = _capture_notify(monkeypatch)
    tool = tmp_path / "prog"
    tool.write_text("#!/bin/sh\necho hi\n", encoding="utf-8")
    entry = store.add_exe(tool, name="prog")
    entry = store.write_parameters(entry.slug, [ParamDecl(name="a", delivery="flag", type="str")])
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(entry)
        app.push_screen(screen)
        await pilot.pause()
        screen.query(DeclParamRow).first().query_one(".d-type", Input).value = "choice"
        # leave Choices empty
        await pilot.pause()
        screen.action_save()
        await pilot.pause()
        assert isinstance(app.screen, ScriptSettingsScreen)  # rejected, stays
    assert any("fill its Choices field" in m for m in notes)


# ---------------------------------------------------------------- unknown kind (defensive)


async def test_save_launch_noop_for_unknown_kind(tmp_path):
    # A meta.toml kind a newer skit defined and this one doesn't recognize: the launch
    # section offers no policies, and _save_launch returns True without touching anything.
    p = tmp_path / "x.py"
    p.write_text("print(1)\n", encoding="utf-8")
    entry = store.add_python(p, name="x")
    meta_path = entry.dir / "meta.toml"
    meta_path.write_text(
        meta_path.read_text(encoding="utf-8").replace('kind = "python"', 'kind = "mystery"'),
        encoding="utf-8",
    )
    reloaded = store.resolve("x")
    assert reloaded.meta.kind == "mystery"
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(reloaded)
        app.push_screen(screen)
        await pilot.pause()
        assert not screen.query("#st-workdir")  # no launch policies offered
        screen.action_save()
        await pilot.pause()
        assert not isinstance(app.screen, ScriptSettingsScreen)  # saved & dismissed cleanly
