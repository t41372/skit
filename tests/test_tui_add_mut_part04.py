"""Mutation-kill tests for src/skit/tui_add.py — chunk 4.

Covers the review panel's three action bodies (``AddReviewScreen.on_mount``,
``action_accept`` and ``action_edit_source``). Every test drives the real Textual
screen with ``Pilot`` and asserts an OBSERVABLE contract of the add flow — the entry
that lands in the store, the params written into the stored copy, the overrides that
survive an edit→rescan, the editor error surfaced to the user, and the panel title.

A handful of the mutants on these lines are genuine equivalents: ``query_one``'s
``expect_type`` is a pure runtime assertion (None/omitted return the same unique match),
``query_one(Type)`` resolves the same first/only widget, and ``read_text``/``write_text``
encoding spellings decode identically under skit's UTF-8-mode runtime. Those lines carry
``# pragma: no mutate`` in the source, and the tests below pin the surrounding behaviour
so the pragma never masks a real regression (the maintainer's tui_form.py convention).
"""

from __future__ import annotations

import contextlib

import pytest
from textual.widgets import Checkbox, Input, RadioButton, RadioSet

from skit import editor, pep723, store, tui
from skit.langs.python import metawriter
from skit.tui_add import AddReviewScreen


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


def _quiet_editor(monkeypatch):
    """Make ``action_edit_source`` runnable off-terminal: a no-op suspend and a no-op editor."""
    monkeypatch.setattr(tui.MenuApp, "suspend", lambda self: contextlib.nullcontext())
    monkeypatch.setattr("skit.tui_add.editor.open_in_editor", lambda path: None)


# ---------------------------------------------------------------------------
# on_mount — the panel title
# ---------------------------------------------------------------------------


async def test_on_mount_sets_add_border_title(tmp_path):
    """on_mount stamps ``Add <filename>`` onto the panel border. Kills the border_title=None,
    the ``XXAdd %(name)sXX`` msgid-garble, and the lowercase ``add %(name)s`` mutants."""
    p = _py(tmp_path, "print(1)\n", "hello.py")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = AddReviewScreen(p)
        app.push_screen(screen)
        await pilot.pause()
        title = str(screen.query_one("#review-body").border_title)
    assert title == "Add hello.py"


# ---------------------------------------------------------------------------
# action_accept — storage mode
# ---------------------------------------------------------------------------


async def test_accept_reference_mode_records_reference(tmp_path):
    """Picking "Link the original" (radio index 1) must add a REFERENCE entry. Kills the
    mode-logic mutants: reference=None, ``pressed_index == 2``, the dropped ``mode=`` kwarg,
    and the ``"reference"`` string garbles (``XXreferenceXX`` / ``REFERENCE``) — every one of
    those flips the stored entry to copy mode (or an unknown mode)."""
    p = _py(tmp_path, "print(1)\n", "linkme.py")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = AddReviewScreen(p)
        app.push_screen(screen)
        await pilot.pause()
        list(screen.query_one("#rv-mode", RadioSet).query(RadioButton))[1].value = True
        await pilot.pause()
        assert screen.query_one("#rv-mode", RadioSet).pressed_index == 1
        screen.action_accept()
        await pilot.pause()
    entries = store.list_entries()
    assert len(entries) == 1
    assert entries[0].meta.mode == "reference"


async def test_accept_copy_uses_typed_name_desc_and_deps(tmp_path):
    """A copy add carries the user's typed name, description and dependencies through to the
    stored entry — pinning the ``query_one`` reads whose expect_type is pragma'd equivalent."""
    p = _py(tmp_path, "print(1)\n", "raw.py")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = AddReviewScreen(p)
        app.push_screen(screen)
        await pilot.pause()
        screen.query_one("#rv-name", Input).value = "chosen-name"
        screen.query_one("#rv-desc", Input).value = "one line of docs"
        screen.query_one("#rv-deps", Input).value = "httpx"
        await pilot.pause()
        screen.action_accept()
        await pilot.pause()
    entries = {e.meta.name: e for e in store.list_entries()}
    assert "chosen-name" in entries
    entry = entries["chosen-name"]
    assert entry.meta.mode == "copy"
    assert entry.meta.description == "one line of docs"
    # copy-mode deps are injected into the stored copy's PEP 723 block (comment-only, A5-safe).
    block = pep723.parse_block(entry.script_path.read_text(encoding="utf-8"))
    assert block is not None
    assert block["dependencies"] == ["httpx"]


