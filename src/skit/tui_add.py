"""Add flow in the TUI: source step → single review panel.

The review panel is one always-editable surface — no wizard sequence to march through;
Enter accepts everything as reviewed. Detection honesty rules render here: signal-
driven checkbox defaults, the accumulator warning, filename-literal hints, and the
"the script declares its own dependencies" read-only variant.

Every review panel has two faces: pushed from the Library (`a`), and hosted alone by
a _ReviewHost app when a terminal `skit add` runs interactively — same screen, so the
CLI and the TUI can never drift apart. Python gets AddReviewScreen; prompts get its
twin PromptReviewScreen (insertion switch, placeholder ticks, runner pick).
"""

from __future__ import annotations

from pathlib import Path
from typing import override

from rich.markup import escape
from textual import on
from textual.app import App, ComposeResult
from textual.binding import Binding
from textual.containers import Vertical
from textual.screen import ModalScreen, Screen
from textual.widgets import Checkbox, Input, OptionList, RadioButton, RadioSet, Select, Static
from textual.widgets.option_list import Option

from . import (
    analysis,
    argstate,
    config,
    editor,
    pep723,
    store,
    theme,
    tui_footer,
    tui_layout,
    tui_runner,
)
from .i18n import gettext, ngettext
from .langs.prompt import analyzer as prompt_analyzer
from .params import ParamDecl, is_secret_name
from .paths import is_draft

# How many kept drafts the add screen lists at once; anything past the cap is counted
# out loud by the overflow line, never silently hidden.
_DRAFTS_LISTED = 20


class KindPickModal(ModalScreen[str | None]):
    """The TUI twin of --kind/--exe/--prompt: an unclassifiable file gets an ASK, not
    an error message that teaches CLI flags to a user who is here to avoid them."""

    AUTO_FOCUS = "OptionList"
    BINDINGS = [Binding("escape", "cancel", gettext("Cancel"))]
    DEFAULT_CSS = """
    KindPickModal { align: center middle; }
    KindPickModal > Vertical { border: round $accent; padding: 1 2; width: 56;
        max-width: 100%; height: auto; max-height: 100%; background: $background; }
    KindPickModal OptionList { height: auto; max-height: 10; border: none; }
    /* Short terminals: cap the list and flatten the chrome so the Esc chip — the
       modal's mouse path out — stays on screen across the whole tier band; the
       OptionList scrolls internally (the TokenMenuModal discipline). */
    KindPickModal.-h-short > Vertical, KindPickModal.-h-tiny > Vertical { padding: 0 2; }
    KindPickModal.-h-short OptionList { max-height: 4; }
    KindPickModal.-h-tiny OptionList { max-height: 2; }
    KindPickModal Static { width: auto; margin: 1 0 0 0; }
    KindPickModal.-h-short Static, KindPickModal.-h-tiny Static { margin: 0; }
    """

    def __init__(self, filename: str, *, has_shebang: bool = False, offer_exe: bool = True) -> None:
        super().__init__()
        self._filename: str = filename
        # Two different truths need two different labels: with a #! present, "can't
        # tell from the name" is false — the name (and the shebang) told skit plenty;
        # what it can't do is honor an interpreter it doesn't know.
        self._has_shebang: bool = has_shebang
        # The draft lanes pass offer_exe=False: a just-authored text file is never a
        # binary, and an exe entry is reference-by-construction — the one mode the
        # drafts boundary forbids (the store would hold nothing while the success
        # path deletes, or the resumable list advertises, the only copy).
        self._offer_exe: bool = offer_exe

    @override
    def compose(self) -> ComposeResult:
        from textual.widgets import Label
        from textual.widgets.option_list import Option

        from .langs.registry import KNOWN_KINDS, spec_for

        # "prompt" is family "interpreted" too, but it has its OWN dedicated option below
        # ("A prompt for an AI agent") — listing it here as well would duplicate the id
        # (OptionList raises DuplicateID) and offer the same kind twice.
        interpreted = sorted(
            k
            for k in KNOWN_KINDS
            if (spec := spec_for(k)) is not None and spec.family == "interpreted" and k != "prompt"
        )
        with Vertical():
            yield Label(
                (
                    gettext("The #! in %(file)s names no interpreter skit knows. What is it?")
                    if self._has_shebang
                    else gettext("What is %(file)s? skit can't tell from the name.")
                )
                % {"file": self._filename}
            )
            from .kindnames import kind_label

            options = [Option(kind_label(kind), id=kind) for kind in interpreted]
            if self._offer_exe:
                options.append(Option(gettext("A program (run it directly)"), id="exe"))
            options.append(Option(gettext("A prompt for an AI agent"), id="prompt"))
            yield OptionList(*options)
            yield Static(
                tui_footer.bar(tui_footer.chip("screen.cancel", "Esc", gettext("Cancel"))),
                markup=True,
            )

    @on(OptionList.OptionSelected)
    def _picked(self, event: OptionList.OptionSelected) -> None:
        self.dismiss(str(event.option.id))

    def action_cancel(self) -> None:
        self.dismiss(None)


class DraftDeleteConfirm(ModalScreen[bool]):
    """Deleting a kept draft destroys the user's ONLY copy of what they wrote — it
    gets the same ask entry removal gets, never a bare one-keystroke delete."""

    BINDINGS = [
        Binding("y", "confirm", gettext("Delete")),
        Binding("escape,n", "cancel", gettext("Keep")),
    ]
    DEFAULT_CSS = """
    DraftDeleteConfirm { align: center middle; }
    DraftDeleteConfirm > Vertical { border: round $accent; padding: 1 2; width: auto;
        max-width: 100%; height: auto; max-height: 100%; background: $background; }
    DraftDeleteConfirm Static { width: auto; margin: 1 0 0 0; }
    """

    def __init__(self, name: str) -> None:
        super().__init__()
        self._name: str = name

    @override
    def compose(self) -> ComposeResult:
        from textual.widgets import Label

        with Vertical():
            yield Label(
                gettext('Delete the draft "%(name)s"? It is the only copy.') % {"name": self._name}
            )
            yield Static(
                tui_footer.bar(
                    tui_footer.chip("screen.confirm", "y", gettext("Delete")),
                    tui_footer.chip("screen.cancel", "Esc", gettext("Keep")),
                ),
                markup=True,
            )

    def action_confirm(self) -> None:
        self.dismiss(True)

    def action_cancel(self) -> None:
        self.dismiss(False)


