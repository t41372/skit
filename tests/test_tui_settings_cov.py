"""Exact-behavior tests for the merged Script settings screen (tui_settings.py).

Behaviour is asserted — the written [tool.skit] copy, store/argstate mutations, the
rendered section/hint text, resync warnings — never a line executed for its own sake.
The heavy save/load logic lives in store/argstate/metawriter (tested there); these cover
the settings screen's presentation glue and its one atomic save.
"""

from __future__ import annotations

import pytest
from textual.widgets import Checkbox, Input, Static

from skit import argstate, metawriter, reconcile, store, tui
from skit.metawriter import ParamSpec
from skit.tui_settings import DiscardChangesModal, ParamRow, ScriptSettingsScreen


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
            ParamSpec(name="CITY", kind="const", type="str", default="Taipei"),
            ParamSpec(name="API_KEY", kind="const", type="str", default="k", secret=True),
            ParamSpec(name="OLD", kind="const", type="str", default="o"),
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
        [ParamSpec(name="CITY", kind="const", type="str", default="x")],
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
            ParamSpec(name="input-1", kind="input", type="str", prompt="Name? ", order=0),
            ParamSpec(name="input-2", kind="input", type="str", prompt="Age? ", order=1),
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


# ---------------------------------------------------------------------------
# Non-python / reference storage: early returns and read-only param views
# ---------------------------------------------------------------------------


async def test_command_entry_shows_template_hides_storage_and_deps(tmp_path):
    """A command entry has no stored file: no Storage/Dependencies sections, its template and named
    placeholders are shown read-only, resync is a no-op, and save still dismisses cleanly."""
    entry = store.add_command("echo {msg} {name}", name="cmd")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(entry)
        app.push_screen(screen)
        await pilot.pause()
        body = _body(screen)
        assert "echo {msg} {name}" in body  # the template
        assert "· msg" in body  # its placeholders
        assert "· name" in body
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


async def test_exe_entry_states_it_has_no_managed_parameters(tmp_path):
    """An executable ("program") entry carries no managed parameters — the screen says so."""
    tool = tmp_path / "mytool"
    tool.write_text("#!/bin/sh\necho hi\n", encoding="utf-8")
    entry = store.add_exe(tool, name="prog")
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
            [ParamSpec(name="CITY", kind="const", type="str", default="x")],
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
            ParamSpec(name="CITY", kind="const", type="str", default="x"),
            ParamSpec(name="GONE", kind="const", type="str", default="y"),
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
        [ParamSpec(name="CITY", kind="const", type="str", default="x")],
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
