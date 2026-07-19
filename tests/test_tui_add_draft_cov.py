"""The TUI add flow's authoring lanes and the kind-parametric review panel.

Covers the Ctrl+N / Ctrl+P draft-in-$EDITOR lanes (fresh mode: no Storage section,
accept always copies), a review panel for a kind with NO analyzer, and the js deps
prefill. Every test asserts the entry that landed (or that nothing did), never that a
widget merely mounted.
"""

from __future__ import annotations

import contextlib
import os
from pathlib import Path

import pytest
from textual.widgets import Checkbox, Input, OptionList, RadioSet, Static

from skit import editor, store, tui
from skit.langs.registry import spec_for
from skit.tui_add import (
    _DRAFTS_LISTED,
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


@contextlib.contextmanager
def _noop_suspend():
    yield


@pytest.fixture
def no_suspend(monkeypatch):
    monkeypatch.setattr(tui.MenuApp, "suspend", lambda self: _noop_suspend())


def _editor_writes(monkeypatch, content: str | None):
    """Monkeypatch the $EDITOR hop to write `content` (or leave the file untouched when
    None), recording the temp path it was handed so a test can assert cleanup."""
    seen: dict[str, object] = {}

    def fake(path):
        seen["path"] = path
        if content is not None:
            path.write_text(content, encoding="utf-8")

    monkeypatch.setattr(editor, "open_in_editor", fake)
    return seen


# ---------------------------------------------------------------- Ctrl+N draft a script


async def test_draft_script_opens_fresh_review_and_copies(tmp_path, no_suspend, monkeypatch):
    seen = _editor_writes(monkeypatch, "import sys\nprint('drafted')\n")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        app.push_screen(AddSourceScreen())
        await pilot.pause()
        await pilot.press("ctrl+n")  # the advertised Write a script… key
        await pilot.pause()
        review = app.screen
        assert isinstance(review, AddReviewScreen)
        assert review._fresh is True
        assert not review.query("#rv-mode")  # fresh: no Storage section (nothing to link)
        review.query_one("#rv-name", Input).value = "drafted"
        # A re-edit on the fresh panel must not touch a (nonexistent) mode radio.
        _editor_writes(monkeypatch, "import sys\nprint('drafted again')\n")
        review.action_edit_source()
        await pilot.pause()
        app.screen.query_one("#rv-name", Input).value = "drafted"
        app.screen.action_accept()
        await pilot.pause()
    entry = store.resolve("drafted")
    assert entry.meta.kind == "python"
    assert entry.meta.mode == "copy"  # fresh always copies
    assert not seen["path"].exists()  # the temp starter file was cleaned up


async def test_draft_script_unchanged_starter_adds_nothing(tmp_path, no_suspend, monkeypatch):
    seen = _editor_writes(monkeypatch, None)  # editor leaves the starter as-is
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        source = AddSourceScreen()
        app.push_screen(source)
        await pilot.pause()
        await pilot.press("ctrl+n")
        await pilot.pause()
        assert isinstance(app.screen, AddSourceScreen)  # no review panel opened
    assert store.list_entries() == []
    assert not seen["path"].exists()  # temp unlinked


@pytest.mark.parametrize("key", ["ctrl+n", "ctrl+p"])
async def test_draft_editor_error_is_visible_after_resume_for_both_authoring_lanes(
    tmp_path, no_suspend, monkeypatch, key
):
    def boom(path):
        raise editor.EditorError("no editor configured")

    monkeypatch.setattr(editor, "open_in_editor", boom)
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        source = AddSourceScreen()
        app.push_screen(source)
        await pilot.pause()
        await pilot.press(key)
        await pilot.pause()
        assert isinstance(app.screen, AddSourceScreen)
        assert any(note.message == "no editor configured" for note in app._notifications)
    assert store.list_entries() == []


@pytest.mark.parametrize("key", ["ctrl+n", "ctrl+p"])
async def test_draft_deleted_by_editor_is_a_clean_visible_error(
    tmp_path, no_suspend, monkeypatch, key
):
    monkeypatch.setattr(editor, "open_in_editor", lambda path: path.unlink())
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        source = AddSourceScreen()
        app.push_screen(source)
        await pilot.pause()
        await pilot.press(key)
        await pilot.pause()
        assert app.screen is source
        messages = [note.message for note in app._notifications]
        assert any("Can't read" in message and "skit-new-" in message for message in messages)
    assert store.list_entries() == []


async def test_draft_script_cancelled_review_keeps_the_draft_and_notifies(
    tmp_path, no_suspend, monkeypatch
):
    """A cancelled review must NEVER silently delete the draft — it is the user's only copy
    of what they just wrote. The temp file is kept and a notification says where it lives;
    nothing lands in the store."""
    seen = _editor_writes(monkeypatch, "import sys\nprint('drafted')\n")
    notes: list[str] = []
    monkeypatch.setattr(
        AddSourceScreen, "notify", lambda self, message, **kw: notes.append(message)
    )
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        source = AddSourceScreen()
        app.push_screen(source)
        await pilot.pause()
        await pilot.press("ctrl+n")
        await pilot.pause()
        review = app.screen
        assert isinstance(review, AddReviewScreen)
        review.action_cancel()  # dismiss(None) → the draft callback keeps the file
        await pilot.pause()
    assert store.list_entries() == []  # nothing added
    assert seen["path"].exists()  # the draft survived the cancel
    assert any("Your draft was kept at" in n for n in notes)  # and the user was told where
    seen["path"].unlink(missing_ok=True)


async def test_draft_bash_shebang_becomes_a_shell_entry(tmp_path, no_suspend, monkeypatch):
    """The draft lane honors a changed shebang: writing a #!/usr/bin/env bash body into
    the python starter re-infers the kind, so the entry lands as SHELL — never a broken
    python entry with a bash body."""
    seen = _editor_writes(monkeypatch, "#!/usr/bin/env bash\n# Ship it\necho drafted\n")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        app.push_screen(AddSourceScreen())
        await pilot.pause()
        await pilot.press("ctrl+n")
        await pilot.pause()
        review = app.screen
        assert isinstance(review, AddReviewScreen)
        assert review._kind == "shell"  # re-inferred, not the .py suffix's python
        assert review._fresh is True
        review.query_one("#rv-name", Input).value = "shipit"
        review.action_accept()
        await pilot.pause()
    entry = store.resolve("shipit")
    assert entry.meta.kind == "shell"
    assert entry.meta.mode == "copy"  # fresh always copies
    assert not seen["path"].exists()


# ---------------------------------------------------------------- Ctrl+P draft a prompt


async def test_draft_prompt_opens_fresh_prompt_review_and_copies(tmp_path, no_suspend, monkeypatch):
    seen = _editor_writes(monkeypatch, "Summarize {{url}} briefly\n")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        app.push_screen(AddSourceScreen())
        await pilot.pause()
        await pilot.press("ctrl+p")  # the advertised Draft a prompt… key
        await pilot.pause()
        review = app.screen
        assert isinstance(review, PromptReviewScreen)
        assert review._fresh is True
        assert not review.query("#pv-mode")  # fresh: no Storage section
        # A fresh re-edit must not touch the (absent) mode radio.
        _editor_writes(monkeypatch, "Summarize {{url}} in one line\n")
        review.action_edit_source()
        await pilot.pause()
        app.screen.query_one("#pv-name", Input).value = "summ"
        app.screen.action_accept()
        await pilot.pause()
    entry = store.resolve("summ")
    assert entry.meta.kind == "prompt"
    assert entry.meta.mode == "copy"
    assert entry.meta.params == ["url"]
    assert not seen["path"].exists()


# ------------------------------------------- draft with an UNREGISTERED shebang: ASK


def _select_option(option_list: OptionList, option_id: str) -> None:
    """Highlight and select an OptionList entry by id (the real OptionSelected path)."""
    idx = next(
        i
        for i in range(option_list.option_count)
        if option_list.get_option_at_index(i).id == option_id
    )
    option_list.highlighted = idx
    option_list.action_select()


async def _draft_awk(app, pilot, monkeypatch) -> tuple[KindPickModal, Path]:
    """Ctrl+N a draft whose shebang names awk (unregistered): the draft lane must ASK via
    KindPickModal rather than fabricate a python entry. Returns the modal and the kept draft
    path."""
    seen = _editor_writes(monkeypatch, "#!/usr/bin/awk -f\nBEGIN { print 1 }\n")
    app.push_screen(AddSourceScreen())
    await pilot.pause()
    await pilot.press("ctrl+n")
    await pilot.pause()
    modal = app.screen
    assert isinstance(modal, KindPickModal)
    draft = seen["path"]
    assert isinstance(draft, Path)
    return modal, draft


async def test_draft_unregistered_shebang_pick_shell_lands_as_shell(
    tmp_path, no_suspend, monkeypatch
):
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        modal, draft = await _draft_awk(app, pilot, monkeypatch)
        _select_option(modal.query_one(OptionList), "shell")
        await pilot.pause()
        review = app.screen
        assert isinstance(review, AddReviewScreen)
        assert review._kind == "shell"  # the chosen kind, not a fabricated python
        review.query_one("#rv-name", Input).value = "awky"
        review.action_accept()
        await pilot.pause()
    assert store.resolve("awky").meta.kind == "shell"
    assert not draft.exists()  # committed → the store holds the copy


async def test_draft_unregistered_shebang_modal_omits_the_program_option(
    tmp_path, no_suspend, monkeypatch
):
    """A fresh draft is authored text, never a binary — and an exe entry is
    reference-by-construction, the one mode the drafts boundary forbids. So the draft
    lane's KindPickModal offers NO "A program" option (offer_exe=False): there is no
    ExeReviewScreen route out of a draft at all, only languages and a prompt."""
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        modal, _draft = await _draft_awk(app, pilot, monkeypatch)
        options = modal.query_one(OptionList)
        ids = {options.get_option_at_index(i).id for i in range(options.option_count)}
        assert "exe" not in ids  # the drafts boundary refuses a program entry
        assert "prompt" in ids  # a prompt is still offered
        assert "shell" in ids  # and the real languages
        assert not modal._offer_exe


async def test_nondraft_unknown_shebang_modal_offers_program_and_reaches_exe_review(
    tmp_path, no_suspend, monkeypatch
):
    """The complement / regression pin: a NON-draft file with the same unregistered #! (awk)
    DOES offer 'A program' in its KindPickModal (offer_exe=True), and picking it still reaches
    ExeReviewScreen — an awk script runs fine as a program, and only the drafts boundary refuses
    that route."""
    unknown = tmp_path / "report.awkish"
    unknown.write_text("#!/usr/bin/awk -f\nBEGIN { print 1 }\n", encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        source = AddSourceScreen()
        app.push_screen(source)
        await pilot.pause()
        source.query_one("#add-path", Input).value = str(unknown)
        source.action_continue_add()
        await pilot.pause()
        modal = app.screen
        assert isinstance(modal, KindPickModal)
        assert modal._offer_exe  # non-draft: the program option is offered
        options = modal.query_one(OptionList)
        ids = {options.get_option_at_index(i).id for i in range(options.option_count)}
        assert "exe" in ids
        _select_option(options, "exe")
        await pilot.pause()
        assert isinstance(app.screen, ExeReviewScreen)  # the program route is still reachable


async def test_draft_unregistered_shebang_pick_prompt_routes_to_prompt_review(
    tmp_path, no_suspend, monkeypatch
):
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        modal, _draft = await _draft_awk(app, pilot, monkeypatch)
        _select_option(modal.query_one(OptionList), "prompt")
        await pilot.pause()
        assert isinstance(app.screen, PromptReviewScreen)


async def test_draft_unregistered_shebang_cancel_keeps_the_draft_and_notifies(
    tmp_path, no_suspend, monkeypatch
):
    """Esc on the KindPickModal keeps the draft (the user's only copy) and says where —
    never silently deletes it, never fabricates an entry."""
    notes: list[str] = []
    monkeypatch.setattr(
        AddSourceScreen, "notify", lambda self, message, **kw: notes.append(message)
    )
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        _modal, draft = await _draft_awk(app, pilot, monkeypatch)
        await pilot.press("escape")  # dismiss the modal with None
        await pilot.pause()
    assert store.list_entries() == []  # nothing fabricated
    assert draft.exists()  # the draft survived
    assert any("Your draft was kept at" in n for n in notes)
    draft.unlink(missing_ok=True)


# ------------------------------------------- the add screen lists resumable drafts


async def test_add_source_lists_and_resumes_a_kept_draft(tmp_path, monkeypatch):
    """Kept drafts are resumable, not lore: the add screen lists them, and selecting one
    routes it through the normal path lane (fills #add-path → the review panel)."""
    from skit.paths import drafts_dir

    drafts_dir().mkdir(parents=True, exist_ok=True)
    draft = drafts_dir() / "skit-new-resume.py"
    draft.write_text("print('resume me')\n", encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        source = AddSourceScreen()
        app.push_screen(source)
        await pilot.pause()
        drafts_list = source.query_one("#add-drafts", OptionList)
        assert drafts_list.option_count == 1  # the kept draft is listed
        _select_option(drafts_list, str(draft))
        await pilot.pause()
        assert source.query_one("#add-path", Input).value == str(draft)  # routed to the path lane
        assert isinstance(app.screen, AddReviewScreen)  # opened the review panel


async def test_add_source_hides_the_draft_list_when_none_kept(tmp_path):
    """No kept drafts → no list (advertising an empty picker teaches a dead control)."""
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        source = AddSourceScreen()
        app.push_screen(source)
        await pilot.pause()
        assert not source.query("#add-drafts")  # the OptionList is absent
        assert "resume a kept draft" not in "".join(str(s.render()) for s in source.query(Static))


# ---------------------------------------------------------------- review with no analyzer


async def test_review_panel_for_analyzer_less_kind_reviews_identity_only(tmp_path):
    # ruby is in the data-driven tail: interpreted, but with no analyzer capability —
    # the panel reviews identity/storage and simply shows no tick list.
    rb = tmp_path / "task.rb"
    rb.write_text("# Tidy up\nputs 'hi'\n", encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        review = AddReviewScreen(rb, kind="ruby")
        app.push_screen(review)
        await pilot.pause()
        assert not review.query(Checkbox)  # no candidate ticks
        review.action_accept()
        await pilot.pause()
    assert store.resolve("task").meta.kind == "ruby"


# ---------------------------------------------------------------- js deps prefill


async def test_js_path_add_prefills_and_persists_scanned_deps(tmp_path):
    js = tmp_path / "tool.js"
    js.write_text("import chalk from 'chalk'\nconsole.log(chalk)\n", encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        source = AddSourceScreen()
        app.push_screen(source)
        await pilot.pause()
        source.query_one("#add-path", Input).value = str(js)
        source.action_continue_add()
        await pilot.pause()
        review = app.screen
        assert isinstance(review, AddReviewScreen)
        deps_input = review.query_one("#rv-deps", Input)
        assert "chalk" in deps_input.value  # detected from imports, editable
        deps_input.value = "chalk, zod"
        review.action_accept()
        await pilot.pause()
    entry = store.resolve("tool")
    assert entry.meta.kind == "js"
    assert set(entry.meta.dependencies or []) == {"chalk", "zod"}


async def test_review_deps_override_prefills_the_input(tmp_path):
    # The CLI face passes `skit add --dep` through as a `deps=` prefill (line 313).
    js = tmp_path / "t.js"
    js.write_text("console.log(1)\n", encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        review = AddReviewScreen(js, kind="js", deps=["left-pad"])
        app.push_screen(review)
        await pilot.pause()
        assert review.query_one("#rv-deps", Input).value == "left-pad"


# ---------------------------------------------------------------- shell candidate tick


async def test_shell_review_tick_writes_managed_block_into_copy(tmp_path):
    sh = tmp_path / "deploy.sh"
    sh.write_text(
        "#!/usr/bin/env bash\n# Deploy helper\nCITY=Taipei\necho $CITY\n", encoding="utf-8"
    )
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        source = AddSourceScreen()
        app.push_screen(source)
        await pilot.pause()
        source.query_one("#add-path", Input).value = str(sh)
        source.action_continue_add()
        await pilot.pause()
        review = app.screen
        assert isinstance(review, AddReviewScreen)
        assert review.query_one("#rv-desc", Input).value == "Deploy helper"
        assert not review.query("#rv-deps")  # shell has no dependency story
        cand = review.query_one("#rv-cand-0", Checkbox)
        assert "CITY" in str(cand.label)  # candidate from the shell analyzer
        cand.value = True
        review.action_accept()
        await pilot.pause()
    entry = store.resolve("deploy")
    text = entry.script_path.read_text(encoding="utf-8")
    assert "[tool.skit]" in text  # the pick was written via the shell params_io
    shell_spec = spec_for("shell")
    assert shell_spec is not None
    assert shell_spec.params_io is not None
    assert "CITY" in [d.name for d in shell_spec.params_io.read(text)]


async def test_review_panel_uses_radio_when_not_fresh(tmp_path):
    # The non-fresh path (a real file add) keeps the Storage radio — the twin of the
    # fresh assertions above.
    sh = tmp_path / "keep.sh"
    sh.write_text("#!/usr/bin/env bash\necho hi\n", encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        review = AddReviewScreen(sh, kind="shell")
        app.push_screen(review)
        await pilot.pause()
        assert review.query_one("#rv-mode", RadioSet)  # Storage section present


# --------------------------------------------- drafts list: newest-first, capped, honest overflow


def _seed_drafts(count: int) -> list[Path]:
    """`count` kept drafts under drafts home with ascending mtimes (i is oldest→newest)."""
    from skit.paths import drafts_dir

    drafts_dir().mkdir(parents=True, exist_ok=True)
    paths = []
    for i in range(count):
        p = drafts_dir() / f"skit-new-{i:03d}.py"
        p.write_text("print(1)\n", encoding="utf-8")
        os.utime(p, (1000 + i, 1000 + i))  # deterministic, ascending: paths[-1] is newest
        paths.append(p)
    return paths


async def test_add_source_lists_newest_first_caps_and_counts_overflow(tmp_path):
    """The kept-drafts list is sorted newest-first (mkstemp names are random, so a name sort
    hides an arbitrary tail — the just-lost draft is exactly the one that must surface), capped
    at _DRAFTS_LISTED, and the overflow past the cap is counted out loud (a silent cap reads as
    'this is everything')."""
    paths = _seed_drafts(_DRAFTS_LISTED + 3)  # 23
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        source = AddSourceScreen()
        app.push_screen(source)
        await pilot.pause()
        drafts_list = source.query_one("#add-drafts", OptionList)
        assert drafts_list.option_count == _DRAFTS_LISTED  # exactly the cap, not all 23
        # Newest first: the highest-mtime draft is option 0; the cap slices off the OLDEST tail.
        assert drafts_list.get_option_at_index(0).id == str(paths[-1])
        assert drafts_list.get_option_at_index(_DRAFTS_LISTED - 1).id == str(paths[-_DRAFTS_LISTED])
        hints = " ".join(str(s.render()) for s in source.query(".hint"))
        assert "3 more" in hints  # 23 - 20 counted honestly


async def test_add_source_no_overflow_line_at_the_cap(tmp_path):
    """Exactly _DRAFTS_LISTED drafts → the list is full but there is NO overflow line (an
    '…and 0 more' would be a lie)."""
    _seed_drafts(_DRAFTS_LISTED)
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        source = AddSourceScreen()
        app.push_screen(source)
        await pilot.pause()
        assert source.query_one("#add-drafts", OptionList).option_count == _DRAFTS_LISTED
        hints = " ".join(str(s.render()) for s in source.query(".hint"))
        assert "more" not in hints  # no overflow line at the cap


async def test_add_source_mtime_oserror_sorts_that_draft_last(tmp_path, monkeypatch):
    """The _mtime helper returns 0.0 on OSError (a draft that vanished/became unreadable
    mid-sort must not crash the screen) — so it sorts to the very end despite a newer real
    mtime, and the list still renders."""
    from skit.paths import drafts_dir

    drafts_dir().mkdir(parents=True, exist_ok=True)
    good = drafts_dir() / "skit-new-good.py"
    good.write_text("print(1)\n", encoding="utf-8")
    os.utime(good, (5000, 5000))
    bad = drafts_dir() / "skit-new-bad.py"
    bad.write_text("print(1)\n", encoding="utf-8")
    os.utime(bad, (9000, 9000))  # newer in reality, but its stat will raise → _mtime 0.0

    real_stat = Path.stat

    def fake_stat(self, *a, **k):
        if self == bad:
            raise OSError("simulated stat failure")
        return real_stat(self, *a, **k)

    monkeypatch.setattr(Path, "stat", fake_stat)
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        source = AddSourceScreen()
        app.push_screen(source)
        await pilot.pause()
        drafts_list = source.query_one("#add-drafts", OptionList)
        assert drafts_list.option_count == 2  # both still listed — no crash
        assert drafts_list.get_option_at_index(0).id == str(good)  # 5000 beats the 0.0 fallback
        assert drafts_list.get_option_at_index(1).id == str(bad)  # OSError → sorted last


# --------------------------------------------- resume cleanup: accept unlinks, cancel keeps


async def test_resume_draft_accept_unlinks_the_draft(tmp_path):
    """Resuming a kept draft through the drafts list and completing the review unlinks it in
    copy mode — the same 'success: the store holds the copy' cleanup the authoring lanes do."""
    from skit.paths import drafts_dir

    drafts_dir().mkdir(parents=True, exist_ok=True)
    draft = drafts_dir() / "skit-new-accept.py"
    draft.write_text("print('resume me')\n", encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        source = AddSourceScreen()
        app.push_screen(source)
        await pilot.pause()
        _select_option(source.query_one("#add-drafts", OptionList), str(draft))
        await pilot.pause()
        review = app.screen
        assert isinstance(review, AddReviewScreen)
        review.query_one("#rv-name", Input).value = "resumed"
        review.action_accept()
        await pilot.pause()
    assert store.resolve("resumed").meta.kind == "python"
    assert not draft.exists()  # the resumed draft reached the store → unlinked


async def test_resume_draft_cancel_keeps_the_draft(tmp_path):
    """Cancelling the review of a resumed draft keeps the file (reference-mode safety AND the
    user's only copy) — nothing lands, the draft survives."""
    from skit.paths import drafts_dir

    drafts_dir().mkdir(parents=True, exist_ok=True)
    draft = drafts_dir() / "skit-new-cancel.py"
    draft.write_text("print('keep me')\n", encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        source = AddSourceScreen()
        app.push_screen(source)
        await pilot.pause()
        _select_option(source.query_one("#add-drafts", OptionList), str(draft))
        await pilot.pause()
        review = app.screen
        assert isinstance(review, AddReviewScreen)
        review.action_cancel()  # dismiss(None) → the resume callback returns early, file kept
        await pilot.pause()
    assert store.list_entries() == []  # nothing added
    assert draft.exists()  # the draft survived the cancel
    draft.unlink(missing_ok=True)


# --------------------------------------------- Ctrl+E positive pilot (AGENTS.md keyboard rule)


async def test_review_ctrl_e_opens_editor_from_non_input_focus(tmp_path, no_suspend, monkeypatch):
    """Every advertised footer key has a POSITIVE pilot: on the review panel, focus a NON-Input
    widget (the Storage radio) and press ctrl+e — the screen chord fires and $EDITOR opens on
    the panel's subject. (The Input-focus negative — ctrl+e as end-of-line — is the non-priority
    binding's other half, pinned elsewhere; this is the keyboard path the chip advertises.)"""
    opened: dict[str, Path] = {}

    def fake(path):
        opened["path"] = path
        return 0

    monkeypatch.setattr(editor, "open_in_editor", fake)
    sh = tmp_path / "e.sh"
    sh.write_text("#!/usr/bin/env bash\necho hi\n", encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        review = AddReviewScreen(sh, kind="shell")
        app.push_screen(review)
        await pilot.pause()
        # Focus the RadioSet (focusable, not an Input) so the screen's ctrl+e binding wins —
        # an Input would consume ctrl+e as its own end-of-line (the Ctrl+A rule).
        review.query_one("#rv-mode", RadioSet).focus()
        await pilot.pause()
        await pilot.press("ctrl+e")
        await pilot.pause()
    assert opened.get("path") == sh  # the editor opened on the review's subject
