"""Round-9 design-audit fixes — TUI pilot coverage.

Every advertised key gets a POSITIVE pilot (the AGENTS keyboard-parity rule), and every
assertion pins an observable: the entry that landed, a widget's presence/absence, a
footer chip, the recorded requires-python constraint, or a file's existence on disk.

Covered:
  * a bash-shebang kept draft (skit-new-*.py) RESUMES as a shell entry (kind_for_draft);
  * a versioned-shebang python file SHOWS and STORES its requires-python pin, and the pin
    follows a shebang edit on rescan;
  * the review panel and the settings screen key their manage-a-constant offer on a MODELED
    form (flows.reader_fields): dynamic optstrings keep the ticks + Space chip; modeled
    getopts suppresses them;
  * the singular/plural field count in the read notice;
  * Ctrl+D deletes a kept draft behind a confirm, keeps it on Esc, is the Input's own
    delete-right mid-edit, and the chip only renders when drafts exist.
"""

from __future__ import annotations

import contextlib
from pathlib import Path

import pytest
from textual.widgets import Input, OptionList, Static

from skit import editor, store, tui
from skit.tui_add import AddReviewScreen, AddSourceScreen, DraftDeleteConfirm


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


def _editor_writes(monkeypatch, content: str):
    seen: dict[str, Path] = {}

    def fake(path):
        seen["path"] = path
        path.write_text(content, encoding="utf-8")

    monkeypatch.setattr(editor, "open_in_editor", fake)
    return seen


def _seed_draft(name: str, body: str) -> Path:
    from skit.paths import drafts_dir

    drafts_dir().mkdir(parents=True, exist_ok=True)
    p = drafts_dir() / name
    p.write_text(body, encoding="utf-8")
    return p


def _option_index(option_list: OptionList, option_id: str) -> int:
    return next(
        i
        for i in range(option_list.option_count)
        if option_list.get_option_at_index(i).id == option_id
    )


def _select_option(option_list: OptionList, option_id: str) -> None:
    option_list.highlighted = _option_index(option_list, option_id)
    option_list.action_select()


def _statics(screen) -> str:
    return " ".join(str(s.render()) for s in screen.query(Static))


# ==========================================================================
# 1. A bash-shebang kept draft resumes as a SHELL entry (the round-8 HIGH, TUI lane)
# ==========================================================================


async def test_resume_bash_shebang_draft_lands_as_shell(tmp_path):
    """Resuming `skit-new-*.py` with a bash body opens the review panel as SHELL (kind_for_draft
    reads the shebang, not the mkstemp suffix), the stored entry is shell, and the draft is
    consumed on accept."""
    draft = _seed_draft("skit-new-ship.py", "#!/usr/bin/env bash\necho drafted\n")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        source = AddSourceScreen()
        app.push_screen(source)
        await pilot.pause()
        _select_option(source.query_one("#add-drafts", OptionList), str(draft))
        await pilot.pause()
        review = app.screen
        assert isinstance(review, AddReviewScreen)
        assert review._kind == "shell"  # reclassified by shebang, not the .py suffix
        review.query_one("#rv-name", Input).value = "shipit"
        review.action_accept()
        await pilot.pause()
    assert store.resolve("shipit").meta.kind == "shell"
    assert not draft.exists()  # resumed draft reached the store -> unlinked (is_draft)


# ==========================================================================
# 4. A versioned python shebang shows AND stores its requires-python pin
# ==========================================================================


