"""The TUI workbench (Textual): skit's home surface.

Library screen (docs/ux-redesign.md §1): search + list + detail pane, two-row footer
with every action always visible (design assumption: most TUI users never press ?),
recency sort, contextual r-rerun, lazy drift check on selection. Presentation only —
all logic goes through the headless store/flows/launcher layers.

Keys: Enter run · r rerun (after a first run) · p script settings · e edit script ·
Del remove · a add script · s presets · , preferences · D health check · / search ·
double Ctrl+C / Esc quit.

Focus model: the TABLE owns the keyboard by default, so every advertised single-letter
key actually fires; `/` (or a click) enters the search box, where letters type as text
and Esc returns to the table. Type-to-search-with-permanent-focus was abandoned after
review: an always-focused Input consumes printable keys, which silently killed every
single-letter action the footer advertised.
"""

from __future__ import annotations

import contextlib
import time
from datetime import UTC, datetime
from pathlib import Path
from typing import override

from rich.markup import escape
from rich.text import Text
from textual import events, on
from textual.app import App, ComposeResult
from textual.binding import Binding
from textual.containers import Horizontal, Vertical, VerticalScroll
from textual.screen import ModalScreen
from textual.widgets import DataTable, Input, Label, Static

from . import argstate, editor, flows, launcher, models, store, theme, tui_footer
from .i18n import gettext, ngettext
from .models import Entry
from .theme import CLAUDE_THEME
from .tui_form import FormResult, RunFormScreen

# msgids, translated at render time — a module-level gettext() would freeze whichever
# locale happened to be active at first import.
_KIND_BADGES = {
    "python": ("⬡", "Python"),
    "exe": ("▶", "Program"),
    "command": ("$", "Command"),
}


def _kind_badge(kind: str) -> tuple[str, str]:
    glyph, label = _KIND_BADGES.get(kind, ("?", kind))
    return glyph, gettext(label)


def _fuzzy_match(query: str, text: str) -> bool:
    """Subsequence fuzzy match (case-insensitive)."""
    q = query.lower()
    haystack = text.lower()
    pos = 0  # pragma: no mutate — find(ch, None) == find(ch, 0)
    for ch in q:
        pos = haystack.find(ch, pos)
        if pos == -1:
            return False
        pos += 1
    return True


def _activity_key(entry: Entry) -> str:
    """Recency sort key: last run or added time, whichever is newer (a fresh add must
    surface even though it has never run)."""
    last = argstate.load_state(entry.slug)["last_run"]
    return max(str(last.get("at", "")), entry.meta.added_at or "")


