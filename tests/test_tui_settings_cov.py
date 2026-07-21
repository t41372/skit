"""Exact-behavior tests for the merged Entry settings screen (tui_settings.py).

Behaviour is asserted — the written [tool.skit] copy, store/argstate mutations, the
rendered section/hint text, resync warnings — never a line executed for its own sake.
The heavy save/load logic lives in store/argstate/metawriter (tested there); these cover
the settings screen's presentation glue and its one atomic save.
"""

from __future__ import annotations

import pytest
from textual.widgets import Checkbox, Input, Static

from skit import argstate, store, tui
from skit.langs.python import metawriter, reconcile
from skit.langs.registry import spec_for
from skit.params import ParamDecl
from skit.tui_settings import (
    DeclParamRow,
    DiscardChangesModal,
    ParamRow,
    ScriptSettingsScreen,
)


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


def _body(screen) -> str:
    """All Static/Label text on the screen (bullets, hints, section headers)."""
    return " ".join(str(w.render()) for w in screen.query(Static))


# ---------------------------------------------------------------------------
# ParamRow.collect() + the secret-transition note (managed-parameter editing)
# ---------------------------------------------------------------------------


async def test_save_collects_managed_rows_drops_unticked_and_scrubs_secret(tmp_path, monkeypatch):
    """The one atomic save: collect() keeps ticked rows, drops an unticked one, records the
    per-row prompt / secret / env source, and purge_secret scrubs a value remembered while the
    parameter was still public. Description and dependency edits ride the same save."""
    text = metawriter.write_params(
        'CITY = "Taipei"\nAPI_KEY = "k"\nOLD = "o"\nprint(CITY, API_KEY, OLD)\n',
        [
            ParamDecl(name="CITY", binding="const", type="str", default="Taipei"),
            ParamDecl(name="API_KEY", binding="const", type="str", default="k", secret=True),
            ParamDecl(name="OLD", binding="const", type="str", default="o"),
        ],
    )
    entry = store.add_python(_py(tmp_path, text), name="cfg")
    # A plaintext value remembered while API_KEY was public — must be scrubbed on save.
    argstate.save_last(entry.slug, values={"API_KEY": "topsecret", "CITY": "Taipei"})

    notes: list[str] = []
    monkeypatch.setattr(
        ScriptSettingsScreen, "notify", lambda self, message, **kw: notes.append(message)
    )

    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(entry)
        app.push_screen(screen)
        await pilot.pause()
        rows = {row.spec.name: row for row in screen.query(ParamRow)}

        rows["OLD"].query(Checkbox).first().value = False  # unmanage OLD → collect() returns None
        rows["CITY"].query_one(".p-prompt", Input).value = "City name"  # custom form prompt
        rows["API_KEY"].query_one(".p-env", Input).value = "MY_ENV"  # secret env source
        screen.query_one("#st-desc", Input).value = "new desc"
        screen.query_one("#st-deps", Input).value = "rich"
        await pilot.pause()

        screen.action_save()
        await pilot.pause()
        assert not isinstance(app.screen, ScriptSettingsScreen)  # dismissed after the save

    written = metawriter.read_params((entry.dir / "script.py").read_text(encoding="utf-8"))
    by_name = {s.name: s for s in written}
    assert set(by_name) == {"CITY", "API_KEY"}  # OLD was dropped (unticked)
    assert by_name["CITY"].prompt == "City name"
    assert by_name["CITY"].secret is False
    assert by_name["CITY"].env_source == ""  # non-secret rows never carry an env source
    assert by_name["API_KEY"].secret is True
    assert by_name["API_KEY"].env_source == "MY_ENV"

    assert store.resolve("cfg").meta.description == "new desc"
    assert store.resolve("cfg").meta.dependencies == ["rich"]
    assert "API_KEY" not in argstate.load_state(entry.slug)["values"]  # scrubbed
    assert "CITY" in argstate.load_state(entry.slug)["values"]  # a public value is untouched
    assert any("Deleted previously remembered value(s): API_KEY" in m for m in notes)


async def test_secret_checkbox_warns_then_clears_the_note(tmp_path):
    """Ticking secret on a previously-public parameter warns that its remembered value will be
    deleted; unticking clears the note again."""
    text = metawriter.write_params(
        'CITY = "x"\nprint(CITY)\n',
        [ParamDecl(name="CITY", binding="const", type="str", default="x")],
    )
    entry = store.add_python(_py(tmp_path, text), name="note")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(entry)
        app.push_screen(screen)
        await pilot.pause()
        row = screen.query(ParamRow).first()
        note = row.query_one(".p-note", Static)

        row.query_one(".p-secret", Checkbox).value = True
        await pilot.pause()
        assert "previously remembered" in str(note.render())

        row.query_one(".p-secret", Checkbox).value = False
        await pilot.pause()
        assert str(note.render()) == ""  # note cleared