async def test_accept_writes_only_checked_candidate_params(tmp_path):
    """The panel writes param declarations for exactly the ticked candidate checkboxes, each
    read by its own ``#rv-cand-{i}`` id. Kills the ``query_one(Checkbox)`` selector-drop, which
    reads the FIRST checkbox for every index — with cand-0 unticked that yields an empty pick
    and writes no params at all, so the stored copy would be missing the AREA declaration."""
    p = _py(tmp_path, "CITY = 'x'\nAREA = 'y'\nprint(CITY, AREA)\n", "consts.py")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = AddReviewScreen(p)
        app.push_screen(screen)
        await pilot.pause()
        screen.query_one("#rv-cand-0", Checkbox).value = False  # CITY: not a parameter
        screen.query_one("#rv-cand-1", Checkbox).value = True  # AREA: keep it
        await pilot.pause()
        screen.action_accept()
        await pilot.pause()
    entry = store.list_entries()[0]
    specs = metawriter.read_params(entry.script_path.read_text(encoding="utf-8"))
    assert {s.name for s in specs} == {"AREA"}


async def test_accept_preserves_non_utf8_source_bytes(tmp_path):
    """The review panel's comment insertion must round-trip bytes outside UTF-8."""
    p = tmp_path / "raw.sh"
    p.write_bytes(b"#!/bin/sh\nWIDTH=800\nprintf '\xff\\n'\n")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = AddReviewScreen(p, kind="shell")
        app.push_screen(screen)
        await pilot.pause()
        assert screen.query_one("#rv-cand-0", Checkbox).value
        screen.action_accept()
        await pilot.pause()

    rewritten = store.list_entries()[0].script_path.read_bytes()
    assert b"\xff" in rewritten
    assert b"\xef\xbf\xbd" not in rewritten


# ---------------------------------------------------------------------------
# action_edit_source — overrides survive the edit→rescan recompose
# ---------------------------------------------------------------------------


async def test_edit_source_preserves_typed_deps_across_rescan(tmp_path, monkeypatch):
    """A dependency string the user typed must survive the edit→rescan recompose, stored under
    the ``deps`` override key from the ``#rv-deps`` box. Kills every mutant that loses it: the
    ``deps_box = None`` short-circuit, the garbled ``#rv-deps`` selectors (``XX#rv-depsXX`` /
    ``#RV-DEPS`` both match nothing → the ``if deps_box`` guard is skipped), the ``= None``
    value, and the wrong override keys (``XXdepsXX`` / ``DEPS``) — each drops back to the
    (empty) suggestion instead."""
    _quiet_editor(monkeypatch)
    p = _py(tmp_path, "print(1)\n", "s.py")  # no imports → the suggested-deps box is empty
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = AddReviewScreen(p)
        app.push_screen(screen)
        await pilot.pause()
        screen.query_one("#rv-deps", Input).value = "my-special-pkg"
        await pilot.pause()
        screen.action_edit_source()
        await pilot.pause()
        assert screen.query_one("#rv-deps", Input).value == "my-special-pkg"


async def test_edit_source_preserves_name_desc_and_mode_overrides(tmp_path, monkeypatch):
    """Name, description and the storage-mode choice the user set are captured before the editor
    hand-off and restored into the recomposed panel — pinning the ``query_one`` reads on those
    three lines (expect_type pragma'd equivalent)."""
    _quiet_editor(monkeypatch)
    p = _py(tmp_path, "print(1)\n", "s.py")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = AddReviewScreen(p)
        app.push_screen(screen)
        await pilot.pause()
        screen.query_one("#rv-name", Input).value = "kept-name"
        screen.query_one("#rv-desc", Input).value = "kept desc"
        list(screen.query_one("#rv-mode", RadioSet).query(RadioButton))[1].value = True
        await pilot.pause()
        assert screen.query_one("#rv-mode", RadioSet).pressed_index == 1
        screen.action_edit_source()
        await pilot.pause()
        assert screen.query_one("#rv-name", Input).value == "kept-name"
        assert screen.query_one("#rv-desc", Input).value == "kept desc"
        assert (
            screen.query_one("#rv-mode", RadioSet).pressed_index == 1
        )  # still "Link the original"


