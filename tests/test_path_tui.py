"""The path-entry TUI layer (docs/design/path.md P1b): the ghost suggester's
activation and root rules, the file-picker modal's keys (each advertised chip has a
positive pilot test here), and the per-shape insertion semantics."""

from __future__ import annotations

import os
import sys
from pathlib import Path

import pytest
from textual.content import Content
from textual.message import Message
from textual.suggester import SuggestionReady
from textual.widgets import Checkbox, Input, OptionList, Static

from skit import argv_text, flows, store, tui, tui_footer, tui_pathpick
from skit.tui_form import _EXTRA_KEY, FieldRow, RunFormScreen, TokenMenuModal
from skit.tui_pathpick import FilePickerModal, PathContext, PathSuggester, PickedPath


@pytest.fixture(autouse=True)
def tmp_store(tmp_path, monkeypatch):
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path / "config"))
    monkeypatch.setenv("SKIT_LANG", "en")


def _tree(tmp_path) -> Path:
    """A little filesystem: root/{data.csv, draft.txt, sub/{inner.txt}, .hidden}."""
    root = tmp_path / "root"
    (root / "sub").mkdir(parents=True)
    (root / "data.csv").write_text("x", encoding="utf-8")
    (root / "draft.txt").write_text("x", encoding="utf-8")
    (root / "sub" / "inner.txt").write_text("x", encoding="utf-8")
    (root / ".hidden").write_text("x", encoding="utf-8")
    return root


def _ctx(root: Path, invoke: Path | None = None) -> PathContext:
    return PathContext(workdir=root, invoke_cwd=invoke or root)


def _sugg(root: Path, *, kind: str = "path", shlexy: bool = False, invoke: Path | None = None):
    return PathSuggester(kind=kind, shlexy=shlexy, placeholder_braces=False, ctx=_ctx(root, invoke))


# ---------------------------------------------------------------------------
# PathSuggester: activation and the three-coordinate-system roots (path.md §3-§4)
# ---------------------------------------------------------------------------


async def test_path_field_completes_bare_prefix_at_workdir(tmp_path):
    root = _tree(tmp_path)
    assert await _sugg(root).get_suggestion("da") == "data.csv"
    assert await _sugg(root).get_suggestion("su") == "sub/"  # dirs chain with a slash


async def test_str_field_needs_pathy_text(tmp_path):
    root = _tree(tmp_path)
    s = _sugg(root, kind="str")
    assert await s.get_suggestion("da") is None  # bare word: not path-shaped
    assert await s.get_suggestion("./da") == "./data.csv"
    assert await s.get_suggestion("sub/in") == "sub/inner.txt"


async def test_secretless_activation_never_guesses_beyond_prefix(tmp_path):
    root = _tree(tmp_path)
    assert await _sugg(root).get_suggestion("zzz") is None  # no match, no invention


async def test_hidden_entries_only_behind_a_dot_prefix(tmp_path):
    root = _tree(tmp_path)
    assert await _sugg(root).get_suggestion(".h") == ".hidden"
    # "d" must not surface .hidden; the first visible match wins alphabetically.
    assert await _sugg(root).get_suggestion("d") == "data.csv"


async def test_cwd_token_completes_at_invoke_cwd_not_workdir(tmp_path):
    root = _tree(tmp_path)
    invoke = tmp_path / "elsewhere"
    invoke.mkdir()
    (invoke / "notes.md").write_text("x", encoding="utf-8")
    s = _sugg(root, invoke=invoke)
    assert await s.get_suggestion("{cwd}/no") == "{cwd}/notes.md"


async def test_unset_env_token_is_silence_not_a_traceback(tmp_path, monkeypatch):
    root = _tree(tmp_path)
    monkeypatch.delenv("SKIT_NO_SUCH_VAR", raising=False)
    assert await _sugg(root).get_suggestion("{env:SKIT_NO_SUCH_VAR}/d") is None


async def test_relative_env_token_falls_back_to_the_workdir_rule(tmp_path, monkeypatch):
    root = _tree(tmp_path)
    monkeypatch.setenv("SKIT_REL_DIR", "sub")
    assert (
        await _sugg(root).get_suggestion("{env:SKIT_REL_DIR}/in") == "{env:SKIT_REL_DIR}/inner.txt"
    )


async def test_home_prefix_completes_inside_home(tmp_path, monkeypatch):
    root = _tree(tmp_path)
    home = tmp_path / "home"
    home.mkdir()
    (home / "notes.md").write_text("x", encoding="utf-8")
    monkeypatch.setenv("HOME", str(home))
    s = _sugg(root, kind="str")
    assert await s.get_suggestion("~/no") == "~/notes.md"
    assert await s.get_suggestion("~") is None  # no separator yet: nothing to complete inside


async def test_missing_workdir_silences_bare_completion(tmp_path):
    s = _sugg(tmp_path / "vanished")
    assert await s.get_suggestion("da") is None


async def test_missing_workdir_silences_relative_token_lookup(tmp_path, monkeypatch):
    # The relative-expansion arm of the two-step rule also lands on the bare root;
    # with the workdir gone it must go silent, not resolve somewhere invented.
    monkeypatch.setenv("SKIT_REL_DIR", "sub")
    s = _sugg(tmp_path / "vanished")
    assert await s.get_suggestion("{env:SKIT_REL_DIR}/in") is None


async def test_shlexy_field_completes_only_the_trailing_piece(tmp_path):
    root = _tree(tmp_path)
    s = _sugg(root, shlexy=True)
    assert await s.get_suggestion("first.txt dr") == "first.txt draft.txt"
    assert await s.get_suggestion("'quote in progress") is None
    assert await s.get_suggestion("done.txt ") is None  # empty trailing piece