async def test_managed_public_to_secret_drops_cached_default(tmp_path):
    text = metawriter.write_params(
        'CITY = "source-secret"\nprint(CITY)\n',
        [ParamDecl(name="CITY", binding="const", type="str", default="cached-public")],
    )
    entry = store.add_python(_py(tmp_path, text), name="secret-default")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(entry)
        app.push_screen(screen)
        await pilot.pause()
        screen.query(ParamRow).first().query_one(".p-secret", Checkbox).value = True
        screen.action_save()
        await pilot.pause()

    (written,) = metawriter.read_params(entry.script_path.read_text(encoding="utf-8"))
    assert written.secret is True
    assert written.default is None


# ---------------------------------------------------------------------------
# Detected-but-unmanaged candidates: the manage-these checkboxes + save
# ---------------------------------------------------------------------------


async def test_detected_candidate_can_be_ticked_into_management(tmp_path):
    """A copy-mode script with a bare constant offers a "tick to manage" checkbox; ticking it and
    saving writes that constant into the [tool.skit] block."""
    entry = store.add_python(_py(tmp_path, 'CITY = "Taipei"\nprint(CITY)\n'), name="detect")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(entry)
        app.push_screen(screen)
        await pilot.pause()
        assert "Detected but not yet managed" in _body(screen)
        box = screen.query_one("#st-new-0", Checkbox)
        assert "CITY" in str(box.label)
        box.value = True
        await pilot.pause()
        screen.action_save()
        await pilot.pause()

    written = metawriter.read_params((entry.dir / "script.py").read_text(encoding="utf-8"))
    assert [s.name for s in written] == ["CITY"]  # the ticked candidate is now managed


async def test_all_inputs_managed_shows_no_input_promise(tmp_path):
    """When every managed parameter is an input(), the screen tells the user the script can now run
    under --no-input."""
    body = 'NAME = input("Name? ")\nAGE = input("Age? ")\nprint(NAME, AGE)\n'
    text = metawriter.write_params(
        body,
        [
            ParamDecl(name="input-1", binding="input", type="str", prompt="Name? ", order=0),
            ParamDecl(name="input-2", binding="input", type="str", prompt="Age? ", order=1),
        ],
    )
    entry = store.add_python(_py(tmp_path, text), name="inputs")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(entry)
        app.push_screen(screen)
        await pilot.pause()
        assert "Every input() is managed" in _body(screen)


# ---------------------------------------------------------------------------
# Presets section: delete-on-untick
# ---------------------------------------------------------------------------


async def test_untick_preset_deletes_it_on_save(tmp_path):
    """Each preset is a ticked checkbox; unticking one and saving deletes exactly that preset."""
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="pre")
    argstate.save_preset(entry.slug, "alpha", {"X": "1"})
    argstate.save_preset(entry.slug, "beta", {"Y": "2"})
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(entry)
        app.push_screen(screen)
        await pilot.pause()
        assert "Untick a preset to delete it on save" in _body(screen)
        alpha_box = screen.query_one("#st-preset-0", Checkbox)  # sorted: alpha, beta
        assert "alpha" in str(alpha_box.label)
        alpha_box.value = False  # mark alpha for deletion
        await pilot.pause()
        screen.action_save()
        await pilot.pause()

    assert argstate.load_state(entry.slug)["presets"] == {"beta": {"Y": "2"}}  # only alpha gone


async def test_untick_preset_is_name_keyed_against_a_concurrent_add(tmp_path):
    """The delete pass maps checkbox indices through the names captured AT COMPOSE TIME,
    not a fresh state read. A preset added concurrently (another `skit preset` — the
    product's own agent-coexistence story) must not shift which name an untick deletes:
    unticking beta deletes beta, even when a new name sorts ahead of everything shown."""
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="pre")
    argstate.save_preset(entry.slug, "alpha", {"X": "1"})
    argstate.save_preset(entry.slug, "beta", {"Y": "2"})
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(entry)
        app.push_screen(screen)
        await pilot.pause()
        beta_box = screen.query_one("#st-preset-1", Checkbox)  # composed order: alpha, beta
        assert "beta" in str(beta_box.label)
        # A concurrent add of a name that sorts BEFORE the composed list — the old buggy
        # save (re-reading + re-sorting state) would have shifted index 1 onto "alpha".
        argstate.save_preset(entry.slug, "aardvark", {"Z": "3"})
        beta_box.value = False  # untick beta for deletion
        await pilot.pause()
        screen.action_save()
        await pilot.pause()

    survivors = argstate.load_state(entry.slug)["presets"]
    assert set(survivors) == {"alpha", "aardvark"}  # exactly beta gone; the concurrent add survives


