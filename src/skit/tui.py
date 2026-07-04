"""TUI main menu (Textual).

- Type to fuzzy-search; Up/Down or mouse to navigate; Enter to run (suspend + terminal pass-through,
  C6); Del to remove with confirmation.
- Quit: press Ctrl+C twice (the standard way); Esc and Ctrl+Q also work.
- Presentation only; all logic goes through the headless store/launcher API.
"""

from __future__ import annotations

import contextlib
import time
from dataclasses import dataclass
from typing import TYPE_CHECKING, override

from textual import events, on
from textual.app import App, ComposeResult
from textual.binding import Binding
from textual.containers import Vertical
from textual.screen import ModalScreen
from textual.widgets import DataTable, Footer, Input, Label, Static

from . import argstate, launcher, metawriter, reconcile, shim, store
from .i18n import gettext, ngettext
from .models import Entry

if TYPE_CHECKING:
    from pathlib import Path

    from .metawriter import ParamSpec


def _collect_command_params(entry: Entry, values: dict[str, str]) -> None:
    """Fill placeholders for a command entry, using the last-used value as the default."""
    last = argstate.load_last(entry.slug)["values"]
    for p in entry.meta.params or []:
        default = last.get(p, "")
        hint = f" [{default}]" if default else ""
        try:
            answer = input(f"  {p}{hint}: ").strip()
        except EOFError:
            answer = ""
        values[p] = answer or default


def _fuzzy_match(query: str, text: str) -> bool:
    """Subsequence fuzzy match (case-insensitive)."""
    q = query.lower()
    haystack = text.lower()
    pos = 0
    for ch in q:
        pos = haystack.find(ch, pos)
        if pos == -1:
            return False
        pos += 1
    return True


class ConfirmRemove(ModalScreen[bool]):
    BINDINGS = [
        Binding("y", "confirm", gettext("Confirm")),
        Binding("n,escape", "cancel", gettext("Cancel")),
    ]

    def __init__(self, name: str) -> None:
        super().__init__()
        self._name = name

    @override
    def compose(self) -> ComposeResult:
        yield Vertical(
            Label(gettext('Remove "%(name)s"?') % {"name": self._name}),
            Static(gettext("[b]y[/b] confirm  /  [b]n[/b] cancel"), markup=True),
            id="confirm-box",
        )

    def action_confirm(self) -> None:
        self.dismiss(True)

    def action_cancel(self) -> None:
        self.dismiss(False)


@dataclass
class _EditRow:
    """One row in the edit screen (a managed definition or an unmanaged candidate).

    The ``orig_*`` fields capture the state when the screen opened so that save
    can compute the minimal set of edit_specs operations and avoid spurious
    not-managed warnings.
    """

    name: str
    kind: str
    type: str
    managed: bool
    secret: bool
    prompt: str
    orig_managed: bool
    orig_secret: bool
    orig_prompt: str


