"""Round-10 design-audit fixes — TUI pilot coverage.

Every assertion pins an observable: the entry that landed, the stored copy's [tool.skit]
block, a widget's presence/absence or display flag, the #rv-python field value, or a modal's
label. Covered:
  * THE ROUND-9 HIGH: on an UNMODELED self-parser (dynamic optstring), ticked candidates are
    actually WRITTEN to the stored copy on accept — the collection gate mirrors the mount
    condition (_reader_modeled), and the modeled complement collects nothing without crashing;
  * a .prompt.md kept draft with a #! body resumes into the PromptReviewScreen (not shell);
  * reference-mode note is reader-aware: a MODELED form keeps its params-wrap visible and the
    short "never writes to the file" note; an UNMODELED script folds and keeps the old line;
  * the KindPickModal label switches on has_shebang;
  * the extra-arguments field is named exactly once in the review panel;
  * the #rv-python field is editable: a typed constraint lands in the stored copy, an empty one
    means automatic, and a typed value survives an edit->rescan (theirs beats the auto pin);
  * a resumed draft shows NO Storage section (fresh) — the storage ask is absent.
"""

from __future__ import annotations

import contextlib
from pathlib import Path

import pytest
from textual.widgets import Checkbox, Input, Label, OptionList, RadioButton, RadioSet, Static

from skit import editor, store, tui
from skit.tui_add import AddReviewScreen, AddSourceScreen, KindPickModal, PromptReviewScreen


@pytest.fixture(autouse=True)
def tmp_store(tmp_path, monkeypatch):
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path / "config"))
    monkeypatch.setenv("SKIT_LANG", "en")


@contextlib.contextmanager
def _noop_suspend():
    yield


@pytest.fixture
def no_suspend(monkeypatch):
    monkeypatch.setattr(tui.MenuApp, "suspend", lambda self: _noop_suspend())


def _editor_writes(monkeypatch, content: str) -> None:
    monkeypatch.setattr(editor, "open_in_editor", lambda path: path.write_text(content, "utf-8"))


def _seed_draft(name: str, body: str) -> Path:
    from skit.paths import drafts_dir

    drafts_dir().mkdir(parents=True, exist_ok=True)
    p = drafts_dir() / name
    p.write_text(body, encoding="utf-8")
    return p


def _statics(screen) -> str:
    return " ".join(str(s.render()) for s in screen.query(Static))


def _flip_mode(review, index: int) -> None:
    list(review.query_one("#rv-mode", RadioSet).query(RadioButton))[index].value = True


DYN_SH = '#!/usr/bin/env bash\nOUTDIR=/tmp\nOPTS="n:v"\nwhile getopts "$OPTS" o; do :; done\necho $OUTDIR\n'
MODELED_SH = "#!/usr/bin/env bash\nCITY=Taipei\nwhile getopts 'n:v' o; do :; done\necho $CITY\n"


# ==========================================================================
# 1. THE ROUND-9 HIGH: ticked candidates are WRITTEN on an unmodeled self-parser
# ==========================================================================