def _relative_time(iso: str) -> str:
    try:
        then = datetime.fromisoformat(iso)
    except ValueError:
        return iso
    delta = datetime.now(UTC) - then
    seconds = int(delta.total_seconds())
    if seconds < 90:
        return gettext("just now")
    if seconds < 5400:
        return gettext("%(minutes)s min ago") % {"minutes": seconds // 60}
    if seconds < 129600:
        return gettext("%(hours)s h ago") % {"hours": seconds // 3600}
    return gettext("%(days)s d ago") % {"days": seconds // 86400}


class ConfirmRemove(ModalScreen[bool]):
    """Removal modal: verb keys, and the reassurance that carries the A5 promise."""

    BINDINGS = [
        Binding("y", "confirm", gettext("Remove")),
        Binding("escape,n", "cancel", gettext("Keep")),
    ]
    DEFAULT_CSS = """
    ConfirmRemove { align: center middle; }
    #confirm-box {
        border: round $skit-box-maroon; padding: 1 2; width: auto; height: auto;
        background: $background;
    }
    /* In an auto-width box a 1fr Static collapses to zero columns — every modal child
       must hug its content for the box to measure anything at all. */
    #confirm-box Static { width: auto; }
    #confirm-box > Static:last-of-type { margin: 1 0 0 0; }
    """

    def __init__(self, entry: Entry) -> None:
        super().__init__()
        self._entry: Entry = entry

    @override
    def compose(self) -> ComposeResult:
        lines = [Label(gettext('Remove "%(name)s"?') % {"name": escape(self._entry.meta.name)})]
        if self._entry.meta.kind != "command":
            lines.append(Static(f"[dim]{gettext('Your original file will not be deleted.')}[/dim]"))
        # The verb line IS the button row: y/Esc stay advertised, and each chip is
        # clickable — modals must not be the one place that suddenly demands keys.
        lines.append(
            Static(
                tui_footer.bar(
                    tui_footer.chip("screen.confirm", "y", gettext("Remove")),
                    tui_footer.chip("screen.cancel", "Esc", gettext("Keep")),
                ),
                markup=True,
            )
        )
        yield Vertical(*lines, id="confirm-box")

    def action_confirm(self) -> None:
        self.dismiss(True)

    def action_cancel(self) -> None:
        self.dismiss(False)


class HelpScreen(ModalScreen[None]):
    """? overlay. Everything here is ALSO in the footer — this is a reminder, never the
    only path to a feature (discoverability assumption in the UX spec)."""

    BINDINGS = [Binding("escape,question_mark", "dismiss_help", gettext("Close"))]
    DEFAULT_CSS = """
    HelpScreen { align: center middle; }
    #help-box { border: round $accent; padding: 1 2; width: auto; height: auto;
                background: $background; }
    /* Same zero-width trap as the confirm box: 1fr Statics inside an auto box measure
       as nothing, which rendered the ? overlay as a tiny empty square. */
    #help-box Static { width: auto; }
    #help-box > Static:last-of-type { margin: 1 0 0 0; }
    """

    @override
    def compose(self) -> ComposeResult:
        rows = [
            ("Enter", gettext("Run")),
            ("r", gettext("Rerun with last values")),
            ("p", gettext("Script settings")),
            ("s", gettext("Presets")),
            ("e", gettext("Edit script")),
            ("Del", gettext("Remove")),
            ("a", gettext("Add script")),
            (",", gettext("Preferences")),
            ("D", gettext("Health check")),
            ("Ctrl+C Ctrl+C / Esc", gettext("Quit")),
        ]
        body = "\n".join(f"[$accent]{k:>16}[/]  {escape(v)}" for k, v in rows)
        yield Vertical(
            Static(body, markup=True),
            Static(
                tui_footer.bar(tui_footer.chip("screen.dismiss_help", "Esc", gettext("Close"))),
                markup=True,
            ),
            id="help-box",
        )

    def action_dismiss_help(self) -> None:
        self.dismiss(None)


class MenuApp(App[int]):
    """The Library. Exit code 0 on a clean quit."""

    TITLE = "skit · " + gettext("Library")
    ENABLE_COMMAND_PALETTE = False
    CSS = (
        theme.CHROME_CSS
        + """
    #search { dock: top; }
    #main { height: 1fr; }
    /* btop grammar: each panel is a rounded box with its own muted border tint and its
       title ON the border (list green, detail indigo — the cpu/net pairing). */
    #entry-table { width: 3fr; border: round $skit-box-green; border-title-color: ansi_bright_white; }
    #detail { width: 2fr; border: round $skit-box-indigo; border-title-color: ansi_bright_white;
              padding: 0 1; }
    /* One docked container holds the whole footer. Docking each row separately makes
       every dock:bottom widget land on the SAME bottom line (dock does not stack), so
       the two key rows end up hidden behind the status line — the footer looks empty.
       Stacking them inside a single auto-height docked Vertical is what actually shows
       all three rows. */
    #footer { dock: bottom; height: auto; }
    #status { height: 1; color: $text-muted; padding: 0 1; }
    #keys-local { height: 1; padding: 0 1; }
    #keys-global { height: 1; padding: 0 1; }
    """
    )
    BINDINGS = [
        Binding("ctrl+c", "ctrl_c_quit", gettext("Quit"), priority=True),
        Binding("escape", "back_or_quit", gettext("Quit"), show=False),
        # "delete" is forward-delete (fn+Delete on a Mac); the key most users press to
        # delete — the big ⌫ above Return — sends backspace, which Textual names
        # "backspace". Bind both so the footer's advertised "Del" actually fires. This is
        # safe next to the search box: a focused Input owns backspace for its own
        # delete-left (closer in the focus chain than this non-priority app binding), so
        # backspace only reaches "remove" when the table has focus.
        Binding("delete,backspace", "remove", gettext("Remove")),
        Binding("ctrl+e", "edit", gettext("Edit script")),
        Binding("e", "edit", gettext("Edit script"), show=False),
        Binding("enter", "run", gettext("Run")),
        Binding("r", "rerun", gettext("Rerun"), show=False),
        Binding("p", "settings", gettext("Script settings"), show=False),
        Binding("s", "presets", gettext("Presets"), show=False),
        Binding("a", "add", gettext("Add script"), show=False),
        Binding("comma", "preferences", gettext("Preferences"), show=False),
        Binding("D", "health", gettext("Health check"), show=False),
        Binding("question_mark", "help", gettext("Help"), show=False),
        Binding("slash", "focus_search", gettext("Search"), show=False),
        # priority: Textual's built-in Tab focus-nav would otherwise win. The Library's
        # focus model moves with / (search) and Esc (back to the table), not Tab, so Tab is
        # free to mean "toggle the detail pane" as the spec asks.
        Binding("tab", "toggle_detail", gettext("Detail pane"), show=False, priority=True),
    ]
    CTRL_C_WINDOW = 2.0
    # Below this terminal width the detail pane auto-collapses to give the list the whole
    # row (spec §1); Tab pins it open/closed regardless.
    MIN_DETAIL_WIDTH = 80

    def __init__(self) -> None:
        super().__init__()
        self._entries: list[Entry] = []
        self._visible: list[Entry] = []
        self._ctrl_c_at: float = 0.0
        self._drift_cache: dict[str, tuple[float, bool]] = {}  # slug -> (mtime, has_drift)
        self._detail_manual: bool | None = None  # None = auto by width; else the user's Tab choice

    @override
    def get_css_variables(self) -> dict[str, str]:
        # The first stylesheet parse runs before on_mount activates the theme; without
        # the $skit-box-* merge that parse dies on an unresolved variable.
        return {**super().get_css_variables(), **theme.BOX_VARIABLES}

    def on_mount(self) -> None:
        self.register_theme(CLAUDE_THEME)
        self.theme = "skit-claude"
        table = self.query_one(DataTable)
        table.add_columns(gettext("Name"), gettext("Kind"), " ")
        table.border_title = gettext("Scripts")
        self.query_one("#detail").border_title = gettext("Detail pane")
        self._reload()
        # The table owns the keyboard: that's what makes the advertised single-letter
        # keys real. `/` moves into the search box.
        table.focus()

    @override
    def compose(self) -> ComposeResult:
        yield Input(placeholder=gettext("/ to search names and descriptions…"), id="search")
        with Horizontal(id="main"):
            yield DataTable(cursor_type="row", zebra_stripes=False, id="entry-table")
            yield VerticalScroll(Static("", id="detail-body", markup=True), id="detail")
        with Vertical(id="footer"):
            yield Static("", id="keys-local", markup=True)
            yield Static("", id="keys-global", markup=True)
            yield Static("", id="status")

    # ------------------------------------------------------------------ data

    def _reload(self) -> None:
        self._entries = sorted(store.list_entries(), key=_activity_key, reverse=True)
        self._apply_filter(self.query_one("#search", Input).value)

    def _apply_filter(self, query: str) -> None:
        table = self.query_one(DataTable)
        table.clear()
        self._visible = [
            e
            for e in self._entries
            if not query or _fuzzy_match(query, f"{e.meta.name} {e.meta.description}")
        ]
        for e in self._visible:
            glyph, kind_label = _kind_badge(e.meta.kind)
            if e.meta.kind == "python" and e.meta.mode == "reference":
                # reference: links the original, never copied (spec §1). kind-gated:
                # command templates also carry mode="reference" in their meta, but there
                # is no linked file to point an arrow at.
                kind_label = f"{kind_label} ↗"
            health = "⚠" if launcher.target_missing(e) else ""
            table.add_row(escape(e.meta.name), f"{glyph} {kind_label}", health, key=e.slug)
        self._refresh_status()
        self._refresh_detail()
        self._refresh_footer()

    def _selected(self) -> Entry | None:
        table = self.query_one(DataTable)
        if not self._visible:
            return None
        if 0 <= table.cursor_row < len(self._visible):  # pragma: no mutate — self-clamps cursor
            return self._visible[table.cursor_row]
        return None  # pragma: no cover — Textual clamps cursor_coordinate

    # ---------------------------------------------------------------- render

    def _refresh_status(self, message: str = "") -> None:
        status = self.query_one("#status", Static)
        if message:
            status.update(message)
            return
        if not self._entries:
            status.update(gettext("Your scripts will appear here."))
            return
        status.update(
            ngettext(
                "%(shown)s/%(total)s script", "%(shown)s/%(total)s scripts", len(self._entries)
            )
            % {"shown": len(self._visible), "total": len(self._entries)}
        )

    def _refresh_footer(self) -> None:
        entry = self._selected()
        local: list[str] = []
        if entry is not None:
            local.append(tui_footer.chip("app.run", "Enter", gettext("Run")))
            if argstate.load_state(entry.slug)["last_run"]:
                local.append(tui_footer.chip("app.rerun", "r", gettext("Rerun")))
            local.append(tui_footer.chip("app.settings", "p", gettext("Script settings")))
            local.append(tui_footer.chip("app.edit", "e", gettext("Edit script")))
            local.append(tui_footer.chip("app.remove", "Del", gettext("Remove")))
        globals_row = [
            tui_footer.chip("app.add", "a", gettext("Add script")),
            tui_footer.chip("app.presets", "s", gettext("Presets")),
            tui_footer.chip("app.focus_search", "/", gettext("Search")),
            tui_footer.chip("app.preferences", ",", gettext("Preferences")),
            tui_footer.chip("app.health", "D", gettext("Health check")),
            tui_footer.chip("app.help", "?", gettext("Help")),
        ]
        self.query_one("#keys-local", Static).update(tui_footer.bar(*local))
        self.query_one("#keys-global", Static).update(tui_footer.bar(*globals_row))

    def _has_drift(self, entry: Entry) -> bool:
        """Drift is the expensive check (read + reconcile): lazy, per-selection, mtime-cached."""
        if entry.meta.kind != "python" or not entry.script_path.exists():
            return False
        mtime = entry.script_path.stat().st_mtime
        cached = self._drift_cache.get(entry.slug)
        if cached is not None and cached[0] == mtime:
            return cached[1]
        plan = flows.plan_for_entry(entry)
        drift = bool(plan.drift_lines)
        self._drift_cache[entry.slug] = (mtime, drift)
        return drift

    def _refresh_detail(self) -> None:
        body = self.query_one("#detail-body", Static)
        entry = self._selected()
        if entry is None:
            if not self._entries:
                body.update(
                    "\n".join(
                        (
                            f"[bold]{gettext('Your scripts will appear here.')}[/bold]",
                            "",
                            gettext("Press a to add the first one,"),
                            gettext("or run: skit add <path> in a terminal."),
                        )
                    )
                )
            else:
                body.update("")
            return
        body.update("\n".join(self._detail_lines(entry)))

    def _detail_lines(self, entry: Entry) -> list[str]:
        glyph, kind_label = _kind_badge(entry.meta.kind)
        lines = [f"[bold $accent]{escape(entry.meta.name)}[/]", f"{glyph} {kind_label}"]
        if entry.meta.kind == "python":
            if entry.meta.mode == "copy":
                lines.append(
                    f"[dim]✓ {gettext('The copy is kept by skit; your original file is never modified.')}[/dim]"
                )
            else:
                lines.append(
                    f"[dim]↗ {gettext('Linked to the original: %(path)s') % {'path': escape(entry.meta.source)}}[/dim]"
                )
        if entry.meta.kind == "command":
            lines.append(f"[dim]{escape(entry.meta.template)}[/dim]")
        lines.append("")
        lines.append(
            escape(entry.meta.description)
            if entry.meta.description
            else f"[dim]{gettext('(no description — add one in Script settings)')}[/dim]"
        )
        lines.append("")
        lines.extend(self._detail_state_lines(entry))
        return lines

    def _detail_state_lines(self, entry: Entry) -> list[str]:
        lines: list[str] = []
        state = argstate.load_state(entry.slug)
        plan = flows.plan_for_entry(entry)
        if plan.fields:
            shown: list[str] = []
            for f in plan.fields[:6]:
                if f.secret:
                    shown.append(f"{escape(f.key)}=•••🔒")
                else:
                    value = state["values"].get(f.key, f.default)
                    shown.append(f"{escape(f.key)}={escape(value)}" if value else escape(f.key))
            more = "" if len(plan.fields) <= 6 else " …"
            lines.append(gettext("Parameters  %(summary)s") % {"summary": "  ".join(shown) + more})
        if state["presets"]:
            lines.append(
                gettext("Presets  %(names)s")
                % {"names": " · ".join(escape(p) for p in sorted(state["presets"]))}
            )
        if entry.meta.dependencies:
            lines.append(
                gettext("Depends on  %(deps)s")
                % {"deps": ", ".join(escape(d) for d in entry.meta.dependencies)}
            )
        last = state["last_run"]
        if last:
            outcome = (
                f"[green]✓ {gettext('finished')}[/green]"
                if last.get("exit") == 0
                else f"[yellow]✗ {gettext('failed (code %(code)s)') % {'code': last.get('exit')}}[/yellow]"
            )
            lines.append(
                gettext("Last run  %(when)s · %(outcome)s")
                % {"when": _relative_time(str(last.get("at", ""))), "outcome": outcome}
            )
        else:
            lines.append(f"[dim]{gettext('Not run yet')}[/dim]")
        marker = launcher.missing_marker(entry)
        if marker:
            lines.append(f"[yellow]{escape(marker)}[/yellow]")
        elif self._has_drift(entry):
            lines.append(
                f"[yellow]⚠ {gettext('The script changed — skit checks the form against it before every run.')}[/yellow]"
            )
        return lines

    # ---------------------------------------------------------------- events

    @on(Input.Changed, "#search")
    def _on_search(self, event: Input.Changed) -> None:
        self._apply_filter(event.value)

    @on(Input.Submitted, "#search")
    def _on_search_submitted(self, event: Input.Submitted) -> None:
        # Enter in the search box runs the top match (and returns focus to the table,
        # so the follow-up keys — r, p, e — work immediately).
        self.query_one(DataTable).focus()
        self.action_run()

    @on(DataTable.RowHighlighted)
    def _on_row_highlighted(self, event: DataTable.RowHighlighted) -> None:
        self._refresh_detail()
        self._refresh_footer()

    @on(DataTable.RowSelected)
    def _on_row_selected(self, event: DataTable.RowSelected) -> None:
        self.action_run()

    def on_key(self, event: events.Key) -> None:
        # While searching, Up/Down still drive the table (browse results as you type).
        search = self.query_one("#search", Input)
        if event.key in ("up", "down") and self.focused is search:
            table = self.query_one(DataTable)
            table.action_cursor_up() if event.key == "up" else table.action_cursor_down()
            event.stop()

    _LIBRARY_ACTIONS = (
        "run",
        "remove",
        "edit",
        "rerun",
        "settings",
        "presets",
        "add",
        "preferences",
        "health",
        "help",
        "focus_search",
        "toggle_detail",
    )

    @override
    def check_action(self, action: str, parameters: tuple[object, ...]) -> bool | None:
        """Library keys act only on the Library: keys that bubble out of a pushed
        screen's own widgets must not trigger surprise actions underneath it."""
        return not (action in self._LIBRARY_ACTIONS and len(self.screen_stack) > 1)

    def action_focus_search(self) -> None:
        self.query_one("#search", Input).focus()

    def action_back_or_quit(self) -> None:
        """Esc in the search box returns to the table; Esc on the table quits."""
        search = self.query_one("#search", Input)
        if self.focused is search:
            self.query_one(DataTable).focus()
            return
        self.exit(0)

    def action_ctrl_c_quit(self) -> None:
        now = time.monotonic()
        if now - self._ctrl_c_at <= self.CTRL_C_WINDOW:
            self.exit(0)
            return
        self._ctrl_c_at = now
        self.notify(gettext("Press Ctrl+C again to quit"), timeout=self.CTRL_C_WINDOW)

    # ------------------------------------------------------------------- run

    def action_run(self) -> None:
        if len(self.screen_stack) > 1:
            return
        entry = self._selected()
        if entry is None:
            return
        try:
            launcher.preflight(entry)
        except launcher.LaunchError as exc:
            self._refresh_status(gettext("Error: %(error)s") % {"error": escape(str(exc))})
            return
        plan = flows.plan_for_entry(entry)
        if not plan.fields and not plan.degraded_reason:
            self._execute(entry, plan, {}, argstate.load_state(entry.slug)["extra_args"])
            return
        prefill = flows.prefill(plan, entry.slug)

        def _submitted(result: FormResult) -> None:
            if result is None:
                return
            values, extra = result
            self._execute(entry, plan, values, extra, show_drift=False)

        self.push_screen(RunFormScreen(entry, plan, prefill), _submitted)

    def action_rerun(self) -> None:
        """r: skip the form, rerun with the last values — but never skip the checks."""
        entry = self._selected()
        if entry is None:
            return
        if not argstate.load_state(entry.slug)["last_run"]:
            self._refresh_status(
                gettext("%(name)s hasn't run yet — press Enter to fill the form first.")
                % {"name": escape(entry.meta.name)}
            )
            return
        try:
            launcher.preflight(entry)
        except launcher.LaunchError as exc:
            self._refresh_status(gettext("Error: %(error)s") % {"error": escape(str(exc))})
            return
        plan = flows.plan_for_entry(entry)
        prefill = flows.prefill(plan, entry.slug)
        if flows.validate(plan, prefill):
            # The last values no longer satisfy the form (e.g. a new required field):
            # fall back to the form rather than assembling a broken command.
            self.action_run()
            return
        self._execute(entry, plan, prefill, argstate.load_state(entry.slug)["extra_args"])

    def _execute(
        self,
        entry: Entry,
        plan: flows.FormPlan,
        values: dict[str, str],
        extra: list[str],
        *,
        show_drift: bool = True,
    ) -> None:
        """Suspend, deliver (inject/flags/template), pass the terminal through, record.

        show_drift=False when the form was just shown (its banner already said it)."""
        try:
            asm = flows.assemble(plan, values, list(extra), cwd=Path.cwd())
        except flows.FormError as exc:
            self._refresh_status(gettext("Error: %(error)s") % {"error": escape(str(exc))})
            return
        with self.suspend():
            print(f"\n── {gettext('Run %(name)s') % {'name': entry.meta.name}} ──\n", flush=True)
            if show_drift:
                for line in plan.drift_lines:
                    print(line, flush=True)
            # The shared delivery pipeline: inject, transparency, run, cleanup. The TUI
            # just prints what it emits (bare, inside the suspend) and shows a banner.
            outcome = flows.execute(entry, plan, asm, emit=lambda line: print(line, flush=True))
            if outcome.code is None:
                print(gettext("Error: %(error)s") % {"error": outcome.message}, flush=True)
            print(f"\n{self._run_banner(outcome)}", flush=True)
            with contextlib.suppress(EOFError):
                input()
        code = outcome.code
        if code is None:
            # The script never ran: recording it would light up r-rerun and stamp a
            # "last run" that never happened.
            self._reload()
            self._refresh_status(
                gettext("Last: %(name)s ✗ couldn't launch") % {"name": escape(entry.meta.name)}
            )
            return
        flows.save_after_run(entry.slug, plan, values, list(extra), code, at=models.now_iso())
        self._reload()
        status = (
            gettext("Last: %(name)s ✓ finished")
            if code == 0
            else gettext("Last: %(name)s ✗ failed (code %(code)s)")
        )
        self._refresh_status(status % {"name": escape(entry.meta.name), "code": code})

    @staticmethod
    def _run_banner(outcome: flows.RunOutcome) -> str:
        if outcome.code == 0:
            return gettext("✓ finished — press Enter to return")
        if outcome.launched:
            return gettext("✗ failed (code %(code)s) — press Enter to return") % {
                "code": outcome.code
            }
        return gettext("✗ couldn't launch — press Enter to return")

    # --------------------------------------------------------------- actions

    def action_edit(self) -> None:
        if len(self.screen_stack) > 1:
            return
        entry = self._selected()
        if entry is None:
            return
        target = self._editable_source(entry)
        if target is None or not target.exists():
            self._refresh_status(
                gettext("%(name)s: no editable script source (only Python scripts have one).")
                % {"name": escape(entry.meta.name)}
            )
            return
        with self.suspend():
            try:
                editor.open_in_editor(target)
            except editor.EditorError as exc:
                print(str(exc), flush=True)
                with contextlib.suppress(EOFError):
                    input(gettext("Press Enter to return"))
        self._drift_cache.pop(entry.slug, None)
        self._reload()
        self._refresh_status(gettext("Edited %(name)s.") % {"name": escape(entry.meta.name)})

    def _editable_source(self, entry: Entry) -> Path | None:
        if entry.meta.kind != "python":
            return None
        if entry.meta.mode == "reference":
            return Path(entry.meta.source)
        return entry.dir / "script.py"

    def action_remove(self) -> None:
        if len(self.screen_stack) > 1:
            return
        entry = self._selected()
        if entry is None:
            return

        def _done(confirmed: bool | None) -> None:
            if confirmed:
                store.remove(entry.slug)
                self._reload()

        self.push_screen(ConfirmRemove(entry), _done)

    def action_add(self) -> None:
        from .tui_add import AddSourceScreen

        def _added(slug: str | None) -> None:
            self._reload()
            if slug:
                self._select_slug(slug)
                self._refresh_status(gettext("✓ added"))

        self.push_screen(AddSourceScreen(), _added)

    def action_settings(self, section: str = "") -> None:
        entry = self._selected()
        if entry is None:
            return
        from .tui_settings import ScriptSettingsScreen

        def _closed(_changed: bool | None) -> None:
            self._drift_cache.pop(entry.slug, None)
            self._reload()

        self.push_screen(ScriptSettingsScreen(entry, initial_section=section), _closed)

    def action_presets(self) -> None:
        self.action_settings(section="presets")

    def action_preferences(self) -> None:
        from .tui_prefs import PreferencesScreen

        def _applied(_result: object) -> None:
            self._retranslate_chrome()  # a language change must hit the chrome, not just rows
            self._reload()

        self.push_screen(PreferencesScreen(), _applied)

    def action_health(self) -> None:
        from .tui_health import HealthScreen

        def _jump(slug: str | None) -> None:
            self._reload()
            if slug:
                self._select_slug(slug)

        self.push_screen(HealthScreen(), _jump)

    def action_help(self) -> None:
        self.push_screen(HelpScreen())

    def _select_slug(self, slug: str) -> None:
        for i, e in enumerate(self._visible):
            if e.slug == slug:
                self.query_one(DataTable).move_cursor(row=i)
                break
        self._refresh_detail()
        self._refresh_footer()

    # ------------------------------------------------------------- detail pane

    def on_resize(self, event: events.Resize) -> None:
        """Narrow terminals give the whole row to the list (spec §1). Auto-collapse only
        while the user hasn't pinned the pane with Tab."""
        if self._detail_manual is None:
            self._set_detail_visible(event.size.width >= self.MIN_DETAIL_WIDTH)

    def action_toggle_detail(self) -> None:
        """Tab: show/hide the detail pane and pin that choice against auto-collapse."""
        self._detail_manual = not self.query_one("#detail").display
        self._set_detail_visible(self._detail_manual)

    def _set_detail_visible(self, visible: bool) -> None:
        self.query_one("#detail").display = visible

    # ------------------------------------------------------------- language

    def _retranslate_chrome(self) -> None:
        """Re-translate the static chrome that compose/on_mount set once, so a language
        change in Preferences applies on the spot (spec §6) rather than only to the rows
        _reload rebuilds: the window title, the search placeholder, the column headers."""
        self.title = "skit · " + gettext("Library")
        self.query_one("#search", Input).placeholder = gettext(
            "/ to search names and descriptions…"
        )
        headers = [gettext("Name"), gettext("Kind"), " "]
        for column, label in zip(self.query_one(DataTable).ordered_columns, headers, strict=False):
            column.label = Text(label)
        self.query_one(DataTable).border_title = gettext("Scripts")
        self.query_one("#detail").border_title = gettext("Detail pane")
        self.query_one(DataTable).refresh()


def run_menu() -> int:
    app = MenuApp()
    result = app.run()
    return result if isinstance(result, int) else 0
