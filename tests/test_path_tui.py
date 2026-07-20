"""The path-entry TUI layer (docs/design/path.md P1b): the ghost suggester's
activation and root rules, the file-picker modal's keys (each advertised chip has a
positive pilot test here), and the per-shape insertion semantics."""

from __future__ import annotations

import shlex
from pathlib import Path

import pytest
from textual.widgets import Input, OptionList, Static

from skit import store, tui, tui_footer, tui_pathpick
from skit.tui_form import FieldRow, RunFormScreen, TokenMenuModal
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
    from skit import flows

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
        # Appended as ONE quoted piece: shlex re-parsing keeps the filename whole.
        assert extra_row.value == "--verbose 'a b.txt'"
        assert shlex.split(extra_row.value) == ["--verbose", "a b.txt"]


async def test_picker_appends_quoted_to_a_multiple_field(tmp_path, monkeypatch):
    """The nargs='*' path field: append as one shlex-quoted piece, and the value
    survives the ACTUAL splitter of multiple fields (flows._split_multi)."""
    from skit import flows

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
        # The extra-args dialect on Windows: CRT quoting (double quotes), where the
        # POSIX spelling 'a b.txt' would shatter into two literal-quoted arguments.
        box.value = "--verbose"
        with monkeypatch.context() as m:
            m.setattr(tui_pathpick.argv_text.sys, "platform", "win32")
            tui_pathpick.insert_picked(box, PickedPath("a b.txt"), mode="argv")
            assert box.value == '--verbose "a b.txt"'
            assert tui_pathpick.argv_text.split(box.value) == ["--verbose", "a b.txt"]


async def test_secret_field_never_gets_a_suggester(tmp_path):
    from skit import flows

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