class AddSourceScreen(Screen[str | None]):
    """Step 1: where does the script come from? Returns the new entry's slug, or None."""

    BINDINGS = [
        Binding("escape", "cancel", gettext("Cancel")),
        # Ctrl+N, not Ctrl+E: everywhere else in the product Ctrl+E means "open $EDITOR
        # on the CURRENT subject" (review panels, the Library) — one chord, one verb.
        Binding("ctrl+n", "draft_script", gettext("Write a new script"), priority=True),
        Binding("ctrl+p", "draft_prompt", gettext("Draft a prompt"), priority=True),
        # NOT priority: Ctrl+D is the Input's own delete-right while a field has focus
        # (the AGENTS grammar rule for editing chords) — the chip is the path mid-edit.
        Binding("ctrl+d", "delete_draft", gettext("Delete a kept draft")),
        *tui_footer.FIELD_NAV_BINDINGS,
    ]
    # Boot on the path field, not the "*" pick (the body scroll container).
    AUTO_FOCUS = "Input"
    DEFAULT_CSS = """
    /* The border lives on the body, not the Screen: a bordered Screen offsets its
       coordinate space and bottom-docked footer clicks land "outside" it. */
    AddSourceScreen #add-body {
        padding: 1;
        border: round $skit-box-olive;
        border-title-color: ansi_bright_white;
        border-title-style: bold;
    }
    AddSourceScreen .hint { color: $text-muted; }
    AddSourceScreen #add-drafts { height: auto; max-height: 4; border: none; }
    /* Chips wrap pill-by-pill; visible lines follow the height tier and anything
       past the cap stays wheel-reachable — see tui_footer.KeysBar. */
    AddSourceScreen KeysBar { dock: bottom; }
    """

    def on_mount(self) -> None:
        self.query_one("#add-body").border_title = gettext("Add a script")

    @override
    def compose(self) -> ComposeResult:
        # FormBody, not a plain Vertical: on a short terminal a fixed body puts the
        # template/name fields under the docked footer with no way to reveal them —
        # the scroll body keeps every field reachable (focus pulls it into view).
        with tui_footer.FormBody(id="add-body"):
            yield Static(gettext("Path to a script, executable, or prompt:"))
            yield Input(placeholder="~/scripts/tool.py", id="add-path")
            yield Static("", id="add-error", markup=True)
            yield Static(
                gettext("…or register a command template below (e.g. ffmpeg -i {input}):"),
                classes="hint",
            )
            yield Input(placeholder="ffmpeg -i {input} {output}", id="add-template")
            yield Input(placeholder=gettext("Name for the command"), id="add-template-name")
            from .paths import drafts_dir

            def _mtime(d: Path) -> float:
                try:
                    return d.stat().st_mtime
                except OSError:
                    return 0.0

            # Newest first: mkstemp names are random, so a name sort hides an
            # ARBITRARY tail — the draft the user just lost is exactly the one that
            # must surface. The cap keeps the screen usable; the overflow line keeps
            # it honest (a silent cap reads as "this is everything").
            drafts = (
                sorted(drafts_dir().glob("skit-*"), key=_mtime, reverse=True)
                if drafts_dir().is_dir()
                else []
            )
            if drafts:
                # Kept drafts are resumable, not lore: list them where adding happens —
                # and deletable here too (an accumulation the user "can see and manage"
                # with no way to manage it would be half a promise).
                yield Static(gettext("…or resume a kept draft:"), classes="hint")
                yield OptionList(
                    *(Option(escape(d.name), id=str(d)) for d in drafts[:_DRAFTS_LISTED]),
                    id="add-drafts",
                )
                if len(drafts) > _DRAFTS_LISTED:
                    yield Static(
                        "  "
                        + gettext("…and %(count)s more") % {"count": len(drafts) - _DRAFTS_LISTED},
                        classes="hint",
                    )
                yield Static(
                    tui_footer.bar(
                        tui_footer.chip("screen.delete_draft", "Ctrl+D", gettext("Delete draft…"))
                    ),
                    id="add-draft-actions",
                    markup=True,
                )
            yield Static(gettext("…or start from a blank page:"), classes="hint")
            # The authoring lanes were CLI-only (skit add --edit / --prompt) — a
            # TUI-first user could never discover them (zero-memorization).
            yield Static(
                tui_footer.bar(
                    tui_footer.chip("screen.draft_script", "Ctrl+N", gettext("Write a script…")),
                    tui_footer.chip("screen.draft_prompt", "Ctrl+P", gettext("Draft a prompt…")),
                ),
                id="add-draft",
                markup=True,
            )
        yield tui_footer.KeysBar(
            Static(
                tui_footer.bar(
                    tui_footer.chip("screen.continue_add", "Enter", gettext("Continue")),
                    tui_footer.chip("screen.cancel", "Esc", gettext("Cancel")),
                    tui_footer.nav_chip(),
                ),
                id="add-keys",
                markup=True,
            )
        )

    @on(Input.Submitted, "#add-path")
    def _path_given(self, event: Input.Submitted) -> None:
        self._submit_path()

    @on(OptionList.OptionSelected, "#add-drafts")
    def _draft_resumed(self, event: OptionList.OptionSelected) -> None:
        """Picking a kept draft routes it through the same path lane as any file."""
        self.query_one("#add-path", Input).value = str(event.option.id)
        self._submit_path()

    def _submit_path(self) -> None:
        """Every scripty kind gets the SAME review panel (identity, storage, deps,
        candidate ticks per its own analyzer) — the add flow must not be four
        different products depending on file extension. Only exe (nothing to detect
        inside a binary) takes the direct lane; unknown files get the honest error."""
        from .langs.registry import spec_for
        from .store import infer_kind

        raw = self.query_one("#add-path", Input).value.strip()
        if not raw:
            return
        path = Path(raw).expanduser()
        error = self.query_one("#add-error", Static)
        if not path.is_file():
            error.update(
                f"[red]{gettext('File not found: %(path)s') % {'path': escape(str(path))}}[/red]"
            )
            return
        kind = infer_kind(path)
        if kind == "unknown" and path.suffix.lower() == ".md":
            # The TUI twin of the CLI's bare-.md ask: the user explicitly picked this
            # file, and a .md that is neither script nor executable is a prompt in all
            # but name — say so instead of dead-ending (mirrors issue #10's direction).
            kind = "prompt"

        # A kept draft is skit's own artifact destined for consumption: resuming it is
        # fresh authoring (fresh=True — no "Link the original" ask). A reference entry
        # pointing into drafts/ would leave its script in the resumable list — a live
        # entry's file offered for re-adding and for "Delete draft… (it is the only
        # copy)", both lies.
        draft_resume = is_draft(path)

        def _reviewed(slug: str | None) -> None:
            if slug is None:
                return
            if draft_resume and store.resolve(slug).meta.mode == "copy":
                # A resumed draft that reached the store is done accumulating — the
                # same "success: the store holds the copy" unlink every authoring
                # lane performs. (fresh always copies; the mode check is the belt.)
                path.unlink(missing_ok=True)
            self.dismiss(slug)

        if kind == "exe" and draft_resume:
            # A draft is authored text, never a binary — and an exe entry is
            # reference-by-construction, the one mode the drafts boundary forbids
            # (a hand-planted executable bit must not smuggle one past the rule).
            # Fall to the ask; the modal below won't offer "A program" for a draft.
            kind = "unknown"
        if kind == "prompt":
            self.app.push_screen(PromptReviewScreen(path, fresh=draft_resume), _reviewed)
            return
        kind_spec = spec_for(kind)
        if kind_spec is not None and kind_spec.family == "interpreted":
            self.app.push_screen(AddReviewScreen(path, kind=kind, fresh=draft_resume), _reviewed)
            return
        if kind == "exe":
            self.app.push_screen(ExeReviewScreen(path), _reviewed)
            return

        # Unclassifiable: ASK, don't teach CLI flags (the TUI twin of --kind/--exe).
        def _kind_picked(picked: str | None) -> None:
            if picked is None:
                return
            if picked == "prompt":
                self.app.push_screen(PromptReviewScreen(path, fresh=draft_resume), _reviewed)
            elif picked == "exe":
                self.app.push_screen(ExeReviewScreen(path), _reviewed)
            else:
                self.app.push_screen(
                    AddReviewScreen(path, kind=picked, fresh=draft_resume), _reviewed
                )

        from .langs.registry import shebang_program

        self.app.push_screen(
            KindPickModal(
                path.name,
                has_shebang=shebang_program(path) is not None,
                offer_exe=not draft_resume,
            ),
            _kind_picked,
        )

    @on(Input.Submitted, "#add-template")
    @on(Input.Submitted, "#add-template-name")
    def _template_given(self, event: Input.Submitted) -> None:
        self._submit_template()

    def _submit_template(self) -> None:
        template = self.query_one("#add-template", Input).value.strip()
        name = self.query_one("#add-template-name", Input).value.strip()
        error = self.query_one("#add-error", Static)
        if not template:
            return
        if not name:
            error.update(f"[red]{gettext('A name is required.')}[/red]")
            return
        try:
            entry = store.add_command(template, name=name)
        except store.StoreError as exc:
            error.update(f"[red]{escape(str(exc))}[/red]")
            return
        self.dismiss(entry.slug)

    def action_continue_add(self) -> None:
        """Footer/Enter twin: submit whichever field the user filled — the script path
        takes precedence, else the command template."""
        if self.query_one("#add-path", Input).value.strip():
            self._submit_path()
        else:
            self._submit_template()

    def action_draft_script(self) -> None:
        """Ctrl+N / the Write a script… chip: author a brand-new script in $EDITOR,
        then review it in the same panel a path-add gets. The starter is python, but a
        CHANGED SHEBANG is an explicit signal and is honored — the chip says "script",
        not "python script", and a bash draft must never be stored as a broken python
        entry."""
        from .cli import _STARTER_SCRIPT  # lazy: cli imports this module lazily too

        self._draft(".py", _STARTER_SCRIPT, "python")

    def action_draft_prompt(self) -> None:
        """Ctrl+P / the Draft a prompt… chip: the prompt twin."""
        from .cli import _starter_prompt

        self._draft(".prompt.md", _starter_prompt(), "prompt")

    def action_delete_draft(self) -> None:
        """Ctrl+D / the Delete draft… chip: delete the highlighted kept draft — behind
        the same ask entry removal gets, because the draft is the user's only copy."""
        lists = self.query("#add-drafts")
        if not lists:
            return
        option_list = lists.first(OptionList)
        if option_list.highlighted is None:
            return
        draft = Path(str(option_list.get_option_at_index(option_list.highlighted).id))

        def _confirmed(delete: bool | None) -> None:
            if not delete:
                return
            draft.unlink(missing_ok=True)
            self.notify(gettext("Deleted the draft %(name)s.") % {"name": draft.name})
            self.refresh(recompose=True)

        self.app.push_screen(DraftDeleteConfirm(draft.name), _confirmed)

    def _draft(self, suffix: str, starter: str, kind: str) -> None:
        import os
        import tempfile

        from .cli import _drafts_home

        fd, tmp_name = tempfile.mkstemp(  # pragma: no mutate
            suffix=suffix, prefix="skit-new-", dir=_drafts_home()
        )
        os.close(fd)
        tmp = Path(tmp_name)
        tmp.write_text(starter, encoding="utf-8")  # pragma: no mutate
        with self.app.suspend():
            try:
                editor.open_in_editor(tmp)
            except editor.EditorError as exc:
                print(str(exc), flush=True)
        text = tmp.read_text(encoding="utf-8", errors="replace")
        if text.strip() in ("", starter.strip()):
            tmp.unlink(missing_ok=True)  # pragma: no mutate
            self.notify(gettext("Nothing was written, so no script was added."))
            return

        def _reviewed(slug: str | None) -> None:
            if slug is None:
                # The draft is the user's only copy — a cancelled review must never
                # delete it silently. Keep it and say where it lives.
                self.notify(gettext("Your draft was kept at %(path)s") % {"path": str(tmp)})
                return
            if store.resolve(slug).meta.mode == "copy":
                # Success unlink, MODE-GATED: the store holds the copy — and only a
                # copy. No lane may ever delete what the store doesn't hold, so a
                # non-copy entry (however it got here) keeps the file.
                tmp.unlink(missing_ok=True)  # pragma: no mutate
            self.dismiss(slug)

        if kind == "python":
            from .langs.registry import kind_for_shebang, shebang_program

            mapped = kind_for_shebang(tmp)
            program = shebang_program(tmp)
            if mapped is not None:
                kind = mapped
            elif program is not None:
                # An UNREGISTERED shebang is an explicit signal skit can't honor:
                # ASK (the same modal an unclassifiable file gets), never fabricate
                # a python entry that can only die in uv run. Versioned pythons are
                # the registry's rule now, not a per-lane carve-out. No "A program"
                # option here (offer_exe=False): a just-authored draft is text, and
                # an exe entry's hardcoded reference mode would leave the store
                # holding nothing.
                def _kind_picked(picked: str | None) -> None:
                    if picked is None:
                        self.notify(gettext("Your draft was kept at %(path)s") % {"path": str(tmp)})
                        return
                    chosen: Screen[str | None] = (
                        PromptReviewScreen(tmp, fresh=True)
                        if picked == "prompt"
                        else AddReviewScreen(tmp, kind=picked, fresh=True)
                    )
                    self.app.push_screen(chosen, _reviewed)

                self.app.push_screen(
                    KindPickModal(tmp.name, has_shebang=True, offer_exe=False), _kind_picked
                )
                return
        review: Screen[str | None] = (
            PromptReviewScreen(tmp, fresh=True)
            if kind == "prompt"
            else AddReviewScreen(tmp, kind=kind, fresh=True)
        )
        self.app.push_screen(review, _reviewed)

    def action_cancel(self) -> None:
        self.dismiss(None)


