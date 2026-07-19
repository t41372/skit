"""TUI coverage for the drafts boundary and validate-then-write on the add panel.

Every assertion pins an observable add-review contract:

  * an INFERRED exe on a resumed draft is remapped to the ASK (KindPickModal), which for a
    draft offers NO "A program" option — the drafts boundary forbids a reference-mode entry;
  * the fresh _reviewed success-unlink is MODE-GATED: it deletes the draft only when the
    store holds a copy, so a non-copy dismissal keeps the file (no lane deletes what the
    store doesn't hold);
  * candidate ticks survive the edit→rescan recompose (name-keyed overrides), while a NEW
    candidate takes its detection default;
  * AddReviewScreen normalizes '-'/'none' in #rv-python to automatic, and validates uv-flavor
    deps and the python constraint BEFORE storing (notify error, panel stays open); npm deps
    are NOT validated (the installer owns that grammar).
"""

from __future__ import annotations

import contextlib
import dataclasses
import os
from pathlib import Path

import pytest
from textual.widgets import Checkbox, Input, OptionList

from skit import editor, store, tui
from skit.tui_add import (
    AddReviewScreen,
    AddSourceScreen,
    KindPickModal,
)


@pytest.fixture(autouse=True)
def tmp_store(tmp_path, monkeypatch):
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path / "config"))
    monkeypatch.setenv("SKIT_LANG", "en")


@pytest.fixture
def no_suspend(monkeypatch):
    monkeypatch.setattr(tui.MenuApp, "suspend", lambda self: contextlib.nullcontext())


def _editor_writes(monkeypatch, content: str | None):
    seen: dict[str, object] = {}

    def fake(path):
        seen["path"] = path
        if content is not None:
            path.write_text(content, encoding="utf-8")

    monkeypatch.setattr(editor, "open_in_editor", fake)
    return seen


def _option_ids(modal: KindPickModal) -> set[str | None]:
    ol = modal.query_one(OptionList)
    return {ol.get_option_at_index(i).id for i in range(ol.option_count)}


def _py(tmp_path, body: str, name: str = "job.py") -> Path:
    p = tmp_path / name
    p.write_text(body, encoding="utf-8")
    return p


def _cand_by_prefix(screen: AddReviewScreen, prefix: str) -> Checkbox:
    """The candidate checkbox whose label starts with `prefix` (a const name)."""
    return next(cb for cb in screen.query(Checkbox) if str(cb.label).startswith(prefix))


# ==========================================================================
# 1. An inferred exe on a resumed draft is remapped to the ASK (no program option)
# ==========================================================================


async def test_draft_resume_inferred_exe_routes_to_ask_without_program_option(
    tmp_path, monkeypatch
):
    """A resumed draft that INFERS exe (a hand-planted +x bit on an extensionless draft) is
    remapped to unknown → the KindPickModal, which for a draft offers no 'A program' option:
    an exe entry's reference mode is the one shape the drafts boundary forbids."""
    from skit.paths import drafts_dir

    drafts_dir().mkdir(parents=True, exist_ok=True)
    draft = drafts_dir() / "skit-new-binish"
    draft.write_text("opaque program bytes\n", encoding="utf-8")
    os.chmod(draft, 0o755)  # noqa: S103 — POSIX infer_kind classifies +x as exe
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        source = AddSourceScreen()
        app.push_screen(source)
        await pilot.pause()
        source.query_one("#add-path", Input).value = str(draft)
        source.action_continue_add()
        await pilot.pause()
        modal = app.screen
        assert isinstance(modal, KindPickModal)  # exe→unknown remap → ASK, not ExeReviewScreen
        assert not modal._offer_exe
        assert "exe" not in _option_ids(modal)
    draft.unlink(missing_ok=True)


# ==========================================================================
# 2. The fresh _reviewed success-unlink is MODE-GATED
# ==========================================================================


async def test_fresh_draft_copy_flow_unlinks_the_file(tmp_path, no_suspend, monkeypatch):
    """The copy arc (pin): a normal fresh draft lands as a copy, so the draft is consumed."""
    seen = _editor_writes(monkeypatch, "import sys\nprint('drafted')\n")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        app.push_screen(AddSourceScreen())
        await pilot.pause()
        await pilot.press("ctrl+n")
        await pilot.pause()
        review = app.screen
        assert isinstance(review, AddReviewScreen)
        review.query_one("#rv-name", Input).value = "copied"
        review.action_accept()
        await pilot.pause()
    assert store.resolve("copied").meta.mode == "copy"
    assert not seen["path"].exists()  # copy: the store holds it, so the draft is unlinked