class _FakeEntry:
    def __init__(self, name: str, *, is_dir: bool = False, raises: bool = False) -> None:
        self.name = name
        self._is_dir = is_dir
        self._raises = raises

    def is_dir(self) -> bool:
        if self._raises:
            raise OSError("gone mid-scan")
        return self._is_dir


class _FakeScandir:
    def __init__(self, entries):
        self._entries = list(entries)

    def __call__(self, _base):
        return self

    def __enter__(self):
        return self

    def __exit__(self, *exc: object) -> None:
        pass

    def __iter__(self):
        return iter(self._entries)


async def test_scan_cap_stops_the_scan_exactly(tmp_path, monkeypatch):
    """With the cap shrunk to 3, an entry in scan position 4 must never be offered —
    even though it would win the alphabetical sort. Pins the >= boundary itself."""
    root = _tree(tmp_path)
    fake = _FakeScandir(
        [_FakeEntry("dax3"), _FakeEntry("dax2"), _FakeEntry("dax4"), _FakeEntry("daa-first")]
    )
    monkeypatch.setattr(tui_pathpick.os, "scandir", fake)
    monkeypatch.setattr(tui_pathpick, "SCAN_CAP", 3)
    assert await _sugg(root).get_suggestion("da") == "dax2"  # daa-first was beyond the cap


async def test_scan_degrades_on_oserror(tmp_path, monkeypatch):
    root = _tree(tmp_path)

    def _denied(_base):
        raise PermissionError("no")

    monkeypatch.setattr(tui_pathpick.os, "scandir", _denied)
    assert await _sugg(root).get_suggestion("da") is None
    assert tui_pathpick._list_filtered(root, "") == []


async def test_unstatable_entry_is_treated_as_a_file(tmp_path, monkeypatch):
    root = _tree(tmp_path)
    monkeypatch.setattr(tui_pathpick.os, "scandir", _FakeScandir([_FakeEntry("dax", raises=True)]))
    # No trailing slash: the entry that failed to stat is offered as a plain file.
    assert await _sugg(root).get_suggestion("da") == "dax"


# ---------------------------------------------------------------------------
# PathContext: roots and inserted spellings (path.md §3, §5)
# ---------------------------------------------------------------------------


def test_for_entry_resolves_the_entry_workdir(tmp_path):
    src = tmp_path / "job.py"
    src.write_text("print('hi')\n", encoding="utf-8")
    entry = store.add_python(src, name="job")
    entry.meta.workdir = str(tmp_path)  # an explicit absolute workdir
    ctx = PathContext.for_entry(entry)
    assert ctx.workdir == tmp_path
    assert ctx.invoke_cwd == Path.cwd()


def test_for_entry_reference_entry_roots_at_its_origin(tmp_path):
    origin = tmp_path / "proj"
    origin.mkdir()
    src = origin / "job.py"
    src.write_text("print('hi')\n", encoding="utf-8")
    entry = store.add_python(src, name="job", mode="reference")
    assert PathContext.for_entry(entry).workdir == origin


def test_vanished_origin_reference_entry_degrades(tmp_path):
    """Risk 9's journey: a reference entry whose origin vanished keeps that origin as
    its workdir (launcher deliberately doesn't recover reference mode) — the suggester
    goes silent and the picker opens at the nearest existing ancestor."""
    origin = tmp_path / "proj" / "deep"
    origin.mkdir(parents=True)
    src = origin / "job.py"
    src.write_text("print('hi')\n", encoding="utf-8")
    entry = store.add_python(src, name="job", mode="reference")
    src.unlink()
    origin.rmdir()
    ctx = PathContext.for_entry(entry)
    assert ctx.workdir == origin
    assert ctx.bare_root is None
    start, missing = ctx.picker_start()
    assert start == tmp_path / "proj"
    assert missing is True


def test_picker_start_last_resort_is_the_invoke_cwd(tmp_path, monkeypatch):
    """The whole ancestor chain can be gone on Windows (a vanished drive's anchor);
    pinned portably by refusing every is_dir."""
    monkeypatch.setattr(Path, "is_dir", lambda self: False)
    ctx = PathContext(workdir=tmp_path / "x", invoke_cwd=tmp_path)
    assert ctx.picker_start() == (tmp_path, True)


def test_picker_start_degrades_to_nearest_existing_ancestor(tmp_path):
    gone = tmp_path / "was" / "here"
    ctx = _ctx(gone, invoke=tmp_path)
    start, missing = ctx.picker_start()
    assert start == tmp_path
    assert missing is True


def test_value_for_is_relative_inside_the_root_and_posix_everywhere(tmp_path):
    root = _tree(tmp_path)
    ctx = _ctx(root)
    assert ctx.value_for(root / "sub" / "inner.txt") == "sub/inner.txt"
    assert ctx.value_for(root) == "."
    outside = tmp_path / "other.txt"
    assert ctx.value_for(outside) == outside.as_posix()


# ---------------------------------------------------------------------------
# FilePickerModal: every advertised key, plus the mouse path (path.md §5)
# ---------------------------------------------------------------------------


async def test_picker_enter_descends_then_picks_and_filter_clears(tmp_path):
    root = _tree(tmp_path)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        picked: list[PickedPath | None] = []
        app.push_screen(FilePickerModal(_ctx(root)), picked.append)
        await pilot.pause()
        modal = app.screen
        assert isinstance(modal, FilePickerModal)
        assert str(root) in str(modal.query_one("#picker-dir", Static).render())
        # Empty filter: the pinned row exists, but the highlight sits on the first
        # real entry — sub/ (dirs sort before files here alphabetically by chance;
        # assert by id, not position luck).
        option_list = modal.query_one(OptionList)
        assert option_list.get_option_at_index(0).id == "__use_dir__"
        modal.query_one(Input).value = "su"
        await pilot.pause()
        await pilot.press("enter")  # first match: sub/ → descend
        await pilot.pause()
        assert str(root / "sub") in str(modal.query_one("#picker-dir", Static).render())
        assert modal.query_one(Input).value == ""  # filter cleared on descend
        await pilot.press("enter")  # highlight: first real entry = inner.txt
        await pilot.pause()
    assert picked == [PickedPath("sub/inner.txt")]


