"""Exact-behavior coverage for the Add flow (source step -> review panel).

Every assertion pins an OBSERVABLE contract: the error text a bad path shows, the entry a
good path commits to the store, the checkbox defaults/labels the detection honesty rules
render, the params written into a copy on accept, and the edit->rescan override plumbing.
Nothing here executes a line for its own sake.
"""

from __future__ import annotations

import contextlib
import os

import pytest
from textual.widgets import Checkbox, Input, OptionList, RadioButton, RadioSet, Static

from skit import editor, store, tui
from skit.langs.python import metawriter
from skit.tui_add import (
    AddReviewApp,
    AddReviewScreen,
    AddSourceScreen,
    ExeReviewScreen,
    KindPickModal,
    PromptReviewScreen,
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


def _static_text(screen) -> str:
    """Every Static's rendered text joined — the review panel is built from Statics
    (Checkbox / RadioButton subclass Static, so their labels ride along too)."""
    return "".join(str(w.render()) for w in screen.query(Static))


def _error(screen) -> str:
    return str(screen.query_one("#add-error", Static).render())


# ---------------------------------------------------------------------------
# AddSourceScreen: path field
# ---------------------------------------------------------------------------


async def test_submit_empty_path_is_a_silent_noop(tmp_path):
    """Enter on an empty path field must do nothing — no error, no screen change: the
    user simply hasn't typed anything yet."""
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        source = AddSourceScreen()
        app.push_screen(source)
        await pilot.pause()
        box = source.query_one("#add-path", Input)
        source._path_given(Input.Submitted(box, ""))
        await pilot.pause()
        assert _error(source) == ""  # no complaint
        assert isinstance(app.screen, AddSourceScreen)  # stayed put
        assert store.list_entries() == []


async def test_missing_path_shows_file_not_found(tmp_path):
    ghost = tmp_path / "ghost.py"  # never created
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        source = AddSourceScreen()
        app.push_screen(source)
        await pilot.pause()
        source.query_one("#add-path", Input).value = str(ghost)
        source.action_continue_add()  # path filled -> _submit_path
        await pilot.pause()
        error = _error(source)
        assert "File not found" in error
        assert "ghost.py" in error
        assert store.list_entries() == []


async def test_executable_path_opens_identity_review_then_adds(tmp_path):
    """A recognized executable no longer instant-adds: it gets an identity review
    (name + description) like every other kind — "nothing to detect inside a binary"
    justifies no tick list, not skipping identity. Ctrl+S commits the exe entry with the
    reviewed name/description."""
    exe = tmp_path / "runme.exe"
    # no shebang: a recognized shebang would (correctly) infer an interpreted kind now
    exe.write_text("opaque program bytes\n", encoding="utf-8")
    os.chmod(exe, 0o755)  # noqa: S103 — +x makes POSIX infer_kind classify it "exe" (Win: .exe suffix)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        source = AddSourceScreen()
        app.push_screen(source)
        await pilot.pause()
        source.query_one("#add-path", Input).value = str(exe)
        source.action_continue_add()
        await pilot.pause()
        review = app.screen
        assert isinstance(review, ExeReviewScreen)  # identity review, not an instant add
        assert review.query_one("#xv-name", Input).value == "runme"  # prefilled from the stem
        review.query_one("#xv-name", Input).value = "launcher"
        review.query_one("#xv-desc", Input).value = "the deploy tool"
        review.action_accept()  # Ctrl+S
        await pilot.pause()
        assert not isinstance(app.screen, (ExeReviewScreen, AddSourceScreen))  # both gone
    entries = store.list_entries()
    assert [e.meta.name for e in entries] == ["launcher"]
    assert entries[0].meta.kind == "exe"
    assert entries[0].meta.description == "the deploy tool"


async def test_exe_review_cancel_adds_nothing(tmp_path):
    """Esc on the exe identity review leaves the store untouched (the cancel branch)."""
    exe = tmp_path / "runme.exe"
    exe.write_text("opaque program bytes\n", encoding="utf-8")
    os.chmod(exe, 0o755)  # noqa: S103
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        source = AddSourceScreen()
        app.push_screen(source)
        await pilot.pause()
        source.query_one("#add-path", Input).value = str(exe)
        source.action_continue_add()
        await pilot.pause()
        review = app.screen
        assert isinstance(review, ExeReviewScreen)
        review.action_cancel()  # Esc → dismiss(None)
        await pilot.pause()
        assert isinstance(app.screen, AddSourceScreen)  # returned to the source step
    assert store.list_entries() == []


async def test_shell_script_path_opens_the_review_panel(tmp_path):
    """A shell script gets the SAME review panel python gets (the add flow must not be
    four different products by extension): identity prefilled from the comments, and
    accept records copy mode + the shebang-pinned interpreter."""
    sh = tmp_path / "deploy.sh"
    sh.write_text("#!/usr/bin/env zsh\n# Ship the current build\necho hi\n", encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        source = AddSourceScreen()
        app.push_screen(source)
        await pilot.pause()
        source.query_one("#add-path", Input).value = str(sh)
        source.action_continue_add()
        await pilot.pause()
        review = app.screen
        assert isinstance(review, AddReviewScreen)
        assert review.query_one("#rv-desc", Input).value == "Ship the current build"
        assert not review.query("#rv-deps")  # shell has no dependency story
        review.action_accept()
        await pilot.pause()
        entries = store.list_entries()
        assert [e.meta.kind for e in entries] == ["shell"]
        assert entries[0].meta.interpreter == "zsh"  # shebang outranks the kind default
        assert entries[0].meta.description == "Ship the current build"
        assert (entries[0].dir / "script.sh").exists()  # copy mode, extension kept


async def test_shell_add_surfaces_store_error(tmp_path):
    """add_script failures (name already taken) keep the review panel open with the
    error as a notification — same contract as the python panel."""
    store.add_python(_py(tmp_path, "print(1)\n", "other.py"), name="deploy")
    sh = tmp_path / "deploy.sh"
    sh.write_text("#!/bin/sh\necho hi\n", encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        source = AddSourceScreen()
        app.push_screen(source)
        await pilot.pause()
        source.query_one("#add-path", Input).value = str(sh)
        source.action_continue_add()
        await pilot.pause()
        review = app.screen
        assert isinstance(review, AddReviewScreen)
        review.action_accept()
        await pilot.pause()
        assert app.screen is review  # the error keeps the panel open
    assert len(store.list_entries()) == 1  # nothing new landed


async def test_executable_add_surfaces_store_error(tmp_path, monkeypatch):
    """When add_exe rejects the entry (here: a name already taken), the failure notifies
    and the review panel stays open — nothing is dismissed. The exe review's error branch,
    the twin of the python/shell panels' StoreError handling."""
    store.add_python(_py(tmp_path, "print(1)\n", "other.py"), name="runme")
    exe = tmp_path / "runme.exe"
    exe.write_text("opaque program bytes\n", encoding="utf-8")
    os.chmod(exe, 0o755)  # noqa: S103 — +x makes POSIX infer_kind classify it "exe" (Win: .exe suffix)
    notes: list[str] = []
    monkeypatch.setattr(
        ExeReviewScreen, "notify", lambda self, message, **kw: notes.append(message)
    )
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        source = AddSourceScreen()
        app.push_screen(source)
        await pilot.pause()
        source.query_one("#add-path", Input).value = str(exe)
        source.action_continue_add()
        await pilot.pause()
        review = app.screen
        assert isinstance(review, ExeReviewScreen)
        review.action_accept()  # name "runme" already taken → StoreError
        await pilot.pause()
        assert app.screen is review  # the error keeps the panel open
    assert any("already taken" in n for n in notes)
    assert [e.meta.name for e in store.list_entries()] == ["runme"]  # only the first


def _select_kind(modal: KindPickModal, kind_id: str) -> None:
    """Highlight the KindPickModal option with the given id and select it (the real
    OptionList.OptionSelected path, so the modal's own _picked handler dismisses)."""
    options = modal.query_one(OptionList)
    idx = next(
        i for i in range(options.option_count) if options.get_option_at_index(i).id == kind_id
    )
    options.highlighted = idx
    options.action_select()


async def _ask_kind_for(tmp_path, app, pilot, name: str = "notes.txt") -> KindPickModal:
    unknown = tmp_path / name
    unknown.write_text("some opaque text\n", encoding="utf-8")
    source = AddSourceScreen()
    app.push_screen(source)
    await pilot.pause()
    source.query_one("#add-path", Input).value = str(unknown)
    source.action_continue_add()
    await pilot.pause()
    modal = app.screen
    assert isinstance(modal, KindPickModal)
    return modal


async def test_kind_pick_lists_interpreted_kinds_plus_exe_and_prompt(tmp_path):
    """The ask offers the sorted interpreted kinds and the two catch-alls (a program /
    a prompt) — the exhaustive TUI twin of --kind/--exe/--prompt."""
    from skit.langs.registry import KNOWN_KINDS
    from skit.langs.registry import spec_for as _spec_for

    # prompt is family "interpreted" but appears once, as the dedicated catch-all below.
    interpreted = sorted(
        k
        for k in KNOWN_KINDS
        if (s := _spec_for(k)) is not None and s.family == "interpreted" and k != "prompt"
    )
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        modal = await _ask_kind_for(tmp_path, app, pilot)
        options = modal.query_one(OptionList)
        ids = [options.get_option_at_index(i).id for i in range(options.option_count)]
    assert ids == [*interpreted, "exe", "prompt"]  # interpreted kinds first, catch-alls last
    assert ids.count("prompt") == 1  # never duplicated (would crash OptionList)


async def test_kind_pick_options_show_translated_labels_ids_stay_raw(tmp_path):
    """The interpreted-kind options render their translated display labels (kindnames.
    kind_label), while the option ids stay the raw kinds the add flow routes on."""
    from skit.kindnames import kind_label

    app = tui.MenuApp()
    async with app.run_test() as pilot:
        modal = await _ask_kind_for(tmp_path, app, pilot)
        options = modal.query_one(OptionList)
        by_id = {
            options.get_option_at_index(i).id: str(options.get_option_at_index(i).prompt)
            for i in range(options.option_count)
        }
    # Every interpreted-kind option shows its kind_label, never the raw id.
    assert by_id["shell"] == kind_label("shell") == "Shell"
    assert by_id["js"] == kind_label("js") == "JavaScript"
    assert by_id["ts"] == kind_label("ts") == "TypeScript"
    # The catch-alls keep their own descriptive prompts; ids remain the routing kinds.
    assert set(by_id) >= {"shell", "js", "ts", "exe", "prompt"}


async def test_kind_pick_shell_routes_to_the_add_review_panel(tmp_path):
    """Picking an interpreted kind opens the same AddReviewScreen a recognized shell
    script would, with that kind — and accepting commits it."""
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        modal = await _ask_kind_for(tmp_path, app, pilot)
        _select_kind(modal, "shell")
        await pilot.pause()
        review = app.screen
        assert isinstance(review, AddReviewScreen)
        assert review._kind == "shell"
        review.action_accept()
        await pilot.pause()
    assert store.resolve("notes").meta.kind == "shell"


async def test_kind_pick_exe_routes_to_the_exe_review(tmp_path):
    """Picking "a program" opens the ExeReviewScreen (the identity review), not an
    instant add."""
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        modal = await _ask_kind_for(tmp_path, app, pilot)
        _select_kind(modal, "exe")
        await pilot.pause()
        review = app.screen
        assert isinstance(review, ExeReviewScreen)
        review.action_accept()
        await pilot.pause()
    assert store.resolve("notes").meta.kind == "exe"


async def test_kind_pick_prompt_routes_to_the_prompt_review(tmp_path):
    """Picking "a prompt for an AI agent" opens the PromptReviewScreen."""
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        modal = await _ask_kind_for(tmp_path, app, pilot)
        _select_kind(modal, "prompt")
        await pilot.pause()
        assert isinstance(app.screen, PromptReviewScreen)


async def test_kind_pick_cancel_adds_nothing(tmp_path):
    """Esc on the ask returns to the source step and adds nothing (the None branch of the
    _kind_picked callback)."""
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        modal = await _ask_kind_for(tmp_path, app, pilot)
        modal.action_cancel()
        await pilot.pause()
        assert isinstance(app.screen, AddSourceScreen)
    assert store.list_entries() == []


async def test_py_path_opens_review_and_accept_flows_back_a_slug(tmp_path):
    """A .py file pushes the review panel; accepting it dismisses the review with a slug,
    which the source screen's callback forwards on to dismiss the whole add flow."""
    p = _py(tmp_path, "print(1)\n", "tool.py")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        source = AddSourceScreen()
        app.push_screen(source)
        await pilot.pause()
        source.query_one("#add-path", Input).value = str(p)
        source.action_continue_add()  # path filled -> _submit_path -> push review
        await pilot.pause()
        review = app.screen
        assert isinstance(review, AddReviewScreen)
        review.action_accept()
        await pilot.pause()
        assert not isinstance(app.screen, (AddReviewScreen, AddSourceScreen))  # both gone
        assert [e.meta.name for e in store.list_entries()] == ["tool"]


# ---------------------------------------------------------------------------
# AddSourceScreen: command-template field
# ---------------------------------------------------------------------------


async def test_continue_with_everything_blank_does_nothing(tmp_path):
    """Enter with no path and no template is a no-op: the source step never invents a
    command out of empty fields."""
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        source = AddSourceScreen()
        app.push_screen(source)
        await pilot.pause()
        source.action_continue_add()  # path empty -> _submit_template, template empty -> return
        await pilot.pause()
        assert _error(source) == ""
        assert isinstance(app.screen, AddSourceScreen)
        assert store.list_entries() == []


async def test_template_without_a_name_is_rejected(tmp_path):
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        source = AddSourceScreen()
        app.push_screen(source)
        await pilot.pause()
        box = source.query_one("#add-template", Input)
        box.value = "echo hi"
        source._template_given(Input.Submitted(box, "echo hi"))  # name still blank
        await pilot.pause()
        assert "A name is required." in _error(source)
        assert store.list_entries() == []


async def test_template_with_a_name_creates_a_command(tmp_path):
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        source = AddSourceScreen()
        app.push_screen(source)
        await pilot.pause()
        source.query_one("#add-template", Input).value = "echo {msg}"
        source.query_one("#add-template-name", Input).value = "greet"
        source.action_continue_add()  # path empty -> _submit_template (happy path)
        await pilot.pause()
        assert not isinstance(app.screen, AddSourceScreen)  # dismissed
        entries = store.list_entries()
        assert [e.meta.name for e in entries] == ["greet"]
        assert entries[0].meta.kind == "command"


async def test_template_add_surfaces_store_error(tmp_path):
    """A name collision on the command template is shown inline; no dismiss, no dupe."""
    store.add_command("echo first", name="dup")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        source = AddSourceScreen()
        app.push_screen(source)
        await pilot.pause()
        box = source.query_one("#add-template", Input)
        box.value = "echo second"
        source.query_one("#add-template-name", Input).value = "dup"
        source._template_given(Input.Submitted(box, "echo second"))
        await pilot.pause()
        assert "already taken" in _error(source)
        assert isinstance(app.screen, AddSourceScreen)
        assert len(store.list_entries()) == 1  # the collision was refused


# ---------------------------------------------------------------------------
# AddReviewScreen: detection-honesty rendering
# ---------------------------------------------------------------------------


async def test_candidate_checkboxes_render_const_input_and_accumulator(tmp_path):
    """The parameter section: a clean const is checked by default, an accumulator const is
    unchecked with the loop-accumulator warning, and an input() call renders its own label."""
    src = 'CITY = "Taipei"\nname = input("Name? ")\nTOTAL = 0\nfor i in range(3):\n    TOTAL += i\n'
    p = _py(tmp_path, src, "cands.py")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = AddReviewScreen(p)
        app.push_screen(screen)
        await pilot.pause()
        text = _static_text(screen)
        assert "Tick the ones the run form should ask for:" in text
        assert "loop accumulator" in text  # the accumulator warning
        # Clean const defaults checked; the demoted accumulator defaults unchecked.
        assert screen.query_one("#rv-cand-0", Checkbox).value is True
        assert screen.query_one("#rv-cand-1", Checkbox).value is False
        labels = [str(cb.label) for cb in screen.query(Checkbox)]
        assert any("CITY" in lbl and "'Taipei'" in lbl for lbl in labels)  # const label
        assert any("TOTAL" in lbl for lbl in labels)
        assert any("input()" in lbl and "Name?" in lbl for lbl in labels)  # input label


async def test_uses_argv_shows_passthrough_hint(tmp_path):
    p = _py(tmp_path, "import sys\nprint(sys.argv[1])\n", "argvy.py")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = AddReviewScreen(p)
        app.push_screen(screen)
        await pilot.pause()
        assert "reads command-line arguments" in _static_text(screen)


async def test_cli_framework_reports_read_arguments(tmp_path):
    src = (
        "import argparse\nap = argparse.ArgumentParser()\n"
        "ap.add_argument('-o', '--output', required=True, help='output path')\n"
        "ap.add_argument('--fast', action='store_true')\n"
        "ap.add_argument('--mode', choices=['a', 'b'], default='a')\n"
        "ap.parse_args()\n"
    )
    p = _py(tmp_path, src, "cli.py")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = AddReviewScreen(p)
        app.push_screen(screen)
        await pilot.pause()
        text = _static_text(screen)
        assert "skit read this script's own arguments" in text
        assert "(3 fields)" in text  # all three add_argument calls modelled


async def test_pep723_block_deps_are_shown_read_only(tmp_path):
    src = (
        "# /// script\n"
        '# requires-python = ">=3.11"\n'
        '# dependencies = ["requests", "rich"]\n'
        "# ///\n"
        "print(1)\n"
    )
    p = _py(tmp_path, src, "declared.py")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = AddReviewScreen(p)
        app.push_screen(screen)
        await pilot.pause()
        text = _static_text(screen)
        assert "The script declares its own dependencies (PEP 723):" in text
        assert "needs Python >=3.11" in text
        assert "installs requests" in text
        assert "installs rich" in text
        # A declared block is read-only: no editable deps field.
        assert not screen.query("#rv-deps")


async def test_pep723_empty_block_says_none_declared(tmp_path):
    src = "# /// script\n# dependencies = []\n# ///\nprint(1)\n"
    p = _py(tmp_path, src, "empty.py")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = AddReviewScreen(p)
        app.push_screen(screen)
        await pilot.pause()
        text = _static_text(screen)
        assert "The script declares its own dependencies (PEP 723):" in text
        assert "(none declared)" in text


async def test_review_space_chip_absent_for_argparse_present_for_candidates(tmp_path):
    """The Space (Toggle) chip is advertised only when there ARE candidate checkboxes to
    toggle — the same condition that composes them. An argparse-driven script has none
    (advertising a dead key), a const-bearing script has them."""
    argp = _py(
        tmp_path,
        "import argparse\nap = argparse.ArgumentParser()\nap.add_argument('--x')\nap.parse_args()\n",
        "ap.py",
    )
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(AddReviewScreen(argp))
        await pilot.pause()
        keys = str(app.screen.query_one("#review-keys", Static).render())
        assert "Toggle" not in keys  # no candidates → no Space chip (no dead key taught)

    const = _py(tmp_path, 'CITY = "Taipei"\nprint(CITY)\n', "const.py")
    app2 = tui.MenuApp()
    async with app2.run_test() as pilot:
        app2.push_screen(AddReviewScreen(const))
        await pilot.pause()
        keys2 = str(app2.screen.query_one("#review-keys", Static).render())
        assert "Toggle" in keys2  # candidates present → the Space chip is advertised


async def test_space_toggles_the_focused_candidate(tmp_path):
    """The Space footer twin flips whichever candidate checkbox holds focus."""
    p = _py(tmp_path, 'CITY = "Taipei"\nprint(CITY)\n', "one.py")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = AddReviewScreen(p)
        app.push_screen(screen)
        await pilot.pause()
        cb = screen.query_one("#rv-cand-0", Checkbox)
        assert cb.value is True
        cb.focus()
        await pilot.pause()
        screen.action_toggle_candidate()
        await pilot.pause()
        assert cb.value is False  # flipped by the Space twin


# ---------------------------------------------------------------------------
# AddReviewScreen: edit -> rescan, accept, cancel
# ---------------------------------------------------------------------------


async def test_edit_source_on_pep723_script_records_no_deps_override(tmp_path, monkeypatch):
    """A declared-deps script has no editable deps field, so the edit->rescan override
    capture must skip it (querying it would raise) — yet still preserve the name."""
    monkeypatch.setattr(editor, "open_in_editor", lambda p: 0)
    monkeypatch.setattr(tui.MenuApp, "suspend", lambda self: contextlib.nullcontext())
    src = '# /// script\n# dependencies = ["rich"]\n# ///\nprint(1)\n'
    p = _py(tmp_path, src, "dep.py")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = AddReviewScreen(p)
        app.push_screen(screen)
        await pilot.pause()
        assert not screen.query("#rv-deps")  # nothing to capture
        screen.query_one("#rv-name", Input).value = "kept-name"
        screen.action_edit_source()  # recomposes
        await pilot.pause()
        await pilot.pause()
        assert screen.query_one("#rv-name", Input).value == "kept-name"  # survived rescan
        assert "deps" not in screen._overrides  # deps field skipped, no phantom override


async def test_edit_source_shows_editor_launch_failure_after_resume(tmp_path, monkeypatch):
    """A suspended terminal print disappears when Textual resumes; the failure must be
    delivered through Textual's real notification queue and leave the panel intact."""

    def boom(path):
        raise editor.EditorError("cannot launch editor")

    monkeypatch.setattr(editor, "open_in_editor", boom)
    monkeypatch.setattr(tui.MenuApp, "suspend", lambda self: contextlib.nullcontext())
    p = _py(tmp_path, "print(1)\n", "plain.py")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = AddReviewScreen(p)
        app.push_screen(screen)
        await pilot.pause()
        screen.action_edit_source()
        await pilot.pause()
        assert app.screen is screen
        assert any(note.message == "cannot launch editor" for note in app._notifications)


async def test_review_surfaces_initial_and_post_editor_os_errors(tmp_path, monkeypatch):
    missing = tmp_path / "vanished.py"
    initial = AddReviewScreen(missing)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(initial)
        await pilot.pause()
        assert "vanished.py" in str(initial.query_one("#rv-text-error", Static).render())
        initial.action_accept()
        await pilot.pause()
        assert app.screen is initial

        source = tmp_path / "edited.py"
        source.write_text("print(1)\n", encoding="utf-8")
        review = AddReviewScreen(source)
        app.pop_screen()
        app.push_screen(review)
        await pilot.pause()
        monkeypatch.setattr(tui.MenuApp, "suspend", lambda self: contextlib.nullcontext())
        monkeypatch.setattr(editor, "open_in_editor", lambda path: path.unlink())
        review.action_edit_source()
        await pilot.pause()
        error = str(review.query_one("#rv-text-error", Static).render())
        assert "edited.py" in error
        assert "No such file" in error


async def test_review_ctrl_e_in_input_is_end_of_line_not_editor(tmp_path, monkeypatch):
    """Ctrl+E is non-priority on the review panel (the Ctrl+A rule, one chord left): while
    an Input has focus it is that Input's end-of-line and must NOT open $EDITOR — the chip
    stays the mouse path mid-edit, the chord fires from non-Input focus."""
    edited: list[int] = []
    monkeypatch.setattr(AddReviewScreen, "action_edit_source", lambda self: edited.append(1))
    p = _py(tmp_path, "print(1)\n", "plain.py")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = AddReviewScreen(p)
        app.push_screen(screen)
        await pilot.pause()
        name = screen.query_one("#rv-name", Input)
        name.focus()
        name.value = "hello"
        name.cursor_position = 0
        await pilot.pause()
        await pilot.press("ctrl+e")
        await pilot.pause()
        assert name.cursor_position == len("hello")  # Ctrl+E moved the cursor to end-of-line
        assert edited == []  # …and did NOT open the editor


async def test_accept_pep723_script_keeps_the_declared_block(tmp_path):
    """Accepting a declared-deps script must not re-split a deps field (there is none): the
    stored copy keeps the script's own PEP 723 block verbatim."""
    src = '# /// script\n# dependencies = ["rich"]\n# ///\nprint(1)\n'
    p = _py(tmp_path, src, "dep.py")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = AddReviewScreen(p)
        app.push_screen(screen)
        await pilot.pause()
        screen.action_accept()
        await pilot.pause()
        assert not isinstance(app.screen, AddReviewScreen)  # committed and dismissed
        entries = store.list_entries()
        assert [e.meta.name for e in entries] == ["dep"]
        copy_text = (entries[0].dir / "script.py").read_text(encoding="utf-8")
        assert 'dependencies = ["rich"]' in copy_text  # block preserved, not rewritten


async def test_accept_name_conflict_notifies_and_stays(tmp_path, monkeypatch):
    """A name already in the store fails add_python with a StoreError: the panel notifies
    the user and stays open — it never dismisses over a committed-nothing."""
    store.add_python(_py(tmp_path, "print(1)\n", "taken.py"), name="conflict")
    p = _py(tmp_path, "print(2)\n", "fresh.py")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = AddReviewScreen(p)
        app.push_screen(screen)
        await pilot.pause()
        screen.query_one("#rv-name", Input).value = "conflict"
        captured: list[tuple[str, object]] = []
        monkeypatch.setattr(screen, "notify", lambda message, **kw: captured.append((message, kw)))
        screen.action_accept()
        await pilot.pause()
        assert app.screen is screen  # still open, not dismissed
        assert captured  # the failure was surfaced
        assert "already taken" in captured[0][0]
        assert captured[0][1] == {"severity": "error"}
        assert len(store.list_entries()) == 1  # nothing new committed


async def test_accept_writes_picked_params_into_the_copy(tmp_path):
    """A copy-mode, non-CLI script with a ticked candidate writes that parameter's
    definition into the stored copy's [tool.skit] block."""
    p = _py(tmp_path, 'CITY = "Taipei"\nprint(CITY)\n', "cand.py")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = AddReviewScreen(p)
        app.push_screen(screen)
        await pilot.pause()
        assert screen.query_one("#rv-cand-0", Checkbox).value is True  # CITY ticked by default
        screen.action_accept()
        await pilot.pause()
        assert not isinstance(app.screen, AddReviewScreen)
        entry = store.list_entries()[0]
        copy_text = (entry.dir / "script.py").read_text(encoding="utf-8")
        specs = metawriter.read_params(copy_text)
        assert [s.name for s in specs] == ["CITY"]  # the picked candidate was persisted


async def test_cancel_dismisses_without_committing(tmp_path):
    p = _py(tmp_path, "print(1)\n", "bye.py")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = AddReviewScreen(p)
        app.push_screen(screen)
        await pilot.pause()
        screen.action_cancel()
        await pilot.pause()
        assert not isinstance(app.screen, AddReviewScreen)
        assert store.list_entries() == []


async def test_cancelling_the_review_returns_to_the_source_step(tmp_path):
    """Cancelling the review hands None back to the source step's callback, which must NOT
    dismiss the add flow — the user lands back on the source screen with nothing committed."""
    p = _py(tmp_path, "print(1)\n", "tool.py")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        source = AddSourceScreen()
        app.push_screen(source)
        await pilot.pause()
        source.query_one("#add-path", Input).value = str(p)
        source.action_continue_add()  # -> push review
        await pilot.pause()
        review = app.screen
        assert isinstance(review, AddReviewScreen)
        review.action_cancel()  # dismiss(None) -> callback sees None -> no dismiss
        await pilot.pause()
        assert app.screen is source  # back on the source step, not dismissed
        assert store.list_entries() == []


async def test_space_on_a_non_checkbox_focus_is_a_noop(tmp_path):
    """The Space twin toggles only a focused candidate checkbox: with focus on the name
    field it leaves every checkbox untouched (no accidental flip, no crash)."""
    p = _py(tmp_path, 'CITY = "Taipei"\nprint(CITY)\n', "c.py")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = AddReviewScreen(p)
        app.push_screen(screen)
        await pilot.pause()
        screen.query_one("#rv-name", Input).focus()
        await pilot.pause()
        cb = screen.query_one("#rv-cand-0", Checkbox)
        before = cb.value
        screen.action_toggle_candidate()  # focus is the Input, not a Checkbox
        await pilot.pause()
        assert cb.value == before  # unchanged


# ---------------------------------------------------------------------------
# AddReviewScreen: reference-mode honesty (fold what accept would skip)
# ---------------------------------------------------------------------------


def _flip_mode(review, index: int) -> None:
    list(review.query_one("#rv-mode", RadioSet).query(RadioButton))[index].value = True


async def test_review_reference_mode_folds_params_but_keeps_python_deps(tmp_path):
    """Linking a python original folds the PARAMS section (skit never writes to the file)
    but keeps the deps section — uv deps still apply to a linked file — and shows the
    single-sentence note. Copy mode restores everything."""
    p = _py(tmp_path, "import sys\nprint(sys.argv)\n", "tool.py")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        review = AddReviewScreen(p, kind="python")
        app.push_screen(review)
        await pilot.pause()
        # copy mode on open: params visible, note hidden
        assert review.query_one("#rv-params-wrap").display is True
        assert review.query_one("#rv-ref-note", Static).display is False
        _flip_mode(review, 1)  # "Link the original"
        await pilot.pause()
        assert review.query_one("#rv-params-wrap").display is False  # params folded
        assert review.query_one("#rv-deps-wrap").display is True  # python (uv) deps stay
        note = review.query_one("#rv-ref-note", Static)
        assert note.display is True
        text = str(note.render())
        assert "parameter setup is skipped" in text
        assert "npm dependencies" not in text  # non-npm: single sentence only
        _flip_mode(review, 0)  # back to "Keep a copy"
        await pilot.pause()
        assert review.query_one("#rv-params-wrap").display is True  # restored
        assert review.query_one("#rv-ref-note", Static).display is False


async def test_review_reference_mode_npm_folds_deps_and_adds_second_sentence(tmp_path):
    """For an npm kind, linking folds the DEPS section too (npm deps apply to stored
    copies only) and the note gains the second sentence."""
    js = tmp_path / "tool.js"
    js.write_text("import chalk from 'chalk'\nconsole.log(chalk)\n", encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        review = AddReviewScreen(js, kind="js")
        app.push_screen(review)
        await pilot.pause()
        assert review.query_one("#rv-deps-wrap").display is True
        _flip_mode(review, 1)
        await pilot.pause()
        assert review.query_one("#rv-params-wrap").display is False
        assert review.query_one("#rv-deps-wrap").display is False  # npm deps folded too
        text = str(review.query_one("#rv-ref-note", Static).render())
        assert "parameter setup is skipped" in text
        assert "npm dependencies apply to stored copies only" in text  # second sentence


async def test_review_reference_prefill_folds_on_mount(tmp_path):
    """on_mount applies the initial visibility even when reference is PREFILLED (--ref):
    the panel opens already folded, no flip required."""
    js = tmp_path / "tool.js"
    js.write_text("console.log(1)\n", encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        review = AddReviewScreen(js, kind="js", reference=True)
        app.push_screen(review)
        await pilot.pause()
        assert review.query_one("#rv-mode", RadioSet).pressed_index == 1  # reference prefilled
        assert review.query_one("#rv-params-wrap").display is False  # already folded on open
        assert review.query_one("#rv-deps-wrap").display is False
        assert review.query_one("#rv-ref-note", Static).display is True


# ---------------------------------------------------------------------------
# AddReviewApp: the CLI face of the panel (`skit add x.py` in a terminal)
# ---------------------------------------------------------------------------


async def test_add_review_app_prefills_from_flags_and_accepts(tmp_path):
    """`skit add`'s flags land in the panel prefilled (still editable); accepting
    commits the entry — deps AND --python written as PEP 723 — and exits with the
    slug. (--python used to be silently dropped by the panel.)"""
    p = _py(tmp_path, 'CITY = "x"\nprint(CITY)\n')
    app = AddReviewApp(
        p, name="flagged", description="from flags", deps=["rich>=13"], requires_python=">=3.11"
    )
    async with app.run_test() as pilot:
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, AddReviewScreen)
        assert screen.query_one("#rv-name", Input).value == "flagged"
        assert screen.query_one("#rv-desc", Input).value == "from flags"
        assert screen.query_one("#rv-deps", Input).value == "rich>=13"
        screen.action_accept()
        await pilot.pause()
    entry = store.resolve("flagged")
    assert app.return_value == entry.slug
    assert entry.meta.description == "from flags"
    copy_text = (entry.dir / "script.py").read_text(encoding="utf-8")
    assert "rich>=13" in copy_text
    assert ">=3.11" in copy_text


async def test_add_review_app_ref_prefill_and_cancel_leaves_store_untouched(tmp_path):
    """--ref preselects the link radio; cancelling exits None and adds nothing."""
    p = _py(tmp_path, "print(1)\n")
    app = AddReviewApp(p, reference=True)
    async with app.run_test() as pilot:
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, AddReviewScreen)
        assert screen.query_one("#rv-mode", RadioSet).pressed_index == 1
        screen.action_cancel()
        await pilot.pause()
    assert app.return_value is None
    assert store.list_entries() == []


async def test_add_review_app_forwards_fresh_no_storage_section(tmp_path):
    """AddReviewApp forwards fresh=True to the screen: a freshly-hosted panel has no
    Storage section because a temp draft has no original to link."""
    p = _py(tmp_path, "print(1)\n")
    app = AddReviewApp(p, fresh=True)
    async with app.run_test() as pilot:
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, AddReviewScreen)
        assert screen._fresh is True
        assert not screen.query("#rv-mode")  # fresh: the storage ask is absent


def test_run_add_review_returns_the_apps_result(tmp_path, monkeypatch):
    """The blocking CLI entry: builds the app around the panel and hands back run()'s
    slug verbatim (run() itself needs a terminal — the app is pilot-tested above)."""
    from skit import tui_add

    p = _py(tmp_path, "print(1)\n")
    monkeypatch.setattr(tui_add.AddReviewApp, "run", lambda self: "slug-sentinel")
    assert tui_add.run_add_review(p, name="n") == "slug-sentinel"


# ---------------------------------------------------------------------------
# ExeReviewApp: the CLI face of the program identity review (`skit add ./tool
# --exe`, or an unclassifiable file picked as "A program", in a terminal)
# ---------------------------------------------------------------------------


async def test_exe_review_app_prefills_flags_and_accepts(tmp_path):
    """`skit add`'s --name/--description land in the panel prefilled (still editable);
    accepting commits the exe entry and exits with its slug — parity with the script and
    prompt panels, so a mouse can finish the add the kind modal started."""
    from skit.tui_add import ExeReviewApp, ExeReviewScreen

    prog = tmp_path / "tool"
    prog.write_text("#!/bin/sh\necho hi\n", encoding="utf-8")
    app = ExeReviewApp(prog, name="flagged", description="from flags")
    async with app.run_test() as pilot:
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, ExeReviewScreen)
        # The panel titles itself with the source FILE name (not the prefilled skit name).
        assert screen.query_one("#xv-body").border_title == "Add tool"
        assert screen.query_one("#xv-name", Input).value == "flagged"
        assert screen.query_one("#xv-desc", Input).value == "from flags"
        screen.action_accept()
        await pilot.pause()
    entry = store.resolve("flagged")
    assert app.return_value == entry.slug
    assert entry.meta.kind == "exe"
    assert entry.meta.description == "from flags"


async def test_exe_review_app_duplicate_name_notifies_error_and_stays(tmp_path, monkeypatch):
    """A name collision keeps the panel open: accept surfaces the StoreError as an error
    notification (message AND severity) and does NOT dismiss — nothing is stored twice."""
    from skit.tui_add import ExeReviewApp, ExeReviewScreen

    store.add_command("echo hi", name="taken")  # the exe accept will collide on this name
    prog = tmp_path / "tool"
    prog.write_text("#!/bin/sh\necho hi\n", encoding="utf-8")
    seen: list[tuple[str, object]] = []
    monkeypatch.setattr(
        ExeReviewScreen,
        "notify",
        lambda self, message, **kw: seen.append((message, kw.get("severity"))),
    )
    app = ExeReviewApp(prog, name="taken")
    async with app.run_test() as pilot:
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, ExeReviewScreen)
        screen.action_accept()
        await pilot.pause()
        assert app.screen is screen  # the error kept the panel open (no dismiss)
    assert len(seen) == 1
    message, severity = seen[0]
    assert "taken" in message
    assert "already" in message.lower()
    assert severity == "error"


