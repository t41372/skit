"""Path-aware entry (issue #7): the ghost-text suggester and the file-picker modal.

All of the feature's TUI machinery lives in this one module — tui_form.py takes only
thin hooks (a suggester on each free-text Input, a "File or folder…" row in the insert
menu). That split is deliberate merge containment (docs/design/path.md §4-§5).

Completion follows the design's three coordinate systems (§3): a bare relative path
completes against the entry's resolved workdir — the directory the child will resolve
it in — while a token prefix ({cwd}/~/{env:X}) expands first and completes inside the
directory the expanded value denotes (a relative expansion falls back to the workdir
rule; an unexpandable one suggests nothing). Ghost text only ever APPENDS to what the
user typed, so prefix matching is exact-case by construction, and a suggestion is
never a value until the user accepts it.
"""

from __future__ import annotations

import os
import re
import shlex
from dataclasses import dataclass
from pathlib import Path
from typing import TYPE_CHECKING, override

from rich.markup import escape
from textual import on
from textual.binding import Binding
from textual.containers import Vertical
from textual.message import Message
from textual.screen import ModalScreen
from textual.suggester import Suggester
from textual.widgets import Input, Label, OptionList, Static
from textual.widgets.option_list import Option

from . import tokens, tui_footer
from .i18n import gettext
from .launcher import _resolve_workdir

if TYPE_CHECKING:
    from textual.app import ComposeResult

    from .models import Entry

# Entries examined per directory before the scan stops — a node_modules-sized
# directory must never stall the event loop (path.md risk 4).
SCAN_CAP = 2000  # pragma: no mutate — an arbitrary bound; the cap MECHANISM is tested

# Windows-only extra recognition: a backslash anywhere, or a drive-letter prefix.
_NT_PATHY_RE = re.compile(r"^[A-Za-z]:[\\/]")
_LAST_SEP_RE = re.compile(r"[/\\](?=[^/\\]*$)")


@dataclass(frozen=True)
class PickedPath:
    """A file-picker selection. A discriminated result — never a bare str — so the
    insert callback can tell a picked path from a token by construction (path.md §5:
    three insertion regimes share one channel)."""

    text: str


@dataclass(frozen=True)
class PathContext:
    """The form's completion roots, computed once per form (path.md §3): the entry's
    resolved workdir (where the child resolves bare relative paths) and the invoke
    cwd (where tokens expand at assembly)."""

    workdir: Path
    invoke_cwd: Path

    @classmethod
    def for_entry(cls, entry: Entry) -> PathContext:
        cwd = Path.cwd()
        return cls(workdir=_resolve_workdir(entry, cwd), invoke_cwd=cwd)

    @property
    def bare_root(self) -> Path | None:
        """Where a bare relative path completes — None when the resolved workdir is
        gone (vanished reference origin): the suggester goes silent (path.md §3)."""
        return self.workdir if self.workdir.is_dir() else None

    def picker_start(self) -> tuple[Path, bool]:
        """The picker's opening directory: the workdir, else its nearest existing
        ancestor, else the invoke cwd; True when the workdir itself was missing."""
        if self.workdir.is_dir():
            return self.workdir, False
        for ancestor in self.workdir.parents:
            if ancestor.is_dir():
                return ancestor, True
        return self.invoke_cwd, True

    def value_for(self, target: Path) -> str:
        """The inserted spelling of a picked path: relative to the workdir when the
        selection is inside it, absolute otherwise — `/` separators on every platform
        (POSIX shlex re-parses multi-value fields and eats backslashes)."""
        try:
            rel = target.relative_to(self.workdir)
        except ValueError:
            return target.as_posix()
        return rel.as_posix()  # the workdir itself renders as "."


def looks_pathy(piece: str) -> bool:
    """The universal-affordance activation rule (path.md §4): the text is path-shaped.
    Windows typists get backslash and drive-letter spellings recognized too."""
    if piece.startswith(("./", "../", "/", "~", "{cwd}")) or "/" in piece:
        return True
    return os.name == "nt" and ("\\" in piece or _NT_PATHY_RE.match(piece) is not None)


def _list_matches(base: Path, prefix: str) -> list[tuple[str, bool]]:
    """Directory entries under base whose names start with prefix (exact case —
    ghost text can only append), as (name, is_dir), sorted; hidden entries only when
    the prefix itself starts with a dot; at most SCAN_CAP entries examined."""
    matches: list[tuple[str, bool]] = []
    try:
        with os.scandir(base) as entries:
            for scanned, entry in enumerate(entries):
                if scanned >= SCAN_CAP:
                    break
                if entry.name.startswith(".") and not prefix.startswith("."):
                    continue
                if entry.name.startswith(prefix):
                    try:
                        is_dir = entry.is_dir()
                    except OSError:
                        is_dir = False
                    matches.append((entry.name, is_dir))
    except OSError:
        return []
    return sorted(matches)


