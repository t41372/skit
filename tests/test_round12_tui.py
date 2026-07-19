"""Round-12 TUI coverage — the settings screen's validate-then-write atomicity and the
review panels' self-derived `fresh` (the drafts boundary through the CLI-hosted panel).

Every assertion pins an OBSERVABLE contract of the round-12 fixes:

  * ScriptSettingsScreen.action_save reads AND validates the deps section in the VALIDATION
    pass: an unparseable uv requirement / constraint refuses the WHOLE save (notify error, the
    screen stays open) with NOTHING written — in particular a concurrent rename does NOT
    persist (the atomic contract the audit found violated: rename/desc/params used to land
    before the deps refusal). '-'/'none' normalize to automatic; npm deps are not PEP 508
    validated (the installer owns that grammar);
  * AddReviewScreen / PromptReviewScreen DERIVE `fresh` from is_draft(path): the CLI-hosted
    panel (AddReviewApp / PromptReviewApp — neither passes fresh) opens a kept draft with NO
    Storage section, so the "Link the original" radio — the fourth route to a reference entry
    pointing into drafts/ — is unreachable, and accept can only copy. A non-draft file still
    shows Storage;
  * the panel's dependency prefill runs through suggest_dependencies, so a PEP 508-illegal
    import (café) never seeds the field.

These never chdir and never touch the real user dirs (the local SKIT_* fixture).
"""

from __future__ import annotations

from pathlib import Path

import pytest
from textual.widgets import Input

from skit import store, tui
from skit.paths import drafts_dir
from skit.tui_add import (
    AddReviewApp,
    AddReviewScreen,
    PromptReviewApp,
    PromptReviewScreen,
)
from skit.tui_settings import ScriptSettingsScreen


@pytest.fixture(autouse=True)
def tmp_store(tmp_path, monkeypatch):
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path / "config"))
    monkeypatch.setenv("SKIT_LANG", "en")


def _py(tmp_path, body: str, name: str = "job.py") -> Path:
    p = tmp_path / name
    p.write_text(body, encoding="utf-8")
    return p


def _draft(name: str, body: str) -> Path:
    drafts_dir().mkdir(parents=True, exist_ok=True)
    p = drafts_dir() / name
    p.write_text(body, encoding="utf-8")
    return p


def _capture_notify(monkeypatch, screen) -> list[tuple[str, object]]:
    notes: list[tuple[str, object]] = []
    monkeypatch.setattr(screen, "notify", lambda m, **kw: notes.append((m, kw.get("severity"))))
    return notes


# ==========================================================================
# 1. Settings save: validate-then-write is atomic across the deps section
# ==========================================================================


async def test_settings_bad_dep_refuses_the_whole_save_including_the_rename(tmp_path, monkeypatch):
    """The audit's exact violation: a garbage #st-deps entry + a changed #st-name must refuse
    the ENTIRE save — notify(error), the screen stays open, and the rename did NOT persist."""
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="orig")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(entry)
        app.push_screen(screen)
        await pilot.pause()
        notes = _capture_notify(monkeypatch, screen)
        screen.query_one("#st-name", Input).value = "renamed"
        screen.query_one("#st-deps", Input).value = "@@@"
        screen.action_save()
        await pilot.pause()
        assert app.screen is screen  # still open — the save was refused
    assert any("package requirement" in m and sev == "error" for m, sev in notes)
    with pytest.raises(store.NotFoundError):
        store.resolve("renamed")  # the rename never happened (atomic refusal)
    assert store.resolve("orig").meta.name == "orig"


async def test_settings_bad_python_refuses_the_whole_save_including_the_rename(
    tmp_path, monkeypatch
):
    """The #st-python twin: an unparseable constraint + a changed name refuses everything."""
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="orig2")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(entry)
        app.push_screen(screen)
        await pilot.pause()
        notes = _capture_notify(monkeypatch, screen)
        screen.query_one("#st-name", Input).value = "renamed2"
        screen.query_one("#st-python", Input).value = "not-a-version"
        screen.action_save()
        await pilot.pause()
        assert app.screen is screen
    assert any("version constraint" in m and sev == "error" for m, sev in notes)
    with pytest.raises(store.NotFoundError):
        store.resolve("renamed2")
    assert store.resolve("orig2").meta.name == "orig2"


async def test_settings_dash_python_saves_as_automatic(tmp_path):
    """'-' in #st-python normalizes to automatic: the save commits with meta cleared to ""."""
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="autoset")
    store.update_dependencies(entry.slug, ["requests"], requires_python=">=3.11")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(store.resolve("autoset"))
        app.push_screen(screen)
        await pilot.pause()
        screen.query_one("#st-python", Input).value = "-"
        screen.action_save()
        await pilot.pause()
        assert not isinstance(app.screen, ScriptSettingsScreen)  # committed & dismissed
    assert store.resolve("autoset").meta.requires_python == ""


async def test_settings_valid_deps_and_python_save_normally(tmp_path):
    """The complement: valid values pass the validation pass and land in meta + the block."""
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="okset")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(entry)
        app.push_screen(screen)
        await pilot.pause()
        screen.query_one("#st-deps", Input).value = "requests>=2,<3"
        screen.query_one("#st-python", Input).value = "~=3.12"
        screen.action_save()
        await pilot.pause()
        assert not isinstance(app.screen, ScriptSettingsScreen)
    meta = store.resolve("okset").meta
    assert meta.dependencies == ["requests>=2,<3"]
    assert meta.requires_python == "~=3.12"


