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
    kind_label), while the option ids stay the raw kinds the add flow routes on (finding 5)."""
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


async def test_edit_source_prints_editor_launch_failure(tmp_path, monkeypatch):
    """When the editor can't be launched, action_edit_source prints the error (the run form
    banner is bypassed on this path) rather than crashing the panel."""

    def boom(path):
        raise editor.EditorError("cannot launch editor")

    monkeypatch.setattr(editor, "open_in_editor", boom)
    monkeypatch.setattr(tui.MenuApp, "suspend", lambda self: contextlib.nullcontext())
    printed: list[str] = []
    monkeypatch.setattr("builtins.print", lambda *a, **k: printed.append(" ".join(map(str, a))))
    p = _py(tmp_path, "print(1)\n", "plain.py")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = AddReviewScreen(p)
        app.push_screen(screen)
        await pilot.pause()
        screen.action_edit_source()
        await pilot.pause()
        await pilot.pause()
        assert any("cannot launch editor" in line for line in printed)


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
    Storage section (a temp draft has no original to link) — finding 14."""
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
