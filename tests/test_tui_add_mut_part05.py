"""Mutation-kill tests for the AddSourceScreen half of ``skit.tui_add`` (chunk 5/5).

Every test drives the real "add a script" source screen through Textual's ``Pilot``
(or a direct method call on the mounted screen) and asserts an OBSERVABLE contract:
the stored entry's metadata, the exact toast / error copy, or the value the screen
dismisses with. English catalog throughout, so the message assertions are exact.
"""

from __future__ import annotations

from dataclasses import replace

import pytest
from textual.widgets import Input, Static

from skit import i18n, store, tui
from skit.langs import registry
from skit.tui_add import AddReviewScreen, AddSourceScreen


@pytest.fixture(autouse=True)
def tmp_store(tmp_path, monkeypatch):
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path / "config"))
    monkeypatch.setenv("SKIT_LANG", "en")
    i18n.set_language("en")


def _write(tmp_path, name: str, body: str) -> object:
    p = tmp_path / name
    p.write_text(body, encoding="utf-8")
    return p


async def _mounted(app, pilot, capture: dict[str, str | None] | None = None):
    """Push a fresh AddSourceScreen; optionally capture its dismiss value."""
    screen = AddSourceScreen()
    if capture is not None:
        app.push_screen(screen, lambda v: capture.__setitem__("result", v))
    else:
        app.push_screen(screen)
    await pilot.pause()
    return screen


# ---------------------------------------------------------------------------
# on_mount: the panel's border title
# ---------------------------------------------------------------------------


async def test_on_mount_sets_border_title(tmp_path):
    """on_mount stamps the panel border with the exact, translated 'Add a script'."""
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = await _mounted(app, pilot)
        assert screen.query_one("#add-body").border_title == "Add a script"


# ---------------------------------------------------------------------------
# _add_non_python: interpreter, deps scan, toast, dismiss, rejection
# ---------------------------------------------------------------------------


async def test_non_python_no_shebang_records_empty_interpreter(tmp_path):
    """A JS file with no recognizable shebang falls into the else branch: the recorded
    interpreter is the empty string (skit uses the kind's default runner), not a literal."""
    js = _write(tmp_path, "plain.js", "const x = 1\nconsole.log(x)\n")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = await _mounted(app, pilot)
        screen.query_one("#add-path", Input).value = str(js)
        screen._submit_path()
        await pilot.pause()
    (entry,) = store.list_entries()
    assert entry.meta.kind == "js"
    assert entry.meta.interpreter == ""


async def test_non_python_scans_deps_and_toasts(tmp_path):
    """An npm-flavor copy add scans the script's own imports, records them on the entry,
    and surfaces the exact toast (the direct lane's only visible receipt for something that
    will download packages)."""
    js = _write(
        tmp_path,
        "tool.js",
        '#!/usr/bin/env node\nimport express from "express"\nimport _ from "lodash"\nconsole.log(1)\n',
    )
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = await _mounted(app, pilot)
        screen.query_one("#add-path", Input).value = str(js)
        screen._submit_path()
        await pilot.pause()
        messages = [n.message for n in app._notifications]
    (entry,) = store.list_entries()
    assert entry.meta.mode == "copy"
    assert entry.meta.interpreter == "node"
    assert entry.meta.dependencies == ["express", "lodash"]
    assert messages == ["Dependencies recorded: express, lodash (edit in Script settings)"]


async def test_non_python_dismisses_with_new_slug(tmp_path):
    """The direct lane hands the new entry's slug back to the Library (so it can select the
    freshly-added row) — never None."""
    js = _write(tmp_path, "widget.js", "const x = 1\n")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        captured: dict[str, str | None] = {}
        screen = await _mounted(app, pilot, captured)
        screen.query_one("#add-path", Input).value = str(js)
        screen._submit_path()
        await pilot.pause()
    (entry,) = store.list_entries()
    assert captured["result"] == entry.slug
    assert entry.slug is not None


async def test_non_python_degraded_scanner_skips_deps_block(tmp_path, monkeypatch):
    """When the JS grammar wheel is absent the kind keeps deps_flavor='npm' but its
    dep_scanner degrades to None; the `and dep_scanner is not None` guard must still hold, so
    the add completes without recording deps (and without calling None(text))."""
    real = registry.spec_for

    def degraded(kind):
        spec = real(kind)
        if kind == "js" and spec is not None:
            return replace(spec, dep_scanner=None)
        return spec

    monkeypatch.setattr(registry, "spec_for", degraded)
    js = _write(tmp_path, "deg.js", 'import express from "express"\nconsole.log(1)\n')
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = await _mounted(app, pilot)
        screen.query_one("#add-path", Input).value = str(js)
        screen._submit_path()
        await pilot.pause()
    (entry,) = store.list_entries()
    assert entry.meta.mode == "copy"
    assert entry.meta.dependencies is None  # deps block correctly skipped