class ExeReviewScreen(Screen[str | None]):
    """Identity review for a program add: name + description. "Nothing to detect
    inside a binary" justifies no tick list — not skipping identity while every other
    kind reviews it."""

    BINDINGS = [
        Binding("escape", "cancel", gettext("Cancel")),
        Binding("ctrl+s", "accept", gettext("Add"), priority=True),
        *tui_footer.FIELD_NAV_BINDINGS,
    ]
    AUTO_FOCUS = "Input"
    DEFAULT_CSS = """
    ExeReviewScreen #xv-body {
        padding: 0 1;
        border: round $skit-box-olive;
        border-title-color: ansi_bright_white;
        border-title-style: bold;
    }
    ExeReviewScreen .section { color: $accent; margin: 1 0 0 0; }
    ExeReviewScreen .hint { color: $text-muted; }
    ExeReviewScreen KeysBar { dock: bottom; }
    ExeReviewScreen #xv-keys { color: $text-muted; }
    """

    def __init__(self, path: Path) -> None:
        super().__init__()
        self._path: Path = path

    def on_mount(self) -> None:
        self.query_one("#xv-body").border_title = gettext("Add %(name)s") % {
            "name": escape(self._path.name)
        }

    @override
    def compose(self) -> ComposeResult:
        with tui_footer.FormBody(id="xv-body"):
            yield Static(gettext("Name"), classes="section")
            yield Input(value=self._path.stem, id="xv-name")
            yield Static(gettext("Description"), classes="section")
            yield Input(
                placeholder=gettext("(shown in the Library — you can write one line)"),
                id="xv-desc",
            )
            yield Static(
                gettext("The program runs from its own location; skit never copies a binary."),
                classes="hint",
            )
        yield tui_footer.KeysBar(
            Static(
                tui_footer.bar(
                    tui_footer.chip("screen.accept", "Ctrl+S", gettext("Add")),
                    tui_footer.chip("screen.cancel", "Esc", gettext("Cancel")),
                    tui_footer.nav_chip(),
                ),
                id="xv-keys",
                markup=True,
            )
        )

    def action_accept(self) -> None:
        name = self.query_one("#xv-name", Input).value.strip() or None
        desc = self.query_one("#xv-desc", Input).value.strip()
        try:
            entry = store.add_exe(self._path, name=name, description=desc)
        except store.StoreError as exc:
            self.notify(str(exc), severity="error")
            return
        self.dismiss(entry.slug)

    def action_cancel(self) -> None:
        self.dismiss(None)