async def test_exe_review_app_no_flags_defaults_to_stem_and_empty_desc(tmp_path):
    """No flags: the name field defaults to the file stem (not the whole name) and the
    description starts blank — the same defaults the in-app ExeReviewScreen shows."""
    from skit.tui_add import ExeReviewApp, ExeReviewScreen

    prog = tmp_path / "backup.sh"
    prog.write_text("#!/bin/sh\necho hi\n", encoding="utf-8")
    app = ExeReviewApp(prog)
    async with app.run_test() as pilot:
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, ExeReviewScreen)
        assert screen.query_one("#xv-name", Input).value == "backup"  # stem, not "backup.sh"
        assert screen.query_one("#xv-desc", Input).value == ""


async def test_exe_review_app_cancel_leaves_store_untouched(tmp_path):
    """Cancelling exits None and adds nothing — the panel's Esc, hosted alone."""
    from skit.tui_add import ExeReviewApp

    prog = tmp_path / "tool"
    prog.write_text("#!/bin/sh\necho hi\n", encoding="utf-8")
    app = ExeReviewApp(prog)
    async with app.run_test() as pilot:
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, ExeReviewScreen)
        screen.action_cancel()
        await pilot.pause()
    assert app.return_value is None
    assert store.list_entries() == []


def test_run_exe_review_forwards_flags_and_returns_the_apps_result(tmp_path, monkeypatch):
    """The blocking CLI entry builds ExeReviewApp with path/name/description verbatim
    (no arg dropped) and hands back run()'s slug (run() itself needs a terminal)."""
    from skit import tui_add

    prog = tmp_path / "tool"
    prog.write_text("hi\n", encoding="utf-8")
    seen: dict[str, object] = {}

    class _FakeApp:
        def __init__(self, path, *, name=None, description=None):
            seen.update(path=path, name=name, description=description)

        def run(self):
            return "slug-sentinel"

    monkeypatch.setattr(tui_add, "ExeReviewApp", _FakeApp)
    assert tui_add.run_exe_review(prog, name="n", description="d") == "slug-sentinel"
    assert seen == {"path": prog, "name": "n", "description": "d"}


