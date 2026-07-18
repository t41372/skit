"""The TUI add flow's authoring lanes and the kind-parametric review panel.

Covers the Ctrl+N / Ctrl+P draft-in-$EDITOR lanes (fresh mode: no Storage section,
accept always copies), a review panel for a kind with NO analyzer, and the js deps
prefill. Every test asserts the entry that landed (or that nothing did), never that a
widget merely mounted.
"""

from __future__ import annotations

import contextlib

import pytest
from textual.widgets import Checkbox, Input, RadioSet

from skit import editor, store, tui
from skit.langs.registry import spec_for
from skit.tui_add import AddReviewScreen, AddSourceScreen, PromptReviewScreen


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


async def test_draft_script_editor_error_is_reported_not_crashed(tmp_path, no_suspend, monkeypatch):
    def boom(path):
        raise editor.EditorError("no editor configured")

    monkeypatch.setattr(editor, "open_in_editor", boom)
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        source = AddSourceScreen()
        app.push_screen(source)
        await pilot.pause()
        await pilot.press("ctrl+n")
        await pilot.pause()
        # The editor never wrote → the starter is unchanged → nothing is added, no crash.
        assert isinstance(app.screen, AddSourceScreen)
    assert store.list_entries() == []


async def test_draft_script_cancelled_review_keeps_the_draft_and_notifies(
    tmp_path, no_suspend, monkeypatch
):
    """A cancelled review must NEVER silently delete the draft — it is the user's only copy
    of what they just wrote. The temp file is kept and a notification says where it lives;
    nothing lands in the store (finding R2, the TUI lane)."""
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


def test_kind_for_draft_reads_the_shebang_not_the_suffix(tmp_path):
    """The draft always starts life as a .py temp file, so the SUFFIX must never decide
    its kind — the user's shebang is the explicit signal. A python (or absent) shebang
    stays python; a registered non-python shebang re-infers; an unknown one falls back to
    python."""
    from skit.tui_add import _kind_for_draft

    no_shebang = tmp_path / "a.py"
    no_shebang.write_text("import sys\nprint('hi')\n", encoding="utf-8")
    assert _kind_for_draft(no_shebang) == "python"  # no shebang → python fallback

    py_shebang = tmp_path / "b.py"
    py_shebang.write_text("#!/usr/bin/env python3\nprint('hi')\n", encoding="utf-8")
    assert _kind_for_draft(py_shebang) == "python"  # python shebang matches python

    bash_shebang = tmp_path / "c.py"
    bash_shebang.write_text("#!/usr/bin/env bash\necho hi\n", encoding="utf-8")
    assert _kind_for_draft(bash_shebang) == "shell"  # re-inferred from the bash shebang

    unknown = tmp_path / "d.py"
    unknown.write_text("#!/usr/bin/env cobol\nDISPLAY 'hi'.\n", encoding="utf-8")
    assert _kind_for_draft(unknown) == "python"  # unmatched shebang → python fallback


async def test_draft_bash_shebang_becomes_a_shell_entry(tmp_path, no_suspend, monkeypatch):
    """The draft lane honors a changed shebang: writing a #!/usr/bin/env bash body into
    the python starter re-infers the kind, so the entry lands as SHELL — never a broken
    python entry with a bash body (finding 6)."""
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