class AddReviewScreen(Screen[str | None]):
    """Step 2: the review panel — everything prefilled, Enter is the only required act."""

    BINDINGS = [
        Binding("escape", "cancel", gettext("Cancel")),
        # Non-priority: Ctrl+E is every Input's end-of-line (the Ctrl+A rule, one
        # chord left) — the chip is the path while typing; the chord fires elsewhere.
        Binding("ctrl+e", "edit_source", gettext("Edit script")),
        Binding("ctrl+s", "accept", gettext("Add"), priority=True),
        *tui_footer.FIELD_NAV_BINDINGS,
    ]
    # Boot on the name field, not the "*" pick (the body scroll container): the panel
    # should be typeable the moment it opens.
    AUTO_FOCUS = "Input"
    DEFAULT_CSS = """
    AddReviewScreen #review-body {
        padding: 0 1;
        border: round $skit-box-olive;
        border-title-color: ansi_bright_white;
        border-title-style: bold;
    }
    AddReviewScreen .section { color: $accent; margin: 1 0 0 0; }
    AddReviewScreen .hint { color: $text-muted; }
    AddReviewScreen .warn { color: $warning; }
    AddReviewScreen KeysBar { dock: bottom; }
    AddReviewScreen #review-keys { color: $text-muted; }
    """

    def __init__(
        self,
        path: Path,
        *,
        kind: str = "python",
        name: str | None = None,
        description: str | None = None,
        reference: bool = False,
        deps: list[str] | None = None,
        requires_python: str = "",
        fresh: bool = False,
    ) -> None:
        """The keyword arguments prefill the panel (the CLI face passes `skit add`'s
        flags through them); everything stays editable on screen.

        Kind-parametric (the audit's worst finding): the panel was python-only while
        shell/js/ts/fish shipped full analyzers that no add lane ever showed — the same
        verb gave python a review panel and shell a toast. Identity/storage are
        universal; deps render per deps_flavor; candidates come from the entry's OWN
        analyzer capability (None → no tick list, identity still reviewed)."""
        super().__init__()
        self._path: Path = path
        self._kind: str = kind
        # A freshly-drafted temp file has no original to link — the storage section is
        # skipped and accept always copies (the CLI refuses --ref there for the same
        # reason). DERIVED, never only trusted from the caller: a kept draft opened
        # through ANY face (the CLI-hosted panel included) must not offer "Link the
        # original" — the radio was a fourth route to a reference entry pointing
        # into drafts/, committed by the one face that didn't pass the flag.
        self._fresh: bool = fresh or is_draft(path)
        from .langs.registry import spec_for

        self._spec = spec_for(kind)
        self._text: str = path.read_text(encoding="utf-8", errors="replace")
        self._analysis: analysis.Analysis = self._analyze()
        # A versioned shebang (python3.12) is ONE signal: the kind half and the
        # version half. With no explicit --python and no PEP 723 block, the version
        # half becomes the recorded requires-python default — same rule as the CLI's
        # _resolve_python_metadata, and _compose_deps SHOWS it (an invisibly recorded
        # constraint would be a setting no TUI surface ever admits to).
        self._py_pin_auto = kind == "python" and not requires_python
        self._requires_python = requires_python
        if self._py_pin_auto:
            self._requires_python = self._python_pin()
        # Survives the edit→rescan recompose: the rescan refreshes DETECTION, it must
        # never throw away what the user already typed into the panel. Candidate
        # ticks are panel input too — name-keyed, so a candidate that survives the
        # rescan keeps its tick while genuinely new/removed ones take defaults.
        self._overrides: dict[str, str] = {}
        self._tick_overrides: dict[str, bool] = {}
        if name:
            self._overrides["name"] = name
        if description:
            self._overrides["desc"] = description
        if reference:
            self._overrides["mode"] = "1"
        if deps:
            self._overrides["deps"] = ", ".join(deps)

    def _analyze(self) -> analysis.Analysis:
        if self._spec is None or self._spec.analyzer is None:
            return analysis.Analysis()
        return self._spec.analyzer.analyze(self._text)

    def _python_pin(self) -> str:
        """The requires-python default a versioned shebang implies (registry's one
        rule) — "" when a PEP 723 block owns the constraint."""
        if pep723.has_block(self._text):
            return ""
        from .langs.registry import python_version_pin, shebang_program

        return python_version_pin(shebang_program(self._path))

    def _reader_modeled(self) -> bool:
        """Whether the entry's own reader models a form from the current text — the
        shared trap predicate (flows.reader_fields) the tick list and its Space chip
        both key on."""
        from . import flows

        return flows.reader_fields(self._spec, self._text) > 0

    def _suggest_description(self) -> str:
        if self._kind == "python":
            return store.suggest_description(self._text)
        prefix = self._spec.comment.prefix if self._spec and self._spec.comment else "#"
        return store.extract_comment_description(self._text, prefix)

    def on_mount(self) -> None:
        self.query_one("#review-body").border_title = gettext("Add %(name)s") % {
            "name": escape(self._path.name)
        }
        self._apply_mode_visibility()

    @on(RadioSet.Changed, "#rv-mode")
    def _mode_changed(self, event: RadioSet.Changed) -> None:
        self._apply_mode_visibility()

    def _apply_mode_visibility(self) -> None:
        """Reference mode folds away what accept would skip — never a silent drop:
        the CLI refuses --dep on an npm reference add and says parameter setup is
        skipped; the panel must tell the same truth BEFORE Ctrl+S, not swallow ticks."""
        mode_box = self.query("#rv-mode")
        reference = bool(mode_box) and mode_box.first(RadioSet).pressed_index == 1
        spec = self._spec
        npm = spec is not None and spec.deps_flavor == "npm"
        modeled = self._reader_modeled()
        # A modeled reader form works in reference mode too (plan_for_entry has no
        # mode gate), so its ✓ notice STAYS visible — hiding it while saying "setup
        # is skipped" reads as "the form is lost", the exact misreading the CLI's
        # reference voice was rewritten to correct. Only the tick list is skipped.
        self.query_one("#rv-params-wrap").display = not reference or modeled
        if npm:
            self.query_one("#rv-deps-wrap").display = not reference
        note = self.query_one("#rv-ref-note", Static)
        if reference:
            lines = [
                gettext("Link the original: skit never writes to the file.")
                if modeled
                else gettext(
                    "Link the original: parameter setup is skipped — skit never writes to the file."
                )
            ]
            if npm:
                lines.append(
                    gettext("npm dependencies apply to stored copies only, so none are recorded.")
                )
            note.update(" ".join(lines))
        note.display = reference

    @override
    def compose(self) -> ComposeResult:
        with tui_footer.FormBody(id="review-body"):
            yield Static(gettext("Name"), classes="section")
            yield Input(value=self._overrides.get("name", self._path.stem), id="rv-name")
            yield Static(gettext("Description"), classes="section")
            yield Input(
                value=self._overrides.get("desc", self._suggest_description()),
                placeholder=gettext("(the script has no docstring — you can write one line)"),
                id="rv-desc",
            )
            if not self._fresh:
                # A freshly-drafted temp file has no original to link: no storage ask.
                yield Static(gettext("Storage"), classes="section")
                # "1" == the reference button. Default to copy on first compose (no
                # override); after an edit→rescan, restore whichever the user picked.
                reference = self._overrides.get("mode") == "1"
                with RadioSet(id="rv-mode"):
                    yield RadioButton(
                        gettext(
                            "Keep a copy — skit stores it; your original file is never modified"
                        ),
                        value=not reference,
                    )
                    yield RadioButton(
                        gettext(
                            "Link the original — edits take effect immediately, but skit won't "
                            "write to the file, so parameter definitions are yours to maintain"
                        ),
                        value=reference,
                    )
            with Vertical(id="rv-deps-wrap"):
                yield from self._compose_deps()
            with Vertical(id="rv-params-wrap"):
                yield from self._compose_params()
            yield Static("", id="rv-ref-note", classes="hint", markup=False)
        chips = [tui_footer.chip("screen.accept", "Ctrl+S", gettext("Add"))]
        if (
            self._spec is not None
            and self._spec.analyzer is not None
            and self._analysis.candidates
            and not self._reader_modeled()
        ):
            # The same condition that composes the checkboxes: advertising Space with
            # nothing to toggle teaches a dead key.
            chips.append(tui_footer.chip("screen.toggle_candidate", "Space", gettext("Toggle")))
        chips += [
            tui_footer.chip("screen.edit_source", "Ctrl+E", gettext("Edit script")),
            tui_footer.chip("screen.cancel", "Esc", gettext("Cancel")),
            tui_footer.nav_chip(),
        ]
        yield tui_footer.KeysBar(Static(tui_footer.bar(*chips), id="review-keys", markup=True))

    def action_toggle_candidate(self) -> None:
        """Footer/Space twin: flip the focused candidate checkbox (each checkbox is also
        directly clickable). Named to avoid shadowing DOMNode.action_toggle, the built-in
        reactive-attribute toggle that takes an argument."""
        if isinstance(self.focused, Checkbox):
            self.focused.toggle()

    def _compose_deps(self) -> ComposeResult:
        spec = self._spec
        if spec is None or not spec.deps_flavor:
            return  # kinds with no dependency story (shell/fish/the data-driven tail)
        yield Static(gettext("Dependencies"), classes="section")
        if spec.deps_flavor == "npm":
            scanned = spec.dep_scanner(self._text) if spec.dep_scanner else []
            yield Input(
                value=self._overrides.get("deps", ", ".join(scanned)),
                placeholder=gettext("comma separated, e.g. chalk, @scope/pkg"),
                id="rv-deps",
            )
            yield Static(
                gettext("detected from the script's imports — edit freely"), classes="hint"
            )
            return
        if pep723.has_block(self._text):
            meta = pep723.parse_block(self._text) or {}
            deps = meta.get("dependencies") or []
            python = meta.get("requires-python", "")
            yield Static(gettext("The script declares its own dependencies (PEP 723):"))
            if python:
                yield Static(
                    "· " + gettext("needs Python %(python)s") % {"python": escape(str(python))}
                )
            for d in deps:
                yield Static("· " + gettext("installs %(dep)s") % {"dep": escape(str(d))})
            if not deps and not python:
                yield Static(f"[dim]{gettext('(none declared)')}[/dim]", markup=True)
        else:
            suggested = ", ".join(pep723.suggest_dependencies(self._text))
            yield Input(
                value=self._overrides.get("deps", suggested),
                placeholder=gettext("comma separated, e.g. requests>=2,<3, rich"),
                id="rv-deps",
            )
            yield Static(
                gettext("detected from the script's imports — edit freely"), classes="hint"
            )
            # The recorded constraint is VISIBLE and EDITABLE, whether it rode in on
            # --python or was derived from a versioned shebang — a value the add
            # writes but no surface shows (or shows read-only while the CLI's ask
            # lets you clear it) would be an invisible setting.
            yield Input(
                value=self._overrides.get("python", self._requires_python),
                placeholder=gettext("(automatic)"),
                id="rv-python",
            )
            yield Static(
                gettext(
                    "Python version (requires-python) — prefilled from the #! line when "
                    "it pins one; empty means automatic"
                ),
                classes="hint",
            )

    def _compose_params(self) -> ComposeResult:
        if self._spec is None or self._spec.analyzer is None:
            return  # no analyzer capability: nothing to tick (identity was still reviewed)
        yield Static(gettext("Parameters"), classes="section")
        reader = self._spec.cli_reader
        spec = reader.read_cli(self._text) if reader is not None else None
        if spec is not None and spec.ok and spec.fields:
            # A MODELED reader form IS the interface (the shared trap predicate,
            # flows.reader_fields): no tick list — managing would replace this form.
            yield Static(
                ngettext(
                    "✓ skit read this script's own arguments (%(count)s field). Running it "
                    "opens a form — nothing to memorize.",
                    "✓ skit read this script's own arguments (%(count)s fields). Running it "
                    "opens a form — nothing to memorize.",
                    len(spec.fields),
                )
                % {"count": len(spec.fields)}
            )
            return
        if self._analysis.uses_cli_framework:
            # Self-parsing skit couldn't model: the run form is passthrough-only, so
            # managed constants ADD fields rather than replace any — say so, then
            # keep the tick list.
            yield Static(
                gettext(
                    "This script parses its own arguments (%(names)s); skit couldn't model "
                    "them statically, so the run form offers an extra-arguments field."
                )
                % {"names": ", ".join(self._analysis.frameworks)},
                classes="hint",
            )
        if self._analysis.candidates:
            yield Static(gettext("Tick the ones the run form should ask for:"), classes="hint")
        for i, c in enumerate(self._analysis.candidates):
            label = (
                f"{c.name}  ({c.type} = {c.default!r})"
                if c.binding == "const"
                else gettext("input() #%(n)s: %(prompt)s")
                % {"n": c.order + 1, "prompt": repr(c.prompt)}
            )
            yield Checkbox(
                escape(label),
                value=self._tick_overrides.get(c.name, not c.demoted),
                id=f"rv-cand-{i}",
            )
            if c.demoted:
                yield Static(
                    "  ⚠ " + gettext("looks like a loop accumulator — probably not a parameter"),
                    classes="warn",
                )
        if self._analysis.filename_literals:
            names = ", ".join(repr(s) for s in self._analysis.filename_literals)
            yield Static(
                "💡 "
                + gettext(
                    "%(names)s are written directly inside the code, so skit can't turn them "
                    "into form fields. To manage one, first give it a name at the top of the "
                    "script, e.g. OUTPUT = '…' (Ctrl+E edits it now)."
                )
                % {"names": escape(names)},
                classes="hint",
            )
        # Yields to the reader notice above (the CLI's _print_add_hints rule): a
        # detected framework already named the extra-arguments field.
        if self._analysis.uses_argv and not self._analysis.uses_cli_framework:
            yield Static(
                "ℹ "  # noqa: RUF001 — intended info glyph, completing the 💡 tip / ⚠ warning set
                + gettext(
                    "This script reads command-line arguments; the run form has an "
                    "extra-arguments field for them."
                ),
                classes="hint",
            )

    def action_edit_source(self) -> None:
        """Ctrl+E: open the USER'S original file in their editor, then rescan on return
        (the edit→return→rescan loop; A5 is not involved — it's their file, their editor)."""
        self._overrides["name"] = self.query_one("#rv-name", Input).value
        self._overrides["desc"] = self.query_one("#rv-desc", Input).value
        if not self._fresh:
            self._overrides["mode"] = str(self.query_one("#rv-mode", RadioSet).pressed_index)
        deps_box = self.query("#rv-deps")
        if deps_box:
            self._overrides["deps"] = deps_box.first(Input).value
        py_box = self.query("#rv-python")
        if py_box:
            typed = py_box.first(Input).value.strip()
            if typed != self._requires_python:
                # The user edited the constraint: theirs wins over any future auto
                # pin (the rescan-must-never-throw-away-typed-input rule).
                self._overrides["python"] = typed
                self._py_pin_auto = False
        for i, c in enumerate(self._analysis.candidates):
            boxes = self.query(f"#rv-cand-{i}")
            if boxes:
                self._tick_overrides[c.name] = boxes.first(Checkbox).value
        with self.app.suspend():
            try:
                editor.open_in_editor(self._path)
            except editor.EditorError as exc:
                print(str(exc), flush=True)
        self._text = self._path.read_text(encoding="utf-8", errors="replace")
        self._analysis = self._analyze()
        if self._py_pin_auto:
            # The pin came from the shebang, and the shebang may just have changed.
            self._requires_python = self._python_pin()
        self.refresh(recompose=True)

    def _collected_python(self) -> str:
        """The requires-python the panel records: the editable #rv-python field when
        it is mounted (uv, no PEP 723 block), else whatever rode in (a block-carrying
        script owns its constraint; non-python kinds have none). '-'/'none' mean
        automatic — the CLI ask's own token, honored on this intake too."""
        boxes = self.query("#rv-python")
        value = boxes.first(Input).value.strip() if boxes else self._requires_python
        if value.lower() in ("-", "none"):
            return ""
        return value

    def _collected_deps(self) -> list[str]:
        flavor = self._spec.deps_flavor if self._spec is not None else ""
        if flavor == "npm":
            from .langs.javascript import deps as js_deps

            return js_deps.split_requirements(self.query_one("#rv-deps", Input).value)
        if flavor == "uv" and not pep723.has_block(self._text):
            return pep723.split_requirements(self.query_one("#rv-deps", Input).value)
        return []

    def _store_entry(self, name: str | None, desc: str, reference: bool, deps: list[str]):
        if self._kind == "python":
            return store.add_python(
                self._path,
                name=name,
                mode="reference" if reference else "copy",
                description=desc,
                dependencies=deps or None,
                requires_python=self._collected_python(),
            )
        from .langs.registry import shebang_program

        program = shebang_program(self._path)
        interpreter = program if self._spec is not None and program in self._spec.shebangs else ""
        entry = store.add_script(
            self._path,
            kind=self._kind,
            name=name,
            mode="reference" if reference else "copy",
            description=desc,
            interpreter=interpreter,
        )
        if deps and entry.meta.mode == "copy":
            entry = store.update_dependencies(entry.slug, deps)
        return entry

    def action_accept(self) -> None:
        name = self.query_one("#rv-name", Input).value.strip() or None
        desc = self.query_one("#rv-desc", Input).value.strip()
        reference = not self._fresh and self.query_one("#rv-mode", RadioSet).pressed_index == 1
        deps = self._collected_deps()
        if self._spec is not None and self._spec.deps_flavor == "uv":
            # Validate-then-write, same rule as every CLI intake: an unparseable
            # requirement or constraint written into the PEP 723 block would brick
            # every subsequent run with uv's raw error. (npm deps are the npm
            # installer's grammar — not validated here.)
            for dep in deps:
                if (error := pep723.requirement_error(dep)) is not None:
                    self.notify(error, severity="error")
                    return
            python = self._collected_python()
            if python and (error := pep723.requires_python_error(python)) is not None:
                self.notify(error, severity="error")
                return
        try:
            entry = self._store_entry(name, desc, reference, deps)
        except store.StoreError as exc:
            self.notify(str(exc), severity="error")
            return
        # The candidate checkboxes exist exactly when _compose_params rendered them:
        # analyzer present and NO modeled reader form (_reader_modeled — the shared
        # flows.reader_fields predicate; an unmodeled self-parser gets ticks because
        # managing there is additive). The collection gate MIRRORS the mount condition:
        # collecting more would query checkboxes that don't exist and crash after the
        # entry committed; collecting less silently drops ticks the screen itself
        # advertised — the drop this panel exists to refuse.
        if (
            entry.meta.mode == "copy"
            and self._spec is not None
            and self._spec.analyzer is not None
            and self._spec.params_io is not None
            and not self._reader_modeled()
        ):
            picked = [
                self._analysis.candidates[i]
                for i in range(len(self._analysis.candidates))
                if self.query_one(f"#rv-cand-{i}", Checkbox).value
            ]
            if picked:
                specs = [ParamDecl.from_candidate(c) for c in picked]
                copy_path = entry.script_path
                current = copy_path.read_text(encoding="utf-8")
                copy_path.write_text(self._spec.params_io.write(current, specs), encoding="utf-8")
        self.dismiss(entry.slug)

    def action_cancel(self) -> None:
        self.dismiss(None)