# ---------------------------------------------------------------------------
# Non-python / reference storage: early returns and read-only param views
# ---------------------------------------------------------------------------


async def test_command_entry_shows_template_hides_storage_and_deps(tmp_path):
    """A command entry has no stored file: no Storage/Dependencies sections, its template is shown
    read-only above the declared editor's add-a-parameter field, resync is a no-op, and save still
    dismisses cleanly."""
    entry = store.add_command("echo {msg} {name}", name="cmd")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(entry)
        app.push_screen(screen)
        await pilot.pause()
        body = _body(screen)
        # The template is EDITABLE now (it is the program; freezing it forced
        # remove + re-add over a typo).
        assert screen.query_one("#st-template", Input).value == "echo {msg} {name}"
        assert screen.query("#st-add-param")  # the declared editor's add-a-parameter field
        assert "Storage" not in body  # storage section skipped for a non-python entry
        assert not screen.query("#st-deps")  # dependencies section skipped too

        screen.action_resync()  # kind != python → no-op, no report produced
        assert screen._resync_report == ""

        screen.query_one("#st-desc", Input).value = "cmd desc"
        await pilot.pause()
        screen.action_save()
        await pilot.pause()
        assert not isinstance(app.screen, ScriptSettingsScreen)  # saved & dismissed

    assert store.resolve("cmd").meta.description == "cmd desc"


async def test_ctrl_a_in_a_focused_input_moves_home_without_saving(tmp_path, monkeypatch):
    """The exact regression that forced Save from Ctrl+A to Ctrl+S: Ctrl+A is every Input's
    cursor-home, so on a screen full of Inputs it must move the cursor to the start and NOT
    save/close. The save chord now lives on Ctrl+S, so Ctrl+A belongs entirely to the Input."""
    shell_src = tmp_path / "s.sh"
    shell_src.write_text("#!/usr/bin/env bash\necho hi\n", encoding="utf-8")
    store.add_script(shell_src, kind="shell", name="sh")
    saved: list[int] = []
    monkeypatch.setattr(ScriptSettingsScreen, "action_save", lambda self: saved.append(1))
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(store.resolve("sh"))
        app.push_screen(screen)
        await pilot.pause()
        desc = screen.query_one("#st-desc", Input)
        desc.focus()
        desc.value = "hello world"
        desc.cursor_position = len(desc.value)
        await pilot.pause()
        await pilot.press("ctrl+a")
        await pilot.pause()
        assert desc.cursor_position == 0  # Ctrl+A moved the cursor home (the Input owns it)
        assert saved == []  # …and did NOT save
        assert isinstance(app.screen, ScriptSettingsScreen)  # still open, nothing dismissed


async def test_resync_chip_only_where_resync_can_act(tmp_path):
    """The Resync chip renders only where action_resync actually does something — a
    copy-mode analyzable entry (python/shell). For a command/exe/prompt entry resync is a
    no-op, so advertising the key would teach a dead chord (the mouse's click path must not
    point at a no-op)."""
    # copy-mode shell: the chip is advertised
    shell_src = tmp_path / "s.sh"
    shell_src.write_text("#!/usr/bin/env bash\necho hi\n", encoding="utf-8")
    store.add_script(shell_src, kind="shell", name="sh")
    # a command entry: resync is a no-op, so no chip
    store.add_command("echo {msg}", name="cmd")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        shell_screen = ScriptSettingsScreen(store.resolve("sh"))
        app.push_screen(shell_screen)
        await pilot.pause()
        assert "Resync" in str(shell_screen.query_one("#st-keys", Static).render())
        shell_screen.dismiss(False)
        await pilot.pause()

        cmd_screen = ScriptSettingsScreen(store.resolve("cmd"))
        app.push_screen(cmd_screen)
        await pilot.pause()
        assert "Resync" not in str(cmd_screen.query_one("#st-keys", Static).render())


async def test_exe_entry_shows_the_declared_params_editor(tmp_path):
    """An executable ("program") entry has no managed parameters in-file, but its declared schema is
    editable: the screen offers the add-a-parameter field (no rows until one is declared)."""
    tool = tmp_path / "mytool"
    tool.write_text("#!/bin/sh\necho hi\n", encoding="utf-8")
    entry = store.add_exe(tool, name="prog")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(entry)
        app.push_screen(screen)
        await pilot.pause()
        assert screen.query("#st-add-param")  # the declared editor is present
        assert not screen.query(DeclParamRow)  # no declared parameters yet