class PathSuggester(Suggester):
    """Fish-style ghost completion for one form field; → accepts. The accept gesture
    is keyboard sugar, not a mouse-orphaned capability — the mouse path to the same
    outcome is the ▾ insert link → "File or folder…" picker (path.md §4)."""

    def __init__(
        self, *, kind: str, shlexy: bool, placeholder_braces: bool, ctx: PathContext
    ) -> None:
        super().__init__(use_cache=False, case_sensitive=True)
        self._kind = kind
        self._shlexy = shlexy  # multiple/extra-args: complete the trailing piece only
        self._brace_escapes = not placeholder_braces
        self._ctx = ctx

    @override
    async def get_suggestion(self, value: str) -> str | None:
        piece = self._trailing_piece(value)
        if not piece:
            # No piece begun (empty field, or a fresh space in a multi-value field):
            # a ghost extends what the user started, it never opens the bidding.
            return None
        if self._kind != "path" and not looks_pathy(piece):
            return None
        located = self._lookup(piece)
        if located is None:
            return None
        base, prefix = located
        for name, is_dir in _list_matches(base, prefix):
            remainder = name[len(prefix) :] + ("/" if is_dir else "")
            if remainder:
                return value + remainder
        return None

    def _trailing_piece(self, value: str) -> str | None:
        """The piece being typed: the whole value for a single-value field, the text
        after the last whitespace for a shlex-parsed one. A quote-in-progress piece
        never completes — appended ghost text can't be re-quoted honestly."""
        if not self._shlexy:
            return value
        piece = re.split(r"\s", value)[-1]
        if '"' in piece or "'" in piece:
            return None
        return piece

    def _lookup(self, piece: str) -> tuple[Path, str] | None:
        """(directory to list, typed name prefix) — the two-step rule of path.md §3:
        expand the head first; an absolute expansion is its own root, a relative one
        resolves where the child will (the bare root), a failed one is silence."""
        m = _LAST_SEP_RE.search(piece)
        head = piece[: m.end()] if m else ""
        prefix = piece[m.end() :] if m else piece
        if not head:
            if piece.startswith(("~", "{")):
                return None  # token without a separator yet: nothing to complete inside
            root = self._ctx.bare_root
            return (root, prefix) if root else None
        if tokens.has_tokens(head):
            try:
                expanded = tokens.expand(
                    head, cwd=self._ctx.invoke_cwd, brace_escapes=self._brace_escapes
                )
            except tokens.TokenError:
                return None
        else:
            expanded = head
        base = Path(expanded)
        if not base.is_absolute():
            root = self._ctx.bare_root
            if root is None:
                return None
            base = root / base
        return base, prefix


class _FilterInput(Input):
    """The picker's filter. Backspace-ascend rides the Input's OWN delete action —
    an empty value posts Ascend, anything else deletes exactly as every Input does.
    No priority binding, no exception to the editing-chord rule (path.md §5)."""

    class Ascend(Message):
        pass

    @override
    def action_delete_left(self) -> None:
        if not self.value:
            self.post_message(self.Ascend())
        else:
            super().action_delete_left()