# ---------------------------------------------------------------------------
# AddSourceApp / KindPickApp: the bare `skit add` (no path) CLI faces (issue #10)
# ---------------------------------------------------------------------------


async def test_add_source_app_command_lane_returns_slug(tmp_path):
    """Bare `skit add` in a terminal hosts the SAME source step the Library's `a`
    pushes; the command-template lane commits and the app exits with the new slug."""
    from skit.tui_add import AddSourceApp

    app = AddSourceApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        source = app.screen
        assert isinstance(source, AddSourceScreen)
        source.query_one("#add-template", Input).value = "echo {msg}"
        source.query_one("#add-template-name", Input).value = "greet"
        source.action_continue_add()
        await pilot.pause()
    entry = store.resolve("greet")
    assert app.return_value == entry.slug
    assert entry.meta.kind == "command"


async def test_add_source_app_escape_returns_none(tmp_path):
    from skit.tui_add import AddSourceApp

    app = AddSourceApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        assert isinstance(app.screen, AddSourceScreen)
        app.screen.action_cancel()
        await pilot.pause()
    assert app.return_value is None
    assert store.list_entries() == []


async def test_kind_pick_app_pick_returns_the_kind_string(tmp_path):
    """KindPickApp returns the picked KIND (not a slug): the CLI feeds it back into the
    ordinary per-kind dispatch. The constructor forwards filename/has_shebang/offer_exe
    into the hosted modal verbatim."""
    from skit.tui_add import KindPickApp

    app = KindPickApp("weird.xyz", has_shebang=False, offer_exe=True)
    async with app.run_test() as pilot:
        await pilot.pause()
        modal = app.screen
        assert isinstance(modal, KindPickModal)
        assert modal._filename == "weird.xyz"
        assert modal._has_shebang is False
        assert modal._offer_exe is True
        _select_kind(modal, "shell")
        await pilot.pause()
    assert app.return_value == "shell"