async def test_unknown_kind_entry_states_no_managed_parameters(tmp_path):
    """A forward-compat entry whose kind this skit version doesn't know (spec_for → None) shows the
    plain no-managed-parameters note in settings rather than crashing."""
    from skit.models import ScriptMeta

    store._add_entry(ScriptMeta(name="alien", kind="martian"), payload=None)
    entry = store.resolve("alien")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(entry)
        app.push_screen(screen)
        await pilot.pause()
        assert "programs have no managed parameters" in _body(screen)


async def test_reference_entry_is_read_only_and_linked(tmp_path):
    """A reference-mode python entry shows the original path as linked (never a copy) and lists its
    parameters read-only — skit never writes the source (A7)."""
    src = _py(
        tmp_path,
        metawriter.write_params(
            'CITY = "x"\nprint(CITY)\n',
            [ParamDecl(name="CITY", binding="const", type="str", default="x")],
        ),
        "orig.py",
    )
    entry = store.add_python(src, name="ref", mode="reference")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(entry)
        app.push_screen(screen)
        await pilot.pause()
        body = _body(screen)
        assert "Linked to the original" in body
        assert str(src) in body  # the source path is shown
        assert "skit doesn't write to this file" in body  # read-only maintenance note
        assert "· CITY (str)" in body  # parameter listed read-only


# ---------------------------------------------------------------------------
# Resync: safety-rebind / drift warnings survive the recompose
# ---------------------------------------------------------------------------


async def test_resync_reports_a_dropped_definition(tmp_path):
    """Resyncing a copy whose managed constant no longer exists drops it and surfaces the warning in
    the (recomposed) report line."""
    text = metawriter.write_params(
        'CITY = "x"\nprint(CITY)\n',
        [
            ParamDecl(name="CITY", binding="const", type="str", default="x"),
            ParamDecl(name="GONE", binding="const", type="str", default="y"),
        ],
    )
    entry = store.add_python(_py(tmp_path, text), name="drift")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(entry)
        app.push_screen(screen)
        await pilot.pause()
        screen.action_resync()
        await pilot.pause()
        report = str(screen.query_one("#st-resync-report", Static).render())
        assert "Dropped GONE" in report
        assert "no longer exists in the script" in report


# ---------------------------------------------------------------------------
# Esc with unsaved changes → "keep editing" leaves the screen open
# ---------------------------------------------------------------------------


async def test_discard_modal_keep_editing_stays_on_the_screen(tmp_path):
    """Esc on a dirty screen asks; choosing "keep editing" dismisses the modal only — the settings
    screen (and the pending edit) survives."""
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="keep")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(entry)
        app.push_screen(screen)
        await pilot.pause()
        desc = screen.query_one("#st-desc", Input)
        desc.focus()
        await pilot.pause()
        desc.value = "edited"
        await pilot.pause()
        await pilot.press("escape")
        await pilot.pause()
        assert isinstance(app.screen, DiscardChangesModal)
        await pilot.press("n")  # keep editing
        await pilot.pause()
        assert app.screen is screen  # back on settings, not dismissed
        assert screen.query_one("#st-desc", Input).value == "edited"  # edit preserved


# ---------------------------------------------------------------------------
# Defensive guards (unreachable through normal composition; exercised directly so a
# future refactor that removes the guard is caught).
# ---------------------------------------------------------------------------


async def test_save_tolerates_reconcile_returning_nothing(tmp_path, monkeypatch):
    """Defensive: action_save guards `if report is not None` even though reconcile always returns a
    Report in this path. Forcing None must still write the managed copy and dismiss, never crash."""
    text = metawriter.write_params(
        'CITY = "x"\nprint(CITY)\n',
        [ParamDecl(name="CITY", binding="const", type="str", default="x")],
    )
    entry = store.add_python(_py(tmp_path, text), name="nonerep")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(entry)
        app.push_screen(screen)
        await pilot.pause()
        # Force the defensive `report is None` path for the synchronous save only, then restore it
        # before the pause so the Library's post-dismiss re-plan uses the real reconcile.
        real = reconcile.reconcile
        monkeypatch.setattr(reconcile, "reconcile", lambda _text, _specs: None)
        screen.action_save()
        monkeypatch.setattr(reconcile, "reconcile", real)
        await pilot.pause()
        assert not isinstance(app.screen, ScriptSettingsScreen)  # dismissed, not crashed

    written = metawriter.read_params((entry.dir / "script.py").read_text(encoding="utf-8"))
    assert [s.name for s in written] == ["CITY"]  # the managed copy was still written