class EditParams(ModalScreen[str | None]):
    """Parameter definition editor (TUI counterpart of ``skit edit``).

    Presentation and state collection only; all rules go through the
    reconcile.edit_specs pure function (the same path as the CLI). On save,
    the full operation set is applied in one shot and written back to the
    copy's [tool.skit]. dismiss value: result message (success) or None
    (cancelled).
    """

    BINDINGS = [
        Binding("space", "toggle_managed", gettext("Manage")),
        Binding("s", "toggle_secret", gettext("Secret")),
        Binding("p", "edit_prompt", gettext("Prompt")),
        Binding("r", "toggle_resync", gettext("Resync")),
        Binding("ctrl+s", "save", gettext("Save")),
        Binding("escape", "cancel", gettext("Cancel")),
    ]

    def __init__(self, entry: Entry) -> None:
        super().__init__()
        self._entry = entry
        self._text = (entry.dir / "script.py").read_text(encoding="utf-8", errors="replace")
        self._original = metawriter.read_params(self._text)
        self._rows: list[_EditRow] = []
        self._resync = False

    @override
    def compose(self) -> ComposeResult:
        table = DataTable(cursor_type="row", zebra_stripes=True)
        yield Vertical(
            Label(gettext("Parameters for %(name)s") % {"name": self._entry.meta.name}),
            table,
            Static(gettext("resync: off"), id="edit-resync"),
            Input(id="prompt-input"),
            Static(
                gettext(
                    "[b]space[/b] manage  [b]s[/b] secret  [b]p[/b] prompt  [b]r[/b] resync  [b]ctrl+s[/b] save  [b]esc[/b] cancel"
                ),
                markup=True,
                id="edit-hint",
            ),
            id="edit-box",
        )

    def on_mount(self) -> None:
        # Managed definitions come first (original order), followed by
        # unmanaged candidates (reconcile's "new" set).
        report = reconcile.reconcile(self._text, self._original)
        for s in self._original:
            self._rows.append(
                _EditRow(
                    name=s.name,
                    kind=s.kind,
                    type=s.type,
                    managed=True,
                    secret=s.secret,
                    prompt=s.prompt or "",
                    orig_managed=True,
                    orig_secret=s.secret,
                    orig_prompt=s.prompt or "",
                )
            )
        for c in report.new:
            self._rows.append(
                _EditRow(
                    name=c.name,
                    kind=c.kind,
                    type=c.type,
                    managed=False,
                    secret=c.secret,
                    prompt=c.prompt or "",
                    orig_managed=False,
                    orig_secret=c.secret,
                    orig_prompt=c.prompt or "",
                )
            )
        table = self.query_one(DataTable)
        table.add_columns(
            gettext("Parameter"),
            gettext("Kind"),
            gettext("Type"),
            gettext("Managed"),
            gettext("Secret"),
            gettext("Prompt"),
        )
        self._refresh_table()
        if not self._rows:
            self.query_one("#edit-resync", Static).update(
                gettext("No parameter candidates were detected in this script.")
            )
        table.focus()

    def _refresh_table(self) -> None:
        table = self.query_one(DataTable)
        cursor = table.cursor_row
        table.clear()
        for r in self._rows:
            table.add_row(
                r.name,
                r.kind,
                r.type,
                gettext("yes") if r.managed else "—",
                gettext("yes") if r.secret else "—",
                r.prompt or "—",
                key=r.name,
            )
        if self._rows and cursor is not None:
            table.move_cursor(row=min(cursor, len(self._rows) - 1))

    def _selected_row(self) -> _EditRow | None:
        table = self.query_one(DataTable)
        if not self._rows or table.cursor_row is None:
            return None
        if 0 <= table.cursor_row < len(self._rows):
            return self._rows[table.cursor_row]
        return None

    def action_toggle_managed(self) -> None:
        row = self._selected_row()
        if row is None:
            return
        row.managed = not row.managed
        self._refresh_table()

    def action_toggle_secret(self) -> None:
        row = self._selected_row()
        if row is None or not row.managed:
            return
        row.secret = not row.secret
        self._refresh_table()

    def action_edit_prompt(self) -> None:
        row = self._selected_row()
        if row is None or not row.managed:
            return
        box = self.query_one("#prompt-input", Input)
        box.placeholder = gettext("Form prompt for %(name)s (Enter to apply, Esc to cancel)") % {
            "name": row.name
        }
        box.value = row.prompt
        box.display = True
        box.focus()

    def submit_prompt_if_open(self) -> None:
        """Apply the value from the prompt input box.

        Enter is captured by MenuApp's priority binding and forwarded here
        because Input.Submitted doesn't fire for Enter in that context.
        """
        box = self.query_one("#prompt-input", Input)
        if not box.display:
            return
        row = self._selected_row()
        if row is not None and row.managed:
            row.prompt = box.value
        self._hide_prompt_input()
        self._refresh_table()

    def _hide_prompt_input(self) -> None:
        box = self.query_one("#prompt-input", Input)
        box.display = False
        self.query_one(DataTable).focus()

    def action_toggle_resync(self) -> None:
        self._resync = not self._resync
        self.query_one("#edit-resync", Static).update(
            gettext(
                "resync: on — definitions that no longer match the script will be pruned when you save"
            )
            if self._resync
            else gettext("resync: off")
        )

    def action_save(self) -> None:
        # Only send the delta relative to when the screen opened to minimise
        # warning noise; all rules are delegated to edit_specs.
        add = [r.name for r in self._rows if r.managed and not r.orig_managed]
        remove = [r.name for r in self._rows if r.orig_managed and not r.managed]
        secret = [r.name for r in self._rows if r.managed and r.secret and not r.orig_secret]
        no_secret = [r.name for r in self._rows if r.managed and not r.secret and r.orig_secret]
        prompts = {r.name: r.prompt for r in self._rows if r.managed and r.prompt != r.orig_prompt}
        result = reconcile.edit_specs(
            self._text,
            self._original,
            resync=self._resync,
            add=add,
            remove=remove,
            secret=secret,
            no_secret=no_secret,
            prompts=prompts,
        )
        copy_path = self._entry.dir / "script.py"
        copy_path.write_text(metawriter.write_params(self._text, result.specs), encoding="utf-8")
        remaining = ", ".join(s.name for s in result.specs) or "—"
        message = gettext("Updated %(name)s. Managed: %(names)s") % {
            "name": self._entry.meta.name,
            "names": remaining,
        }
        for w in result.warnings:
            message += "  " + reconcile.render_warning(w)
        self.dismiss(message)

    def action_cancel(self) -> None:
        # When the prompt input box is open, Esc only closes it, not the whole screen.
        box = self.query_one("#prompt-input", Input)
        if box.display:
            self._hide_prompt_input()
            return
        self.dismiss(None)