async def test_fresh_draft_keeps_the_file_when_the_entry_is_not_a_copy(
    tmp_path, no_suspend, monkeypatch
):
    """The non-copy arc: the mode-gate reads mode, so a dismissal that resolves to a
    non-copy entry keeps the file (no lane deletes what the store doesn't hold). Real fresh
    authoring always copies, so the arc is exercised by making resolve report reference mode.
    Kills the mutant that drops the `== "copy"` condition (would delete the file regardless)."""
    seen = _editor_writes(monkeypatch, "import sys\nprint('drafted')\n")
    orig_resolve = store.resolve

    def ref_resolve(slug):
        e = orig_resolve(slug)
        return dataclasses.replace(e, meta=dataclasses.replace(e.meta, mode="reference"))

    monkeypatch.setattr(store, "resolve", ref_resolve)
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        app.push_screen(AddSourceScreen())
        await pilot.pause()
        await pilot.press("ctrl+n")
        await pilot.pause()
        review = app.screen
        assert isinstance(review, AddReviewScreen)
        review.query_one("#rv-name", Input).value = "kept"
        review.action_accept()
        await pilot.pause()
    assert seen["path"].exists()  # non-copy dismissal: the gate kept the file
    seen["path"].unlink(missing_ok=True)


# ==========================================================================
# 3. Candidate ticks survive the edit→rescan recompose
# ==========================================================================


async def test_candidate_tick_survives_a_noop_edit_rescan(tmp_path, monkeypatch):
    """Untick a candidate, Ctrl+E a no-op edit, return → the tick is still unticked (the
    rescan refreshes detection but must not throw away the user's tick)."""
    monkeypatch.setattr(editor, "open_in_editor", lambda p: 0)
    monkeypatch.setattr(tui.MenuApp, "suspend", lambda self: contextlib.nullcontext())
    p = _py(tmp_path, 'CITY = "Taipei"\nprint(CITY)\n', "cand.py")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = AddReviewScreen(p)
        app.push_screen(screen)
        await pilot.pause()
        cb = screen.query_one("#rv-cand-0", Checkbox)
        assert cb.value is True  # CITY ticked by default
        cb.value = False  # the user unticks it
        screen.action_edit_source()  # no-op editor, then rescan/recompose
        await pilot.pause()
        await pilot.pause()
        assert screen.query_one("#rv-cand-0", Checkbox).value is False  # tick persisted


async def test_edit_source_capture_skips_a_candidate_with_no_checkbox(tmp_path, monkeypatch):
    """A getopts (modeled-reader) shell with a bare const has a candidate in analysis but NO
    tick checkbox rendered — the modeled form replaces the list. action_edit_source's capture
    loop guards on `if boxes:`, so it never queries a checkbox that isn't mounted: no crash,
    no phantom override (the guarded arc of the tick-capture loop)."""
    monkeypatch.setattr(editor, "open_in_editor", lambda p: 0)
    monkeypatch.setattr(tui.MenuApp, "suspend", lambda self: contextlib.nullcontext())
    sh = tmp_path / "opt.sh"
    sh.write_text(
        "#!/usr/bin/env bash\nREGION=us-east-1\n"
        'while getopts "n:" o; do case $o in n) NAME=$OPTARG;; esac; done\n'
        'echo "$REGION $NAME"\n',
        encoding="utf-8",
    )
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = AddReviewScreen(sh, kind="shell")
        app.push_screen(screen)
        await pilot.pause()
        assert screen._analysis.candidates  # REGION is a candidate...
        assert not screen.query("#rv-cand-0")  # ...but the modeled reader hid the tick list
        screen.action_edit_source()  # the capture loop runs with boxes empty (the guarded arc)
        await pilot.pause()
        await pilot.pause()
        assert screen._tick_overrides == {}  # nothing captured — no checkbox to read
        assert app.screen is screen  # still the review, no crash