async def test_review_versioned_shebang_shows_and_stores_pin(tmp_path):
    py = tmp_path / "v.py"
    py.write_text("#!/usr/bin/env python3.12\nprint('hi')\n", encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        review = AddReviewScreen(py, kind="python")
        app.push_screen(review)
        await pilot.pause()
        assert review._requires_python == ">=3.12,<3.13"  # derived from the shebang
        # shown AND editable in the #rv-python field, not invisibly recorded (round-10).
        assert review.query_one("#rv-python", Input).value == ">=3.12,<3.13"
        review.query_one("#rv-name", Input).value = "vpin"
        review.action_accept()
        await pilot.pause()
    text = (store.resolve("vpin").dir / "script.py").read_text(encoding="utf-8")
    assert 'requires-python = ">=3.12,<3.13"' in text  # landed in the stored copy's PEP 723 block


async def test_review_pin_follows_a_shebang_edit_on_rescan(tmp_path, no_suspend, monkeypatch):
    py = tmp_path / "v.py"
    py.write_text("#!/usr/bin/env python3.12\nprint('hi')\n", encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        review = AddReviewScreen(py, kind="python")
        app.push_screen(review)
        await pilot.pause()
        assert review._requires_python == ">=3.12,<3.13"
        _editor_writes(monkeypatch, "#!/usr/bin/env python3.11\nprint('hi')\n")
        review.action_edit_source()  # edit -> rescan recomputes the auto pin
        await pilot.pause()
        assert review._requires_python == ">=3.11,<3.12"  # the pin followed the shebang


async def test_review_explicit_python_is_not_overwritten_by_the_shebang(tmp_path):
    """An explicit requires-python (the CLI --python face) is the user's own value; the
    auto-pin never fires over it, so it is shown verbatim."""
    py = tmp_path / "v.py"
    py.write_text("#!/usr/bin/env python3.12\nprint('hi')\n", encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        review = AddReviewScreen(py, kind="python", requires_python=">=3.9")
        app.push_screen(review)
        await pilot.pause()
        assert review._requires_python == ">=3.9"  # explicit value, not the shebang's 3.12
        assert review.query_one("#rv-python", Input).value == ">=3.9"  # prefilled, editable


# ==========================================================================
# 5. Review panel + settings screen key the manage offer on a MODELED form
# ==========================================================================

DYN_SH = '#!/usr/bin/env bash\nOUTDIR=/tmp\nOPTS="n:v"\nwhile getopts "$OPTS" o; do :; done\necho $OUTDIR\n'
MODELED_SH = "#!/usr/bin/env bash\nCITY=Taipei\nwhile getopts 'n:v' o; do :; done\necho $CITY\n"


async def test_review_dynamic_optstring_keeps_ticks_and_space_chip(tmp_path):
    """A dynamic optstring shell self-parses but can't be modeled: the panel prints the
    passthrough hint AND keeps the candidate ticks, and the Space/Toggle chip is advertised."""
    sh = tmp_path / "dyn.sh"
    sh.write_text(DYN_SH, encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        review = AddReviewScreen(sh, kind="shell")
        app.push_screen(review)
        await pilot.pause()
        assert "parses its own arguments" in _statics(review)  # the passthrough notice
        assert review.query("#rv-cand-0")  # ...and the ticks remain (constants are additive)
        keys = str(review.query_one("#review-keys", Static).render())
        assert "Space" in keys  # the Space chip key hint is advertised
        assert "Toggle" in keys  # ...as a real toggle path


async def test_review_modeled_getopts_suppresses_ticks_and_space_chip(tmp_path):
    """The complement: a MODELED getopts form IS the interface — the ✓ read notice prints,
    no candidate ticks, and Space is not advertised (a dead key)."""
    sh = tmp_path / "mod.sh"
    sh.write_text(MODELED_SH, encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        review = AddReviewScreen(sh, kind="shell")
        app.push_screen(review)
        await pilot.pause()
        assert "skit read this script's own arguments" in _statics(review)
        assert not review.query("#rv-cand-0")  # managing would replace the modeled form
        keys = str(review.query_one("#review-keys", Static).render())
        assert "Toggle" not in keys  # no dead Space key


async def test_settings_dynamic_optstring_offers_tick_checkboxes(tmp_path):
    """The settings screen keys `_cli_driven` on flows.reader_fields now: a dynamic optstring
    (read_cli returns ok=False) is NOT cli-driven, so it offers the manage-these checkboxes —
    the old read_cli-is-not-None gate wrongly suppressed them."""
    from skit.tui_settings import ScriptSettingsScreen

    p = tmp_path / "dyn.sh"
    p.write_text(DYN_SH, encoding="utf-8")
    entry = store.add_script(p, kind="shell", name="dyn")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(entry)
        app.push_screen(screen)
        await pilot.pause()
        assert screen.query("#st-new-0")  # tick-to-manage checkboxes are offered
        assert "comes from its own command-line arguments" not in _statics(screen)


async def test_settings_modeled_getopts_hides_tick_checkboxes(tmp_path):
    """The unchanged True branch on the shell path: a MODELED getopts form suppresses the
    checkboxes and shows the leave-it-as-is hint."""
    from skit.tui_settings import ScriptSettingsScreen

    p = tmp_path / "mod.sh"
    p.write_text(MODELED_SH, encoding="utf-8")
    entry = store.add_script(p, kind="shell", name="mod")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(entry)
        app.push_screen(screen)
        await pilot.pause()
        assert not screen.query("#st-new-0")  # modeled form: no manage checkboxes
        assert "comes from its own command-line arguments" in _statics(screen)


# ==========================================================================
# 7. Singular vs plural field count in the review panel notice
# ==========================================================================


async def test_review_one_field_getopts_says_singular(tmp_path):
    sh = tmp_path / "one.sh"
    sh.write_text('#!/usr/bin/env bash\nwhile getopts "n:" o; do :; done\n', encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        review = AddReviewScreen(sh, kind="shell")
        app.push_screen(review)
        await pilot.pause()
        blurb = _statics(review)
        assert "(1 field)" in blurb
        assert "(1 fields)" not in blurb


async def test_review_multi_field_getopts_says_plural(tmp_path):
    sh = tmp_path / "many.sh"
    sh.write_text('#!/usr/bin/env bash\nwhile getopts "n:v" o; do :; done\n', encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        review = AddReviewScreen(sh, kind="shell")
        app.push_screen(review)
        await pilot.pause()
        assert "(2 fields)" in _statics(review)


# ==========================================================================
# 8. Ctrl+D deletes a kept draft (confirm), keeps it on Esc, yields to Input mid-edit
# ==========================================================================


async def test_ctrl_d_deletes_the_highlighted_draft_after_confirm(tmp_path, monkeypatch):
    """Ctrl+D from the drafts OptionList opens the confirm; y deletes the highlighted draft
    (the user's only copy), notifies, and recomposes the list — the other draft survives."""
    keep = _seed_draft("skit-new-keep.py", "print('keep')\n")
    doomed = _seed_draft("skit-new-doomed.py", "print('doomed')\n")
    notes: list[str] = []
    monkeypatch.setattr(
        AddSourceScreen, "notify", lambda self, message, **kw: notes.append(str(message))
    )
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        source = AddSourceScreen()
        app.push_screen(source)
        await pilot.pause()
        drafts = source.query_one("#add-drafts", OptionList)
        drafts.focus()
        drafts.highlighted = _option_index(drafts, str(doomed))  # highlight, do NOT select
        await pilot.pause()
        await pilot.press("ctrl+d")  # the advertised key, from the OptionList (not an Input)
        await pilot.pause()
        assert isinstance(app.screen, DraftDeleteConfirm)
        await pilot.press("y")  # confirm
        await pilot.pause()
        assert not doomed.exists()  # the highlighted draft is gone
        assert keep.exists()  # the other survived
        assert any("Deleted the draft" in n for n in notes)
        # Recomposed: exactly the surviving draft remains listed.
        remaining = source.query_one("#add-drafts", OptionList)
        assert remaining.option_count == 1


async def test_ctrl_d_confirm_esc_keeps_the_draft(tmp_path):
    """Esc on the confirm keeps the file — a draft is never lost to a single keystroke."""
    draft = _seed_draft("skit-new-safe.py", "print('safe')\n")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        source = AddSourceScreen()
        app.push_screen(source)
        await pilot.pause()
        drafts = source.query_one("#add-drafts", OptionList)
        drafts.focus()
        drafts.highlighted = 0
        await pilot.press("ctrl+d")
        await pilot.pause()
        assert isinstance(app.screen, DraftDeleteConfirm)
        await pilot.press("escape")  # keep
        await pilot.pause()
        assert draft.exists()  # kept
        assert isinstance(app.screen, AddSourceScreen)


async def test_ctrl_d_while_editing_a_field_is_the_inputs_delete_right(tmp_path):
    """Ctrl+D is NOT priority-bound: with an Input focused it is the Input's own delete-right,
    so no confirm opens and no draft is touched (the AGENTS editing-chord rule)."""
    draft = _seed_draft("skit-new-edit.py", "print('edit')\n")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        source = AddSourceScreen()
        app.push_screen(source)
        await pilot.pause()
        path_input = source.query_one("#add-path", Input)
        path_input.focus()
        path_input.value = "abc"
        path_input.cursor_position = 1  # cursor before "b": delete_right removes "b"
        await pilot.pause()
        await pilot.press("ctrl+d")
        await pilot.pause()
        assert isinstance(app.screen, AddSourceScreen)  # no confirm modal opened
        assert draft.exists()  # the draft was never touched
        assert path_input.value == "ac"  # the Input consumed ctrl+d as delete-right


async def test_delete_draft_action_is_a_noop_when_no_drafts(tmp_path):
    """action_delete_draft with no drafts list present returns early — the key must never
    crash or open a confirm on an empty screen."""
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        source = AddSourceScreen()
        app.push_screen(source)
        await pilot.pause()
        assert not source.query("#add-drafts")  # no drafts
        source.action_delete_draft()  # the `if not lists: return` guard
        await pilot.pause()
        assert isinstance(app.screen, AddSourceScreen)  # no modal opened


async def test_delete_draft_action_is_a_noop_when_nothing_highlighted(tmp_path):
    """The drafts list exists but nothing is highlighted (it isn't focused): the action
    returns early and the draft is untouched."""
    draft = _seed_draft("skit-new-none.py", "print('x')\n")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        source = AddSourceScreen()
        app.push_screen(source)
        await pilot.pause()
        source.query_one("#add-drafts", OptionList).highlighted = None  # nothing selected
        source.action_delete_draft()  # the `if highlighted is None: return` guard
        await pilot.pause()
        assert isinstance(app.screen, AddSourceScreen)  # no confirm modal
        assert draft.exists()  # untouched


async def test_delete_draft_chip_only_renders_when_drafts_exist(tmp_path):
    """The Ctrl+D chip is the mouse path — it appears only when there are drafts to delete
    (advertising it on an empty screen would teach a dead control)."""
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        source = AddSourceScreen()
        app.push_screen(source)
        await pilot.pause()
        assert not source.query("#add-draft-actions")  # no drafts -> no chip
    _seed_draft("skit-new-present.py", "print('x')\n")
    app2 = tui.MenuApp()
    async with app2.run_test(size=(100, 40)) as pilot:
        source = AddSourceScreen()
        app2.push_screen(source)
        await pilot.pause()
        chip = source.query_one("#add-draft-actions", Static)
        assert "Ctrl+D" in str(chip.render())  # the mouse path is advertised
