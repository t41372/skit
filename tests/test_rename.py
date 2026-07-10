"""store.rename: display name changes; the slug (dir, state key) never moves."""

from __future__ import annotations

import pytest

from skit import argstate, store


@pytest.fixture(autouse=True)
def tmp_store(tmp_path, monkeypatch):
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path / "config"))
    monkeypatch.setenv("SKIT_LANG", "en")


def _py(tmp_path, name="a.py"):
    p = tmp_path / name
    p.write_text("print(1)\n", encoding="utf-8")
    return p


def test_rename_changes_name_and_keeps_slug_dir_and_state(tmp_path):
    entry = store.add_python(_py(tmp_path), name="old")
    argstate.save_last(entry.slug, values={"X": "1"})
    renamed = store.rename("old", "new")
    assert renamed.meta.name == "new"
    assert renamed.slug == entry.slug  # immutable: nothing on disk moves
    assert renamed.dir == entry.dir
    assert argstate.load_state(entry.slug)["values"] == {"X": "1"}  # values survive


def test_rename_updates_resolution_and_listing(tmp_path):
    # Use a name whose slug differs, so "old name gone" and "slug survives" are
    # observable separately (the slug is the immutable internal id).
    store.add_python(_py(tmp_path), name="Old Name")
    entry = store.resolve("Old Name")
    store.rename("Old Name", "new")
    assert store.resolve("new").meta.name == "new"
    with pytest.raises(store.NotFoundError):
        store.resolve("Old Name")
    assert store.resolve(entry.slug).meta.name == "new"  # the slug keeps resolving
    assert [e.meta.name for e in store.list_entries()] == ["new"]


def test_rename_conflict_is_a_clean_error(tmp_path):
    store.add_python(_py(tmp_path, "a.py"), name="alpha")
    store.add_python(_py(tmp_path, "b.py"), name="beta")
    with pytest.raises(store.StoreError) as exc:
        store.rename("beta", "alpha")
    assert "alpha" in str(exc.value)
    assert store.resolve("beta").meta.name == "beta"  # untouched


def test_rename_to_own_name_is_a_no_op(tmp_path):
    store.add_python(_py(tmp_path), name="same")
    assert store.rename("same", "same").meta.name == "same"


def test_rename_empty_name_rejected(tmp_path):
    store.add_python(_py(tmp_path), name="x")
    with pytest.raises(store.StoreError):
        store.rename("x", "   ")


def test_rename_survives_doctor_rebuild(tmp_path):
    store.add_python(_py(tmp_path), name="old")
    store.rename("old", "new")
    count, problems = store.doctor_rebuild()
    assert count == 1
    assert problems == []
    assert store.resolve("new").meta.name == "new"  # meta.toml is the truth


async def test_settings_screen_renames_on_save(tmp_path, monkeypatch):
    from textual.widgets import Input

    from skit import tui
    from skit.tui_settings import ScriptSettingsScreen

    store.add_python(_py(tmp_path), name="old")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.action_settings()
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, ScriptSettingsScreen)
        screen.query_one("#st-name", Input).value = "shiny"
        screen.action_save()
        await pilot.pause()
    assert store.resolve("shiny").meta.name == "shiny"


async def test_settings_screen_rename_conflict_stays_open(tmp_path):
    from textual.widgets import Input

    from skit import tui
    from skit.tui_settings import ScriptSettingsScreen

    store.add_python(_py(tmp_path, "a.py"), name="alpha")
    store.add_python(_py(tmp_path, "b.py"), name="beta")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app._select_slug(store.resolve("beta").slug)
        app.action_settings()
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, ScriptSettingsScreen)
        screen.query_one("#st-name", Input).value = "alpha"
        screen.action_save()
        await pilot.pause()
        assert isinstance(app.screen, ScriptSettingsScreen)  # refused; still editing
    assert store.resolve("beta").meta.name == "beta"


# ---------------------------------------------------------------------------
# A2 short-term: the settings screen must not lead an argparse script into a
# source-flip trap (managing a constant would shadow its whole argparse form)
# ---------------------------------------------------------------------------

ARGPARSE_WITH_CONST = (
    "import argparse\n"
    "TIMEOUT = 30\n"  # a hardcoded constant reconcile would offer to manage
    "ap = argparse.ArgumentParser()\n"
    "ap.add_argument('--out', required=True)\n"
    "ap.parse_args()\n"
)


async def test_settings_hides_manage_checkboxes_for_argparse_script(tmp_path):
    from textual.widgets import Static

    from skit import flows, store, tui
    from skit.tui_settings import ScriptSettingsScreen

    entry = store.add_python(_py_text(tmp_path, ARGPARSE_WITH_CONST), name="ap")
    assert flows.plan_for_entry(entry).source == "argparse"  # served by its own args
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.action_settings()
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, ScriptSettingsScreen)
        # No "tick to manage" candidate checkboxes (managing TIMEOUT would flip the source):
        assert not screen.query("#st-new-0")
        blurb = " ".join(str(w.render()) for w in screen.query(Static))
        assert "comes from its own command-line arguments" in blurb


async def test_settings_save_keeps_argparse_source(tmp_path):
    from skit import flows, store, tui
    from skit.tui_settings import ScriptSettingsScreen

    store.add_python(_py_text(tmp_path, ARGPARSE_WITH_CONST), name="ap2")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.action_settings()
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, ScriptSettingsScreen)
        screen.action_save()
        await pilot.pause()
    # Saving from settings must NOT have written a [tool.skit] block that shadows argparse.
    assert flows.plan_for_entry(store.resolve("ap2")).source == "argparse"


def _py_text(tmp_path, text, name="s.py"):
    p = tmp_path / name
    p.write_text(text, encoding="utf-8")
    return p