async def test_presets_deeplink_tolerates_a_missing_presets_section(tmp_path):
    """Defensive: the `s` deep-link scrolls to the Presets section only `if section:`. When that
    section is absent, on_mount must skip the scroll silently rather than raise NoMatches."""
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="deeplink")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(entry, initial_section="presets")
        app.push_screen(screen)
        await pilot.pause()  # first on_mount: section present → scroll scheduled
        await screen.query_one("#st-presets-section", Static).remove()
        await pilot.pause()
        assert not screen.query("#st-presets-section")
        screen.on_mount()  # section now absent → the `if section:` guard skips the scroll
        await pilot.pause()


# ---------------------------------------------------------------------------
# Declared-schema editor (exe / command entries: meta.toml [[parameters]])
# ---------------------------------------------------------------------------


def _exe(tmp_path, name="prog"):
    tool = tmp_path / "mytool"
    tool.write_text("#!/bin/sh\necho hi\n", encoding="utf-8")
    tool.chmod(0o755)
    return store.add_exe(tool, name=name)


async def test_exe_declared_editor_shows_rows_and_saves_edits(tmp_path):
    """An exe with declared flag params shows one editable row each (with a Flag field), and one
    atomic save writes the edited type/default/flag/required/prompt back to meta.toml."""
    entry = _exe(tmp_path)
    entry = store.write_parameters(
        entry.slug,
        [ParamDecl(name="width", delivery="flag", flag="--width", type="int", default=800)],
    )
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(entry)
        app.push_screen(screen)
        await pilot.pause()
        row = screen.query(DeclParamRow).first()
        assert row.query(".d-flag")  # binary kinds expose the Flag field
        assert row.query_one(".d-default", Input).value == "800"  # non-bool default rendered
        row.query_one(".d-type", Input).value = "float"
        row.query_one(".d-default", Input).value = "1.5"
        row.query_one(".d-flag", Input).value = "--w"
        row.query_one(".d-required", Checkbox).value = True
        row.query_one(".p-prompt", Input).value = "Width?"
        await pilot.pause()
        screen.action_save()
        await pilot.pause()
        assert not isinstance(app.screen, ScriptSettingsScreen)  # saved & dismissed

    written = store.read_parameters(entry.slug)
    d = written[0]
    assert (d.type, d.default, d.flag, d.required, d.prompt) == (
        "float",
        1.5,
        "--w",
        True,
        "Width?",
    )


async def test_command_declared_placeholder_row_has_no_flag_field(tmp_path):
    """A command's declared placeholder row omits the Flag field (argv is not a template's
    interface), and its delivery is shown read-only in the header."""
    entry = store.add_command("convert {size}", name="conv")
    entry = store.write_parameters(
        entry.slug, [ParamDecl(name="size", delivery="placeholder", type="str")]
    )
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(entry)
        app.push_screen(screen)
        await pilot.pause()
        assert screen.query_one("#st-template", Input).value == "convert {size}"
        row = screen.query(DeclParamRow).first()
        assert not row.query(".d-flag")  # template kinds: no flag field
        assert "placeholder" in str(row.query_one(".d-keep", Checkbox).label)
        # collect() runs with show_flag=False here — editing + saving persists the row
        row.query_one(".d-default", Input).value = "square"
        await pilot.pause()
        screen.action_save()
        await pilot.pause()
        assert not isinstance(app.screen, ScriptSettingsScreen)
    assert store.read_parameters(entry.slug)[0].default == "square"


async def test_declared_add_parameter_flow_on_exe(tmp_path):
    """Typing a name into the add-a-parameter field and saving declares a new flag parameter."""
    entry = _exe(tmp_path)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(entry)
        app.push_screen(screen)
        await pilot.pause()
        screen.query_one("#st-add-param", Input).value = "verbose"
        await pilot.pause()
        screen.action_save()
        await pilot.pause()

    written = store.read_parameters(entry.slug)
    assert [d.name for d in written] == ["verbose"]
    assert written[0].delivery == "flag"  # kind-appropriate default for a binary


def test_new_declared_delivery_defaults(tmp_path):
    """_new_declared: exe → flag; a command placeholder name → required placeholder; any other
    command name → env."""
    exe = _exe(tmp_path, name="p1")
    cmd = store.add_command("echo {msg}", name="c1")
    exe_screen = ScriptSettingsScreen(exe)
    cmd_screen = ScriptSettingsScreen(cmd)
    assert exe_screen._new_declared("x").delivery == "flag"
    placeholder = cmd_screen._new_declared("msg")
    assert placeholder.delivery == "placeholder"
    assert placeholder.required is True
    assert cmd_screen._new_declared("RETRIES").delivery == "env"