async def test_high_unmodeled_self_parser_writes_ticked_candidate(tmp_path):
    """A dynamic-optstring shell self-parses (uses_cli_framework) but can't be modeled, so the
    candidate ticks render and — the round-9 HIGH — are actually collected on accept: the stored
    copy's [tool.skit] block holds the ticked constant. The old gate (not uses_cli_framework)
    dropped it silently."""
    sh = tmp_path / "dyn.sh"
    sh.write_text(DYN_SH, encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        review = AddReviewScreen(sh, kind="shell")
        app.push_screen(review)
        await pilot.pause()
        assert review._reader_modeled() is False  # unmodeled -> the ticks are additive
        assert review.query("#rv-cand-0")  # the checkbox the accept gate must honor
        review.query_one("#rv-name", Input).value = "dynh"
        review.query_one("#rv-cand-0", Checkbox).value = True  # tick the constant
        review.action_accept()
        await pilot.pause()
    entry = store.resolve("dynh")
    stored = entry.script_path.read_text(encoding="utf-8")
    assert "[tool.skit]" in stored  # the block was written at all (it was dropped before)
    assert 'name = "OUTDIR"' in stored  # ...and holds the ticked constant


async def test_high_modeled_form_collects_nothing_without_crashing(tmp_path):
    """The complement: a MODELED getopts form has no checkboxes, so the collection gate skips —
    accept commits the entry and never queries a #rv-cand that doesn't exist (the crash the gate
    must not cause)."""
    sh = tmp_path / "mod.sh"
    sh.write_text(MODELED_SH, encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        review = AddReviewScreen(sh, kind="shell")
        app.push_screen(review)
        await pilot.pause()
        assert review._reader_modeled() is True
        assert not review.query("#rv-cand-0")
        review.query_one("#rv-name", Input).value = "modh"
        review.action_accept()  # must not raise
        await pilot.pause()
    assert store.resolve("modh").meta.kind == "shell"


# ==========================================================================
# 2. A .prompt.md kept draft with a #! body resumes into the PromptReviewScreen
# ==========================================================================


async def test_prompt_draft_with_shebang_body_resumes_into_prompt_review(tmp_path):
    """Resuming `skit-new-*.prompt.md` (bash-shebang body) opens the PROMPT review panel, not
    the shell one — the compound suffix is the user's lane choice (kind_for_draft)."""
    draft = _seed_draft("skit-new-p.prompt.md", "#!/usr/bin/env bash\nSummarize {{text}}.\n")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        source = AddSourceScreen()
        app.push_screen(source)
        await pilot.pause()
        drafts = source.query_one("#add-drafts", OptionList)
        drafts.highlighted = next(
            i for i in range(drafts.option_count) if drafts.get_option_at_index(i).id == str(draft)
        )
        drafts.action_select()
        await pilot.pause()
        assert isinstance(app.screen, PromptReviewScreen)  # prompt lane, not AddReviewScreen


# ==========================================================================
# 3. Reference-mode note is reader-aware (modeled keeps the wrap; unmodeled folds)
# ==========================================================================


async def test_reference_note_modeled_keeps_wrap_and_short_line(tmp_path):
    """A MODELED getopts script in reference mode keeps #rv-params-wrap visible (the ✓ notice
    stays — the reader works in reference mode) and the note is the short 'never writes to the
    file' line; accept in reference mode does not crash and the entry is a reference."""
    sh = tmp_path / "mod.sh"
    sh.write_text(MODELED_SH, encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        review = AddReviewScreen(sh, kind="shell")
        app.push_screen(review)
        await pilot.pause()
        _flip_mode(review, 1)  # Link the original
        await pilot.pause()
        assert review.query_one("#rv-params-wrap").display is True  # modeled -> stays visible
        note = str(review.query_one("#rv-ref-note", Static).render())
        assert note == "Link the original: skit never writes to the file."
        review.query_one("#rv-name", Input).value = "modref"
        review.action_accept()
        await pilot.pause()
    assert store.resolve("modref").meta.mode == "reference"


async def test_reference_note_unmodeled_folds_and_keeps_old_line(tmp_path):
    """An UNMODELED script (dynamic optstring) in reference mode folds #rv-params-wrap and keeps
    the old 'parameter setup is skipped' line — nothing to preserve, so say so plainly."""
    sh = tmp_path / "dyn.sh"
    sh.write_text(DYN_SH, encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        review = AddReviewScreen(sh, kind="shell")
        app.push_screen(review)
        await pilot.pause()
        _flip_mode(review, 1)
        await pilot.pause()
        assert review.query_one("#rv-params-wrap").display is False  # unmodeled -> folds
        note = str(review.query_one("#rv-ref-note", Static).render())
        assert "parameter setup is skipped" in note


# ==========================================================================
# 4. KindPickModal label switches on has_shebang
# ==========================================================================


async def test_kind_pick_modal_label_switches_on_shebang(tmp_path):
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        app.push_screen(KindPickModal("foo.xyz", has_shebang=True))
        await pilot.pause()
        assert (
            str(app.screen.query_one(Label).render())
            == "The #! in foo.xyz names no interpreter skit knows. What is it?"
        )
        app.pop_screen()
        await pilot.pause()
        app.push_screen(KindPickModal("foo.xyz", has_shebang=False))
        await pilot.pause()
        assert (
            str(app.screen.query_one(Label).render())
            == "What is foo.xyz? skit can't tell from the name."
        )


# ==========================================================================
# 5. The extra-arguments field is named exactly once in the review panel
# ==========================================================================


async def test_review_names_extra_arguments_field_once(tmp_path):
    """A dynamic-optstring shell that ALSO reads $@ (uses_argv AND a framework) mentions the
    extra-arguments field exactly ONCE — the reader notice, with the argv info hint suppressed."""
    sh = tmp_path / "dynargv.sh"
    sh.write_text(
        '#!/usr/bin/env bash\nOPTS="n:v"\nwhile getopts "$OPTS" o; do :; done\necho "$@"\n',
        encoding="utf-8",
    )
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        review = AddReviewScreen(sh, kind="shell")
        app.push_screen(review)
        await pilot.pause()
        assert review._analysis.uses_argv  # reads $@
        assert review._analysis.uses_cli_framework  # ...and getopts self-parses
        count = sum("extra-arguments field" in str(s.render()) for s in review.query(Static))
        assert count == 1


# ==========================================================================
# 6. The #rv-python field is editable
# ==========================================================================


async def test_rv_python_typed_constraint_lands_in_stored_copy(tmp_path):
    """Typing a constraint into #rv-python records it verbatim in the stored copy's PEP 723
    block (the field is the surface for a value the add writes)."""
    p = tmp_path / "plain.py"
    p.write_text("print(1)\n", encoding="utf-8")  # no shebang pin, no block: the field is editable
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        review = AddReviewScreen(p, kind="python")
        app.push_screen(review)
        await pilot.pause()
        assert review.query_one("#rv-python", Input).value == ""  # no auto pin
        review.query_one("#rv-python", Input).value = ">=3.10"
        review.query_one("#rv-name", Input).value = "pytyped"
        review.action_accept()
        await pilot.pause()
    stored = (store.resolve("pytyped").dir / "script.py").read_text(encoding="utf-8")
    assert 'requires-python = ">=3.10"' in stored


async def test_rv_python_empty_means_automatic(tmp_path):
    """Clearing #rv-python (or leaving it empty) records NO requires-python — automatic. With no
    deps either, the stored copy carries no PEP 723 block at all."""
    p = tmp_path / "plain.py"
    p.write_text("print(1)\n", encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        review = AddReviewScreen(p, kind="python")
        app.push_screen(review)
        await pilot.pause()
        review.query_one("#rv-python", Input).value = ""  # explicit clear
        review.query_one("#rv-name", Input).value = "pyauto"
        review.action_accept()
        await pilot.pause()
    stored = (store.resolve("pyauto").dir / "script.py").read_text(encoding="utf-8")
    assert "requires-python" not in stored  # automatic -> nothing recorded


async def test_rv_python_typed_value_survives_an_edit_rescan(tmp_path, no_suspend, monkeypatch):
    """A typed constraint is the user's own value: it survives an edit->rescan even when the
    shebang changes underneath (theirs beats the auto pin — the override capture in
    action_edit_source turns _py_pin_auto off)."""
    p = tmp_path / "v.py"
    p.write_text("#!/usr/bin/env python3.12\nprint(1)\n", encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        review = AddReviewScreen(p, kind="python")
        app.push_screen(review)
        await pilot.pause()
        assert review.query_one("#rv-python", Input).value == ">=3.12,<3.13"  # auto pin
        review.query_one("#rv-python", Input).value = ">=3.9"  # the user's own constraint
        _editor_writes(monkeypatch, "#!/usr/bin/env python3.11\nprint(1)\n")  # shebang moves
        review.action_edit_source()  # capture-then-rescan
        await pilot.pause()
        # The auto pin would be >=3.11,<3.12; the typed override wins instead.
        assert review.query_one("#rv-python", Input).value == ">=3.9"
        assert review._py_pin_auto is False


# ==========================================================================
# 7. A resumed draft shows NO Storage section (fresh)
# ==========================================================================


async def test_resumed_draft_has_no_storage_section(tmp_path):
    """Resuming a kept draft is fresh authoring (fresh=True): there's no original to link, so the
    Storage radio set is absent — a --ref there would have made the delete-confirm's 'only copy'
    a lie."""
    draft = _seed_draft("skit-new-fresh.py", "print('fresh')\n")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        source = AddSourceScreen()
        app.push_screen(source)
        await pilot.pause()
        drafts = source.query_one("#add-drafts", OptionList)
        drafts.highlighted = next(
            i for i in range(drafts.option_count) if drafts.get_option_at_index(i).id == str(draft)
        )
        drafts.action_select()
        await pilot.pause()
        review = app.screen
        assert isinstance(review, AddReviewScreen)
        assert not review.query("#rv-mode")  # fresh resume: no Storage ask