async def test_non_python_tolerates_non_utf8_bytes(tmp_path):
    """A script carrying a stray non-UTF-8 byte must still add cleanly: the read uses
    errors='replace' so a rogue byte degrades to U+FFFD instead of crashing the add. The
    ASCII import line still scans, so deps are recorded."""
    js = tmp_path / "latin.js"
    js.write_bytes(b'#!/usr/bin/env node\nimport express from "express"\nconst s = "caf\xe9"\n')
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = await _mounted(app, pilot)
        screen.query_one("#add-path", Input).value = str(js)
        screen._submit_path()
        await pilot.pause()
    (entry,) = store.list_entries()
    assert entry.meta.dependencies == ["express"]


async def test_non_python_rejects_unknown_kind(tmp_path):
    """A plain data file is neither a known script kind nor an executable: the screen shows
    the exact guidance line pointing at --exe / --cmd, and adds nothing."""
    notes = _write(tmp_path, "notes.txt", "just prose\n")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = await _mounted(app, pilot)
        screen.query_one("#add-path", Input).value = str(notes)
        screen._submit_path()
        await pilot.pause()
        rendered = str(screen.query_one("#add-error", Static).render())
    assert rendered == (
        "notes.txt isn't a script or an executable — pass --exe for a program, "
        "or --cmd for a command template."
    )
    assert store.list_entries() == []


# ---------------------------------------------------------------------------
# _submit_path: file-not-found + the python review hand-off
# ---------------------------------------------------------------------------


async def test_submit_path_reports_missing_file(tmp_path):
    """A path that isn't a file surfaces the exact 'File not found' line and adds nothing."""
    missing = tmp_path / "ghost.py"
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = await _mounted(app, pilot)
        screen.query_one("#add-path", Input).value = str(missing)
        screen._submit_path()
        await pilot.pause()
        rendered = str(screen.query_one("#add-error", Static).render())
    assert rendered == f"File not found: {missing}"
    assert store.list_entries() == []


async def test_submit_path_python_review_returns_slug(tmp_path):
    """A .py path opens the review panel; when review accepts and returns a slug, the source
    screen forwards THAT slug to the Library (not None)."""
    py = _write(tmp_path, "job.py", "print(1)\n")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        captured: dict[str, str | None] = {}
        screen = await _mounted(app, pilot, captured)
        screen.query_one("#add-path", Input).value = str(py)
        screen._submit_path()
        await pilot.pause()
        assert isinstance(app.screen, AddReviewScreen)  # review opened
        app.screen.action_accept()
        await pilot.pause()
        await pilot.pause()
    (entry,) = store.list_entries()
    assert captured["result"] == entry.slug  # slug forwarded, not None


# ---------------------------------------------------------------------------
# _submit_template
# ---------------------------------------------------------------------------


async def test_submit_template_requires_name(tmp_path):
    """A template with no name is blocked with the exact 'A name is required.' error, and no
    command entry is created."""
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = await _mounted(app, pilot)
        screen.query_one("#add-template", Input).value = "ffmpeg -i {input}"
        screen.query_one("#add-template-name", Input).value = ""
        screen._submit_template()
        await pilot.pause()
        rendered = str(screen.query_one("#add-error", Static).render())
    assert rendered == "A name is required."
    assert store.list_entries() == []


async def test_submit_template_creates_entry_and_returns_slug(tmp_path):
    """A filled template + name creates the command entry and dismisses with its slug."""
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        captured: dict[str, str | None] = {}
        screen = await _mounted(app, pilot, captured)
        screen.query_one("#add-template", Input).value = "echo {msg}"
        screen.query_one("#add-template-name", Input).value = "say"
        screen._submit_template()
        await pilot.pause()
    (entry,) = store.list_entries()
    assert entry.meta.name == "say"
    assert entry.meta.template == "echo {msg}"
    assert captured["result"] == entry.slug


# ---------------------------------------------------------------------------
# action_continue_add: field-precedence twin of Enter
# ---------------------------------------------------------------------------


async def test_continue_add_prefers_the_path_field(tmp_path):
    """action_continue_add submits the script path when it's filled, in preference to the
    command template."""
    py = _write(tmp_path, "prog.py", "print(1)\n")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = await _mounted(app, pilot)
        screen.query_one("#add-path", Input).value = str(py)
        screen.query_one("#add-template", Input).value = "echo hi"
        screen.query_one("#add-template-name", Input).value = "hello"
        screen.action_continue_add()
        await pilot.pause()
        assert isinstance(app.screen, AddReviewScreen)  # took the path branch → review


async def test_continue_add_falls_back_to_template(tmp_path):
    """With the path field empty, action_continue_add submits the command template instead."""
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = await _mounted(app, pilot)
        screen.query_one("#add-template", Input).value = "echo hi"
        screen.query_one("#add-template-name", Input).value = "hello"
        screen.action_continue_add()
        await pilot.pause()
    (entry,) = store.list_entries()
    assert entry.meta.name == "hello"
    assert entry.meta.kind == "command"