async def test_kind_pick_app_escape_returns_none(tmp_path):
    from skit.tui_add import KindPickApp

    app = KindPickApp("weird.xyz", has_shebang=True, offer_exe=False)
    async with app.run_test() as pilot:
        await pilot.pause()
        modal = app.screen
        assert isinstance(modal, KindPickModal)
        # The other end of the has_shebang / offer_exe flags: a shebang'd draft.
        assert modal._has_shebang is True
        assert modal._offer_exe is False
        modal.action_cancel()
        await pilot.pause()
    assert app.return_value is None


def test_run_add_source_returns_the_apps_result(tmp_path, monkeypatch):
    from skit import tui_add

    monkeypatch.setattr(tui_add.AddSourceApp, "run", lambda self: "slug-sentinel")
    assert tui_add.run_add_source() == "slug-sentinel"


def test_run_kind_pick_returns_the_apps_result(tmp_path, monkeypatch):
    from skit import tui_add

    monkeypatch.setattr(tui_add.KindPickApp, "run", lambda self: "shell")
    assert tui_add.run_kind_pick("x.xyz", has_shebang=False, offer_exe=True) == "shell"


def test_run_kind_pick_forwards_every_arg_into_the_app(tmp_path, monkeypatch):
    """run_kind_pick builds KindPickApp with filename/has_shebang/offer_exe and hands
    back run()'s result — no arg dropped or defaulted on the way in."""
    from skit import tui_add

    seen: dict[str, object] = {}

    class _FakeApp:
        def __init__(self, filename, *, has_shebang, offer_exe, suggested=None):
            seen.update(
                filename=filename, has_shebang=has_shebang, offer_exe=offer_exe, suggested=suggested
            )

        def run(self):
            return "picked-kind"

    monkeypatch.setattr(tui_add, "KindPickApp", _FakeApp)
    assert (
        tui_add.run_kind_pick("f.xyz", has_shebang=True, offer_exe=False, suggested="prompt")
        == "picked-kind"
    )
    assert seen == {
        "filename": "f.xyz",
        "has_shebang": True,
        "offer_exe": False,
        "suggested": "prompt",
    }


