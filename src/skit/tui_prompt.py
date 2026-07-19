"""Shared prompt-only TUI widgets.

The inline add/settings surfaces deliberately preview only a small number of detected
variables.  This modal is the complete, searchable view behind that preview: no
placeholder becomes unreachable merely because it appeared after the flood cap.
"""

from __future__ import annotations

from typing import override

from rich.markup import escape
from textual import on
from textual.app import ComposeResult
from textual.binding import Binding
from textual.containers import Vertical
from textual.screen import ModalScreen
from textual.widgets import Checkbox, Input, Label, SelectionList, Static

from . import tui_footer
from .i18n import gettext


class PromptCandidatePickerModal(ModalScreen[set[str] | None]):
    """Search and choose any detected prompt variable, without a long checkbox wall.

    Names are SelectionList *values*, never widget ids.  Placeholder identifiers are
    Unicode and user-controlled; keeping them out of Textual's id namespace also makes
    filtering/reordering incapable of changing which name a selection belongs to.
    """

    AUTO_FOCUS = "Input"
    BINDINGS = [
        Binding("ctrl+s", "done", gettext("Done")),
        Binding("escape", "cancel", gettext("Cancel")),
        *tui_footer.FIELD_NAV_BINDINGS,
    ]
    DEFAULT_CSS = """
    PromptCandidatePickerModal { align: center middle; }
    PromptCandidatePickerModal > Vertical {
        border: round $accent; padding: 1 2; width: 64; max-width: 100%;
        height: auto; max-height: 100%; background: $background;
    }
    PromptCandidatePickerModal SelectionList { border: none; max-height: 12; }
    PromptCandidatePickerModal.-h-normal SelectionList { max-height: 6; }
    PromptCandidatePickerModal.-h-short > Vertical,
    PromptCandidatePickerModal.-h-tiny > Vertical { padding: 0 2; }
    PromptCandidatePickerModal.-h-short Label,
    PromptCandidatePickerModal.-h-tiny Label { display: none; }
    PromptCandidatePickerModal.-h-short SelectionList { max-height: 1; }
    PromptCandidatePickerModal.-h-tiny SelectionList { max-height: 1; }
    PromptCandidatePickerModal Static { width: auto; margin: 1 0 0 0; }
    PromptCandidatePickerModal #prompt-candidate-keys { width: 100%; height: auto; }
    PromptCandidatePickerModal.-h-short Static,
    PromptCandidatePickerModal.-h-tiny Static { margin: 0; }
    """

    def __init__(self, names: list[str], selected: set[str]) -> None:
        super().__init__()
        # Detection already dedupes, but this boundary stays deterministic for any future
        # caller and preserves first appearance (the same order the inline preview uses).
        self._names: list[str] = list(dict.fromkeys(names))
        known = set(self._names)
        self._selected: set[str] = set(selected) & known
        self._visible_indices: list[int] = []

    def _selections(self, needle: str) -> list[tuple[str, int, bool]]:
        folded = needle.casefold()
        self._visible_indices = [
            index for index, name in enumerate(self._names) if folded in name.casefold()
        ]
        return [
            (escape(self._names[index]), index, self._names[index] in self._selected)
            for index in self._visible_indices
        ]

    @override
    def compose(self) -> ComposeResult:
        with Vertical():
            yield Label(gettext("Choose prompt variables"))
            yield Input(placeholder=gettext("type to filter…"), id="prompt-candidate-filter")
            yield Checkbox(
                gettext("Select all variables"),
                value=bool(self._names) and len(self._selected) == len(self._names),
                id="prompt-candidate-all",
            )
            yield SelectionList[int](
                *self._selections(""), id="prompt-candidate-list", compact=True
            )
            yield Static(
                tui_footer.bar(
                    tui_footer.chip("screen.done", "Ctrl+S", gettext("Done")),
                    tui_footer.chip("screen.cancel", "Esc", gettext("Cancel")),
                    tui_footer.nav_chip(),
                ),
                id="prompt-candidate-keys",
                markup=True,
            )

    def _capture_visible(self) -> None:
        listing = self.query_one("#prompt-candidate-list", SelectionList)
        for index in self._visible_indices:
            self._selected.discard(self._names[index])
        for index in listing.selected:
            self._selected.add(self._names[int(index)])

    def _sync_all_checkbox(self) -> None:
        checkbox = self.query_one("#prompt-candidate-all", Checkbox)
        # Programmatic synchronization is not a user selection.  Suppress that exact
        # message instead of maintaining an `_updating` flag whose timing depends on
        # Textual's queued event delivery.
        with checkbox.prevent(Checkbox.Changed):
            checkbox.value = bool(self._names) and len(self._selected) == len(self._names)

    def _rebuild(self, needle: str) -> None:
        listing = self.query_one("#prompt-candidate-list", SelectionList)
        selections = self._selections(needle)
        with listing.prevent(SelectionList.SelectedChanged):
            listing.clear_options()
            listing.add_options(selections)
        if listing.option_count:
            listing.highlighted = 0
        self._sync_all_checkbox()

    @on(Input.Changed, "#prompt-candidate-filter")
    def _filter(self, event: Input.Changed) -> None:
        self._capture_visible()
        self._rebuild(event.value.strip())

    @on(Input.Submitted, "#prompt-candidate-filter")
    def _focus_results(self) -> None:
        listing = self.query_one("#prompt-candidate-list", SelectionList)
        if listing.option_count:
            listing.focus()

    @on(SelectionList.SelectedChanged, "#prompt-candidate-list")
    def _selection_changed(self) -> None:
        self._capture_visible()
        self._sync_all_checkbox()

    @on(Checkbox.Changed, "#prompt-candidate-all")
    def _select_all_changed(self, event: Checkbox.Changed) -> None:
        self._selected = set(self._names) if event.value else set()
        needle = self.query_one("#prompt-candidate-filter", Input).value.strip()
        self._rebuild(needle)

    def action_done(self) -> None:
        self._capture_visible()
        self.dismiss(set(self._selected))

    def action_cancel(self) -> None:
        self.dismiss(None)
