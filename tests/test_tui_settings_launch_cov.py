"""Entry settings' launch section (workdir radio, interpreter pin, editable command
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
        wd = str(tmp_path / "wd")  # absolute on every platform (a Unix "/opt" is not, on Windows)
        path_input.value = wd
        screen.action_save()
        await pilot.pause()
    assert store.resolve("sh").meta.workdir == wd


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


# ------------------------------------------------------- workdir options are kind-aware


def _exe(tmp_path, name="ex"):
    import os

    p = tmp_path / f"{name}.bin"
    p.write_text("opaque\n", encoding="utf-8")
    os.chmod(p, 0o755)  # noqa: S103
    return store.add_exe(p, name=name)


async def _workdir_shape(entry):
    """Open Entry settings for an entry and read back the workdir radio's value-keyed
    choices and the number of buttons rendered."""
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        screen = ScriptSettingsScreen(entry)
        app.push_screen(screen)
        await pilot.pause()
        choices = list(screen._workdir_choices)
        count = len(list(screen.query_one("#st-workdir", RadioSet).query(RadioButton)))
    return choices, count


async def test_workdir_options_shell_has_all_four(tmp_path):
    _shell(tmp_path)
    choices, count = await _workdir_shape(store.resolve("sh"))
    assert choices == ["origin", "store", "invoke", "custom"]  # a real file with a stored copy
    assert count == 4


async def test_workdir_options_command_drops_origin_and_store(tmp_path):
    store.add_command("echo hi", name="cmd")
    choices, count = await _workdir_shape(store.resolve("cmd"))
    # A command template has no file (no origin) and no stored copy (no store).
    assert choices == ["invoke", "custom"]
    assert count == 2


async def test_workdir_options_exe_drops_store_only(tmp_path):
    _exe(tmp_path)
    choices, count = await _workdir_shape(store.resolve("ex"))
    # An exe references a real program (origin) but is never copied (no store).
    assert choices == ["origin", "invoke", "custom"]
    assert "store" not in choices
    assert count == 3


async def test_command_workdir_saves_custom_path_from_index_one(tmp_path):
    """Value-keyed save: for a command the custom option is at index 1 (not the shell's
    index 3), and it must still resolve to the typed path — not a hardcoded slot."""
    store.add_command("echo hi", name="cmd")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        screen = ScriptSettingsScreen(store.resolve("cmd"))
        app.push_screen(screen)
        await pilot.pause()
        buttons = list(screen.query_one("#st-workdir", RadioSet).query(RadioButton))
        assert len(buttons) == 2
        buttons[1].value = True  # "A fixed folder" — index 1 for a command
        await pilot.pause()
        assert screen.query_one("#st-workdir-path", Input).display is True
        wd = str(tmp_path / "wd")  # absolute on every platform (a Unix "/opt" is not, on Windows)
        screen.query_one("#st-workdir-path", Input).value = wd
        screen.action_save()
        await pilot.pause()
    assert store.resolve("cmd").meta.workdir == wd


async def test_command_workdir_saves_invoke_from_index_zero(tmp_path):
    store.add_command("echo hi", name="cmd")
    # A non-invoke seed to switch away from; absolute on every platform.
    store.write_workdir(store.resolve("cmd").slug, str(tmp_path / "seed"))
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        screen = ScriptSettingsScreen(store.resolve("cmd"))
        app.push_screen(screen)
        await pilot.pause()
        buttons = list(screen.query_one("#st-workdir", RadioSet).query(RadioButton))
        buttons[0].value = True  # "Wherever skit is run from" (invoke) — index 0
        await pilot.pause()
        screen.action_save()
        await pilot.pause()
    assert store.resolve("cmd").meta.workdir == "invoke"


async def test_exe_workdir_saves_custom_from_index_two(tmp_path):
    """The exe custom slot is index 2 (no store option between origin and invoke) — the
    value-keyed mapping keeps it honest."""
    _exe(tmp_path)
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        screen = ScriptSettingsScreen(store.resolve("ex"))
        app.push_screen(screen)
        await pilot.pause()
        buttons = list(screen.query_one("#st-workdir", RadioSet).query(RadioButton))
        assert len(buttons) == 3
        buttons[2].value = True  # custom is index 2 for an exe
        await pilot.pause()
        assert screen.query_one("#st-workdir-path", Input).display is True
        wd = str(tmp_path / "wd")  # absolute on every platform (a Unix "/opt" is not, on Windows)
        screen.query_one("#st-workdir-path", Input).value = wd
        screen.action_save()
        await pilot.pause()
    assert store.resolve("ex").meta.workdir == wd


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


# ------------------------------------------------ validate-then-write ordering


async def test_save_refuses_invalid_workdir_before_the_rename(tmp_path, monkeypatch):
    """action_save validates EVERYTHING before writing anything: an invalid custom workdir
    refuses BEFORE the rename/description writes, so a new name typed alongside it is NEVER
    persisted (the half-commit the Esc guard's "unsaved changes" would otherwise lie about)."""
    _capture_notify(monkeypatch)
    _shell(tmp_path)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(store.resolve("sh"))
        app.push_screen(screen)
        await pilot.pause()
        screen.query_one("#st-name", Input).value = "renamed"  # a new name rides along
        list(screen.query_one("#st-workdir", RadioSet).query(RadioButton))[3].value = True
        await pilot.pause()
        screen.query_one("#st-workdir-path", Input).value = "rel/ative"  # invalid
        screen.action_save()
        await pilot.pause()
        assert isinstance(app.screen, ScriptSettingsScreen)  # stayed on the screen
    assert store.resolve("sh").meta.name == "sh"  # the rename never ran
    with pytest.raises(store.NotFoundError):
        store.resolve("renamed")


async def test_save_refuses_empty_template_before_the_rename(tmp_path, monkeypatch):
    """An empty command template refuses before the rename write — a new name alongside it
    is not persisted, and the template is left as it was."""
    _capture_notify(monkeypatch)
    store.add_command("echo {x}", name="cmd")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(store.resolve("cmd"))
        app.push_screen(screen)
        await pilot.pause()
        screen.query_one("#st-name", Input).value = "renamed"
        screen.query_one("#st-template", Input).value = "   "  # empty is not a program
        screen.action_save()
        await pilot.pause()
        assert isinstance(app.screen, ScriptSettingsScreen)
    entry = store.resolve("cmd")
    assert entry.meta.name == "cmd"  # the rename never ran
    assert entry.meta.template == "echo {x}"  # template unchanged


async def test_save_refuses_invalid_declared_row_before_the_rename(tmp_path, monkeypatch):
    """An invalid declared row (a choice type with no Choices) refuses before the rename
    write — a new name alongside it is not persisted."""
    _capture_notify(monkeypatch)
    tool = tmp_path / "prog"
    tool.write_text("#!/bin/sh\necho hi\n", encoding="utf-8")
    entry = store.add_exe(tool, name="prog")
    entry = store.write_parameters(entry.slug, [ParamDecl(name="a", delivery="flag", type="str")])
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(entry)
        app.push_screen(screen)
        await pilot.pause()
        screen.query_one("#st-name", Input).value = "renamed"
        screen.query(DeclParamRow).first().query_one(".d-type", Input).value = "choice"
        # leave Choices empty → the row is invalid
        await pilot.pause()
        screen.action_save()
        await pilot.pause()
        assert isinstance(app.screen, ScriptSettingsScreen)
    assert store.resolve("prog").meta.name == "prog"  # the rename never ran


async def test_save_rename_conflict_aborts_before_the_description_write(tmp_path, monkeypatch):
    """The rename is the FIRST write (its name-conflict failure can't be pre-checked), so it
    aborts ALONE: a description typed alongside a conflicting rename is NOT persisted after
    the rename fails — zero prior writes, exactly the method's docstring promise."""
    _capture_notify(monkeypatch)
    _shell(tmp_path, name="sh")
    _shell(tmp_path, name="taken")  # the conflicting name already exists
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(store.resolve("sh"))
        app.push_screen(screen)
        await pilot.pause()
        screen.query_one("#st-name", Input).value = "taken"  # collides with the other entry
        screen.query_one("#st-desc", Input).value = "a fresh description"
        screen.action_save()
        await pilot.pause()
        assert isinstance(app.screen, ScriptSettingsScreen)  # stayed on the screen
    entry = store.resolve("sh")
    assert entry.meta.name == "sh"  # the rename failed…
    assert entry.meta.description == ""  # …and the description was NOT written half-way


async def test_save_launch_noop_for_unknown_kind(tmp_path):
    # A meta.toml kind a newer skit defined and this one doesn't recognize: the launch
    # section offers no policies, and _write_launch returns without touching anything.
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