async def test_picker_use_this_directory_row_by_real_keys(tmp_path):
    """↑ from the initial highlight reaches the pinned row while the FILTER keeps
    focus (the arrow bindings steer the list), and Enter picks the directory itself."""
    root = _tree(tmp_path)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        picked: list[PickedPath | None] = []
        app.push_screen(FilePickerModal(_ctx(root)), picked.append)
        await pilot.pause()
        modal = app.screen
        assert isinstance(modal, FilePickerModal)
        assert isinstance(app.focused, Input)  # the filter holds focus throughout
        await pilot.press("up")
        await pilot.pause()
        assert modal.query_one(OptionList).highlighted == 0  # the pinned row
        await pilot.press("enter")
        await pilot.pause()
    assert picked == [PickedPath(".")]


async def test_picker_arrows_steer_highlight_without_leaving_the_filter(tmp_path):
    root = _tree(tmp_path)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(FilePickerModal(_ctx(root)))
        await pilot.pause()
        modal = app.screen
        assert isinstance(modal, FilePickerModal)
        option_list = modal.query_one(OptionList)
        start = option_list.highlighted
        await pilot.press("down")
        await pilot.pause()
        assert option_list.highlighted == (start or 0) + 1
        assert isinstance(app.focused, Input)  # focus never moved
        # All four advertised steering keys must actually run (OptionList spells its
        # actions cursor_up/cursor_down but page_up/page_down — a name mismatch here
        # was an app crash, not a no-op).
        await pilot.press("pagedown")
        await pilot.pause()
        assert option_list.highlighted == option_list.option_count - 1
        await pilot.press("pageup")
        await pilot.pause()
        assert option_list.highlighted == 0
        assert app.screen is modal  # still alive


async def test_picker_prefix_matches_outrank_substring_hits(tmp_path):
    """Filter `da`: data.csv (prefix match, a file) must sit above Anaconda/ (a
    substring-matching directory that ASCII sort would float to the top) — Enter
    picks what the user typed, it never surprise-descends."""
    root = _tree(tmp_path)
    (root / "Anaconda").mkdir()
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        picked: list[PickedPath | None] = []
        app.push_screen(FilePickerModal(_ctx(root)), picked.append)
        await pilot.pause()
        modal = app.screen
        assert isinstance(modal, FilePickerModal)
        modal.query_one(Input).value = "da"
        await pilot.pause()
        option_list = modal.query_one(OptionList)
        ids = [str(option_list.get_option_at_index(i).id) for i in range(option_list.option_count)]
        assert ids == ["f:data.csv", "d:Anaconda"]
        assert option_list.highlighted == 0
        await pilot.press("enter")
        await pilot.pause()
    assert picked == [PickedPath("data.csv")]