class FilePickerModal(ModalScreen[PickedPath | None]):
    """Type-to-filter directory browser (the EnvPickerModal shape plus directory
    state). Enter acts on the highlighted row — descend into a directory, pick a
    file; Backspace on an empty filter ascends; the header always shows the current
    directory absolute, because on origin/store-workdir entries the browse root is
    not the user's cwd (path.md §5). Not a sandbox: browsing above the root is fine,
    the value just goes absolute."""

    AUTO_FOCUS = "Input"
    BINDINGS = [
        Binding("escape", "cancel", gettext("Cancel")),
        # Non-priority: while the filter Input has focus its own delete runs (see
        # _FilterInput); this covers Backspace when the OptionList holds focus.
        Binding("backspace", "ascend", gettext("Up"), show=False),
    ]
    DEFAULT_CSS = """
    FilePickerModal { align: center middle; }
    FilePickerModal > Vertical { border: round $accent; padding: 1 2; width: 72;
        max-width: 100%; height: auto; max-height: 100%; background: $background; }
    FilePickerModal OptionList { border: none; max-height: 12; }
    FilePickerModal #picker-dir { color: $text-muted; width: 1fr; }
    FilePickerModal #picker-notice { color: $warning; width: 1fr; }
    /* Same tier math as EnvPickerModal: shrink the list and flatten the chrome so
       the Esc chip — the modal's mouse path out — stays on screen at every height. */
    FilePickerModal.-h-normal OptionList { max-height: 6; }
    FilePickerModal.-h-short > Vertical, FilePickerModal.-h-tiny > Vertical { padding: 0 2; }
    FilePickerModal.-h-short OptionList { max-height: 3; }
    FilePickerModal.-h-tiny OptionList { max-height: 1; }
    FilePickerModal Static { width: auto; margin: 1 0 0 0; }
    FilePickerModal.-h-short Static, FilePickerModal.-h-tiny Static { margin: 0; }
    FilePickerModal #picker-dir, FilePickerModal #picker-notice { margin: 0; }
    """

    _USE_DIR = "__use_dir__"

    def __init__(self, ctx: PathContext) -> None:
        super().__init__()
        self._ctx = ctx
        self._dir, self._missing_root = ctx.picker_start()

    @override
    def compose(self) -> ComposeResult:
        with Vertical():
            yield Label(gettext("Insert a file or folder"))
            yield Static("", id="picker-dir", markup=False)
            if self._missing_root:
                yield Static(
                    gettext("The entry's working directory is missing — starting here instead."),
                    id="picker-notice",
                    markup=False,
                )
            yield _FilterInput(placeholder=gettext("type to filter…"))
            yield OptionList()
            yield Static(
                tui_footer.bar(
                    tui_footer.chip("screen.pick_highlighted", "Enter", gettext("Select")),
                    tui_footer.chip("screen.ascend", "Backspace", gettext("Up")),
                    tui_footer.chip("screen.cancel", "Esc", gettext("Cancel")),
                ),
                markup=True,
            )

    def on_mount(self) -> None:
        self._show_dir()
        self._populate("")

    def _show_dir(self) -> None:
        self.query_one("#picker-dir", Static).update(str(self._dir))

    def _populate(self, needle: str) -> None:
        option_list = self.query_one(OptionList)
        option_list.clear_options()
        options: list[Option] = []
        if not needle:
            # Pinned only while the filter is empty: a typed filter means "find me a
            # named entry", and Enter must then act on the first MATCH, not this row.
            options.append(
                Option(f"[dim]📂[/dim] {gettext('(use this directory)')}", id=self._USE_DIR)
            )
        dirs = [(n, d) for n, d in _list_matches(self._dir, needle) if d]
        files = [(n, d) for n, d in _list_matches(self._dir, needle) if not d]
        for name, _ in dirs:
            options.append(Option(f"▸ {escape(name)}/", id=f"d:{name}"))
        for name, _ in files:
            options.append(Option(escape(name), id=f"f:{name}"))
        option_list.add_options(options)
        if option_list.option_count:
            # Empty filter: highlight the first real entry (the pinned row stays one ↑
            # away); filtered: highlight the first match, so Enter-from-filter picks it.
            option_list.highlighted = 1 if not needle and option_list.option_count > 1 else 0

    @on(Input.Changed)
    def _filter(self, event: Input.Changed) -> None:
        self._populate(event.value.strip())

    @on(Input.Submitted)
    def _submitted(self, _event: Input.Submitted) -> None:
        # The EnvPicker precedent accepts the typed text here; a half-typed filter is
        # not a path, so the picker routes Enter to the highlighted row instead.
        self.action_pick_highlighted()

    @on(OptionList.OptionSelected)
    def _picked(self, event: OptionList.OptionSelected) -> None:
        self._act(str(event.option.id))

    @on(_FilterInput.Ascend)
    def _ascend_from_input(self) -> None:
        self.action_ascend()

    def action_pick_highlighted(self) -> None:
        option_list = self.query_one(OptionList)
        if option_list.highlighted is not None:
            option = option_list.get_option_at_index(option_list.highlighted)
            self._act(str(option.id))

    def action_ascend(self) -> None:
        parent = self._dir.parent
        if parent == self._dir:
            return  # filesystem root
        self._dir = parent
        self._after_move()

    def action_cancel(self) -> None:
        self.dismiss(None)

    def _act(self, option_id: str) -> None:
        if option_id == self._USE_DIR:
            self.dismiss(PickedPath(self._ctx.value_for(self._dir)))
        elif option_id.startswith("d:"):
            self._dir = self._dir / option_id[2:]
            self._after_move()
        elif option_id.startswith("f:"):
            self.dismiss(PickedPath(self._ctx.value_for(self._dir / option_id[2:])))

    def _after_move(self) -> None:
        """Descend/ascend housekeeping: show the new directory and clear the filter —
        a sticky filter would land every move on an empty list (path.md §5)."""
        self._show_dir()
        filter_input = self.query_one(_FilterInput)
        if filter_input.value:
            filter_input.value = ""  # Input.Changed repopulates
        else:
            self._populate("")
        filter_input.focus()


def insert_picked(target: Input, picked: PickedPath, *, shlexy: bool) -> None:
    """Apply a picked path to a field per its shape (path.md §5): a single-value field
    is REPLACED (at-cursor insertion corrupts a prefilled value); a shlex-parsed field
    appends the pick as one quoted piece at the end."""
    if shlexy:
        piece = shlex.quote(picked.text)
        existing = target.value.strip()
        target.value = f"{existing} {piece}" if existing else piece
    else:
        target.value = picked.text
    target.cursor_position = len(target.value)