async def test_declared_row_removed_when_unticked(tmp_path):
    entry = _exe(tmp_path)
    entry = store.write_parameters(
        entry.slug,
        [ParamDecl(name="a", delivery="flag"), ParamDecl(name="b", delivery="flag")],
    )
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(entry)
        app.push_screen(screen)
        await pilot.pause()
        rows = {r.decl.name: r for r in screen.query(DeclParamRow)}
        rows["a"].query_one(".d-keep", Checkbox).value = False  # remove a
        await pilot.pause()
        screen.action_save()
        await pilot.pause()

    assert [d.name for d in store.read_parameters(entry.slug)] == ["b"]


async def test_declared_invalid_type_notifies_and_stays(tmp_path, monkeypatch):
    entry = _exe(tmp_path)
    entry = store.write_parameters(entry.slug, [ParamDecl(name="a", delivery="flag", type="str")])
    notes: list[str] = []
    monkeypatch.setattr(
        ScriptSettingsScreen, "notify", lambda self, message, **kw: notes.append(message)
    )
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(entry)
        app.push_screen(screen)
        await pilot.pause()
        screen.query(DeclParamRow).first().query_one(".d-type", Input).value = "integer"
        await pilot.pause()
        screen.action_save()
        await pilot.pause()
        assert isinstance(app.screen, ScriptSettingsScreen)  # not dismissed
    assert any("unknown type" in m for m in notes)
    assert store.read_parameters(entry.slug)[0].type == "str"  # nothing written


async def test_declared_bad_default_notifies_and_stays(tmp_path, monkeypatch):
    entry = _exe(tmp_path)
    entry = store.write_parameters(entry.slug, [ParamDecl(name="a", delivery="flag", type="int")])
    notes: list[str] = []
    monkeypatch.setattr(
        ScriptSettingsScreen, "notify", lambda self, message, **kw: notes.append(message)
    )
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(entry)
        app.push_screen(screen)
        await pilot.pause()
        screen.query(DeclParamRow).first().query_one(".d-default", Input).value = "notanint"
        await pilot.pause()
        screen.action_save()
        await pilot.pause()
        assert isinstance(app.screen, ScriptSettingsScreen)
    assert any("default doesn't match" in m for m in notes)


async def test_declared_choice_without_choices_notifies(tmp_path, monkeypatch):
    entry = _exe(tmp_path)
    entry = store.write_parameters(entry.slug, [ParamDecl(name="a", delivery="flag", type="str")])
    notes: list[str] = []
    monkeypatch.setattr(
        ScriptSettingsScreen, "notify", lambda self, message, **kw: notes.append(message)
    )
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(entry)
        app.push_screen(screen)
        await pilot.pause()
        screen.query(DeclParamRow).first().query_one(".d-type", Input).value = "choice"
        await pilot.pause()
        screen.action_save()
        await pilot.pause()
        assert isinstance(app.screen, ScriptSettingsScreen)
    assert any("choice parameter but has no choices" in m for m in notes)


async def test_declared_existing_choice_with_choices_saves(tmp_path):
    """An existing choice row keeps its (non-editable) choices, so it validates and saves."""
    entry = _exe(tmp_path)
    entry = store.write_parameters(
        entry.slug,
        [ParamDecl(name="a", delivery="flag", type="choice", choices=("x", "y"))],
    )
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(entry)
        app.push_screen(screen)
        await pilot.pause()
        screen.action_save()
        await pilot.pause()
        assert not isinstance(app.screen, ScriptSettingsScreen)
    assert store.read_parameters(entry.slug)[0].choices == ("x", "y")


async def test_declared_bool_default_round_trips_text(tmp_path):
    """A bool default renders as the true/false word the editor round-trips through save."""
    entry = _exe(tmp_path)
    entry = store.write_parameters(
        entry.slug, [ParamDecl(name="flag", delivery="flag", type="bool", default=True)]
    )
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(entry)
        app.push_screen(screen)
        await pilot.pause()
        assert screen.query(DeclParamRow).first().query_one(".d-default", Input).value == "true"
        screen.action_save()
        await pilot.pause()
    assert store.read_parameters(entry.slug)[0].default is True