async def test_edit_source_opens_original_and_rescans_new_content(tmp_path, monkeypatch):
    """Ctrl+E opens the user's OWN path in their editor, then re-reads and re-analyses the file
    and recomposes the panel. Kills ``open_in_editor(None)`` (the wrong target) and both
    ``refresh(recompose=None|False)`` mutants (no rebuild → the freshly-detected constant never
    surfaces as a candidate checkbox)."""
    monkeypatch.setattr(tui.MenuApp, "suspend", lambda self: contextlib.nullcontext())
    p = _py(tmp_path, "print(1)\n", "s.py")  # starts with no candidates
    opened: dict[str, object] = {}

    def fake_open(path):
        opened["path"] = path
        if path is not None:
            path.write_text("NEWCONST = 'z'\nprint(NEWCONST)\n", encoding="utf-8")

    monkeypatch.setattr("skit.tui_add.editor.open_in_editor", fake_open)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = AddReviewScreen(p)
        app.push_screen(screen)
        await pilot.pause()
        assert not screen.query("#rv-cand-0")  # nothing to tick yet
        screen.action_edit_source()
        await pilot.pause()
        assert opened["path"] == p  # opened the real file, not None
        assert screen.query("#rv-cand-0")  # rescan + recompose surfaced the new constant
        assert "NEWCONST" in str(screen.query_one("#rv-cand-0", Checkbox).label)


async def test_edit_source_editor_error_notifies_and_skips_rescan(tmp_path, monkeypatch):
    """When the editor can't be launched the failure surfaces AFTER resume as an
    error-severity notification (#14 retired the in-suspend print — a suspended Textual
    app repaints over raw writes), and the early return skips the rescan. The file is
    deleted before the action: a dropped return would fall through to the re-read and
    toast a second, "Can't read" error — the single-toast assertion pins both."""
    monkeypatch.setattr(tui.MenuApp, "suspend", lambda self: contextlib.nullcontext())

    def boom(path):
        raise editor.EditorError("no editor configured")

    monkeypatch.setattr("skit.tui_add.editor.open_in_editor", boom)
    toasts: list[tuple[str, str]] = []
    monkeypatch.setattr(
        tui.MenuApp,
        "notify",
        lambda self, message, **kw: toasts.append((str(message), str(kw.get("severity")))),
    )
    p = _py(tmp_path, "print(1)\n", "s.py")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = AddReviewScreen(p)
        app.push_screen(screen)
        await pilot.pause()
        p.unlink()  # a rescan would now fail loudly — the early return must prevent it
        screen.action_edit_source()
        await pilot.pause()
    assert toasts == [("no editor configured", "error")]


async def test_edit_source_rescans_non_utf8_original_with_replace(tmp_path, monkeypatch):
    """The rescan re-reads the user's original with ``errors="replace"``, so a byte that is not
    valid UTF-8 (a latin-1 é here) never crashes the return-from-editor path — it decodes to the
    replacement character. Pins the ``read_text`` errors handler on the pragma'd encoding line."""
    _quiet_editor(monkeypatch)
    p = tmp_path / "weird.py"
    p.write_bytes(b"NAME = 'caf\xe9'\nprint(NAME)\n")  # 0xE9 is invalid UTF-8
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = AddReviewScreen(p)
        app.push_screen(screen)
        await pilot.pause()
        screen.action_edit_source()  # must not raise
        await pilot.pause()
        assert "�" in screen._text  # the bad byte decoded via the replacement char


def _capture_toasts(monkeypatch):
    """Capture every (message, severity) the screen toasts through the app."""
    toasts: list[tuple[str, str]] = []
    monkeypatch.setattr(
        tui.MenuApp,
        "notify",
        lambda self, message, **kw: toasts.append((str(message), str(kw.get("severity")))),
    )
    return toasts


# ---------------------------------------------------------------------------
# action_accept — refuse a Ctrl+S while the source text is unreadable
# ---------------------------------------------------------------------------


async def test_accept_refuses_when_source_unreadable(tmp_path, monkeypatch):
    """A panel whose source couldn't be read must refuse Ctrl+S: it toasts the exact read
    error at severity 'error' and stores nothing (never a half-known entry). Kills notify(None),
    the nulled/garbled/omitted severity, and would catch a dropped early return."""
    toasts = _capture_toasts(monkeypatch)
    bad = tmp_path / "adir"
    bad.mkdir()  # reading a directory raises OSError → __init__ records _text_error
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = AddReviewScreen(bad)
        app.push_screen(screen)
        await pilot.pause()
        assert screen._text_error  # the panel opened in the read-error state
        expected = screen._text_error
        screen.action_accept()
        await pilot.pause()
    assert toasts == [(expected, "error")]
    assert store.list_entries() == []  # nothing stored


# ---------------------------------------------------------------------------
# action_edit_source — the rescan read-error path (distinct from the editor-launch error)
# ---------------------------------------------------------------------------