class PromptReviewScreen(Screen[str | None]):
    """The prompt twin of AddReviewScreen — same contract (everything prefilled, Ctrl+S
    is the only required act), prompt-shaped sections: the insertion master switch, the
    placeholder tick list (flood-capped), and the runner pick with the New agent… door.

    Nothing is stored until Ctrl+S: the flood warning, the tick defaults and the switch
    all happen BEFORE the entry exists, so a long prompt never lands half-configured."""

    BINDINGS = [
        Binding("escape", "cancel", gettext("Cancel")),
        # Non-priority: see AddReviewScreen — Ctrl+E belongs to the Input mid-edit.
        Binding("ctrl+e", "edit_source", gettext("Edit prompt")),
        Binding("ctrl+s", "accept", gettext("Add"), priority=True),
        Binding("ctrl+n", "new_runner", gettext("New agent"), show=False, priority=True),
        *tui_footer.FIELD_NAV_BINDINGS,
    ]
    # Boot on the name field, not the "*" pick (the body scroll container).
    AUTO_FOCUS = "Input"
    DEFAULT_CSS = """
    PromptReviewScreen #pv-body {
        padding: 0 1;
        border: round $skit-box-olive;
        border-title-color: ansi_bright_white;
        border-title-style: bold;
    }
    PromptReviewScreen .section { color: $accent; margin: 1 0 0 0; }
    PromptReviewScreen .hint { color: $text-muted; }
    PromptReviewScreen .warn { color: $warning; }
    PromptReviewScreen #pv-holes { height: auto; }
    PromptReviewScreen KeysBar { dock: bottom; }
    PromptReviewScreen #pv-keys { color: $text-muted; }
    """

    def __init__(
        self,
        path: Path,
        *,
        name: str | None = None,
        description: str | None = None,
        reference: bool = False,
        runner: str | None = None,
        interpolate: bool = True,
        fresh: bool = False,
    ) -> None:
        """The keyword arguments prefill the panel (the CLI face passes `skit add`'s
        flags through them); everything stays editable on screen. fresh=True is the
        drafted-in-$EDITOR lane: a temp file with no original to link, so the storage
        section is skipped and accept always copies. Like AddReviewScreen, fresh is
        DERIVED for kept drafts — no face may offer "Link the original" on a file
        the success path consumes."""
        super().__init__()
        self._path: Path = path
        self._fresh: bool = fresh or is_draft(path)
        self._text: str = path.read_text(encoding="utf-8", errors="replace")
        self._detected: list[str] = prompt_analyzer.placeholder_names(self._text)
        self._runner_names: list[str] = []
        # The names behind the tick checkboxes, in compose order (flooded lists show
        # only a preview, so index math must go through this, never self._detected).
        self._shown_names: list[str] = []
        # Survives the edit→rescan recompose: the rescan refreshes DETECTION, it must
        # never throw away what the user already set on the panel.
        self._overrides: dict[str, str] = {}
        if name:
            self._overrides["name"] = name
        if description:
            self._overrides["desc"] = description
        if reference:
            self._overrides["mode"] = "1"
        if runner:
            self._overrides["runner"] = runner
        if not interpolate:
            self._overrides["interpolate"] = "off"

    def on_mount(self) -> None:
        self.query_one("#pv-body").border_title = gettext("Add %(name)s") % {
            "name": escape(self._path.name)
        }
        self.query_one("#pv-holes").display = self.query_one("#pv-interpolate", Checkbox).value

    def _default_runner(self) -> str:
        """Prefill: the CLI's --runner (or the pick kept across a rescan), else the
        last-picked runner, else no pin ("" = ask on the run form)."""
        preferred = self._overrides.get("runner") or argstate.load_last_runner()
        return preferred if preferred in self._runner_names else ""

    @override
    def compose(self) -> ComposeResult:
        self._runner_names = [r.name for r in config.load_prompt_runners()]
        with tui_footer.FormBody(id="pv-body"):
            yield Static(gettext("Name"), classes="section")
            yield Input(
                value=self._overrides.get("name", self._path.stem.removesuffix(".prompt")),
                id="pv-name",
            )
            yield Static(gettext("Description"), classes="section")
            yield Input(
                value=self._overrides.get("desc", store.prompt_description(self._text)),
                placeholder=gettext("(taken from the first line — you can write your own)"),
                id="pv-desc",
            )
            if not self._fresh:
                yield from self._compose_storage()
            yield Static(gettext("Variable insertion"), classes="section")
            yield Checkbox(
                gettext("Fill {{name}} placeholders from a form before each run"),
                value=self._overrides.get("interpolate") != "off",
                id="pv-interpolate",
            )
            with Vertical(id="pv-holes"):
                yield from self._compose_placeholders()
            yield Static(gettext("Runner (the agent this prompt runs with)"), classes="section")
            yield Select(
                [(gettext("ask on the run form"), "")]
                + [(name, name) for name in self._runner_names],
                value=self._default_runner(),
                allow_blank=False,
                id="pv-runner-select",
            )
            yield Static(tui_runner.new_runner_chip(), id="pv-runner-new", markup=True)
        yield tui_footer.KeysBar(
            Static(
                tui_footer.bar(
                    tui_footer.chip("screen.accept", "Ctrl+S", gettext("Add")),
                    tui_footer.chip("screen.toggle_candidate", "Space", gettext("Toggle")),
                    tui_footer.chip("screen.edit_source", "Ctrl+E", gettext("Edit prompt")),
                    tui_footer.chip("screen.cancel", "Esc", gettext("Cancel")),
                    tui_footer.nav_chip(),
                ),
                id="pv-keys",
                markup=True,
            )
        )

    def _compose_storage(self) -> ComposeResult:
        yield Static(gettext("Storage"), classes="section")
        reference = self._overrides.get("mode") == "1"
        with RadioSet(id="pv-mode"):
            yield RadioButton(
                gettext("Keep a copy — skit stores it; your original file is never modified"),
                value=not reference,
            )
            yield RadioButton(
                gettext(
                    "Link the original — edits take effect immediately; skit never "
                    "writes to the file"
                ),
                value=reference,
            )

    def _compose_placeholders(self) -> ComposeResult:
        """The tick list. Flood honesty (docs/design/prompt.md): past AUTO_MANAGE_LIMIT
        the prompt was clearly not written for insertion — nothing is pre-ticked, only a
        preview is shown, and the warning points at the master switch."""
        detected = self._detected
        flooded = len(detected) > prompt_analyzer.AUTO_MANAGE_LIMIT
        self._shown_names = detected[: prompt_analyzer.LIST_PREVIEW_LIMIT] if flooded else detected
        if not detected:
            yield Static(
                gettext(
                    "No {{name}} placeholders detected — the body travels to the agent as written."
                ),
                classes="hint",
            )
            return
        if flooded:
            yield Static(
                gettext(
                    "Detected %(count)s placeholders — probably not written for "
                    "insertion. Tick only the ones you need, or untick the switch above."
                )
                % {"count": len(detected)},
                classes="warn",
            )
        else:
            yield Static(gettext("Tick the ones the run form should ask for:"), classes="hint")
        for i, hole_name in enumerate(self._shown_names):
            mark = gettext(" (secret)") if is_secret_name(hole_name) else ""
            yield Checkbox(escape(hole_name) + mark, value=not flooded, id=f"pv-hole-{i}")
        if len(detected) > len(self._shown_names):
            yield Static(
                gettext("…and %(count)s more (manage them later in Script settings)")
                % {"count": len(detected) - len(self._shown_names)},
                classes="hint",
            )

    @on(Checkbox.Changed, "#pv-interpolate")
    def _toggle_holes(self, event: Checkbox.Changed) -> None:
        """The master switch folds the tick list away — off means NO insertion machinery,
        and the panel should look like it. Tick states survive underneath for a re-tick."""
        self.query_one("#pv-holes").display = event.value

    def action_toggle_candidate(self) -> None:
        """Footer/Space twin: flip the focused checkbox (each is also clickable)."""
        if isinstance(self.focused, Checkbox):
            self.focused.toggle()

    def _picked_runner(self) -> str:
        """The runner dropdown's pick ("" = no pin). Value-keyed — no index math."""
        value = self.query_one("#pv-runner-select", Select).value
        return "" if value is Select.NULL else str(value)

    def action_new_runner(self) -> None:
        """Ctrl+N / the New agent… chip: define a custom runner without leaving the
        panel — it lands in config, joins the picker, and is selected immediately."""

        def _added(runner_name: str | None) -> None:
            if not runner_name:
                return
            self._runner_names.append(runner_name)
            select = self.query_one("#pv-runner-select", Select)
            select.set_options(
                [(gettext("ask on the run form"), "")]
                + [(name, name) for name in self._runner_names]
            )
            select.value = runner_name

        self.app.push_screen(tui_runner.RunnerAddModal(), _added)

    def action_edit_source(self) -> None:
        """Ctrl+E: open the USER'S original file in their editor, then rescan on return
        (the same edit→return→rescan loop as the python panel — their file, their editor)."""
        self._overrides["name"] = self.query_one("#pv-name", Input).value
        self._overrides["desc"] = self.query_one("#pv-desc", Input).value
        if not self._fresh:
            self._overrides["mode"] = str(self.query_one("#pv-mode", RadioSet).pressed_index)
        self._overrides["interpolate"] = (
            "on" if self.query_one("#pv-interpolate", Checkbox).value else "off"
        )
        picked = self._picked_runner()
        if picked:
            self._overrides["runner"] = picked
        else:
            self._overrides.pop("runner", None)
        with self.app.suspend():
            try:
                editor.open_in_editor(self._path)
            except editor.EditorError as exc:
                print(str(exc), flush=True)
        self._text = self._path.read_text(encoding="utf-8", errors="replace")
        self._detected = prompt_analyzer.placeholder_names(self._text)
        self.refresh(recompose=True)

    def action_accept(self) -> None:
        name = self.query_one("#pv-name", Input).value.strip() or None
        desc = self.query_one("#pv-desc", Input).value.strip()
        reference = not self._fresh and self.query_one("#pv-mode", RadioSet).pressed_index == 1
        interpolate = self.query_one("#pv-interpolate", Checkbox).value
        managed: list[str] | None = None
        if interpolate:
            # The EXPLICIT kept subset — including the honest empty list. Flooded
            # panels tick nothing by default, and add_prompt honors what was asked.
            managed = [
                self._shown_names[i]
                for i in range(len(self._shown_names))
                if self.query_one(f"#pv-hole-{i}", Checkbox).value
            ]
        runner = self._picked_runner()
        try:
            entry = store.add_prompt(
                self._path,
                name=name,
                mode="reference" if reference else "copy",
                description=desc,
                managed=managed,
                runner=runner,
                interpolate=interpolate,
            )
        except store.StoreError as exc:
            self.notify(str(exc), severity="error")
            return
        if runner:
            # A real pick prefills the next picker (never a non-interactive resolve).
            argstate.save_last_runner(runner)
        self.dismiss(entry.slug)

    def action_cancel(self) -> None:
        self.dismiss(None)