async def test_picker_filter_is_case_insensitive_substring(tmp_path):
    """`re` must find README.md — the picker redraws true names, so it filters like
    EnvPickerModal (case-insensitive substring), unlike the append-only ghost."""
    root = _tree(tmp_path)
    (root / "README.md").write_text("x", encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(FilePickerModal(_ctx(root)))
        await pilot.pause()
        modal = app.screen
        assert isinstance(modal, FilePickerModal)
        modal.query_one(Input).value = "eadm"
        await pilot.pause()
        option_list = modal.query_one(OptionList)
        ids = [str(option_list.get_option_at_index(i).id) for i in range(option_list.option_count)]
        assert ids == ["f:README.md"]


async def test_picker_row_click_is_the_mouse_path(tmp_path):
    root = _tree(tmp_path)
    app = tui.MenuApp()
    async with app.run_test(size=(100, 30)) as pilot:
        picked: list[PickedPath | None] = []
        app.push_screen(FilePickerModal(_ctx(root)), picked.append)
        await pilot.pause()
        modal = app.screen
        assert isinstance(modal, FilePickerModal)
        modal.query_one(Input).value = "data"
        await pilot.pause()
        option_list = modal.query_one(OptionList)
        assert str(option_list.get_option_at_index(0).id) == "f:data.csv"
        await pilot.click(option_list, offset=(2, 0))
        await pilot.pause()
        await pilot.pause()
    assert picked == [PickedPath("data.csv")]


async def test_picker_zero_match_enter_is_a_noop(tmp_path):
    root = _tree(tmp_path)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(FilePickerModal(_ctx(root)))
        await pilot.pause()
        modal = app.screen
        assert isinstance(modal, FilePickerModal)
        modal.query_one(Input).value = "zzz-no-such"
        await pilot.pause()
        assert modal.query_one(OptionList).option_count == 0
        await pilot.press("enter")  # nothing highlighted: no dismissal, no crash
        await pilot.pause()
        assert app.screen is modal


async def test_picker_filtering_hides_the_pinned_row(tmp_path):
    root = _tree(tmp_path)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(FilePickerModal(_ctx(root)))
        await pilot.pause()
        modal = app.screen
        assert isinstance(modal, FilePickerModal)
        modal.query_one(Input).value = "d"
        await pilot.pause()
        option_list = modal.query_one(OptionList)
        ids = [option_list.get_option_at_index(i).id for i in range(option_list.option_count)]
        assert ids == ["f:data.csv", "f:draft.txt"]
        assert option_list.highlighted == 0  # Enter acts on the first MATCH


async def test_picker_backspace_ascends_only_on_empty_filter(tmp_path):
    root = _tree(tmp_path)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(FilePickerModal(PathContext(workdir=root / "sub", invoke_cwd=root)))
        await pilot.pause()
        modal = app.screen
        assert isinstance(modal, FilePickerModal)
        box = modal.query_one(Input)
        box.value = "in"
        box.cursor_position = 2
        await pilot.pause()
        await pilot.press("backspace")  # editing: deletes, does NOT ascend
        await pilot.pause()
        assert box.value == "i"
        assert str(root / "sub") in str(modal.query_one("#picker-dir", Static).render())
        await pilot.press("backspace")
        await pilot.pause()
        assert box.value == ""
        await pilot.press("backspace")  # empty: ascends to the parent
        await pilot.pause()
        rendered = str(modal.query_one("#picker-dir", Static).render())
        assert str(root) in rendered
        assert str(root / "sub") not in rendered


async def test_picker_backspace_noops_at_the_filesystem_root(tmp_path):
    root = _tree(tmp_path)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(FilePickerModal(_ctx(root)))
        await pilot.pause()
        modal = app.screen
        assert isinstance(modal, FilePickerModal)
        anchor = Path(str(root.anchor))
        modal._dir = anchor
        modal.action_ascend()
        assert modal._dir == anchor


async def test_picker_esc_cancels_and_up_chip_is_clickable(tmp_path):
    root = _tree(tmp_path)
    app = tui.MenuApp()
    async with app.run_test(size=(100, 30)) as pilot:
        picked: list[object] = ["sentinel"]
        app.push_screen(
            FilePickerModal(PathContext(workdir=root / "sub", invoke_cwd=root)),
            lambda result: picked.__setitem__(0, result),
        )
        await pilot.pause()
        modal = app.screen
        assert isinstance(modal, FilePickerModal)
        # Mouse path for ascend: click the Backspace chip's label.
        chips = modal.query(Static).last()
        plain = str(chips.render()).replace(tui_footer.GLUE, " ")
        position = plain.find("Up")
        assert position >= 0, plain
        await pilot.click(chips, offset=(position + 1, 0))
        await pilot.pause()
        assert str(root) in str(modal.query_one("#picker-dir", Static).render())
        await pilot.press("escape")
        await pilot.pause()
    assert picked == [None]


async def test_picker_missing_workdir_opens_at_ancestor_with_notice(tmp_path):
    root = _tree(tmp_path)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(FilePickerModal(_ctx(root / "gone" / "deeper", invoke=root)))
        await pilot.pause()
        modal = app.screen
        assert isinstance(modal, FilePickerModal)
        assert str(root) in str(modal.query_one("#picker-dir", Static).render())
        assert "missing" in str(modal.query_one("#picker-notice", Static).render())


# ---------------------------------------------------------------------------
# The insert flow: token menu row, replace vs append (path.md §5)
# ---------------------------------------------------------------------------

ARGPARSE_PATHS = (
    "import argparse\nfrom pathlib import Path\n"
    "ap = argparse.ArgumentParser()\n"
    "ap.add_argument('--src', type=Path)\n"
    "ap.add_argument('files', nargs='*', type=Path)\n"
    "ap.parse_args()\n"
)


def _path_entry(tmp_path):
    p = tmp_path / "job.py"
    p.write_text(ARGPARSE_PATHS, encoding="utf-8")
    return store.add_python(p, name="job")


async def _open_form(app, pilot, tmp_path, monkeypatch):
    root = _tree(tmp_path)
    monkeypatch.setattr(
        tui_pathpick.PathContext, "for_entry", classmethod(lambda cls, entry: _ctx(root))
    )
    entry = _path_entry(tmp_path)
    plan = flows.plan_for_entry(entry)
    screen = RunFormScreen(entry, plan, {})
    app.push_screen(screen)
    await pilot.pause()
    return screen, root


async def test_path_fields_render_hint_and_suggester(tmp_path, monkeypatch):
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen, _root = await _open_form(app, pilot, tmp_path, monkeypatch)
        src_row = next(r for r in screen.query(FieldRow) if r.field.key == "src")
        assert src_row.field.kind == "path"
        assert isinstance(src_row.query_one(Input).suggester, PathSuggester)
        assert "path" in str(src_row.query_one(".field-label", Static).render())


async def test_token_menu_puts_file_row_first_on_path_fields_and_picker_replaces(
    tmp_path, monkeypatch
):
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen, _root = await _open_form(app, pilot, tmp_path, monkeypatch)
        src_row = next(r for r in screen.query(FieldRow) if r.field.key == "src")
        src_row.set_value("old-prefill.csv")
        screen.action_insert_token("src")
        await pilot.pause()
        menu = app.screen
        assert isinstance(menu, TokenMenuModal)
        first = menu.query_one(OptionList).get_option_at_index(0)
        assert first.id == "__file__"  # path field: browse is the Enter default
        await pilot.press("enter")
        await pilot.pause()
        picker = app.screen
        assert isinstance(picker, FilePickerModal)
        picker.query_one(Input).value = "data"
        await pilot.pause()
        await pilot.press("enter")
        await pilot.pause()
        # Flagship journey: the picked path REPLACES the prefilled value.
        assert src_row.value == "data.csv"


async def test_picker_appends_quoted_to_the_extra_args_row(tmp_path, monkeypatch):
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen, root = await _open_form(app, pilot, tmp_path, monkeypatch)
        (root / "a b.txt").write_text("x", encoding="utf-8")
        extra_row = next(r for r in screen.query(FieldRow) if r.field.key == "__extra_args__")
        extra_row.set_value("--verbose")
        screen.action_insert_token("__extra_args__")
        await pilot.pause()
        menu = app.screen
        assert isinstance(menu, TokenMenuModal)
        option_list = menu.query_one(OptionList)
        ids = [str(option_list.get_option_at_index(i).id) for i in range(option_list.option_count)]
        assert ids[-1] == "__file__"  # non-path field: the row sits with the env picker
        option_list.highlighted = ids.index("__file__")
        await pilot.press("enter")
        await pilot.pause()
        picker = app.screen
        assert isinstance(picker, FilePickerModal)
        picker.query_one(Input).value = "a b"
        await pilot.pause()
        await pilot.press("enter")
        await pilot.pause()
        # Appended as ONE quoted piece in the extra-args row's OWN dialect (argv_text:
        # CRT double quotes on Windows, shlex single quotes on POSIX), so re-parsing
        # with the row's actual splitter keeps the filename whole.
        quoted = '"a b.txt"' if sys.platform == "win32" else "'a b.txt'"
        assert extra_row.value == f"--verbose {quoted}"
        assert argv_text.split(extra_row.value) == ["--verbose", "a b.txt"]


async def test_picker_appends_quoted_to_a_multiple_field(tmp_path, monkeypatch):
    """The nargs='*' path field: append as one shlex-quoted piece, and the value
    survives the ACTUAL splitter of multiple fields (flows._split_multi)."""
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen, root = await _open_form(app, pilot, tmp_path, monkeypatch)
        (root / "a b.txt").write_text("x", encoding="utf-8")
        files_row = next(r for r in screen.query(FieldRow) if r.field.key == "files")
        assert files_row.field.multiple is True
        files_row.set_value("first.txt")
        screen.action_insert_token("files")
        await pilot.pause()
        menu = app.screen
        assert isinstance(menu, TokenMenuModal)
        assert menu.query_one(OptionList).get_option_at_index(0).id == "__file__"  # path field
        await pilot.press("enter")
        await pilot.pause()
        picker = app.screen
        assert isinstance(picker, FilePickerModal)
        picker.query_one(Input).value = "a b"
        await pilot.pause()
        await pilot.press("enter")
        await pilot.pause()
        assert files_row.value == "first.txt 'a b.txt'"
        assert flows._split_multi(files_row.value, root) == ["first.txt", "a b.txt"]


async def test_token_rows_still_insert_at_cursor(tmp_path, monkeypatch):
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen, _root = await _open_form(app, pilot, tmp_path, monkeypatch)
        src_row = next(r for r in screen.query(FieldRow) if r.field.key == "src")
        box = src_row.query_one(Input)
        box.value = "out-.csv"
        box.cursor_position = 4  # between "out-" and ".csv"
        screen.action_insert_token("src")
        await pilot.pause()
        menu = app.screen
        assert isinstance(menu, TokenMenuModal)
        option_list = menu.query_one(OptionList)
        ids = [str(option_list.get_option_at_index(i).id) for i in range(option_list.option_count)]
        option_list.highlighted = ids.index("{today}")
        await pilot.press("enter")
        await pilot.pause()
        assert src_row.value == "out-{today}.csv"


# ---------------------------------------------------------------------------
# The 📁 browse link: the picker's own door, on the field (issue #7 follow-up)
# ---------------------------------------------------------------------------

MIXED_TYPES = (
    "import argparse\nfrom pathlib import Path\n"
    "ap = argparse.ArgumentParser()\n"
    "ap.add_argument('--src', type=Path)\n"
    "ap.add_argument('--note')\n"
    "ap.add_argument('--count', type=int)\n"
    "ap.add_argument('--loud', action='store_true')\n"
    "ap.parse_args()\n"
)


async def _open_mixed_form(app, pilot, tmp_path, monkeypatch):
    root = _tree(tmp_path)
    monkeypatch.setattr(
        tui_pathpick.PathContext, "for_entry", classmethod(lambda cls, entry: _ctx(root))
    )
    p = tmp_path / "mixed.py"
    p.write_text(MIXED_TYPES, encoding="utf-8")
    entry = store.add_python(p, name="mixed")
    screen = RunFormScreen(entry, flows.plan_for_entry(entry), {})
    app.push_screen(screen)
    await pilot.pause()
    return screen, root


def _label(row: FieldRow) -> str:
    return str(row.query_one(".field-label", Static).render())


def _label_actions(row: FieldRow) -> list[str]:
    """The screen actions the label row's links actually fire, read off the rendered
    spans — a typo in the markup would leave a link that clicks into nothing."""
    rendered = row.query_one(".field-label", Static).render()
    assert isinstance(rendered, Content)
    spans = rendered.spans
    return [
        str(span.style).split("screen.", 1)[1].split("(", 1)[0]
        for span in spans
        if "@click=screen." in str(span.style)
    ]


async def test_browse_link_renders_on_text_fields_only(tmp_path, monkeypatch):
    """The affordance the picker shipped without: a per-field door that says what it
    does. It rides EVERY insertable text field (a shell/JS entry never gets an inferred
    `path` type, so type-gating it would leave those with no visible door) — but never a
    numeric or non-text one, where a picked path is a guaranteed validation error."""
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen, _root = await _open_mixed_form(app, pilot, tmp_path, monkeypatch)
        rows = {r.field.key: r for r in screen.query(FieldRow)}
        for key in ("src", "note", _EXTRA_KEY):  # path, plain str, the extra-args row
            assert rows[key].browsable is True
            assert "browse" in _label(rows[key])
            assert "insert" in _label(rows[key])  # both doors, never one replacing the other
            # Browse reads first: it is the primary act on a field that holds a path.
            assert _label_actions(rows[key]) == ["browse_path", "insert_token"]
        for key in ("count", "loud"):  # whole number, on/off
            assert rows[key].browsable is False
            assert "browse" not in _label(rows[key])
        assert rows["count"].insertable is True  # the ▾ menu is unchanged there
        assert _label_actions(rows["count"]) == ["insert_token"]
        # A link that clicks into nothing would fail silently; pin both actions exist.
        for name in ("browse_path", "insert_token"):
            assert callable(getattr(RunFormScreen, f"action_{name}", None))


async def test_browse_link_opens_the_picker_directly_and_replaces(tmp_path, monkeypatch):
    """The flagship journey, now one click: 📁 browse → pick → the value is in the field.
    No token menu in between."""
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen, _root = await _open_form(app, pilot, tmp_path, monkeypatch)
        src_row = next(r for r in screen.query(FieldRow) if r.field.key == "src")
        src_row.set_value("old-prefill.csv")
        screen.action_browse_path("src")
        await pilot.pause()
        picker = app.screen
        assert isinstance(picker, FilePickerModal)
        picker.query_one(Input).value = "data"
        await pilot.pause()
        await pilot.press("enter")
        await pilot.pause()
        assert src_row.value == "data.csv"
        assert src_row.query_one(Input).has_focus


async def test_browse_without_a_key_uses_the_focused_field_and_its_dialect(tmp_path, monkeypatch):
    """Keyless entry (the focused-field route the footer chip uses) lands on the focused
    row — and honours THAT row's insert mode: the extra-args row appends a CRT-quoted
    piece rather than replacing."""
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen, root = await _open_form(app, pilot, tmp_path, monkeypatch)
        (root / "a b.txt").write_text("x", encoding="utf-8")
        extra_row = next(r for r in screen.query(FieldRow) if r.field.key == _EXTRA_KEY)
        extra_row.set_value("--verbose")
        extra_row.query_one(Input).focus()
        await pilot.pause()
        screen.action_browse_path()
        await pilot.pause()
        picker = app.screen
        assert isinstance(picker, FilePickerModal)
        picker.query_one(Input).value = "a b"
        await pilot.pause()
        await pilot.press("enter")
        await pilot.pause()
        quoted = '"a b.txt"' if sys.platform == "win32" else "'a b.txt'"
        assert extra_row.value == f"--verbose {quoted}"


async def test_browse_refuses_numeric_secret_and_unknown_rows(tmp_path, monkeypatch):
    """Both gates hold: `_insert_target` rejects what has no text field to fill, and
    `browsable` rejects the numeric row the token menu still serves."""
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen, _root = await _open_mixed_form(app, pilot, tmp_path, monkeypatch)
        for key in ("count", "loud", "row-that-no-longer-exists"):
            screen.action_browse_path(key)
            await pilot.pause()
            assert app.screen is screen  # no picker was pushed
        screen.query_one(Checkbox).focus()  # the keyless route, focus on a non-text field
        await pilot.pause()
        screen.action_browse_path()
        await pilot.pause()
        assert app.screen is screen


def test_fieldrow_browsable_needs_a_context():
    """@property bodies are a mutmut blind spot (see below), and this one has a branch no
    form exercises: a FieldRow built without a completion context cannot browse — there is
    no root to open the picker at."""
    field = flows.FormField(key="x", label="x", source="flag")
    assert FieldRow(field, "").browsable is False
    assert FieldRow(field, "", path_ctx=_ctx(Path.cwd())).browsable is True


def test_fieldrow_shlexy_and_insert_mode_all_branches():
    """FieldRow.shlexy and .insert_mode are @property methods, whose bodies mutmut does
    not mutate — so their every branch is pinned directly here: a single-value field
    replaces; a `multiple` field appends in the POSIX-shlex dialect; the extra-args row
    appends in the argv/CRT dialect (path.md §5).

    The same blind spot covers any DECORATED class body — mutmut skips the whole
    ClassDef — so tui_pathpick's `@dataclass PathContext`/`PickedPath` generate zero
    mutants too; their §3 logic is likewise carried by direct tests (value_for,
    picker_start, bare_root, for_entry above), never by the mutation gate. New logic
    added to a decorated class needs its own direct pins."""

    def _row(*, key: str = "x", multiple: bool = False) -> FieldRow:
        field = flows.FormField(key=key, label="x", source="flag", multiple=multiple)
        return FieldRow(field, "")

    single = _row()
    assert single.shlexy is False
    assert single.insert_mode == "replace"

    multiple = _row(multiple=True)
    assert multiple.shlexy is True
    assert multiple.insert_mode == "shlex"

    extra = _row(key=_EXTRA_KEY)
    assert extra.shlexy is True
    assert extra.insert_mode == "argv"


async def test_insert_picked_shapes(monkeypatch):
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        box = Input(value="old.csv")
        await app.screen.mount(box)
        await pilot.pause()
        tui_pathpick.insert_picked(box, PickedPath("new.csv"), mode="replace")
        assert box.value == "new.csv"
        assert box.cursor_position == len("new.csv")
        box.value = ""
        tui_pathpick.insert_picked(box, PickedPath("a b.txt"), mode="shlex")
        assert box.value == "'a b.txt'"  # a lone piece: no leading space invented
        # The two append dialects diverge only off-POSIX, so pin them under a win32
        # patch: "shlex" mode uses shlex.quote (single quotes, platform-agnostic),
        # "argv" mode uses argv_text/CRT (double quotes) — where the POSIX spelling
        # 'a b.txt' would shatter into two literal-quoted arguments.
        with monkeypatch.context() as m:
            m.setattr(tui_pathpick.argv_text.sys, "platform", "win32")
            box.value = ""
            tui_pathpick.insert_picked(box, PickedPath("a b.txt"), mode="shlex")
            assert box.value == "'a b.txt'"  # shlex, not the CRT dialect
            box.value = "--verbose"
            tui_pathpick.insert_picked(box, PickedPath("a b.txt"), mode="argv")
            assert box.value == '--verbose "a b.txt"'
            assert tui_pathpick.argv_text.split(box.value) == ["--verbose", "a b.txt"]


async def test_insert_picked_escapes_glob_metacharacters(tmp_path):
    """A picked file whose real name holds a glob metacharacter must reach the run as that ONE
    file. Both parsed shapes re-expand globs at assembly (flows._split_multi for `multiple`, the
    extra-args lane for the argv row) and quoting alone doesn't suppress it — insert_picked
    glob-escapes the pick so the piece matches only its own literal self. `[` is a legal filename
    character on every platform (unlike `*`/`?`), so this guard runs everywhere: an UNescaped
    `data[1].csv` is a char-class matching `data1.csv`, so dropping the escape fails the assert."""
    for name in ("data1.csv", "data2.csv", "data[1].csv"):
        (tmp_path / name).write_text("x", encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        box = Input()
        await app.screen.mount(box)
        await pilot.pause()
        tui_pathpick.insert_picked(box, PickedPath("data[1].csv"), mode="shlex")
        assert box.value == "'data[[]1].csv'"  # glob.escape wraps the [ in a [[] char-class
        # The multiple field's REAL splitter yields only the picked file — the data1 sibling the
        # unescaped `data[1].csv` char-class would have matched is gone.
        assert flows._split_multi(box.value, tmp_path) == ["data[1].csv"]
        box.value = ""
        tui_pathpick.insert_picked(box, PickedPath("data[1].csv"), mode="argv")
        assert argv_text.split(box.value)[-1] == "data[[]1].csv"  # same escaped literal for argv


async def test_secret_field_never_gets_a_suggester(tmp_path):
    src = tmp_path / "job.py"
    src.write_text("print('hi')\n", encoding="utf-8")
    entry = store.add_python(src, name="job")
    plan = flows.FormPlan(
        source="argparse",
        fields=[
            flows.FormField(key="token", label="token", source="flag", secret=True),
            flows.FormField(key="out", label="out", source="flag"),
        ],
    )
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = RunFormScreen(entry, plan, {})
        app.push_screen(screen)
        await pilot.pause()
        rows = {r.field.key: r for r in screen.query(FieldRow)}
        assert rows["token"].query_one(Input).suggester is None
        assert isinstance(rows["out"].query_one(Input).suggester, PathSuggester)


async def test_token_menu_without_context_has_no_file_row():
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(TokenMenuModal())
        await pilot.pause()
        menu = app.screen
        assert isinstance(menu, TokenMenuModal)
        option_list = menu.query_one(OptionList)
        ids = [str(option_list.get_option_at_index(i).id) for i in range(option_list.option_count)]
        assert "__file__" not in ids


def test_looks_pathy_windows_recognition(monkeypatch):
    assert tui_pathpick.looks_pathy(r"..\data") is (tui_pathpick.os.name == "nt")
    monkeypatch.setattr(tui_pathpick.os, "name", "nt")
    assert tui_pathpick.looks_pathy(r"..\data") is True
    assert tui_pathpick.looks_pathy(r"C:\Users") is True
    assert tui_pathpick.looks_pathy("C:/Users") is True
    assert tui_pathpick.looks_pathy("data") is False  # a bare word stays a bare word


def test_looks_pathy_token_and_separator_spellings():
    # The two prefixes that carry no separator of their own must each be recognized,
    # and the token match is case-sensitive ({cwd}, not {CWD}); a slash anywhere
    # (which covers ./, ../, /) activates on its own.
    assert tui_pathpick.looks_pathy("~") is True
    assert tui_pathpick.looks_pathy("~project") is True
    assert tui_pathpick.looks_pathy("{cwd}") is True
    assert tui_pathpick.looks_pathy("{CWD}") is False
    assert tui_pathpick.looks_pathy("a/b") is True
    assert tui_pathpick.looks_pathy("./x") is True
    assert tui_pathpick.looks_pathy("plain") is False


# ---------------------------------------------------------------------------
# PathSuggester constructor contract, observed through Textual's _get_suggestion
# ---------------------------------------------------------------------------


async def _record_suggestions(app, pilot, monkeypatch):
    """A mounted Input whose posted SuggestionReady values are captured synchronously
    (Suggester._get_suggestion calls requester.post_message inline), so the suggester's
    real Textual entry point is observable without worker-timing flake."""
    inp = Input()
    await app.screen.mount(inp)
    await pilot.pause()
    recorded: list[str] = []
    original = inp.post_message

    def _capture(message: Message) -> bool:
        if isinstance(message, SuggestionReady):
            recorded.append(message.suggestion)
        return original(message)

    monkeypatch.setattr(inp, "post_message", _capture)
    return inp, recorded


async def test_suggester_is_case_sensitive_query_not_casefolded(tmp_path, monkeypatch):
    root = tmp_path / "root"
    root.mkdir()
    (root / "DATA.csv").write_text("x", encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        inp, recorded = await _record_suggestions(app, pilot, monkeypatch)
        # case_sensitive=True: "DA" reaches get_suggestion verbatim and matches the
        # uppercase file. Casefolding (case_sensitive False) would send "da", which the
        # exact-case matcher rejects — no suggestion.
        await _sugg(root)._get_suggestion(inp, "DA")
        assert recorded == ["DATA.csv"]


async def test_suggester_does_not_cache_stale_results(tmp_path, monkeypatch):
    root = _tree(tmp_path)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        inp, recorded = await _record_suggestions(app, pilot, monkeypatch)
        sugg = _sugg(root)
        await sugg._get_suggestion(inp, "da")
        assert recorded == ["data.csv"]
        (root / "data.csv").unlink()
        recorded.clear()
        # use_cache=False: the second identical query re-scans and finds the file gone.
        # A cache would replay the stale "data.csv".
        await sugg._get_suggestion(inp, "da")
        assert recorded == []


# ---------------------------------------------------------------------------
# PathSuggester internals: brace-escape flag, quote refusal, token-without-sep
# ---------------------------------------------------------------------------


def _brace_dir(tmp_path) -> Path:
    """A workdir holding a directory literally named ``{x}`` with a file inside — so a
    doubled-brace head (`{{x}}`) that expands to `{x}` can be told apart from one kept
    literal."""
    root = tmp_path / "root"
    (root / "{x}").mkdir(parents=True)
    (root / "{x}" / "data.csv").write_text("x", encoding="utf-8")
    return root


async def test_brace_escapes_on_a_normal_field_halves_doubled_braces(tmp_path):
    # A normal field has brace_escapes=True: `{{x}}` in the head expands to the real
    # directory `{x}`, so the completion resolves.
    root = _brace_dir(tmp_path)
    assert await _sugg(root).get_suggestion("{{x}}/da") == "{{x}}/data.csv"


async def test_brace_escapes_off_on_a_placeholder_field_keeps_doubled_braces(tmp_path):
    # A placeholder-source field has brace_escapes=False: `{{x}}` stays literal, points
    # at no directory, and the completion is silent — proving the flag is threaded
    # through (not defaulted).
    root = _brace_dir(tmp_path)
    sugg = PathSuggester(kind="path", shlexy=False, placeholder_braces=True, ctx=_ctx(root))
    assert await sugg.get_suggestion("{{x}}/da") is None


async def test_shlexy_trailing_piece_refuses_either_quote(tmp_path):
    root = _tree(tmp_path)
    (root / "'q.txt").write_text("x", encoding="utf-8")
    if os.name != "nt":  # a double quote can't appear in a Windows filename
        (root / '"q.txt').write_text("x", encoding="utf-8")
    s = _sugg(root, shlexy=True)
    # A trailing piece bearing EITHER quote refuses to complete (appended ghost text
    # can't be re-quoted honestly); without that, it would complete these odd names.
    assert await s.get_suggestion("done.txt 'q") is None
    assert await s.get_suggestion('done.txt "q') is None
    # A clean trailing piece still completes, so the refusal isn't blanket.
    assert await s.get_suggestion("done.txt dr") == "done.txt draft.txt"


async def test_bare_token_prefix_without_separator_is_silent(tmp_path):
    root = _tree(tmp_path)
    (root / "~data.txt").write_text("x", encoding="utf-8")
    (root / "{data.txt").write_text("x", encoding="utf-8")
    s = _sugg(root)
    # A "~" or "{" that hasn't reached a separator yet completes nothing — even when a
    # file literally starting with that character sits in the workdir.
    assert await s.get_suggestion("~da") is None
    assert await s.get_suggestion("{da") is None


# ---------------------------------------------------------------------------
# _list_filtered ranking and hidden-entry rules (picker only)
# ---------------------------------------------------------------------------


def test_list_filtered_reveals_hidden_only_behind_a_dot_filter(tmp_path):
    root = tmp_path / "root"
    root.mkdir()
    (root / ".env").write_text("x", encoding="utf-8")
    (root / "readme").write_text("x", encoding="utf-8")
    assert tui_pathpick._list_filtered(root, "en") == []  # substring "en" ⊄ visible names
    assert tui_pathpick._list_filtered(root, ".en") == [(".env", False)]  # dot filter reveals it


def test_list_filtered_dir_sorts_before_an_earlier_file_within_a_rank(tmp_path):
    root = tmp_path / "root"
    (root / "xz").mkdir(parents=True)  # a directory
    (root / "xa").write_text("x", encoding="utf-8")  # an alphabetically-earlier file
    # Both prefix-match "x" (same rank); the directory wins regardless of the later name.
    assert tui_pathpick._list_filtered(root, "x") == [("xz", True), ("xa", False)]


def test_list_filtered_tiebreak_is_case_insensitive(tmp_path):
    root = tmp_path / "root"
    root.mkdir()
    # "_z.txt" and "a.txt" are both files, both prefix-rank equal under an empty needle.
    # ASCII '_' (95) sits between 'Z' (90) and 'a' (97): a case-insensitive tiebreak
    # keeps '_' before 'a'; an upper() tiebreak would flip them.
    (root / "_z.txt").write_text("x", encoding="utf-8")
    (root / "a.txt").write_text("x", encoding="utf-8")
    assert tui_pathpick._list_filtered(root, "") == [("_z.txt", False), ("a.txt", False)]


# ---------------------------------------------------------------------------
# FilePickerModal display + navigation refresh
# ---------------------------------------------------------------------------


async def test_picker_pinned_row_shows_its_label(tmp_path):
    root = _tree(tmp_path)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(FilePickerModal(_ctx(root)))
        await pilot.pause()
        modal = app.screen
        assert isinstance(modal, FilePickerModal)
        first = modal.query_one(OptionList).get_option_at_index(0)
        assert first.id == "__use_dir__"
        # endswith, not `in`: a corrupted msgid ("XX(use this directory)XX") would still
        # CONTAIN the phrase — the exact tail is what proves the real label rendered.
        assert str(first.prompt).endswith("(use this directory)")


async def test_picker_empty_directory_highlights_the_pinned_row(tmp_path):
    empty = tmp_path / "empty"
    empty.mkdir()
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(FilePickerModal(_ctx(empty)))
        await pilot.pause()
        modal = app.screen
        assert isinstance(modal, FilePickerModal)
        option_list = modal.query_one(OptionList)
        assert option_list.option_count == 1  # only the pinned row
        assert option_list.highlighted == 0


async def test_picker_ascend_repopulates_the_parent_listing(tmp_path):
    root = _tree(tmp_path)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(FilePickerModal(PathContext(workdir=root / "sub", invoke_cwd=root)))
        await pilot.pause()
        modal = app.screen
        assert isinstance(modal, FilePickerModal)
        await pilot.press("backspace")  # empty filter → ascend to root
        await pilot.pause()
        option_list = modal.query_one(OptionList)
        ids = [str(option_list.get_option_at_index(i).id) for i in range(option_list.option_count)]
        # The parent's real entries are shown (not an empty filtered-to-nothing list).
        assert "d:sub" in ids
        assert "f:data.csv" in ids