async def test_edit_source_rescan_read_error_toasts_and_records(tmp_path, monkeypatch):
    """When the post-edit rescan can't read the file, the panel records the exact 'Can't read
    <path>: <error>' message and toasts it at severity 'error'. The error text comes from
    ``exc.strerror or str(exc)``; forcing a strerror-less OSError('boom') pins that the message
    is the exception text, not 'None' (the ``or``->``and`` and ``str(None)`` mutants), and pins
    the msgid, the notify target and the severity."""
    from pathlib import Path

    monkeypatch.setattr(tui.MenuApp, "suspend", lambda self: contextlib.nullcontext())
    toasts = _capture_toasts(monkeypatch)
    state = {"raise": False}
    real_read = Path.read_text

    def fake_read(self, *a, **k):
        if state["raise"]:
            raise OSError("boom")  # strerror is None → falls back to str(exc) == "boom"
        return real_read(self, *a, **k)

    monkeypatch.setattr(Path, "read_text", fake_read)

    def fake_open(path):
        state["raise"] = True  # the very next read (the rescan) fails

    monkeypatch.setattr("skit.tui_add.editor.open_in_editor", fake_open)
    p = _py(tmp_path, "print(1)\n", "s.py")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = AddReviewScreen(p)
        app.push_screen(screen)
        await pilot.pause()
        screen.action_edit_source()
        state["raise"] = False  # let the recompose read normally
        await pilot.pause()
        recorded = screen._text_error
    assert recorded == f"Can't read {p}: boom"
    assert (f"Can't read {p}: boom", "error") in toasts


async def test_edit_source_clears_text_error_on_clean_rescan(tmp_path, monkeypatch):
    """A rescan that reads cleanly resets ``_text_error`` to the empty string (never None): the
    panel returns to a no-error state so Ctrl+S is unblocked."""
    _quiet_editor(monkeypatch)
    p = _py(tmp_path, "print(1)\n", "s.py")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = AddReviewScreen(p)
        app.push_screen(screen)
        await pilot.pause()
        screen.action_edit_source()
        await pilot.pause()
        assert screen._text_error == ""  # cleared to "", not None


# ---------------------------------------------------------------------------
# action_edit_source — the python-pin typed-wins rule and tick survival
# ---------------------------------------------------------------------------


async def test_edit_source_python_typed_wins_over_auto_pin(tmp_path, monkeypatch):
    """A versioned shebang auto-fills requires-python, but a value the user typed into #rv-python
    must win over that auto pin across the edit→rescan (the typed override survives, and the auto
    pin no longer overwrites it)."""
    _quiet_editor(monkeypatch)
    p = _py(tmp_path, "#!/usr/bin/env python3.12\nprint(1)\n", "pinned.py")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = AddReviewScreen(p)
        app.push_screen(screen)
        await pilot.pause()
        assert screen.query_one("#rv-python", Input).value == ">=3.12,<3.13"  # auto pin
        screen.query_one("#rv-python", Input).value = ">=3.10"
        await pilot.pause()
        screen.action_edit_source()
        await pilot.pause()
        assert screen.query_one("#rv-python", Input).value == ">=3.10"  # the typed value won


async def test_edit_source_tick_override_survives_rescan(tmp_path, monkeypatch):
    """A candidate the user unticked keeps that decision across the edit→rescan recompose (the
    name-keyed tick_overrides), so a rescan of unchanged text never silently re-ticks it."""
    _quiet_editor(monkeypatch)
    p = _py(tmp_path, "CITY = 'x'\nprint(CITY)\n", "c.py")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = AddReviewScreen(p)
        app.push_screen(screen)
        await pilot.pause()
        assert screen.query_one("#rv-cand-0", Checkbox).value is True  # ticked by default
        screen.query_one("#rv-cand-0", Checkbox).value = False
        await pilot.pause()
        screen.action_edit_source()
        await pilot.pause()
        assert screen.query_one("#rv-cand-0", Checkbox).value is False  # decision survived


# ---------------------------------------------------------------------------
# _collected_python — the '-'/'none' → "" automatic token
# ---------------------------------------------------------------------------