class _ReviewHost(App[str | None]):
    """A review panel hosted alone (the CLI face). Exits with the new entry's slug, or
    None on cancel — the SAME screen the TUI's `a` pushes, so the two can't drift."""

    ENABLE_COMMAND_PALETTE = False
    HORIZONTAL_BREAKPOINTS = tui_layout.HORIZONTAL_BREAKPOINTS
    VERTICAL_BREAKPOINTS = tui_layout.VERTICAL_BREAKPOINTS

    def __init__(self, screen: Screen[str | None]) -> None:
        super().__init__()
        self._screen: Screen[str | None] = screen

    @override
    def get_css_variables(self) -> dict[str, str]:
        # Same bootstrap as the Library app: the first stylesheet parse runs before
        # on_mount activates the theme and would die on $skit-box-*.
        return {**super().get_css_variables(), **theme.BOX_VARIABLES}

    def on_mount(self) -> None:
        self.register_theme(theme.CLAUDE_THEME)
        self.theme = "skit-claude"
        self.push_screen(self._screen, self.exit)


class AddReviewApp(_ReviewHost):
    """`skit add x.py` / `skit add x.sh` in an interactive terminal."""

    def __init__(
        self,
        path: Path,
        *,
        kind: str = "python",
        name: str | None = None,
        description: str | None = None,
        reference: bool = False,
        deps: list[str] | None = None,
        requires_python: str = "",
        fresh: bool = False,
    ) -> None:
        super().__init__(
            AddReviewScreen(
                path,
                kind=kind,
                name=name,
                description=description,
                reference=reference,
                deps=deps,
                requires_python=requires_python,
                fresh=fresh,
            )
        )