# ---------------------------------------------------------------------------
# KindPickModal: suggested-option ordering (the pre-highlighted likely answer)
# ---------------------------------------------------------------------------


def _kind_ids(modal: KindPickModal) -> list[str | None]:
    options = modal.query_one(OptionList)
    return [options.get_option_at_index(i).id for i in range(options.option_count)]


async def test_kind_pick_suggested_moves_the_option_to_first(tmp_path):
    """suggested='prompt' lifts the prompt option to FIRST position (so it is
    pre-highlighted); the rest keep their stable order."""
    from skit.tui_add import _ScreenHost

    plain = _ScreenHost(KindPickModal("notes.md", suggested=None))
    async with plain.run_test() as pilot:
        await pilot.pause()
        assert isinstance(plain.screen, KindPickModal)
        base = _kind_ids(plain.screen)
    assert base[-1] == "prompt"  # without a suggestion, prompt is the last catch-all

    picked = _ScreenHost(KindPickModal("notes.md", suggested="prompt"))
    async with picked.run_test() as pilot:
        await pilot.pause()
        assert isinstance(picked.screen, KindPickModal)
        suggested = _kind_ids(picked.screen)
    assert suggested[0] == "prompt"  # suggested is moved to first (pre-highlighted)
    # Same membership, only the prompt entry relocated to the front.
    assert set(suggested) == set(base)
    assert suggested[1:] == [i for i in base if i != "prompt"]