async def test_new_candidate_after_a_real_edit_takes_its_default(tmp_path, monkeypatch):
    """A NEW candidate appearing after a real edit takes its detection default (ticked),
    while the earlier candidate's unticked override is preserved."""

    def add_region(path):
        path.write_text(
            'CITY = "Taipei"\nREGION = "us-east-1"\nprint(CITY, REGION)\n', encoding="utf-8"
        )

    monkeypatch.setattr(editor, "open_in_editor", add_region)
    monkeypatch.setattr(tui.MenuApp, "suspend", lambda self: contextlib.nullcontext())
    p = _py(tmp_path, 'CITY = "Taipei"\nprint(CITY)\n', "cand2.py")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = AddReviewScreen(p)
        app.push_screen(screen)
        await pilot.pause()
        _cand_by_prefix(screen, "CITY").value = False  # untick CITY
        screen.action_edit_source()
        await pilot.pause()
        await pilot.pause()
        assert _cand_by_prefix(screen, "CITY").value is False  # override preserved
        assert _cand_by_prefix(screen, "REGION").value is True  # new candidate: default tick


# ==========================================================================
# 4. AddReviewScreen: '-' normalization + validate-then-write (uv), npm not validated
# ==========================================================================


async def test_review_dash_python_is_stored_as_automatic(tmp_path):
    """'-' in #rv-python normalizes to automatic: the entry commits with no requires-python."""
    p = _py(tmp_path, "print(1)\n", "auto.py")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = AddReviewScreen(p)
        app.push_screen(screen)
        await pilot.pause()
        screen.query_one("#rv-python", Input).value = "-"
        screen.query_one("#rv-name", Input).value = "autoentry"
        screen.action_accept()
        await pilot.pause()
        assert not isinstance(app.screen, AddReviewScreen)  # committed
    stored = (store.resolve("autoentry").dir / "script.py").read_text(encoding="utf-8")
    assert "requires-python" not in stored


async def test_review_rejects_a_bad_uv_dep_and_keeps_the_panel_open(tmp_path, monkeypatch):
    """An unparseable uv requirement is refused BEFORE storing: notify(severity=error), the
    panel stays open, nothing lands."""
    p = _py(tmp_path, "print(1)\n", "baddep.py")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = AddReviewScreen(p)
        app.push_screen(screen)
        await pilot.pause()
        notes: list[tuple[str, object]] = []
        monkeypatch.setattr(screen, "notify", lambda m, **kw: notes.append((m, kw.get("severity"))))
        screen.query_one("#rv-deps", Input).value = "@@@"
        screen.query_one("#rv-name", Input).value = "baddep"
        screen.action_accept()
        await pilot.pause()
        assert app.screen is screen  # still open
    assert any("package requirement" in m and sev == "error" for m, sev in notes)
    assert store.list_entries() == []  # nothing stored


async def test_review_rejects_a_bad_python_constraint_and_keeps_the_panel_open(
    tmp_path, monkeypatch
):
    p = _py(tmp_path, "print(1)\n", "badpy.py")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = AddReviewScreen(p)
        app.push_screen(screen)
        await pilot.pause()
        notes: list[tuple[str, object]] = []
        monkeypatch.setattr(screen, "notify", lambda m, **kw: notes.append((m, kw.get("severity"))))
        screen.query_one("#rv-python", Input).value = "not-a-version"
        screen.query_one("#rv-name", Input).value = "badpy"
        screen.action_accept()
        await pilot.pause()
        assert app.screen is screen  # still open
    assert any("version constraint" in m and sev == "error" for m, sev in notes)
    assert store.list_entries() == []


async def test_review_does_not_validate_npm_deps(tmp_path):
    """The complement: an npm dep string that would FAIL PEP 508 (a scoped package) still
    commits on a js add — the npm installer owns that grammar, not skit's validator."""
    src = tmp_path / "tool.js"
    src.write_text('import thing from "@scope/thing";\nconsole.log(thing);\n', encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = AddReviewScreen(src, kind="js")
        app.push_screen(screen)
        await pilot.pause()
        screen.query_one("#rv-deps", Input).value = "@scope/thing"
        screen.query_one("#rv-name", Input).value = "jstool"
        screen.action_accept()
        await pilot.pause()
        assert not isinstance(app.screen, AddReviewScreen)  # committed, not rejected
    entry = store.resolve("jstool")
    assert entry.meta.kind == "js"
    assert entry.meta.dependencies == ["@scope/thing"]