async def test_declared_secret_toggle_purges_and_notes(tmp_path, monkeypatch):
    entry = _exe(tmp_path)
    entry = store.write_parameters(
        entry.slug, [ParamDecl(name="TOKEN", delivery="env", type="str")]
    )
    argstate.save_last(entry.slug, values={"TOKEN": "plaintext"})
    notes: list[str] = []
    monkeypatch.setattr(
        ScriptSettingsScreen, "notify", lambda self, message, **kw: notes.append(message)
    )
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(entry)
        app.push_screen(screen)
        await pilot.pause()
        row = screen.query(DeclParamRow).first()
        row.query_one(".p-secret", Checkbox).value = True
        await pilot.pause()
        assert "previously remembered" in str(row.query_one(".p-note", Static).render())
        row.query_one(".p-secret", Checkbox).value = False  # unticking clears the note
        await pilot.pause()
        assert str(row.query_one(".p-note", Static).render()) == ""
        row.query_one(".p-secret", Checkbox).value = True  # re-secret it for the save assertions
        row.query_one(".p-env", Input).value = "TOKEN_ENV"
        await pilot.pause()
        screen.action_save()
        await pilot.pause()

    d = store.read_parameters(entry.slug)[0]
    assert d.secret is True
    assert d.env_source == "TOKEN_ENV"
    assert "TOKEN" not in argstate.load_state(entry.slug)["values"]  # purged
    assert any("Deleted previously remembered" in m for m in notes)


async def test_declared_editor_is_keyboard_reachable(tmp_path):
    """Every declared-editor widget is Tab-reachable (full keyboard operability)."""
    entry = _exe(tmp_path)
    entry = store.write_parameters(entry.slug, [ParamDecl(name="a", delivery="flag", type="str")])
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(entry)
        app.push_screen(screen)
        await pilot.pause()
        reached: set[str] = set()
        for _ in range(40):
            await pilot.press("tab")
            focused = app.focused
            if focused is not None:
                reached.update(focused.classes)
                if focused.id == "st-add-param":
                    reached.add("st-add-param")
        assert "d-type" in reached  # a declared row's type field
        assert "st-add-param" in reached  # the add-a-parameter field


# ---------------------------------------------------------------------------
# Needs (external commands): a section for every kind, saved via store.update_needs
# ---------------------------------------------------------------------------


async def test_settings_saves_needs_for_shell_entry(tmp_path):
    """Every kind gets the "Needs (external commands)" input; saving comma-separated
    values writes them through store.update_needs."""
    sh = tmp_path / "d.sh"
    sh.write_text("#!/bin/bash\necho hi\n", encoding="utf-8")
    entry = store.add_script(sh, kind="shell", name="d")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(entry)
        app.push_screen(screen)
        await pilot.pause()
        assert "Needs (external commands)" in _body(screen)
        screen.query_one("#st-needs", Input).value = "ffmpeg, jq"
        await pilot.pause()
        screen.action_save()
        await pilot.pause()
        assert not isinstance(app.screen, ScriptSettingsScreen)  # dismissed after save
    assert store.resolve("d").meta.needs == ["ffmpeg", "jq"]


async def test_settings_needs_prefilled_and_clearable(tmp_path):
    """The input shows the current needs; blanking it clears them on save."""
    sh = tmp_path / "d.sh"
    sh.write_text("#!/bin/bash\necho hi\n", encoding="utf-8")
    entry = store.add_script(sh, kind="shell", name="d")
    store.update_needs("d", ["jq"])
    entry = store.resolve("d")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(entry)
        app.push_screen(screen)
        await pilot.pause()
        assert screen.query_one("#st-needs", Input).value == "jq"  # prefilled
        screen.query_one("#st-needs", Input).value = "  "  # blank it out
        await pilot.pause()
        screen.action_save()
        await pilot.pause()
    assert store.resolve("d").meta.needs is None  # cleared


# ---- TUI↔CLI parity: the settings screen uses the ENTRY'S analyzer, never Python's ---------------


def _shell(tmp_path, text: str, name: str = "sh"):
    src = tmp_path / f"{name}.sh"
    src.write_text(text, encoding="utf-8")
    return store.add_script(src, kind="shell", name=name)


async def test_settings_detects_shell_candidates_with_the_shell_analyzer(tmp_path):
    # Hardcoding Python's analyzer here made the screen show ZERO detected candidates and report a
    # perfectly valid shell script as a syntax error (the Python ast.parse fails on shell source).
    entry = _shell(tmp_path, '#!/bin/bash\nGREETING="hello"\nPORT="${PORT:-8080}"\necho hi\n')
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(entry)
        app.push_screen(screen)
        await pilot.pause()
        report = screen._reconcile()
        assert report is not None
        assert not report.syntax_error  # a valid shell script is NOT a syntax error
        assert {c.name for c in report.new} == {"GREETING", "PORT"}