class PromptReviewApp(_ReviewHost):
    """`skit add x.prompt.md` in an interactive terminal."""

    def __init__(
        self,
        path: Path,
        *,
        name: str | None = None,
        description: str | None = None,
        reference: bool = False,
        runner: str | None = None,
        interpolate: bool = True,
    ) -> None:
        super().__init__(
            PromptReviewScreen(
                path,
                name=name,
                description=description,
                reference=reference,
                runner=runner,
                interpolate=interpolate,
            )
        )


def run_add_review(
    path: Path,
    *,
    kind: str = "python",
    name: str | None = None,
    description: str | None = None,
    reference: bool = False,
    deps: list[str] | None = None,
    requires_python: str = "",
) -> str | None:
    """Blocking CLI entry to the review panel. Returns the new slug, or None."""
    return AddReviewApp(
        path,
        kind=kind,
        name=name,
        description=description,
        reference=reference,
        deps=deps,
        requires_python=requires_python,
    ).run()


def run_prompt_review(
    path: Path,
    *,
    name: str | None = None,
    description: str | None = None,
    reference: bool = False,
    runner: str | None = None,
    interpolate: bool = True,
) -> str | None:
    """Blocking CLI entry to the prompt review panel. Returns the new slug, or None."""
    return PromptReviewApp(
        path,
        name=name,
        description=description,
        reference=reference,
        runner=runner,
        interpolate=interpolate,
    ).run()