async def test_collected_python_none_token_means_automatic(tmp_path):
    """The #rv-python field collects verbatim, except the CLI's automatic tokens: '-' and any
    case of 'none' collapse to "" (automatic). Kills the .upper() case-fold and both 'none'
    garbles, and pins that a real constraint passes through."""
    p = _py(tmp_path, "print(1)\n", "s.py")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = AddReviewScreen(p)
        app.push_screen(screen)
        await pilot.pause()
        field = screen.query_one("#rv-python", Input)
        field.value = "none"
        assert screen._collected_python() == ""
        field.value = "NONE"
        assert screen._collected_python() == ""
        field.value = "-"
        assert screen._collected_python() == ""
        field.value = ">=3.13"
        assert screen._collected_python() == ">=3.13"  # a real constraint is kept verbatim


# ---------------------------------------------------------------------------
# _collected_deps — the npm vs uv split branches
# ---------------------------------------------------------------------------


async def test_collected_deps_uv_splits_on_the_pep723_grammar(tmp_path):
    """A python (uv) entry collects deps through the PEP 508-aware splitter, so a single
    requirement carrying an internal comma (requests>=2,<3) stays one item."""
    p = _py(tmp_path, "print(1)\n", "s.py")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = AddReviewScreen(p)
        app.push_screen(screen)
        await pilot.pause()
        screen.query_one("#rv-deps", Input).value = "requests>=2,<3, rich"
        assert screen._collected_deps() == ["requests>=2,<3", "rich"]


async def test_collected_deps_npm_splits_on_the_npm_grammar(tmp_path):
    """A js (npm) entry collects deps through the npm splitter — a scoped package name and a
    plain one, comma separated."""
    js = tmp_path / "w.js"
    js.write_text("const x = 1\n", encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = AddReviewScreen(js, kind="js")
        app.push_screen(screen)
        await pilot.pause()
        screen.query_one("#rv-deps", Input).value = "chalk, @scope/pkg"
        assert screen._collected_deps() == ["chalk", "@scope/pkg"]


# ---------------------------------------------------------------------------
# _store_entry — non-python interpreter-from-shebang, description, npm update gate
# ---------------------------------------------------------------------------


async def test_store_entry_records_empty_interpreter_for_unregistered_shebang(tmp_path):
    """The recorded interpreter is the shebang's program ONLY when that program is one the kind
    knows (`spec is not None and program in spec.shebangs`). An unregistered program (customruby,
    not in ruby's shebangs) records "" — the ``and``->``or`` mutant would record 'customruby'
    instead. The typed description also lands (kills description=None / dropped kwarg)."""
    rb = tmp_path / "tool.rb"
    rb.write_text("#!/usr/bin/env customruby\nputs 1\n", encoding="utf-8")  # no leading comment
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = AddReviewScreen(rb, kind="ruby")
        app.push_screen(screen)
        await pilot.pause()
        screen.query_one("#rv-name", Input).value = "rubytool"
        screen.query_one("#rv-desc", Input).value = "does a ruby thing"
        await pilot.pause()
        screen.action_accept()
        await pilot.pause()
    entry = store.resolve("rubytool")
    assert entry.meta.kind == "ruby"
    assert entry.meta.interpreter == ""  # unregistered program → the kind's default runner
    assert entry.meta.description == "does a ruby thing"  # typed desc, not the (empty) extract


async def test_store_entry_npm_reference_records_no_dependencies(tmp_path, monkeypatch):
    """The deps→copy update gate (`deps and mode == "copy"`) must NOT fire on a reference add:
    an npm reference records nothing (its script lives in its own project). The ``and``->``or``
    mutant fires update_dependencies on the reference entry — npm reference is refused loudly, so
    the add errors out instead of completing. Pinned by the clean dismiss + no error toast (the
    reference entry is created by add_script either way, so entry count alone can't tell them
    apart)."""
    toasts = _capture_toasts(monkeypatch)
    js = tmp_path / "ref.js"
    js.write_text('import express from "express"\nconsole.log(1)\n', encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        captured: dict[str, str | None] = {}
        screen = AddReviewScreen(js, kind="js")
        app.push_screen(screen, lambda v: captured.__setitem__("result", v))
        await pilot.pause()
        assert screen.query_one("#rv-deps", Input).value  # scanned deps are present in the field
        list(screen.query_one("#rv-mode", RadioSet).query(RadioButton))[1].value = True  # link it
        await pilot.pause()
        assert screen.query_one("#rv-mode", RadioSet).pressed_index == 1
        screen.action_accept()
        await pilot.pause()
        await pilot.pause()
    (entry,) = store.list_entries()
    assert entry.meta.mode == "reference"
    assert entry.meta.dependencies is None  # nothing recorded on the reference
    assert captured["result"] == entry.slug  # accept completed and dismissed (no update failure)
    assert toasts == []  # no error toast — update_dependencies was never called