async def test_settings_manages_a_shell_const_end_to_end(tmp_path):
    entry = _shell(tmp_path, '#!/bin/bash\nGREETING="hello"\necho "$GREETING"\n', name="sh2")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(entry)
        app.push_screen(screen)
        await pilot.pause()
        # tick the detected candidate, then save: it must land in the copy's [tool.skit] block
        screen.query_one("#st-new-0", Checkbox).value = True
        await pilot.pause()
        screen.action_save()
        await pilot.pause()
    spec = spec_for(entry.meta.kind)
    assert spec is not None
    assert spec.params_io is not None
    written = spec.params_io.read(entry.script_path.read_text(encoding="utf-8"))
    assert [s.name for s in written] == ["GREETING"]


async def test_settings_reads_a_js_block_with_the_slash_engine(tmp_path):
    # JS/TS carry their [tool.skit] block behind '//' — Python's '#' engine returns [] for a
    # perfectly valid managed JS entry, so its managed params were invisible in the screen.
    src = tmp_path / "a.mjs"
    src.write_text("const WIDTH = 800;\nconsole.log(WIDTH);\n", encoding="utf-8")
    entry = store.add_script(src, kind="js", name="jsx")
    spec = spec_for(entry.meta.kind)
    assert spec is not None
    assert spec.params_io is not None
    text = entry.script_path.read_text(encoding="utf-8")
    entry.script_path.write_text(
        spec.params_io.write(
            text, [ParamDecl(name="WIDTH", binding="const", type="int", default=800)]
        ),
        encoding="utf-8",
    )
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(store.resolve(entry.slug))
        app.push_screen(screen)
        await pilot.pause()
        assert [row.spec.name for row in screen.query(ParamRow)] == ["WIDTH"]


async def test_settings_resync_on_a_shell_entry_does_not_report_a_false_syntax_error(tmp_path):
    entry = _shell(tmp_path, '#!/bin/bash\nGREETING="hello"\necho "$GREETING"\n', name="sh3")
    spec = spec_for(entry.meta.kind)
    assert spec is not None
    assert spec.params_io is not None
    text = entry.script_path.read_text(encoding="utf-8")
    entry.script_path.write_text(
        spec.params_io.write(
            text, [ParamDecl(name="GREETING", binding="const", type="str", default="hello")]
        ),
        encoding="utf-8",
    )
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(store.resolve(entry.slug))
        app.push_screen(screen)
        await pilot.pause()
        screen.action_resync()
        await pilot.pause()
        assert "syntax error" not in screen._resync_report.lower()


async def test_settings_reconcile_is_none_when_the_script_is_gone(tmp_path):
    # An analyzable entry whose stored copy vanished: there is no text to analyze, so the screen
    # must degrade to "no report" rather than analyzing an empty string as if it were the script.
    entry = _shell(tmp_path, '#!/bin/bash\nGREETING="hello"\n', name="sh4")
    entry.script_path.unlink()
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(store.resolve(entry.slug))
        app.push_screen(screen)
        await pilot.pause()
        assert screen._reconcile() is None


async def test_settings_cli_driven_is_false_when_the_kind_has_no_reader(tmp_path, monkeypatch):
    # Defensive contract: a spec carrying an analyzer but no cli_reader (a future kind, or a
    # partially-degraded one) must report "not CLI-driven" rather than crash on a missing reader.
    import dataclasses

    entry = _shell(tmp_path, '#!/bin/bash\nGREETING="hello"\n', name="sh5")
    spec = spec_for("shell")
    assert spec is not None
    monkeypatch.setattr(
        "skit.tui_settings.spec_for", lambda _kind: dataclasses.replace(spec, cli_reader=None)
    )
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(store.resolve(entry.slug))
        app.push_screen(screen)
        await pilot.pause()
        assert screen._cli_driven() is False


async def test_settings_declared_editor_opens_for_an_interpreted_meta_kind(tmp_path):
    # ruby/perl/lua/r/powershell store their schema in meta.toml too — the declared editor must
    # open for them, not the "(programs have no managed parameters)" dead end.
    from skit.tui_settings import DeclParamRow

    src = tmp_path / "t.rb"
    src.write_text('#!/usr/bin/env ruby\nputs "hi"\n', encoding="utf-8")
    entry = store.add_script(src, kind="ruby", name="rbt")
    store.write_parameters(entry.slug, [ParamDecl(name="GREETING", delivery="flag", flag="--g")])
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(store.resolve(entry.slug))
        app.push_screen(screen)
        await pilot.pause()
        rows = list(screen.query(DeclParamRow))
        assert [r.decl.name for r in rows] == ["GREETING"]
        # argv IS this kind's interface, so the Flag field must be editable (not template-hidden)
        assert rows[0].query(".d-flag")