async def test_kind_pick_suggested_none_keeps_stable_order(tmp_path):
    """suggested=None (default) leaves the option order untouched."""
    from skit.tui_add import _ScreenHost

    host = _ScreenHost(KindPickModal("x.xyz"))
    async with host.run_test() as pilot:
        await pilot.pause()
        assert isinstance(host.screen, KindPickModal)
        ids = _kind_ids(host.screen)
    assert ids[-1] == "prompt"  # unmoved
    assert ids[-2] == "exe"


async def test_kind_pick_default_has_shebang_asks_the_cant_tell_question(tmp_path):
    """Constructed with the DEFAULT has_shebang (False), the modal asks the can't-tell
    question, never the shebang variant. Every production caller passes has_shebang
    explicitly, so only a default-relying construction can pin the default itself."""
    from textual.widgets import Label

    from skit.tui_add import _ScreenHost

    host = _ScreenHost(KindPickModal("x.xyz"))
    async with host.run_test() as pilot:
        await pilot.pause()
        assert isinstance(host.screen, KindPickModal)
        label = str(host.screen.query(Label).first().render())
    assert label == "What is x.xyz? skit can't tell from the name."


# ---------------------------------------------------------------------------
# AddSourceScreen._submit_path: bare .md, directory, and description round-trip
# ---------------------------------------------------------------------------