async def test_settings_npm_deps_are_not_pep508_validated(tmp_path):
    """A js entry's #st-deps is split with the npm splitter and NOT PEP 508-validated: a
    scoped package (@scope/thing, which requirement_error rejects) still saves. There is no
    #st-python widget on the npm flavor either."""
    src = tmp_path / "tool.js"
    src.write_text("console.log(1)\n", encoding="utf-8")
    entry = store.add_script(src, kind="js", name="jsset")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(entry)
        app.push_screen(screen)
        await pilot.pause()
        assert not screen.query("#st-python")  # npm flavor: no Python constraint field
        screen.query_one("#st-deps", Input).value = "@scope/thing"
        screen.action_save()
        await pilot.pause()
        assert not isinstance(app.screen, ScriptSettingsScreen)  # committed, not refused
    assert store.resolve("jsset").meta.dependencies == ["@scope/thing"]


# ==========================================================================
# 2. Review panels DERIVE fresh from is_draft — the CLI-hosted panel hides Storage
# ==========================================================================


async def test_add_panel_on_a_kept_draft_hides_storage_and_copies(tmp_path):
    """AddReviewApp is EXACTLY what `skit add <file>` (form=tui) builds — and it never passes
    fresh. On a kept draft the panel must still hide the Storage section (the derived fresh),
    so the reference radio is unreachable and accept can only copy."""
    draft = _draft("skit-new-resume.py", "print('resumed')\n")
    app = AddReviewApp(draft, kind="python")
    async with app.run_test(size=(100, 40)) as pilot:
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, AddReviewScreen)
        assert screen._fresh is True  # derived from is_draft(path), not the (unset) flag
        assert not screen.query("#rv-mode")  # no Storage radio → reference is unreachable
        screen.query_one("#rv-name", Input).value = "resumed"
        screen.action_accept()
        await pilot.pause()
    assert store.resolve("resumed").meta.mode == "copy"  # the only shape the panel can reach


async def test_prompt_panel_on_a_kept_draft_hides_storage_and_copies(tmp_path):
    """The PromptReviewApp face of the same fix: a kept prompt draft opened through the
    CLI-hosted panel hides Storage (derived fresh), so the entry can only be a copy."""
    draft = _draft("skit-new-ask.prompt.md", "Summarize {{text}}.\n")
    app = PromptReviewApp(draft)
    async with app.run_test(size=(100, 40)) as pilot:
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, PromptReviewScreen)
        assert screen._fresh is True
        assert not screen.query("#pv-mode")  # no Storage radio on the prompt panel either
        screen.query_one("#pv-name", Input).value = "asker"
        screen.action_accept()
        await pilot.pause()
    entry = store.resolve("asker")
    assert entry.meta.kind == "prompt"
    assert entry.meta.mode == "copy"


async def test_add_panel_on_a_nondraft_still_shows_storage(tmp_path):
    """The complement (the derivation must not over-fire): a NON-draft on-disk file opened
    through the same CLI-hosted panel still shows the Storage section — its original is real
    and linkable, so fresh stays False."""
    src = _py(tmp_path, "print('ondisk')\n", "ondisk.py")
    app = AddReviewApp(src, kind="python")
    async with app.run_test(size=(100, 40)) as pilot:
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, AddReviewScreen)
        assert screen._fresh is False
        assert screen.query("#rv-mode")  # Storage present: copy vs link the original


async def test_resumed_draft_through_the_tui_add_lane_is_consumed(tmp_path, monkeypatch):
    """The panel-hosted CLI lane's success arc: `skit add <draft>` (form=tui, interactive)
    hosts the panel, and on a copy result the shared consume-on-success unlink fires (cli.py:
    a resumed skit draft is done accumulating). The panel internals are stubbed to a copy
    accept — this pins the tui-branch wiring + the draft consumption, not the panel UI."""
    from typer.testing import CliRunner

    from skit import cli, i18n

    i18n.init("en")
    draft = _draft("skit-new-consumeme.py", "print('bye')\n")

    def fake_panel(path, **kw):
        e = store.add_python(Path(path), name="consumed", mode="copy")
        return e.slug

    monkeypatch.setattr("skit.tui_add.run_add_review", fake_panel)
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    monkeypatch.setattr(cli.config, "load_form", lambda: "tui")
    monkeypatch.setenv("TERM", "xterm")
    result = CliRunner().invoke(cli.app, ["add", str(draft)])
    assert result.exit_code == 0, result.output
    assert store.resolve("consumed").meta.mode == "copy"
    assert not draft.exists()  # consumed on success (the shared path-lane unlink)


# ==========================================================================
# 3. The panel's dependency prefill runs through suggest_dependencies
# ==========================================================================


async def test_add_panel_prefill_drops_a_pep508_illegal_import(tmp_path):
    """The #rv-deps prefill is `", ".join(suggest_dependencies(text))`, which now filters
    PEP 508-illegal names: an `import café` (legal identifier, illegal distribution name)
    never seeds the field, while a legal import beside it does."""
    src = _py(tmp_path, "import café\nimport requests\nprint(café, requests)\n", "mixed.py")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = AddReviewScreen(src)
        app.push_screen(screen)
        await pilot.pause()
        prefill = screen.query_one("#rv-deps", Input).value
    assert "café" not in prefill
    assert "requests" in prefill