class MenuApp(App[int]):
    """Main menu. Exit code: 0 for a clean quit; shows the script's exit code after a run."""

    TITLE = "skit"
    CSS = """
    #search { dock: top; }
    #status { dock: bottom; height: 1; color: $text-muted; padding: 0 1; }
    DataTable { height: 1fr; }
    #confirm-box {
        align: center middle;
        background: $surface;
        border: round $primary;
        padding: 1 2;
        width: auto;
        height: auto;
    }
    ConfirmRemove { align: center middle; }
    EditParams { align: center middle; }
    #edit-box {
        background: $surface;
        border: round $primary;
        padding: 1 2;
        width: 90%;
        height: 80%;
    }
    #edit-box DataTable { height: 1fr; }
    #edit-resync { height: 1; color: $text-muted; }
    #prompt-input { display: none; }
    #edit-hint { height: 1; color: $text-muted; }
    """
    BINDINGS = [
        # Double Ctrl+C is the standard quit gesture (like many REPLs). priority: it must fire
        # even while the search Input has focus, and it shadows Textual's built-in ctrl+c
        # system binding (help_quit), which would otherwise just point at ctrl+q.
        Binding("ctrl+c", "ctrl_c_quit", gettext("Quit"), priority=True),
        Binding("escape", "quit", gettext("Quit"), show=False),
        Binding("delete", "remove", gettext("Remove")),
        # priority: the search Input has a built-in Emacs ctrl+e (move to end of line); we
        # must claim it first so our "edit" action wins.
        Binding("ctrl+e", "edit", gettext("Params"), priority=True),
        Binding("enter", "run", gettext("Run"), priority=True),
    ]

    # Seconds within which a second Ctrl+C press counts as "quit".
    CTRL_C_WINDOW = 2.0

    def __init__(self) -> None:
        super().__init__()
        self._entries: list[Entry] = []
        self._visible: list[Entry] = []
        self._ctrl_c_at: float = 0.0

    def action_ctrl_c_quit(self) -> None:
        """First Ctrl+C shows a hint; a second press within the window quits."""
        now = time.monotonic()
        if now - self._ctrl_c_at <= self.CTRL_C_WINDOW:
            self.exit(0)
            return
        self._ctrl_c_at = now
        # notify() renders above modal screens too, unlike the docked #status bar.
        self.notify(gettext("Press Ctrl+C again to quit"), timeout=self.CTRL_C_WINDOW)

    @override
    def compose(self) -> ComposeResult:
        yield Input(
            placeholder=gettext(
                "Type to search… (Enter to run, Ctrl+C twice to quit, Del to remove)"
            ),
            id="search",
        )
        table = DataTable(cursor_type="row", zebra_stripes=True)
        yield table
        yield Static("", id="status")
        yield Footer()

    def on_mount(self) -> None:
        table = self.query_one(DataTable)
        table.add_columns(gettext("Name"), gettext("Kind"), gettext("Description"))
        self._reload()
        self.query_one("#search", Input).focus()

    def _reload(self) -> None:
        self._entries = store.list_entries()
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
            table.add_row(e.meta.name, e.meta.kind, e.meta.description or "—", key=e.slug)
        status = self.query_one("#status", Static)
        if not self._entries:
            status.update(gettext("No scripts yet. Quit and add one with: skit add <path>"))
        else:
            status.update(
                ngettext(
                    "%(shown)s/%(total)s script", "%(shown)s/%(total)s scripts", len(self._entries)
                )
                % {"shown": len(self._visible), "total": len(self._entries)}
            )

    def _selected(self) -> Entry | None:
        table = self.query_one(DataTable)
        if not self._visible or table.cursor_row is None:
            return None
        if 0 <= table.cursor_row < len(self._visible):
            return self._visible[table.cursor_row]
        return None

    @on(Input.Changed, "#search")
    def _on_search(self, event: Input.Changed) -> None:
        self._apply_filter(event.value)

    @on(DataTable.RowSelected)
    def _on_row_selected(self, event: DataTable.RowSelected) -> None:
        self.action_run()

    def on_key(self, event: events.Key) -> None:
        # When the search box has focus, forward Up/Down to the table.
        if event.key in ("up", "down") and self.focused is self.query_one("#search", Input):
            table = self.query_one(DataTable)
            table.action_cursor_up() if event.key == "up" else table.action_cursor_down()
            event.stop()

    def action_run(self) -> None:
        # Enter is a priority binding; when any modal is open it fires here first.
        # EditParams needs Enter for its prompt input box — forward it there.
        # For any other modal, ignore it entirely.
        if len(self.screen_stack) > 1:
            screen = self.screen
            if isinstance(screen, EditParams):
                screen.submit_prompt_if_open()
            return
        entry = self._selected()
        if entry is None:
            return
        with self.suspend():
            print(
                f"\n{gettext('── Running %(name)s ──') % {'name': entry.meta.name}}\n", flush=True
            )
            injected_path = None
            try:
                values: dict[str, str] = {}
                secret_names: set[str] = set()
                specs, text = self._load_specs(entry)
                if specs:
                    injected_path, secret_names = self._collect_python_params(
                        entry, specs, text, values
                    )
                elif entry.meta.params:
                    _collect_command_params(entry, values)
                code = launcher.run_entry(entry, values=values, script_override=injected_path)
                if values:
                    argstate.save_last(entry.slug, values=values, secret_names=secret_names)
            except launcher.LaunchError as exc:
                print(gettext("Error: %(error)s") % {"error": str(exc)})
                code = 1
            finally:
                if injected_path is not None and injected_path.exists():
                    injected_path.unlink(missing_ok=True)
            print(
                f"\n{gettext('── Finished (exit code %(code)s) — press Enter to return ──') % {'code': code}}",
                flush=True,
            )
            with contextlib.suppress(EOFError):
                input()
        self._reload()

    def _load_specs(self, entry: Entry) -> tuple[list[ParamSpec], str]:
        """Read the script and reconcile (same as CLI): drop missing, warn on changed."""
        specs: list[ParamSpec] = []
        text = ""
        if entry.meta.kind == "python" and entry.script_path.exists():
            text = entry.script_path.read_text(encoding="utf-8", errors="replace")
            specs = metawriter.read_params(text)
        if specs:
            report = reconcile.reconcile(text, specs)
            if report.has_drift:
                for line in reconcile.drift_lines(report, entry.meta.name):
                    print(line, flush=True)
            specs = report.usable
        return specs, text

    def _collect_python_params(
        self,
        entry: Entry,
        specs: list[ParamSpec],
        text: str,
        values: dict[str, str],
    ) -> tuple[Path, set[str]]:
        """Layer 2 form (plain input/getpass while suspended; secrets are not echoed or saved)."""
        import getpass

        secret_names = {s.name for s in specs if s.secret}
        prefill = argstate.resolve_defaults(specs, entry.slug)
        print(
            gettext("Parameters for %(name)s (press Enter to keep the value shown):")
            % {"name": entry.meta.name},
            flush=True,
        )
        for s in specs:
            label = s.prompt or s.name
            default = prefill.get(s.name, "")
            try:
                if s.secret:
                    answer = getpass.getpass(f"  {label}: ")
                else:
                    hint = f" [{default}]" if default else ""
                    answer = input(f"  {label}{hint}: ").strip()
            except EOFError:
                answer = ""
            values[s.name] = answer or default
        try:
            injected = shim.inject(text, specs, values)
        except shim.ShimError as exc:
            raise launcher.LaunchError(
                gettext(
                    "Can't inject parameters into %(name)s: targets not found (%(detail)s). The script may have drifted from its [tool.skit] definitions — re-add it or edit the block."
                )
                % {"name": entry.meta.name, "detail": str(exc)}
            ) from exc
        return shim.write_injected(entry.dir, injected), secret_names

    def action_edit(self) -> None:
        # ctrl+e is also a priority binding; don't open a second modal if one is already up.
        if len(self.screen_stack) > 1:
            return
        entry = self._selected()
        if entry is None:
            return
        status = self.query_one("#status", Static)
        copy_path = entry.dir / "script.py"
        # Same guards as CLI `skit edit`: only python + copy-mode copies are editable (A7).
        if entry.meta.kind != "python" or entry.meta.mode == "reference" or not copy_path.exists():
            status.update(
                gettext(
                    "%(name)s: only Python copy-mode entries have editable parameter definitions."
                )
                % {"name": entry.meta.name}
            )
            return

        def _done(message: str | None) -> None:
            if message:
                status.update(message)

        self.push_screen(EditParams(entry), _done)

    def action_remove(self) -> None:
        entry = self._selected()
        if entry is None:
            return

        def _done(confirmed: bool | None) -> None:
            if confirmed:
                store.remove(entry.slug)
                self._reload()

        self.push_screen(ConfirmRemove(entry.meta.name), _done)


def run_menu() -> int:
    app = MenuApp()
    result = app.run()
    return result if isinstance(result, int) else 0