async def test_bare_md_path_asks_with_prompt_pre_highlighted(tmp_path):
    """A bare .md (no shebang, name gives no interpreter) opens the kind ask with the
    prompt option lifted to first — the TUI twin of the plain 'looks like a prompt?'."""
    md = tmp_path / "notes.md"
    md.write_text("just some prose\n", encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        source = AddSourceScreen()
        app.push_screen(source)
        await pilot.pause()
        source.query_one("#add-path", Input).value = str(md)
        source.action_continue_add()
        await pilot.pause()
        modal = app.screen
        assert isinstance(modal, KindPickModal)
        assert _kind_ids(modal)[0] == "prompt"  # suggested, pre-highlighted


async def test_directory_path_routes_to_the_exe_review(tmp_path):
    """An existing DIRECTORY (an .app bundle, a dir-shaped tool) is a valid exe target:
    it goes straight to the ExeReviewScreen — wired to _reviewed, so accepting forwards
    the slug out of the source step — not a 'File not found' lie."""
    d = tmp_path / "Tool.app"
    d.mkdir()
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        captured: dict[str, str | None] = {}
        source = AddSourceScreen()
        app.push_screen(source, lambda v: captured.__setitem__("result", v))
        await pilot.pause()
        source.query_one("#add-path", Input).value = str(d)
        source.action_continue_add()
        await pilot.pause()
        review = app.screen
        assert isinstance(review, ExeReviewScreen)
        review.query_one("#xv-name", Input).value = "tool"
        review.action_accept()
        await pilot.pause()
        await pilot.pause()
    entry = store.resolve("tool")
    assert entry.meta.kind == "exe"
    assert captured["result"] == entry.slug  # _reviewed forwarded it, not dropped


async def test_template_description_round_trips(tmp_path):
    """The template lane's #add-template-desc value lands as the command's description;
    Input.Submitted on the description input also submits the whole form."""
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        source = AddSourceScreen()
        app.push_screen(source)
        await pilot.pause()
        source.query_one("#add-template", Input).value = "echo {msg}"
        source.query_one("#add-template-name", Input).value = "greet"
        desc = source.query_one("#add-template-desc", Input)
        desc.value = "shout a message"
        source._template_given(Input.Submitted(desc, "shout a message"))  # submit from desc
        await pilot.pause()
        assert not isinstance(app.screen, AddSourceScreen)  # dismissed
    entry = store.resolve("greet")
    assert entry.meta.kind == "command"
    assert entry.meta.description == "shout a message"


# ---------------------------------------------------------------------------
# run_candidate_picker: the terminal-hosted prompt-variable picker (skit edit)
# ---------------------------------------------------------------------------


async def test_run_candidate_picker_done_returns_the_selection(tmp_path):
    from skit.tui_add import _ScreenHost
    from skit.tui_prompt import PromptCandidatePickerModal

    host = _ScreenHost(PromptCandidatePickerModal(["a", "b", "c"], {"a", "c"}))
    async with host.run_test() as pilot:
        await pilot.pause()
        modal = host.screen
        assert isinstance(modal, PromptCandidatePickerModal)
        modal.action_done()  # Ctrl+S
        await pilot.pause()
    assert host.return_value == {"a", "c"}


async def test_run_candidate_picker_escape_returns_none(tmp_path):
    from skit.tui_add import _ScreenHost
    from skit.tui_prompt import PromptCandidatePickerModal

    host = _ScreenHost(PromptCandidatePickerModal(["a", "b"], {"a"}))
    async with host.run_test() as pilot:
        await pilot.pause()
        modal = host.screen
        assert isinstance(modal, PromptCandidatePickerModal)
        modal.action_cancel()  # Esc
        await pilot.pause()
    assert host.return_value is None


def test_run_candidate_picker_returns_the_apps_result(tmp_path, monkeypatch):
    """run_candidate_picker hosts a PromptCandidatePickerModal built from names/selected
    verbatim (no arg dropped or replaced on the way in) and hands back run()'s result."""
    from skit import tui_add, tui_prompt

    seen: dict[str, object] = {}

    class _FakeHost:
        def __init__(self, screen):
            seen["screen"] = screen

        def run(self):
            return {"a"}

    monkeypatch.setattr(tui_add, "_ScreenHost", _FakeHost)
    assert tui_add.run_candidate_picker(["a", "b"], {"a"}) == {"a"}
    modal = seen["screen"]
    assert isinstance(modal, tui_prompt.PromptCandidatePickerModal)
    assert modal._names == ["a", "b"]
    assert modal._selected == {"a"}


# ---------------------------------------------------------------------------
# Drafts listing lists FILES only; advertised keys have positive key tests
# ---------------------------------------------------------------------------


async def test_drafts_list_skips_planted_directories(tmp_path):
    """The resumable list is built from drafts_dir(): a hand-planted skit-* DIRECTORY
    must not appear — resuming it would route the dir lane to an exe reference inside
    drafts/ (the shape the boundary forbids), and "Delete draft…" would unlink() a
    directory (IsADirectoryError)."""
    from skit.paths import drafts_dir
    from skit.tui_add import AddSourceApp

    drafts_dir().mkdir(parents=True, exist_ok=True)
    (drafts_dir() / "skit-real.py").write_text("print(1)\n", encoding="utf-8")
    (drafts_dir() / "skit-planted").mkdir()

    app = AddSourceApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        options = app.screen.query_one("#add-drafts", OptionList)
        ids = [str(options.get_option_at_index(i).id) for i in range(options.option_count)]
    assert [i.rsplit("/", 1)[-1] for i in ids] == ["skit-real.py"]


async def test_template_desc_enter_key_submits(tmp_path):
    """The footer's advertised Enter, pressed for real on the NEW description input —
    the key must reach _submit_template through the actual @on(Input.Submitted)
    wiring, not a synthetically constructed event."""
    from skit.tui_add import AddSourceApp

    app = AddSourceApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        app.screen.query_one("#add-template", Input).value = "echo {x}"
        app.screen.query_one("#add-template-name", Input).value = "keyed"
        app.screen.query_one("#add-template-desc", Input).focus()
        await pilot.pause()
        await pilot.press("enter")
        await pilot.pause()
    entry = store.resolve("keyed")
    assert entry.meta.kind == "command"
    assert app.return_value == entry.slug


async def test_exe_review_ctrl_s_key_adds(tmp_path):
    """The advertised Ctrl+S (priority binding over the AUTO_FOCUSed Input), pressed
    for real on the standalone exe review — the CLI face this branch adds."""
    from skit.tui_add import ExeReviewApp

    exe = tmp_path / "tool"
    exe.write_text("#!/bin/sh\necho hi\n", encoding="utf-8")
    exe.chmod(0o755)
    app = ExeReviewApp(exe, name="kbd", description="via keys")
    async with app.run_test() as pilot:
        await pilot.pause()
        await pilot.press("ctrl+s")
        await pilot.pause()
    entry = store.resolve("kbd")
    assert entry.meta.kind == "exe"
    assert entry.meta.description == "via keys"
    assert app.return_value == entry.slug


async def test_exe_review_escape_key_cancels(tmp_path):
    from skit.tui_add import ExeReviewApp

    exe = tmp_path / "tool"
    exe.write_text("#!/bin/sh\necho hi\n", encoding="utf-8")
    exe.chmod(0o755)
    app = ExeReviewApp(exe)
    async with app.run_test() as pilot:
        await pilot.pause()
        await pilot.press("escape")
        await pilot.pause()
    assert app.return_value is None
    assert store.list_entries() == []


async def test_kind_pick_question_shows_bracketed_filename_verbatim(tmp_path):
    """The modal's question escapes the filename like its plain twin does: a bracketed
    name must display verbatim, not have its tag-shaped segment swallowed as markup —
    the question's whole job is identifying the file being classified."""
    from textual.widgets import Label

    from skit.tui_add import _ScreenHost

    host = _ScreenHost(KindPickModal("report [draft].md", has_shebang=False))
    async with host.run_test() as pilot:
        await pilot.pause()
        label = str(host.screen.query(Label).first().render())
    assert label == "What is report [draft].md? skit can't tell from the name."
