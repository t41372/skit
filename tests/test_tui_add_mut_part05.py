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
from skit.tui_add import (
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
    """on_mount stamps the panel border with the exact, translated 'Add an entry'."""
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = await _mounted(app, pilot)
        assert screen.query_one("#add-body").border_title == "Add an entry"


# ---------------------------------------------------------------------------
# the unified add lane: every scripty kind rides the SAME review panel (#14
# retired _add_non_python's direct lane), so these drive submit -> review ->
# accept and pin what the panel records for non-python kinds
# ---------------------------------------------------------------------------


async def _submit_and_review(app, pilot, screen, path):
    """Type the path, submit, and hand back the review panel the source screen pushed."""
    screen.query_one("#add-path", Input).value = str(path)
    screen._submit_path()
    await pilot.pause()
    assert isinstance(app.screen, AddReviewScreen)
    return app.screen


async def test_js_no_shebang_records_empty_interpreter(tmp_path):
    """A JS file with no recognizable shebang falls into _store_entry's else branch: the
    recorded interpreter is the empty string (skit uses the kind's default runner), not a
    literal."""
    js = _write(tmp_path, "plain.js", "const x = 1\nconsole.log(x)\n")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = await _mounted(app, pilot)
        review = await _submit_and_review(app, pilot, screen, js)
        review.action_accept()
        await pilot.pause()
    (entry,) = store.list_entries()
    assert entry.meta.kind == "js"
    assert entry.meta.interpreter == ""


async def test_js_review_prefills_scanned_deps_and_accept_records_them(tmp_path):
    """An npm-flavor add scans the script's own imports INTO the visible #rv-deps field
    (the panel is the receipt for something that will download packages — never an
    invisible recording), and accepting records exactly that list on the copy entry."""
    js = _write(
        tmp_path,
        "tool.js",
        '#!/usr/bin/env node\nimport express from "express"\nimport _ from "lodash"\nconsole.log(1)\n',
    )
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = await _mounted(app, pilot)
        review = await _submit_and_review(app, pilot, screen, js)
        assert review.query_one("#rv-deps", Input).value == "express, lodash"
        review.action_accept()
        await pilot.pause()
    (entry,) = store.list_entries()
    assert entry.meta.mode == "copy"
    assert entry.meta.interpreter == "node"
    assert entry.meta.dependencies == ["express", "lodash"]


async def test_js_accept_dismisses_with_new_slug(tmp_path):
    """The review lane hands the new entry's slug back to the Library (so it can select
    the freshly-added row) — never None."""
    js = _write(tmp_path, "widget.js", "const x = 1\n")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        captured: dict[str, str | None] = {}
        screen = await _mounted(app, pilot, captured)
        review = await _submit_and_review(app, pilot, screen, js)
        review.action_accept()
        await pilot.pause()
        await pilot.pause()
    (entry,) = store.list_entries()
    assert captured["result"] == entry.slug
    assert entry.slug is not None


async def test_js_degraded_scanner_leaves_deps_empty(tmp_path, monkeypatch):
    """When the JS grammar wheel is absent the kind keeps deps_flavor='npm' but its
    dep_scanner degrades to None; _compose_deps' `if spec.dep_scanner` guard must hold,
    so the field prefills empty and the accepted entry records no deps (and nothing
    calls None(text))."""
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
        review = await _submit_and_review(app, pilot, screen, js)
        assert review.query_one("#rv-deps", Input).value == ""  # no scan ran
        review.action_accept()
        await pilot.pause()
    (entry,) = store.list_entries()
    assert entry.meta.mode == "copy"
    assert entry.meta.dependencies is None  # deps recording correctly skipped


async def test_js_add_tolerates_non_utf8_bytes(tmp_path):
    """A script carrying a stray non-UTF-8 byte must still add cleanly: the panel's read
    uses errors='replace' so a rogue byte degrades to U+FFFD instead of crashing the
    submit. The ASCII import line still scans, so deps are recorded."""
    js = tmp_path / "latin.js"
    js.write_bytes(b'#!/usr/bin/env node\nimport express from "express"\nconst s = "caf\xe9"\n')
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = await _mounted(app, pilot)
        review = await _submit_and_review(app, pilot, screen, js)
        review.action_accept()
        await pilot.pause()
    (entry,) = store.list_entries()
    assert entry.meta.dependencies == ["express"]


async def test_unknown_kind_asks_instead_of_adding(tmp_path):
    """A plain data file is unclassifiable: the screen ASKS (the KindPickModal — the TUI
    twin of --kind/--exe) instead of erroring or silently storing; declining the ask adds
    nothing."""
    notes = _write(tmp_path, "notes.txt", "just prose\n")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = await _mounted(app, pilot)
        screen.query_one("#add-path", Input).value = str(notes)
        screen._submit_path()
        await pilot.pause()
        assert isinstance(app.screen, KindPickModal)  # ask, don't teach CLI flags
        app.screen.dismiss(None)
        await pilot.pause()
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


# ---------------------------------------------------------------------------
# _reviewed: the success-unlink guard `draft_resume and mode=="copy"`
# ---------------------------------------------------------------------------


async def test_normal_copy_add_keeps_the_original_file(tmp_path):
    """A NORMAL (non-draft) copy add must NEVER delete the user's source file. The success
    unlink is gated on `draft_resume and store.resolve(slug).meta.mode == "copy"`; the
    and->or mutant would fire on every copy add (draft_resume False OR mode "copy" True →
    True) and unlink the original the user still owns. Pins the guard behaviorally."""
    py = tmp_path / "keep.py"
    py.write_text("print(1)\n", encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        captured: dict[str, str | None] = {}
        screen = await _mounted(app, pilot, captured)
        review = await _submit_and_review(app, pilot, screen, py)
        review.action_accept()
        await pilot.pause()
        await pilot.pause()
    assert py.exists()  # a copy add leaves the user's original untouched
    (entry,) = store.list_entries()
    assert entry.meta.mode == "copy"
    assert captured["result"] == entry.slug


async def test_resume_prompt_draft_unlinks_and_dismisses(tmp_path):
    """Resuming a kept PROMPT draft rides _submit_path's prompt branch: accepting the review
    hands the new slug back to the source screen (the _reviewed callback wiring) AND unlinks
    the draft (draft_resume True + copy → the success unlink)."""
    from skit.paths import drafts_dir

    drafts_dir().mkdir(parents=True, exist_ok=True)
    draft = drafts_dir() / "skit-new-p.prompt.md"
    draft.write_text("Summarize {{url}} briefly\n", encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        captured: dict[str, str | None] = {}
        screen = await _mounted(app, pilot, captured)
        screen.query_one("#add-path", Input).value = str(draft)
        screen._submit_path()
        await pilot.pause()
        assert isinstance(app.screen, PromptReviewScreen)
        app.screen.action_accept()
        await pilot.pause()
        await pilot.pause()
    (entry,) = store.list_entries()
    assert entry.meta.kind == "prompt"
    assert captured["result"] == entry.slug
    assert not draft.exists()  # the resumed draft reached the store → unlinked


async def test_resume_exe_inferring_draft_demotes_to_kind_ask(tmp_path):
    """A kept draft that would infer as a program (an executable bit on POSIX and a PATHEXT
    suffix on Windows, with no shebang) is NOT sent to ExeReviewScreen: an exe entry is
    reference-by-construction, forbidden for a draft the success path consumes. It is demoted
    (kind -> "unknown") to the kind ask, whose modal offers no 'A program' route out
    (offer_exe=False)."""
    import os

    from skit.paths import drafts_dir

    drafts_dir().mkdir(parents=True, exist_ok=True)
    # Windows has no executable bit: skit correctly uses PATHEXT there, while POSIX uses the
    # chmod below. The .exe suffix makes the same platform-native "this is runnable" signal on
    # both families without mocking the inference this integration test is meant to exercise.
    draft = drafts_dir() / "skit-new-blob.exe"
    draft.write_bytes(b"\x7fELF not really\n")
    os.chmod(draft, 0o755)  # noqa: S103 — a deliberately-executable fixture: the exe inference is the point
    assert store.infer_kind(draft) == "exe"  # the draft really would infer as a program
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = await _mounted(app, pilot)
        screen.query_one("#add-path", Input).value = str(draft)
        screen._submit_path()
        await pilot.pause()
        modal = app.screen
        assert isinstance(modal, KindPickModal)  # demoted to the ask, NOT ExeReviewScreen
        assert modal._offer_exe is False  # a draft can never become a program entry
    draft.unlink(missing_ok=True)


# ---------------------------------------------------------------------------
# _kind_picked: the KindPickModal routes each pick to its review, wired to _reviewed
# ---------------------------------------------------------------------------


async def _modal_for(app, pilot, path, capture):
    """Submit an unclassifiable path and return the KindPickModal it raises."""
    screen = await _mounted(app, pilot, capture)
    screen.query_one("#add-path", Input).value = str(path)
    screen._submit_path()
    await pilot.pause()
    assert isinstance(app.screen, KindPickModal)
    return app.screen


async def test_modal_pick_exe_dismisses_with_slug(tmp_path):
    """Picking 'A program' in the ask routes to ExeReviewScreen wired to _reviewed: accepting
    it forwards the new slug to the source screen. The dropped-callback mutant would leave the
    source screen hanging (no dismiss)."""
    src = _write(tmp_path, "report.awkish", "#!/usr/bin/awk -f\nBEGIN { print 1 }\n")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        captured: dict[str, str | None] = {}
        modal = await _modal_for(app, pilot, src, captured)
        modal.dismiss("exe")
        await pilot.pause()
        assert isinstance(app.screen, ExeReviewScreen)
        app.screen.action_accept()
        await pilot.pause()
        await pilot.pause()
    (entry,) = store.list_entries()
    assert entry.meta.kind == "exe"
    assert captured["result"] == entry.slug


async def test_modal_pick_kind_dismisses_with_slug(tmp_path):
    """Picking a language routes to AddReviewScreen(kind=picked) wired to _reviewed: accepting
    forwards the slug. Pins the outer push_screen callback on the multi-line _kind_picked push."""
    src = _write(tmp_path, "report.awkish", "#!/usr/bin/awk -f\nputs 1\n")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        captured: dict[str, str | None] = {}
        modal = await _modal_for(app, pilot, src, captured)
        modal.dismiss("ruby")
        await pilot.pause()
        assert isinstance(app.screen, AddReviewScreen)
        assert app.screen._kind == "ruby"
        app.screen.action_accept()
        await pilot.pause()
        await pilot.pause()
    (entry,) = store.list_entries()
    assert entry.meta.kind == "ruby"
    assert captured["result"] == entry.slug


async def test_modal_pick_prompt_dismisses_with_slug(tmp_path):
    """Picking 'A prompt for an AI agent' routes to PromptReviewScreen wired to _reviewed:
    accepting forwards the slug. Pins the callback on the modal-branch prompt push."""
    src = _write(tmp_path, "report.awkish", "#!/usr/bin/awk -f\nSummarize {{url}}\n")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        captured: dict[str, str | None] = {}
        modal = await _modal_for(app, pilot, src, captured)
        modal.dismiss("prompt")
        await pilot.pause()
        assert isinstance(app.screen, PromptReviewScreen)
        app.screen.action_accept()
        await pilot.pause()
        await pilot.pause()
    (entry,) = store.list_entries()
    assert entry.meta.kind == "prompt"
    assert captured["result"] == entry.slug


# ---------------------------------------------------------------------------
# KindPickModal(filename, has_shebang=…): the question label
# ---------------------------------------------------------------------------


async def test_kind_pick_modal_label_names_the_file_and_shebang(tmp_path):
    """An unclassifiable file WITH an unrecognized #! gets the shebang-variant question,
    naming the file verbatim. Kills the filename=None arg, the dropped has_shebang kwarg
    (defaults False → the wrong sentence), and the `is not None`->`is None` inversion."""
    from textual.widgets import Label

    src = _write(tmp_path, "report.awkish", "#!/usr/bin/awk -f\nBEGIN { print 1 }\n")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = await _mounted(app, pilot)
        screen.query_one("#add-path", Input).value = str(src)
        screen._submit_path()
        await pilot.pause()
        modal = app.screen
        assert isinstance(modal, KindPickModal)
        label = str(modal.query(Label).first().render())
    assert label == "The #! in report.awkish names no interpreter skit knows. What is it?"


async def test_kind_pick_modal_label_when_no_shebang(tmp_path):
    """An unclassifiable file with NO shebang gets the can't-tell question (has_shebang False).
    Kills the filename=None arg and the `is not None`->`is None` inversion the other way (a
    no-#! file must not claim to have one)."""
    from textual.widgets import Label

    src = _write(tmp_path, "notes.dat", "just data\n")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = await _mounted(app, pilot)
        screen.query_one("#add-path", Input).value = str(src)
        screen._submit_path()
        await pilot.pause()
        modal = app.screen
        assert isinstance(modal, KindPickModal)
        label = str(modal.query(Label).first().render())
    assert label == "What is notes.dat? skit can't tell from the name."
