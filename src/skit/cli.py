"""CLI entry point (Typer): the v2 command surface.

Running `skit` with no subcommand opens the TUI workbench. The commands here are the
automation/SSH/muscle-memory shortcuts; every interactive flow goes through the shared
form layer (flows) so CLI and TUI behave identically.

Command-surface contracts:
- Exit codes (docker convention): `run` passes the script's exit code through PURE;
  skit's own failures are 125, a target that exists but isn't executable is 126, a
  missing target/name is 127, usage errors are 2. Other commands: 0/1/2.
- Every output has a --json twin where output exists.
- Lists are repeatable flags (--dep), never comma-joined (PEP 508 specifiers contain
  commas).
- Non-interactive contract: on a pipe/CI/--no-input, never prompt, never guess, never
  silently assemble a broken command.

Every user-visible string goes through i18n.gettext()/ngettext(). Help strings resolve at
import time, so i18n initializes lazily on module import (see i18n.py).
"""

from __future__ import annotations

import dataclasses
import json
import os
import sys
from pathlib import Path

import typer
from rich.console import Console
from rich.markup import escape
from rich.prompt import Confirm, Prompt

from . import (
    __version__,
    agentskill,
    argstate,
    config,
    editor,
    flows,
    i18n,
    launcher,
    models,
    pep723,
    promptform,
    store,
)
from .i18n import gettext, ngettext
from .langs.python import analyzer, metawriter, reconcile
from .langs.registry import spec_for

app = typer.Typer(
    name="skit",
    help=gettext(
        "skit — a script launcher and parameter manager. Run it without a subcommand to open the main menu."
    ),
    add_completion=True,
    no_args_is_help=False,
)
console = Console()
err_console = Console(stderr=True)

# Exit-code contract for `skit run` (docker convention; the script's own code passes
# through untouched, so these must stay out of the 0-124 range scripts commonly use).
EXIT_USAGE = 2
EXIT_SKIT = 125
EXIT_NOT_EXECUTABLE = 126
EXIT_NOT_FOUND = 127
EXIT_CANCELLED = 130  # user cancelled the form (128+SIGINT convention) — not a skit failure

# Rich closing tags are case-insensitive, so a mutated-case variant is behaviorally identical.
_DIM_CLOSE = "[/dim]"  # pragma: no mutate
_RED_CLOSE = "[/red]"  # pragma: no mutate


def _fail(message: str, code: int) -> typer.Exit:
    err_console.print(f"[red]{escape(message)}[/red]")
    return typer.Exit(code)


def _is_interactive() -> bool:
    return sys.stdin.isatty() and sys.stdout.isatty()


# --------------------------------------------------------------------------
# dynamic completion (north star: nothing to memorize — not even your own names)
# --------------------------------------------------------------------------


def _complete_script(incomplete: str) -> list[str]:
    try:
        entries = store.list_entries()
    except Exception:  # completion must never crash the shell
        return []
    out = {e.meta.name for e in entries} | {e.slug for e in entries}
    return sorted(c for c in out if c.startswith(incomplete))


def _complete_preset(ctx: typer.Context, incomplete: str) -> list[str]:
    name = ctx.params.get("name")
    if not name:
        return []
    try:
        entry = store.resolve(name)
        presets = argstate.load_state(entry.slug)["presets"]
    except Exception:  # completion must never crash the shell
        return []
    return sorted(p for p in presets if p.startswith(incomplete))


_SCRIPT_ARG = typer.Argument(
    ..., help=gettext("Script name or slug"), autocompletion=_complete_script
)


@app.callback(invoke_without_command=True)
def main(
    ctx: typer.Context,
    version: bool = typer.Option(False, "--version", "-V", help=gettext("Show version")),
) -> None:
    if version:
        console.print(f"skit {__version__}")
        raise typer.Exit()
    if ctx.invoked_subcommand is None:
        _maybe_first_run_setup()
        from .tui import run_menu

        raise typer.Exit(run_menu())


# --------------------------------------------------------------------------
# add
# --------------------------------------------------------------------------


def _resolve_python_metadata(
    text: str, deps_opt: list[str] | None, python_opt: str | None, no_input: bool
) -> tuple[list[str], str]:
    """Decide the (dependencies, requires_python) to fill in.

    - Script already has a PEP 723 block: don't ask, don't fill (the block is the source of truth).
    - Explicit --dep / --python: use them directly, no prompting.
    - Interactive: only ask when the AST reveals likely third-party imports; ask nothing when
      there are no dependencies at all.
    """
    if pep723.has_block(text):
        meta = pep723.parse_block(text) or {}
        deps = meta.get("dependencies")
        if deps:
            console.print(
                gettext("The script declares its own dependencies (PEP 723): %(deps)s")
                % {"deps": ", ".join(escape(d) for d in deps)}
            )
        return [], ""
    if deps_opt is not None or python_opt is not None:
        return list(deps_opt or []), python_opt or ""
    suggested = pep723.suggest_dependencies(text)
    if not suggested:
        return [], ""  # No dependencies: nothing to ask
    if no_input or not sys.stdin.isatty():
        return suggested, ""  # Non-interactive: accept the suggestions as-is
    answer = Prompt.ask(
        gettext("Dependencies to install (Enter to accept, edit the list, or '-' for none)"),
        default=", ".join(suggested),
        console=console,
    )
    if answer.strip().lower() in ("-", "none"):
        deps_list: list[str] = []
    else:
        deps_list = pep723.split_requirements(answer)
    py = Prompt.ask(
        gettext("Python version (leave empty for automatic)"), default="", console=console
    )
    return deps_list, py.strip()


def _prompt_identity(
    p: Path, text: str, name: str | None, description: str | None, no_input: bool
) -> tuple[str | None, str | None]:
    """Interactive name + description prompts for `add`. `None` means "let the store derive it"."""
    if no_input or not sys.stdin.isatty():
        return name, description
    if name is None:
        name = Prompt.ask(gettext("Name in skit"), default=p.stem, console=console).strip() or None
    if description is None:
        description = Prompt.ask(
            gettext("Description (optional)"),
            default=store.suggest_description(text),
            console=console,
        ).strip()
    return name, description


def _require_py_file(resolved: Path) -> None:
    if not resolved.is_file():
        raise store.StoreError(gettext("File not found: %(path)s") % {"path": str(resolved)})


def _parse_selection(answer: str, count: int) -> list[int]:
    """Parse an onboarding selection: 'all' / 'none' (or empty) / '1,3,5'."""
    answer = answer.strip().lower()
    if answer == "all":
        return list(range(count))
    picked: list[int] = []
    for raw_part in answer.split(","):
        part = raw_part.strip()
        # isdecimal() is the predicate whose truth guarantees int() succeeds (isdigit()
        # also accepts superscripts/circled digits that int() rejects).
        if part.isdecimal() and 1 <= int(part) <= count and (int(part) - 1) not in picked:
            picked.append(int(part) - 1)
    return picked


def _default_selection(candidates: list[analyzer.Candidate]) -> str:
    """Signal-driven default (UX spec §0): clean candidates in, demoted candidates out."""
    clean = [i for i, c in enumerate(candidates, start=1) if not c.demoted]
    if len(clean) == len(candidates):
        return "all"
    if not clean:
        return "none"
    return ",".join(str(i) for i in clean)


def _print_candidate(i: int, c: analyzer.Candidate) -> None:
    mark = gettext(" (secret)") if c.secret else ""
    if c.kind == "const":
        console.print(
            "  "
            + gettext("%(num)s. %(name)s (%(type)s) = %(value)s%(secret)s")
            % {
                "num": i,
                "name": escape(c.name),
                "type": c.type,
                "value": escape(repr(c.default)),
                "secret": mark,
            }
        )
    else:
        console.print(
            "  "
            + gettext("%(num)s. input() #%(ordinal)s: %(prompt)s%(secret)s")
            % {"num": i, "ordinal": c.order + 1, "prompt": escape(repr(c.prompt)), "secret": mark}
        )
    if c.demoted:
        console.print(
            f"     [yellow]{gettext('⚠ looks like a loop accumulator — probably not a parameter')}[/yellow]"
        )


def _print_add_hints(result: analyzer.Analysis, script_name: str) -> None:
    """The honest, rule-backed hints (UX spec §0): argv passthrough, extractable filenames."""
    if result.uses_argv:
        console.print(
            "[dim]"
            + gettext(
                "This script reads command-line arguments; the run form has an extra-arguments field for them."
            )
            + _DIM_CLOSE
        )
    if result.filename_literals:
        names = ", ".join(escape(repr(s)) for s in result.filename_literals)
        console.print(
            "[dim]"
            + gettext(
                "💡 %(names)s are written directly inside the code, so skit can't turn them into form fields. To manage one, first give it a name at the top of the script, e.g. OUTPUT = '…' (skit edit %(script)s)."
            )
            % {"names": names, "script": escape(script_name)}
            + _DIM_CLOSE
        )


def _onboard_params(text: str, script_name: str, no_input: bool) -> list[metawriter.ParamSpec]:
    """Parameter onboarding at add time (A4: which constant counts as a parameter is a UX call).

    - argparse detected: nothing to manage — the run form is read statically from the
      script's own argument declarations (the unified form model).
    - Non-interactive: don't guess, don't select, return empty (honesty beats clever).
    """
    result = analyzer.analyze(text)
    if result.uses_cli_framework:
        from .langs.python import argspec

        spec = argspec.read_cli(text)
        if spec is not None and spec.ok and spec.fields:
            console.print(
                gettext(
                    "✓ skit read this script's own arguments (%(count)s fields). Running it opens a form — nothing to memorize."
                )
                % {"count": len(spec.fields)}
            )
        else:
            console.print(
                "[dim]"
                + gettext(
                    "This script parses its own arguments (%(names)s); skit couldn't model them statically, so the run form offers a passthrough-arguments field."
                )
                % {"names": ", ".join(result.frameworks)}
                + _DIM_CLOSE
            )
        return []
    _print_add_hints(result, script_name)
    if not result.candidates or no_input or not sys.stdin.isatty():
        return []
    console.print(
        ngettext(
            "Found %(count)s parameter candidate (constants / input() calls):",
            "Found %(count)s parameter candidates (constants / input() calls):",
            len(result.candidates),
        )
        % {"count": len(result.candidates)}
    )
    for i, c in enumerate(result.candidates, start=1):
        _print_candidate(i, c)
    answer = Prompt.ask(
        gettext("Which ones should skit manage? (e.g. 1,3 / all / none)"),
        default=_default_selection(result.candidates),
        console=console,
    )
    picked = _parse_selection(answer, len(result.candidates))
    return [metawriter.ParamSpec.from_candidate(result.candidates[i]) for i in picked]


# A brand-new script starts from just a shebang; if the editor is closed with nothing more
# than this (or empty), we treat it as "cancelled" and add nothing.
_STARTER_SCRIPT = "#!/usr/bin/env python3\n"


def _onboard_python(
    p: Path,
    text: str,
    *,
    name: str | None,
    description: str | None = None,
    ref: bool = False,
    deps_opt: list[str] | None = None,
    python_opt: str | None = None,
    no_input: bool = False,
) -> tuple[store.Entry, list[str], list[str], list[str]]:
    """Shared add/create pipeline: identity -> dependencies -> store add -> parameter
    onboarding. Returns (entry, deps, managed_names, secret_names) for the summary."""
    name, description = _prompt_identity(p, text, name, description, no_input)
    final_deps, final_py = _resolve_python_metadata(text, deps_opt, python_opt, no_input)
    entry = store.add_python(
        p,
        name=name,
        mode="reference" if ref else "copy",
        description=description,
        dependencies=final_deps or None,
        requires_python=final_py,
    )
    managed: list[str] = []
    secrets: list[str] = []
    if entry.meta.mode == "reference":
        console.print(
            f"[dim]{gettext('Reference mode never touches the original file, so parameter setup was skipped.')}[/dim]"
        )
    else:
        params_specs = _onboard_params(text, entry.meta.name, no_input)
        if params_specs:
            copy_path = entry.script_path
            current = copy_path.read_text(encoding="utf-8")  # pragma: no mutate — utf-8 equivalence
            new_text = metawriter.write_params(current, params_specs)
            copy_path.write_text(new_text, encoding="utf-8")  # pragma: no mutate
            managed = [s.name for s in params_specs]
            secrets = [s.name for s in params_specs if s.secret]
    return entry, final_deps, managed, secrets


def _create_python_in_editor(name: str | None) -> None:
    """Write a starter script to a temp file, open the user's editor, then ingest whatever
    they saved."""
    import tempfile

    if not _is_interactive():
        err_console.print(
            f"[red]{gettext('Writing a new script in an editor needs an interactive terminal.')}[/red]"
        )
        raise typer.Exit(EXIT_USAGE)
    if not name:
        name = Prompt.ask(gettext("Name in skit"), console=console).strip()
        if not name:
            err_console.print(f"[red]{gettext('A name is required.')}[/red]")
            raise typer.Exit(EXIT_USAGE)
    fd, tmp_name = tempfile.mkstemp(suffix=".py", prefix="skit-new-")  # pragma: no mutate
    os.close(fd)
    tmp = Path(tmp_name)
    tmp.write_text(_STARTER_SCRIPT, encoding="utf-8")  # pragma: no mutate
    try:
        console.print(f"[dim]{gettext('Opening your editor…')}[/dim]")
        editor.open_in_editor(tmp)
        text = tmp.read_text(encoding="utf-8", errors="replace")  # pragma: no mutate — utf-8 equiv
        if text.strip() in ("", _STARTER_SCRIPT.strip()):
            console.print(gettext("Nothing was written, so no script was added."))
            return
        entry, deps, managed, secrets = _onboard_python(tmp, text, name=name)
    except (editor.EditorError, store.StoreError) as exc:
        raise _fail(str(exc), 1) from exc
    finally:
        tmp.unlink(missing_ok=True)  # pragma: no mutate — the temp file always exists here
    _print_add_summary(entry, deps, managed, secrets)


def _add_from_stdin(name: str | None, description: str | None) -> None:
    """`skit add -`: ingest a script from stdin (e.g. `pbpaste | skit add - -n clip`).
    stdin is the script, so there is nobody to prompt: the non-interactive contract
    applies, and a name is required up front."""
    import tempfile

    if not name:
        err_console.print(
            f"[red]{gettext('Reading the script from stdin needs an explicit --name.')}[/red]"
        )
        raise typer.Exit(EXIT_USAGE)
    text = sys.stdin.read()
    if not text.strip():
        err_console.print(
            f"[red]{gettext('Nothing arrived on stdin, so there is nothing to add.')}[/red]"
        )
        raise typer.Exit(1)
    fd, tmp_name = tempfile.mkstemp(suffix=".py", prefix="skit-stdin-")  # pragma: no mutate
    os.close(fd)
    tmp = Path(tmp_name)
    tmp.write_text(text, encoding="utf-8")  # pragma: no mutate
    try:
        entry, deps, managed, secrets = _onboard_python(
            tmp, text, name=name, description=description, no_input=True
        )
    except store.StoreError as exc:
        raise _fail(str(exc), 1) from exc
    finally:
        tmp.unlink(missing_ok=True)  # pragma: no mutate
    _print_add_summary(entry, deps, managed, secrets)


def _infer_add_kind(resolved: Path, exe_flag: bool) -> str:
    """Type inference (v2) — delegated to store.infer_kind so the CLI and the TUI add
    panel share one rule and can't drift apart."""
    return store.infer_kind(resolved, force_exe=exe_flag)


@app.command(
    help=gettext("Add a script, executable, or command to skit."),
    epilog=gettext(
        "Examples:  skit add tools/resize.py  ·  skit add --cmd 'ffmpeg -i {input}' -n convert  ·  pbpaste | skit add - -n clip"
    ),
)
def add(
    path: str = typer.Argument(
        None, help=gettext("Path to a script or executable, or '-' to read a script from stdin")
    ),
    name: str = typer.Option(
        None, "--name", "-n", help=gettext("Name / alias (defaults to the file name)")
    ),
    description: str = typer.Option(
        None,
        "--description",
        "-d",
        help=gettext("Description (defaults to the first line of the docstring)"),
    ),
    edit_new: bool = typer.Option(
        False, "--edit", "-e", help=gettext("Write a brand-new script in your editor, then add it")
    ),
    ref: bool = typer.Option(
        False,
        "--ref",
        help=gettext("Reference mode: link to the original file instead of copying it"),
    ),
    exe: bool = typer.Option(
        False,
        "--exe",
        help=gettext("Force the executable kind (normally inferred from the file itself)"),
    ),
    cmd: str = typer.Option(
        None, "--cmd", help=gettext("Register a command template, e.g. --cmd 'ffmpeg -i {input}'")
    ),
    dep: list[str] = typer.Option(
        None,
        "--dep",
        help=gettext("A dependency (repeat for more; skips the interactive question)"),
    ),
    python: str = typer.Option(
        None, "--python", help=gettext('Python version constraint, e.g. ">=3.11"')
    ),
    no_input: bool = typer.Option(
        False, "--no-input", help=gettext("Never prompt; accept the detected suggestions")
    ),
) -> None:
    """Add a script / executable / command to skit."""
    if edit_new:
        if path:
            err_console.print(
                f"[red]{gettext('Use --edit to write a new script, or pass a path to add an existing one — not both.')}[/red]"
            )
            raise typer.Exit(EXIT_USAGE)
        _create_python_in_editor(name)
        return
    if path == "-":
        _add_from_stdin(name, description)
        return
    summary_deps: list[str] = []
    summary_managed: list[str] = []
    summary_secrets: list[str] = []
    try:
        if cmd is not None:
            if not name:
                err_console.print(f"[red]{gettext('A --cmd entry needs a --name')}[/red]")
                raise typer.Exit(EXIT_USAGE)
            entry = store.add_command(cmd, name=name, description=description or "")
            if entry.meta.params:
                console.print(
                    gettext(
                        "Detected parameters: %(names)s (the run form asks for them; your last values are remembered)"
                    )
                    % {"names": ", ".join(escape(p) for p in entry.meta.params)}
                )
        else:
            if not path:
                err_console.print(
                    f"[red]{gettext('Provide a script path, or use --cmd to register a command template')}[/red]"
                )
                raise typer.Exit(EXIT_USAGE)
            resolved = Path(path).expanduser().resolve()
            kind = _infer_add_kind(resolved, exe)
            if kind == "exe":
                entry = store.add_exe(Path(path), name=name, description=description or "")
            elif kind == "unknown":
                err_console.print(
                    f"[red]{gettext("%(file)s isn't a .py file or an executable — pass --exe for a program, or --cmd for a command template.") % {'file': escape(resolved.name)}}[/red]"
                )
                raise typer.Exit(EXIT_USAGE)
            else:
                _require_py_file(resolved)
                try:
                    text = resolved.read_text(encoding="utf-8", errors="replace")
                except OSError as exc:
                    raise store.StoreError(
                        gettext("Can't read %(path)s: %(error)s")
                        % {"path": str(resolved), "error": exc.strerror or str(exc)}
                    ) from exc
                # Interactive + mini-form style: host the SAME review panel the TUI's
                # `a` opens (flags prefill it). Pipes/CI/--no-input/form=plain keep the
                # line-prompt path — the non-interactive contract is untouched.
                if (
                    not no_input
                    and _is_interactive()
                    and os.environ.get("TERM") != "dumb"
                    and config.load_form() == "tui"
                ):
                    from .tui_add import run_add_review

                    slug = run_add_review(
                        Path(path),
                        name=name,
                        description=description,
                        reference=ref,
                        deps=dep,
                        requires_python=python or "",
                    )
                    if slug is None:
                        console.print(f"[dim]{gettext('Cancelled — nothing was added.')}[/dim]")
                        raise typer.Exit(EXIT_CANCELLED)
                    entry = store.resolve(slug)
                else:
                    entry, summary_deps, summary_managed, summary_secrets = _onboard_python(
                        Path(path),
                        text,
                        name=name,
                        description=description,
                        ref=ref,
                        deps_opt=dep,
                        python_opt=python,
                        no_input=no_input,
                    )
    except store.StoreError as exc:
        raise _fail(str(exc), 1) from exc
    _print_add_summary(entry, summary_deps, summary_managed, summary_secrets)


def _print_add_summary(
    entry: store.Entry, deps: list[str], managed: list[str], secrets: list[str]
) -> None:
    """One consolidated block after a successful add."""
    entry_spec = spec_for(entry.meta.kind)
    mode_note = (
        gettext("(%(mode)s mode)") % {"mode": entry.meta.mode}
        if entry_spec is not None and entry_spec.supports_modes
        else ""
    )
    console.print(
        f"[green]{gettext('Added: %(name)s') % {'name': escape(entry.meta.name)}}[/green] {mode_note}"
    )
    if entry.meta.description:
        console.print(
            f"  {gettext('Description: %(desc)s') % {'desc': escape(entry.meta.description)}}"
        )
    if deps:
        console.print(
            f"  {gettext('Dependencies: %(deps)s') % {'deps': ', '.join(escape(d) for d in deps)}}"
        )
    if managed:
        console.print(
            f"  {gettext('Managed parameters: %(names)s') % {'names': ', '.join(escape(n) for n in managed)}}"
        )
    console.print(f"  {gettext('Run it: skit run %(name)s') % {'name': escape(entry.meta.name)}}")
    if secrets:
        console.print(
            f"[dim]{gettext('Secret parameter values are never saved to disk: %(names)s') % {'names': ', '.join(escape(n) for n in secrets)}}[/dim]"
        )


# --------------------------------------------------------------------------
# list / remove / edit
# --------------------------------------------------------------------------


@app.command("list", help=gettext("List every registered script."))
def list_cmd(
    as_json: bool = typer.Option(False, "--json", help=gettext("Output as JSON")),
) -> None:
    """List every registered script."""
    entries = store.list_entries()
    if as_json:
        rows = []
        for e in entries:
            last = argstate.load_state(e.slug)["last_run"]
            rows.append(
                {
                    "name": e.meta.name,
                    "slug": e.slug,
                    "kind": e.meta.kind,
                    "mode": e.meta.mode,
                    "description": e.meta.description,
                    "missing": launcher.target_missing(e),
                    "last_run_at": last.get("at"),
                    "last_exit": last.get("exit"),
                }
            )
        console.print_json(json.dumps(rows, ensure_ascii=False))
        return
    if not entries:
        console.print(gettext("No scripts yet. Add one with: skit add <path>"))
        return
    from rich.table import Table

    table = Table(show_header=True, header_style="bold")
    table.add_column(gettext("Name"))
    table.add_column(gettext("Kind"))
    table.add_column(gettext("Description"))
    for e in entries:
        table.add_row(escape(e.meta.name), e.meta.kind, _list_description(e))
    console.print(table)


def _list_description(e: store.Entry) -> str:
    desc = escape(e.meta.description) if e.meta.description else "—"
    marker = launcher.missing_marker(e)
    if marker is None:
        return desc
    marker = f"[dim]{escape(marker)}[/dim]"
    return marker if desc == "—" else f"{desc}  {marker}"


# --------------------------------------------------------------------------
# show — the full read view of one script (identity + schema + presets)
# --------------------------------------------------------------------------


def _field_secret_cell(f: flows.FormField) -> str:
    """The Secret column for a form field: "—", "yes", or "yes ← $ENVVAR"."""
    if not f.secret:
        return "—"
    if f.env_source:
        return gettext("yes") + f" ← ${escape(f.env_source)}"
    return gettext("yes")


def _field_to_dict(f: flows.FormField) -> dict[str, object]:
    """One form field as a stable-shape JSON object (every key always present).
    `default` is null when the script declares none; a secret's declared default is
    emitted as-is, matching `params --json` (it already lives in the script's own text)."""
    return {
        "key": f.key,
        "label": f.label,
        "type": f.kind,
        "source": f.source,
        "required": f.required,
        "secret": f.secret,
        "multiple": f.multiple,
        "degraded": f.degraded,
        "choices": list(f.choices),
        "default": f.default if f.has_default else None,
        "help": f.help,
        "flag": f.flag,
        "action": f.action,
        "env_source": f.env_source,
    }


def _print_show_human(entry: store.Entry, plan: flows.FormPlan, presets: list[str]) -> None:
    meta = entry.meta
    console.print(f"[bold]{escape(meta.name)}[/bold]  [dim]({meta.kind} · {meta.mode})[/dim]")
    if meta.description:
        console.print(f"  {escape(meta.description)}")
    show_spec = spec_for(meta.kind)
    if show_spec is None or show_spec.has_original_file:
        console.print(f"  {gettext('Source: %(path)s') % {'path': escape(str(meta.source))}}")
    if meta.workdir != "origin":
        console.print(
            f"  {gettext('Working directory: %(dir)s') % {'dir': escape(str(meta.workdir))}}"
        )
    marker = launcher.missing_marker(entry)
    if marker is not None:
        console.print(f"  [yellow]{escape(marker)}[/yellow]")
    if meta.dependencies:
        console.print(
            f"  {gettext('Dependencies: %(deps)s') % {'deps': ', '.join(escape(d) for d in meta.dependencies)}}"
        )
    if meta.requires_python:
        console.print(
            f"  {gettext('Python constraint: %(python)s') % {'python': escape(meta.requires_python)}}"
        )
    if meta.template:
        console.print(
            f"  {gettext('Command template: %(template)s') % {'template': escape(meta.template)}}"
        )
    _print_drift(plan)
    if plan.degraded_reason:
        console.print(
            f"[dim]{gettext("skit could not model this script's own arguments; pass them after -- instead.")}[/dim]"
        )
    if plan.fields:
        _print_show_fields(plan)
    else:
        console.print(
            f"  {gettext('No form fields — arguments after -- pass straight through to the script.')}"
        )
    if presets:
        console.print(
            f"  {gettext('Presets: %(names)s') % {'names': ', '.join(escape(p) for p in presets)}}"
        )
    console.print(f"  {gettext('Run it: skit run %(name)s') % {'name': escape(meta.name)}}")


def _print_show_fields(plan: flows.FormPlan) -> None:
    from rich.table import Table

    table = Table(show_header=True, header_style="bold")  # pragma: no mutate — cosmetic
    table.add_column(gettext("Parameter"))
    table.add_column(gettext("Type"))
    table.add_column(gettext("Required"))
    table.add_column(gettext("Default"))
    table.add_column(gettext("Choices"))
    table.add_column(gettext("Secret"))
    table.add_column(gettext("Help"))
    for f in plan.fields:
        if not f.has_default:
            default_shown = "—"
        elif f.secret:
            default_shown = gettext("•••")
        else:
            default_shown = f.default or "—"
        # Inject fields carry their form prompt in `label`; argparse fields carry help text.
        help_shown = f.help or (f.label if f.label != f.key else "")
        table.add_row(
            escape(f.key),
            escape(f.kind),
            gettext("yes") if f.required else "—",
            escape(default_shown),
            escape(", ".join(f.choices)) if f.choices else "—",
            _field_secret_cell(f),
            escape(help_shown) if help_shown else "—",
        )
    console.print(table)


@app.command(
    help=gettext("Show everything about one script: metadata, dependencies, parameters, presets."),
    epilog=gettext("Examples:  skit show resize  ·  skit show resize --json"),
)
def show(
    name: str = _SCRIPT_ARG,
    as_json: bool = typer.Option(False, "--json", help=gettext("Output as JSON")),
) -> None:
    """The single read view an automation (or a human) needs before running a script:
    identity, dependencies, the unified parameter schema (all three sources), presets."""
    try:
        entry = store.resolve(name)
    except store.NotFoundError as exc:
        raise _fail(str(exc), 1) from exc
    plan = flows.plan_for_entry(entry)
    state = argstate.load_state(entry.slug)
    presets = sorted(state["presets"])
    if not as_json:
        _print_show_human(entry, plan, presets)
        return
    last = state["last_run"]
    payload = {
        "name": entry.meta.name,
        "slug": entry.slug,
        "kind": entry.meta.kind,
        "mode": entry.meta.mode,
        "description": entry.meta.description,
        "source": entry.meta.source,
        "workdir": str(entry.meta.workdir),
        "missing": launcher.target_missing(entry),
        "dependencies": list(entry.meta.dependencies or []),
        "requires_python": entry.meta.requires_python,
        "template": entry.meta.template or None,
        "param_source": plan.source,
        "degraded_reason": plan.degraded_reason,
        "drift": bool(plan.drift_lines),
        "fields": [_field_to_dict(f) for f in plan.fields],
        "presets": presets,
        "last_run_at": last.get("at"),
        "last_exit": last.get("exit"),
    }
    console.print_json(json.dumps(payload, ensure_ascii=False))


@app.command(help=gettext("Remove a registered script (the original file is left untouched)."))
def remove(
    name: str = _SCRIPT_ARG,
    yes: bool = typer.Option(False, "--yes", "-y", help=gettext("Skip confirmation")),
) -> None:
    """Remove a script (copy mode deletes the copy in the store; the original is untouched)."""
    try:
        entry = store.resolve(name)
    except store.NotFoundError as exc:
        raise _fail(str(exc), 1) from exc
    if not yes:
        entry_spec = spec_for(entry.meta.kind)
        if entry_spec is not None and not entry_spec.has_original_file:
            question = gettext('Remove "%(name)s"?') % {"name": entry.meta.name}
        else:
            question = gettext('Remove "%(name)s"? Your original file will not be deleted.') % {
                "name": entry.meta.name
            }
        typer.confirm(question, abort=True)
    removed = store.remove(name)
    console.print(f"[green]{gettext('Removed: %(name)s') % {'name': escape(removed)}}[/green]")


def _offer_create_in_editor(name: str) -> None:
    """`skit edit <unknown>`: offer to create a brand-new script under that name."""
    if not _is_interactive():
        err_console.print(
            f"[red]{gettext('No script named %(name)s.') % {'name': escape(name)}}[/red]"
        )
        raise typer.Exit(1)
    if not Confirm.ask(
        gettext('No script named "%(name)s". Create it now?') % {"name": escape(name)},
        default=True,
        console=console,
    ):
        raise typer.Exit(0)  # pragma: no mutate — Exit(0)/Exit(None) both mean a clean exit
    _create_python_in_editor(name)


@app.command(
    help=gettext("Open a script's source in your editor (offers to create it if the name is new)."),
    epilog=gettext("Example:  skit edit resize"),
)
def edit(name: str = _SCRIPT_ARG) -> None:
    """Open a registered script's source in your editor."""
    try:
        entry = store.resolve(name)
    except store.NotFoundError:
        _offer_create_in_editor(name)
        return
    entry_spec = spec_for(entry.meta.kind)
    if entry_spec is None or not entry_spec.editable:
        raise _fail(
            gettext("%(name)s isn't a Python script, so it has no source to edit.")
            % {"name": entry.meta.name},
            1,
        )
    if entry.meta.mode == "reference":
        source = Path(entry.meta.source)
        if not source.exists():
            raise _fail(
                gettext("%(name)s: the referenced source file is gone: %(path)s")
                % {"name": entry.meta.name, "path": str(source)},
                1,
            )
        console.print(
            f"[dim]{gettext('Editing the original file (reference mode): %(path)s') % {'path': escape(str(source))}}[/dim]"
        )
        target = source
    else:
        target = entry.script_path
        if not target.exists():
            raise _fail(
                gettext("%(name)s has no stored copy to edit.") % {"name": entry.meta.name}, 1
            )
    try:
        editor.open_in_editor(target)
    except editor.EditorError as exc:
        raise _fail(str(exc), 1) from exc
    console.print(
        f"[green]{gettext('Saved %(name)s.') % {'name': escape(entry.meta.name)}}[/green]"
    )
    console.print(
        f"[dim]{gettext('skit reconciles parameter drift at run time; review managed parameters with: skit params %(name)s') % {'name': escape(entry.meta.name)}}[/dim]"
    )


# --------------------------------------------------------------------------
# run
# --------------------------------------------------------------------------


def _validate_preset(entry: store.Entry, preset: str | None) -> None:
    if not preset:
        return
    presets = argstate.load_state(entry.slug)["presets"]
    if preset not in presets:
        err_console.print(
            "[red]"
            + gettext('Unknown preset "%(preset)s". Available: %(presets)s')
            % {
                "preset": escape(preset),
                "presets": ", ".join(escape(p) for p in sorted(presets)) or "—",
            }
            + _RED_CLOSE
        )
        raise typer.Exit(EXIT_USAGE)


def _print_drift(plan: flows.FormPlan) -> None:
    for line in plan.drift_lines:
        err_console.print(f"[yellow]{escape(line)}[/yellow]")


def _parse_set_opts(plan: flows.FormPlan, raw: list[str]) -> dict[str, str]:
    """--set NAME=VALUE overrides: parsed strictly (never guess). A malformed item or an
    unknown name is a usage error (exit 2); a value that fails its field's own validation
    is a skit failure (exit 125), like any other bad value."""
    pairs: dict[str, str] = {}
    bad: list[str] = []
    for item in raw:
        key, sep, value = item.partition("=")
        if sep and key.strip():
            pairs[key.strip()] = value
        else:
            bad.append(item)
    if bad:
        err_console.print(
            "[red]"
            + gettext("Malformed --set (expected NAME=VALUE): %(items)s")
            % {"items": ", ".join(escape(b) for b in bad)}
            + _RED_CLOSE
        )
        raise typer.Exit(EXIT_USAGE)
    fields_by_key = {f.key: f for f in plan.fields}
    unknown = sorted(k for k in pairs if k not in fields_by_key)
    if unknown:
        err_console.print(
            "[red]"
            + gettext("Unknown parameter for --set: %(names)s. This script's parameters: %(valid)s")
            % {
                "names": ", ".join(escape(k) for k in unknown),
                "valid": ", ".join(escape(k) for k in sorted(fields_by_key)) or "—",
            }
            + _RED_CLOSE
        )
        raise typer.Exit(EXIT_USAGE)
    for key, value in pairs.items():
        error = flows.validate_value(fields_by_key[key], value)
        if error:
            raise _fail(error, EXIT_SKIT)
    return pairs


def _collect_values(
    entry: store.Entry, plan: flows.FormPlan, prefill: dict[str, str], *, plain: bool
) -> dict[str, str]:
    """Interactive collection through the configured renderer. The inline mini-form is
    the default; "plain" (--plain / form=plain / TERM=dumb) is the line-prompt fallback."""
    style = "plain" if plain or os.environ.get("TERM") == "dumb" else config.load_form()
    if style == "tui":
        import importlib

        try:  # the inline renderer ships with the TUI layer; degrade to plain without it
            inlineform = importlib.import_module("skit.inlineform")
        except ImportError:  # pragma: no cover — transitional
            pass
        else:
            values = inlineform.collect(entry, plan, prefill)
            if values is None:
                raise typer.Exit(EXIT_CANCELLED)  # cancelling is not a skit failure
            return values
    console.print(
        gettext("Parameters for %(name)s (press Enter to keep the value shown):")
        % {"name": escape(entry.meta.name)}
    )
    return promptform.collect(plan, prefill, console=console)


# How a flows.RunOutcome failure maps to skit's exit-code contract (docker convention).
# The numbers live in flows so the TUI's exit-after-run path shares them.
_FAILURE_EXIT = flows.FAILURE_EXIT_CODES


@app.command(
    help=gettext("Run a registered script or command in the terminal."),
    epilog=gettext(
        "Examples:  skit run stitch  ·  skit run stitch -p web -- extra.png  ·  skit run stitch --set width=800 --no-input  ·  skit run stitch --dry-run"
    ),
)
def run(
    name: str = _SCRIPT_ARG,
    args: list[str] = typer.Argument(
        None, help=gettext("Arguments passed through to the script (after --)")
    ),
    no_input: bool = typer.Option(
        False, "--no-input", help=gettext("Never prompt; reuse last values and defaults")
    ),
    preset: str = typer.Option(
        None,
        "--preset",
        "-p",
        help=gettext("Named preset of parameter values to prefill the form with"),
        autocompletion=_complete_preset,
    ),
    set_opts: list[str] = typer.Option(
        None,
        "--set",
        help=gettext(
            "Set a parameter value by name, as NAME=VALUE (repeatable; values may use tokens like {cwd} or {env:VAR}; the form no longer asks for a field you set)"
        ),
    ),
    save_preset: str = typer.Option(
        None, "--save-preset", help=gettext("Save this run's values as a named preset")
    ),
    plain: bool = typer.Option(
        False, "--plain", help=gettext("Line-by-line prompts instead of the inline form")
    ),
    raw: bool = typer.Option(
        False,
        "--raw",
        help=gettext(
            "Skip the parameter form and injection and run the script as-is (escape hatch)"
        ),
    ),
    dry_run: bool = typer.Option(
        False,
        "--dry-run",
        help=gettext(
            "Print the exact command that would run (tokens and globs expanded), then exit"
        ),
    ),
) -> None:
    """Run a script (straight through the terminal). skit's own failures exit 125/126/127;
    the script's exit code passes through untouched."""
    try:
        entry = store.resolve(name)
    except store.NotFoundError as exc:
        raise _fail(str(exc), EXIT_NOT_FOUND) from exc
    _validate_preset(entry, preset)
    # Form-shaped flags contradict "as-is": refusing beats silently dropping a preset
    # (or persisting an empty one) the way a bare warning-less run would. Checked before
    # any raw-mode chatter so the refusal is the whole story.
    if raw and (set_opts or preset or save_preset):
        err_console.print(
            f"[red]{gettext('--raw runs the script as-is; --set, --preset, and --save-preset do not apply.')}[/red]"
        )
        raise typer.Exit(EXIT_USAGE)
    run_spec = spec_for(entry.meta.kind)
    if raw and run_spec is not None and run_spec.analyzer is not None:
        console.print(
            f"[dim]{gettext('Raw mode: skipping the parameter form and injection.')}[/dim]"
        )
        plan = flows.FormPlan(source="none")
    else:
        plan = flows.plan_for_entry(entry)
    _print_drift(plan)
    if plan.degraded_reason:
        console.print(
            f"[dim]{gettext("skit could not model this script's own arguments; pass them after -- instead.")}[/dim]"
        )
    prefilled = flows.prefill(plan, entry.slug, preset)
    overrides = _parse_set_opts(plan, set_opts or [])
    prefilled.update(overrides)
    extra = list(args or [])
    # Both ends must be a terminal: `skit run x | tee log` has a tty stdin but would
    # pump the inline form's ANSI straight into the pipe.
    interactive = not no_input and _is_interactive()
    # An explicitly --set field is final — the form only asks for the rest (and a secret
    # set this way is actually used; the prompt renderers never echo a secret prefill).
    remaining = [f for f in plan.fields if f.key not in overrides]
    if interactive and remaining:
        ask_plan = dataclasses.replace(plan, fields=remaining)
        values = {**prefilled, **_collect_values(entry, ask_plan, prefilled, plain=plain)}
    else:
        values = prefilled
        errors = flows.validate(plan, values)
        # Passthrough args are the legitimate manual escape (skit run x -- <args>):
        # when the user supplies them, the script's own parser is in charge and an
        # unfilled required *field* is not a hole to refuse over.
        if errors and not extra:
            for msg in errors.values():
                err_console.print(f"[red]{escape(msg)}[/red]")
            raise typer.Exit(EXIT_SKIT)
    # --raw promises "run the script as-is": replaying a previous run's arguments would
    # betray exactly the clean slate it exists to provide (and the Agent Skill documents).
    # takes_argv: a command template's "arguments" are its placeholders, so replaying a
    # remembered argv tail there would be surprising rather than helpful.
    if not extra and not raw and run_spec is not None and run_spec.takes_argv:
        last_extra = argstate.load_state(entry.slug)["extra_args"]
        if last_extra:
            extra = last_extra
            # stderr, like the drift banner: skit chrome must not pollute the script's
            # own stdout, and agents watch stderr for skit-side signals (SKILL.md).
            err_console.print(
                f"[dim]{gettext('Reusing your last arguments: %(args)s') % {'args': ' '.join(escape(a) for a in extra)}}[/dim]"
            )
    try:
        asm = flows.assemble(plan, values, extra, cwd=Path.cwd(), expand_extra=False)
    except flows.FormError as exc:
        raise _fail(str(exc), EXIT_SKIT) from exc
    if save_preset:
        argstate.save_preset(
            entry.slug,
            save_preset,
            {k: v for k, v in values.items() if v},
            secret_names=plan.secret_names,
        )
        console.print(
            f"[green]{gettext('Preset "%(preset)s" saved for %(name)s.') % {'preset': escape(save_preset), 'name': escape(entry.meta.name)}}[/green]"
        )
    if dry_run:
        # No temp copy is written for a dry run, so the command line shows the original
        # script path — the shape, not a doomed-to-be-deleted temp file.
        for line in flows.transparency_lines(entry, asm, None):
            console.print(f"[dim]{escape(line)}[/dim]")
        raise typer.Exit(0)
    outcome = flows.execute(
        entry, plan, asm, emit=lambda line: console.print(f"[dim]{escape(line)}[/dim]")
    )
    code = outcome.code
    if code is None:
        raise _fail(outcome.message, _FAILURE_EXIT[outcome.failure])
    if raw:
        # The escape hatch leaves no fingerprints: it consulted no form memory, so it
        # must not rewrite it either (values/extra args survive for the next real run).
        # The run stamp still lands — Library sorting and `r` treat it as a run.
        argstate.record_run(entry.slug, code, at=models.now_iso())
    else:
        flows.save_after_run(entry.slug, plan, values, extra, code, at=models.now_iso())
    if code != 0:
        err_console.print(
            f"[yellow]{gettext('Script exited with code %(code)s') % {'code': code}}[/yellow]"
        )
    raise typer.Exit(code)


# --------------------------------------------------------------------------
# preset
# --------------------------------------------------------------------------

preset_app = typer.Typer(
    help=gettext("Manage named parameter presets for a script."), no_args_is_help=True
)
app.add_typer(preset_app, name="preset")


@preset_app.command("save", help=gettext("Save a set of parameter values as a named preset."))
def preset_save(
    name: str = _SCRIPT_ARG,
    preset_name: str = typer.Argument(..., help=gettext("Preset name")),
    from_last: bool = typer.Option(
        False,
        "--from-last",
        help=gettext("Save the last run's values without asking (automation-friendly)"),
    ),
) -> None:
    """Save a named preset: interactively, or straight from the last run with --from-last.
    Secret values are never persisted (C3)."""
    try:
        entry = store.resolve(name)
    except store.NotFoundError as exc:
        raise _fail(str(exc), 1) from exc
    plan = flows.plan_for_entry(entry)
    if not plan.fields:
        raise _fail(
            gettext("%(name)s has no form fields, so there's nothing to save.")
            % {"name": entry.meta.name},
            1,
        )
    if from_last:
        last = argstate.load_state(entry.slug)["values"]
        keys = {f.key for f in plan.fields}
        values = {k: v for k, v in last.items() if k in keys}
        if not values:
            raise _fail(
                gettext("%(name)s has no remembered values yet — run it once first.")
                % {"name": entry.meta.name},
                1,
            )
    elif sys.stdin.isatty():
        values = promptform.collect(plan, flows.prefill(plan, entry.slug), console=console)
    else:
        # Non-interactive contract: don't prompt — save what the prefill already knows.
        values = flows.prefill(plan, entry.slug)
    secret_overlap = plan.secret_names & values.keys()
    if secret_overlap:
        console.print(
            "[dim]"
            + gettext("Secret values are never stored in presets; skipped: %(names)s")
            % {"names": ", ".join(escape(n) for n in sorted(secret_overlap))}
            + _DIM_CLOSE
        )
    argstate.save_preset(entry.slug, preset_name, values, secret_names=plan.secret_names)
    console.print(
        f"[green]{gettext('Preset "%(preset)s" saved for %(name)s.') % {'preset': escape(preset_name), 'name': escape(entry.meta.name)}}[/green]"
    )


@preset_app.command("list", help=gettext("List a script's saved presets."))
def preset_list(
    name: str = _SCRIPT_ARG,
    as_json: bool = typer.Option(False, "--json", help=gettext("Output as JSON")),
) -> None:
    """List a script's named presets."""
    try:
        entry = store.resolve(name)
    except store.NotFoundError as exc:
        raise _fail(str(exc), 1) from exc
    presets = argstate.load_state(entry.slug)["presets"]
    if as_json:
        console.print_json(json.dumps(presets, ensure_ascii=False))
        return
    if not presets:
        console.print(
            gettext(
                "No presets for %(name)s yet. Create one with: skit run %(name)s --save-preset <preset>"
            )
            % {"name": escape(entry.meta.name)}
        )
        return
    for pname, vals in sorted(presets.items()):
        pairs = ", ".join(f"{escape(k)}={escape(v)}" for k, v in vals.items())
        console.print(f"  [bold]{escape(pname)}[/bold]: {pairs}")


@preset_app.command("delete", help=gettext("Delete a named preset from a script."))
def preset_delete(
    name: str = _SCRIPT_ARG,
    preset_name: str = typer.Argument(..., help=gettext("Preset name")),
) -> None:
    """Delete a named preset."""
    try:
        entry = store.resolve(name)
    except store.NotFoundError as exc:
        raise _fail(str(exc), 1) from exc
    if argstate.delete_preset(entry.slug, preset_name):
        console.print(
            gettext('Preset "%(preset)s" deleted from %(name)s.')
            % {"preset": escape(preset_name), "name": escape(entry.meta.name)}
        )
    else:
        err_console.print(
            "[red]"
            + gettext('Unknown preset "%(preset)s". Available: %(presets)s')
            % {
                "preset": escape(preset_name),
                "presets": ", ".join(
                    escape(p) for p in sorted(argstate.load_state(entry.slug)["presets"])
                )
                or "—",
            }
            + _RED_CLOSE
        )
        raise typer.Exit(1)


# --------------------------------------------------------------------------
# params
# --------------------------------------------------------------------------


def _secret_cell(s: metawriter.ParamSpec) -> str:
    """The Secret column: "—", "yes", or "yes ← $ENVVAR" when an env source is set."""
    if not s.secret:
        return "—"
    if s.env_source:
        return gettext("yes") + f" ← ${escape(s.env_source)}"
    return gettext("yes")


def _show_params(entry: store.Entry, as_json: bool) -> None:
    """Read view: managed parameters + last values + detected-but-unmanaged candidates."""
    last = argstate.load_state(entry.slug)["values"]
    entry_spec = spec_for(entry.meta.kind)
    specs: list[metawriter.ParamSpec] = []
    text = ""
    if entry_spec is not None and entry_spec.params_io is not None and entry.script_path.exists():
        text = entry.script_path.read_text(encoding="utf-8", errors="replace")  # pragma: no mutate
        specs = entry_spec.params_io.read(text)
    unmanaged: list[str] = []
    if (
        entry_spec is not None
        and entry_spec.analyzer is not None
        and entry.meta.mode == "copy"
        and text
    ):
        report = entry_spec.analyzer.reconcile(text, specs)
        unmanaged = [c.name for c in report.new]
    if as_json:
        payload = {
            "params": [s.to_dict() for s in specs],
            "unmanaged": unmanaged,
            "placeholders": entry.meta.params or [],
        }
        console.print_json(json.dumps(payload, ensure_ascii=False))
        return
    if entry_spec is not None and entry_spec.family == "template":
        placeholders = entry.meta.params or []
        if not placeholders:
            console.print(
                escape(gettext("%(name)s has no managed parameters.") % {"name": entry.meta.name})
            )
            return
        console.print(gettext("Command template placeholders (the run form asks for them):"))
        for p in placeholders:
            shown = last.get(p, "—")
            console.print(f"  {escape(p)} = {escape(shown)}")
        return
    if not specs:
        if entry_spec is None or entry_spec.analyzer is None:
            # No analyzer means --manage can't do anything for this kind — suggesting it
            # would send the user down a dead end (`skit params <exe> --manage X` errors).
            console.print(
                escape(gettext("%(name)s has no managed parameters.") % {"name": entry.meta.name})
            )
            return
        console.print(
            escape(
                gettext(
                    "%(name)s has no managed parameters. Use --manage to bring a detected candidate under management."
                )
                % {"name": entry.meta.name}
            )
        )
    else:
        from rich.table import Table

        table = Table(show_header=True, header_style="bold")  # pragma: no mutate — cosmetic
        table.add_column(gettext("Parameter"))
        table.add_column(gettext("Kind"))
        table.add_column(gettext("Type"))
        table.add_column(gettext("Default"))
        table.add_column(gettext("Secret"))
        table.add_column(gettext("Last value"))
        for s in specs:
            if s.secret:
                last_shown = gettext("•••") if s.name in last else "—"
            else:
                last_shown = last.get(s.name, "—")
            default_shown = (
                gettext("•••")
                if s.secret and s.default is not None
                else ("—" if s.default is None else str(s.default))
            )
            table.add_row(
                escape(s.name),
                escape(s.kind),
                escape(s.type),
                escape(default_shown),
                _secret_cell(s),
                escape(str(last_shown)),
            )
        console.print(table)
    if unmanaged:
        console.print(
            gettext("Detected but not yet managed: %(names)s (use --manage to manage them)")
            % {"names": ", ".join(escape(n) for n in unmanaged)}
        )


def _parse_kv_opts(raw: list[str], flag: str) -> tuple[dict[str, str], list[str]]:
    """Parse NAME=value pairs; malformed entries are collected for a warning."""
    pairs: dict[str, str] = {}
    bad: list[str] = []
    for item in raw:
        if "=" in item:
            key, _, value = item.partition("=")
            if key.strip():
                pairs[key.strip()] = value
                continue
        bad.append(f"{flag}: {item}")
    return pairs, bad


@app.command(
    help=gettext("Show or edit a script's managed parameters."),
    epilog=gettext(
        "Examples:  skit params resize --manage WIDTH  ·  skit params api --secret KEY --env-source KEY=OPENAI_API_KEY"
    ),
)
def params(
    name: str = _SCRIPT_ARG,
    resync: bool = typer.Option(
        False,
        "--resync",
        help=gettext("Prune definitions that no longer match the script and refresh changed types"),
    ),
    manage: list[str] = typer.Option(
        None,
        "--manage",
        help=gettext("Bring a currently detected candidate under management (repeatable)"),
    ),
    unmanage: list[str] = typer.Option(
        None, "--unmanage", help=gettext("Drop a managed parameter (repeatable)")
    ),
    secret: list[str] = typer.Option(
        None, "--secret", help=gettext("Mark a managed parameter as secret (repeatable)")
    ),
    no_secret: list[str] = typer.Option(
        None,
        "--no-secret",
        help=gettext("Remove the secret mark from a managed parameter (repeatable)"),
    ),
    prompt: list[str] = typer.Option(
        None, "--prompt", help=gettext("Set a parameter's form prompt, as NAME=text (repeatable)")
    ),
    env_source: list[str] = typer.Option(
        None,
        "--env-source",
        help=gettext(
            "Read a secret parameter from an environment variable at run time, as NAME=ENVVAR (empty ENVVAR clears it; repeatable)"
        ),
    ),
    as_json: bool = typer.Option(False, "--json", help=gettext("Output the read view as JSON")),
) -> None:
    """Show a script's managed parameters, or edit their definitions when any change flag
    is given. Definitions travel with the file; values live in central state; secret
    values are never shown or persisted."""
    try:
        entry = store.resolve(name)
    except store.NotFoundError as exc:
        raise _fail(str(exc), 1) from exc
    prompts, bad_prompts = _parse_kv_opts(prompt or [], "--prompt")
    env_sources, bad_env = _parse_kv_opts(env_source or [], "--env-source")
    if (
        resync
        or manage
        or unmanage
        or secret
        or no_secret
        or prompts
        or env_sources
        or bad_prompts
        or bad_env
    ):
        _edit_params(
            entry,
            resync=resync,
            manage=manage or [],
            unmanage=unmanage or [],
            secret=secret or [],
            no_secret=no_secret or [],
            prompts=prompts,
            env_sources=env_sources,
            malformed=bad_prompts + bad_env,
        )
    else:
        _show_params(entry, as_json)


def _apply_env_sources(specs: list[metawriter.ParamSpec], env_sources: dict[str, str]) -> list[str]:
    """Set/clear env_source on secret specs; returns warnings for unusable requests."""
    warnings: list[str] = []
    by_name = {s.name: s for s in specs}
    for pname, envvar in env_sources.items():
        spec = by_name.get(pname)
        if spec is None:
            warnings.append(
                gettext("%(name)s isn't a managed parameter; --env-source skipped.")
                % {"name": pname}
            )
            continue
        if not spec.secret:
            warnings.append(
                gettext(
                    "%(name)s isn't secret; --env-source only applies to secret parameters (mark it with --secret first)."
                )
                % {"name": pname}
            )
            continue
        spec.env_source = envvar.strip()
    return warnings


def _edit_params(
    entry: store.Entry,
    *,
    resync: bool,
    manage: list[str],
    unmanage: list[str],
    secret: list[str],
    no_secret: list[str],
    prompts: dict[str, str],
    env_sources: dict[str, str],
    malformed: list[str],
) -> None:
    """Apply parameter-definition changes to a copy-mode Python entry (rewrites [tool.skit])."""
    entry_spec = spec_for(entry.meta.kind)
    if entry_spec is None or entry_spec.params_io is None or entry_spec.analyzer is None:
        raise _fail(
            gettext("%(name)s isn't a Python script; only Python entries have managed parameters.")
            % {"name": entry.meta.name},
            1,
        )
    if entry.meta.mode == "reference":
        raise _fail(
            gettext(
                "%(name)s is in reference mode, and skit never writes the original file. "
                "Edit the [tool.skit] block in the source directly."
            )
            % {"name": entry.meta.name},
            1,
        )
    copy_path = entry.script_path
    if not copy_path.exists():
        raise _fail(gettext("%(name)s has no stored copy to edit.") % {"name": entry.meta.name}, 1)
    text = copy_path.read_text(encoding="utf-8", errors="replace")  # pragma: no mutate
    current = entry_spec.params_io.read(text)
    for item in malformed:
        err_console.print(
            f"[yellow]{escape(gettext('Ignored a malformed value: %(item)s (expected NAME=text).') % {'item': item})}[/yellow]"
        )
    result = reconcile.edit_specs(
        text,
        current,
        resync=resync,
        add=manage,
        remove=unmanage,
        secret=secret,
        no_secret=no_secret,
        prompts=prompts,
    )
    for w in result.warnings:
        err_console.print(f"[yellow]{escape(reconcile.render_warning(w))}[/yellow]")
    for w in _apply_env_sources(result.specs, env_sources):
        err_console.print(f"[yellow]{escape(w)}[/yellow]")
    new_text = metawriter.write_params(text, result.specs)
    copy_path.write_text(new_text, encoding="utf-8")  # pragma: no mutate — utf-8 equivalence
    secret_now = {s.name for s in result.specs if s.secret}
    purged = argstate.purge_secret(entry.slug, secret_now)
    if purged:
        console.print(
            "[dim]"
            + gettext(
                "Removed previously stored plaintext value(s) for now-secret parameter(s): %(names)s"
            )
            % {"names": ", ".join(escape(n) for n in sorted(purged))}
            + _DIM_CLOSE
        )
    remaining = ", ".join(escape(s.name) for s in result.specs) or "—"
    console.print(
        f"[green]{gettext('Updated %(name)s. Managed parameters: %(names)s') % {'name': escape(entry.meta.name), 'names': remaining}}[/green]"
    )


# --------------------------------------------------------------------------
# deps
# --------------------------------------------------------------------------


@app.command(
    help=gettext("View or update a script's dependencies and Python constraint."),
    epilog=gettext(
        'Examples:  skit deps tool --dep "requests>=2,<3" --dep rich  ·  skit deps tool --clear'
    ),
)
def deps(
    name: str = _SCRIPT_ARG,
    dep: list[str] = typer.Option(
        None, "--dep", help=gettext("A dependency (repeat for more; replaces the whole list)")
    ),
    clear: bool = typer.Option(False, "--clear", help=gettext("Remove every dependency")),
    python: str = typer.Option(
        None, "--python", help=gettext('Python version constraint, e.g. ">=3.11"')
    ),
    as_json: bool = typer.Option(False, "--json", help=gettext("Output as JSON")),
) -> None:
    """View or update a script's recorded dependencies."""
    try:
        entry = store.resolve(name)
    except store.NotFoundError as exc:
        raise _fail(str(exc), 1) from exc
    deps_spec = spec_for(entry.meta.kind)
    if deps_spec is None or not deps_spec.supports_deps:
        raise _fail(
            gettext("%(name)s is not a Python script entry.") % {"name": entry.meta.name}, 1
        )
    if dep and clear:
        err_console.print(
            f"[red]{gettext('Use --dep to set the list or --clear to empty it — not both.')}[/red]"
        )
        raise typer.Exit(EXIT_USAGE)
    if dep is None and not clear and python is None:
        current = entry.meta.dependencies or []
        if as_json:
            console.print_json(
                json.dumps(
                    {"dependencies": current, "requires_python": entry.meta.requires_python},
                    ensure_ascii=False,
                )
            )
            return
        console.print(
            gettext("Dependencies of %(name)s: %(deps)s")
            % {
                "name": escape(entry.meta.name),
                "deps": ", ".join(escape(d) for d in current) or "—",
            }
        )
        if entry.meta.requires_python:
            console.print(
                gettext("Python constraint: %(python)s")
                % {"python": escape(entry.meta.requires_python)}
            )
        return
    if clear:
        new_deps: list[str] = []
    elif dep is not None:
        new_deps = list(dep)
    else:
        new_deps = list(entry.meta.dependencies or [])
    try:
        entry = store.update_dependencies(entry.slug, new_deps, requires_python=python)
    except store.StoreError as exc:
        raise _fail(str(exc), 1) from exc
    console.print(
        f"[green]{gettext('Dependencies of %(name)s updated: %(deps)s') % {'name': escape(entry.meta.name), 'deps': ', '.join(escape(d) for d in new_deps) or '—'}}[/green]"
    )


# --------------------------------------------------------------------------
# agent — connect skit to AI coding agents
# --------------------------------------------------------------------------

agent_app = typer.Typer(
    help=gettext("Connect skit to AI agents: install the official Agent Skill."),
    no_args_is_help=True,
)
app.add_typer(agent_app, name="agent")


def _agent_install_confirmed(skills_dir: Path) -> None:
    # Read the bundled skill BEFORE the write-error wrap: a broken installation (skill
    # missing from the package) must fail loudly, not as "could not write there".
    text = agentskill.skill_text()
    try:
        written = agentskill.install_into(skills_dir, text)
    except OSError as exc:
        # e.g. --to points at an existing file: a clean one-liner, not a traceback.
        raise _fail(
            gettext("Could not write the skill there: %(error)s") % {"error": exc}, 1
        ) from exc
    console.print(
        f"[green]{gettext('Installed the skit Agent Skill: %(path)s') % {'path': escape(str(written))}}[/green]"
    )


def _agent_pick_target(candidates: list[agentskill.Target]) -> agentskill.Target | None:
    """Numbered picker + confirmation; None means the user backed out."""
    scope_names = {"user": gettext("user"), "project": gettext("project")}
    console.print(gettext("Agent directories on this machine:"))
    for i, t in enumerate(candidates, start=1):
        console.print(f"  {i}. {t.name} ({scope_names[t.scope]})  →  {escape(str(t.skills_dir))}")
    choice = Prompt.ask(
        gettext("Install where?"),
        choices=[str(i) for i in range(1, len(candidates) + 1)],
        default="1",
        console=console,
    )
    target = candidates[int(choice) - 1]
    if not Confirm.ask(
        gettext("Write the skill into %(path)s?") % {"path": escape(str(target.skills_dir))},
        default=True,
        console=console,
    ):
        return None
    return target


@agent_app.command(
    "install",
    help=gettext("Install skit's Agent Skill into an AI agent's skills directory."),
    epilog=gettext(
        "Examples:  skit agent install  ·  skit agent install claude  ·  skit agent install codex --project  ·  skit agent install --to ~/.claude/skills"
    ),
)
def agent_install(
    target: str = typer.Argument(
        None,
        help=gettext(
            "Where to install: claude, codex, or agents (the cross-agent ./.agents directory)"
        ),
    ),
    to: Path = typer.Option(
        None,
        "--to",
        help=gettext("Install into this skills directory instead of a named target"),
    ),
    project: bool = typer.Option(
        False,
        "--project",
        help=gettext(
            "Install into the current project (./.claude, ./.codex) instead of your home directory"
        ),
    ),
) -> None:
    """Teach the user's AI agents to use skit. An explicit TARGET or --to is consent by
    itself; bare `skit agent install` detects agent directories and asks (principle #6:
    skit never touches another tool's directory uninvited)."""
    if to is not None and (target or project):
        err_console.print(
            f"[red]{gettext('Use a named target (with optional --project) or --to — not both.')}[/red]"
        )
        raise typer.Exit(EXIT_USAGE)
    if to is not None:
        _agent_install_confirmed(to.expanduser())
        return
    home, cwd = agentskill.default_roots()
    if target:
        resolved = agentskill.named_target(target, project=project, home=home, cwd=cwd)
        if resolved is None:
            err_console.print(
                "[red]"
                + gettext("Unknown target %(name)s. Valid targets: claude, codex, agents.")
                % {"name": escape(target)}
                + _RED_CLOSE
            )
            raise typer.Exit(EXIT_USAGE)
        _agent_install_confirmed(resolved.skills_dir)
        return
    if not _is_interactive():
        err_console.print(
            f"[red]{gettext('Nothing installed: name a target (claude, codex, agents) or pass --to DIR.')}[/red]"
        )
        raise typer.Exit(EXIT_USAGE)
    candidates = agentskill.detect_targets(home=home, cwd=cwd)
    if not candidates:
        raise _fail(
            gettext(
                "No agent directories detected (~/.claude, ~/.codex, ./.agents, …). Pass --to DIR to choose one yourself."
            ),
            1,
        )
    picked = _agent_pick_target(candidates)
    if picked is None:
        console.print(gettext("Cancelled — nothing was written."))
        raise typer.Exit(0)  # pragma: no mutate — Exit(0)/Exit(None) both mean a clean exit
    _agent_install_confirmed(picked.skills_dir)


# --------------------------------------------------------------------------
# doctor
# --------------------------------------------------------------------------


def _drifted_entries(entries: list[store.Entry]) -> list[str]:
    """The health check is the one place that batch-reconciles the whole library."""
    out: list[str] = []
    for e in entries:
        spec = spec_for(e.meta.kind)
        if (
            spec is None
            or spec.analyzer is None
            or spec.params_io is None
            or not e.script_path.exists()
        ):
            continue
        text = e.script_path.read_text(encoding="utf-8", errors="replace")  # pragma: no mutate
        specs = spec.params_io.read(text)
        if specs and spec.analyzer.reconcile(text, specs).has_drift:
            out.append(e.meta.name)
    return out


def _uv_required(entries: list[store.Entry]) -> bool:
    """Whether a missing uv should fail doctor's exit code. uv is what runs python
    entries, so it's required when any python entry exists — and also for an EMPTY
    library (a fresh install's doctor must still steer the user toward a working
    setup). A non-empty library made purely of exe/command entries runs fine without
    uv, and exiting 1 there sent automation chasing a phantom problem."""
    if not entries:
        return True
    return any(e.meta.kind == "python" for e in entries)


@app.command(help=gettext("Check that uv is available and the script store is intact."))
def doctor(
    rebuild: bool = typer.Option(
        False, "--rebuild", help=gettext("Rebuild the index from each script's meta.toml")
    ),
    as_json: bool = typer.Option(False, "--json", help=gettext("Output as JSON")),
) -> None:
    """Environment self-check (the CLI face of the TUI health-check screen)."""
    from .paths import scripts_dir

    uv = launcher.find_uv()
    if rebuild:
        count, problems = store.doctor_rebuild()
        console.print(
            f"[green]{ngettext('Index rebuilt: %(count)s entry', 'Index rebuilt: %(count)s entries', count) % {'count': count}}[/green]"
        )
        for p in problems:
            console.print(f"  [yellow]{escape(p)}[/yellow]")
    entries = store.list_entries()
    missing = [e.meta.name for e in entries if launcher.target_missing(e)]
    drifted = _drifted_entries(entries)
    mirror = config.load_mirror()
    location = scripts_dir()
    size = store.dir_size(location)
    if as_json:
        console.print_json(
            json.dumps(
                {
                    "uv": uv,
                    "entries": len(entries),
                    "missing": missing,
                    "drift": drifted,
                    "mirror": mirror.pypi if mirror.enabled else "off",
                    "location": str(location),
                    "size_bytes": size,
                },
                ensure_ascii=False,
            )
        )
        raise typer.Exit(0 if uv or not _uv_required(entries) else 1)
    if uv:
        console.print(f"[green]✓ {gettext('uv: %(path)s') % {'path': escape(uv)}}[/green]")
    else:
        console.print(
            f"[red]✗ {gettext('uv: not found. Install it from https://docs.astral.sh/uv/getting-started/installation/')}[/red]"
        )
    console.print(
        "✓ "
        + ngettext("%(count)s script registered", "%(count)s scripts registered", len(entries))
        % {"count": len(entries)}
    )
    for m in missing:
        console.print(
            f"  [yellow]⚠ {gettext('%(name)s: the launch target is gone from disk') % {'name': escape(m)}}[/yellow]"
        )
    for d in drifted:
        console.print(
            f"  [yellow]⚠ {gettext('%(name)s: form definitions are out of sync — run: skit params %(name)s --resync') % {'name': escape(d)}}[/yellow]"
        )
    if mirror.enabled:
        console.print("✓ " + gettext("Mirror: on (%(pypi)s)") % {"pypi": escape(mirror.pypi)})
    else:
        console.print("✓ " + gettext("Mirror: off"))
    console.print(
        gettext("Library: %(path)s (%(count)s · %(size)s)")
        % {"path": escape(str(location)), "count": len(entries), "size": store.human_size(size)}
    )
    raise typer.Exit(0 if uv or not _uv_required(entries) else 1)


# --------------------------------------------------------------------------
# config (git-config grammar: bare = list, KEY = read, KEY VALUE = write)
# --------------------------------------------------------------------------

_CONFIG_KEYS = ("lang", "editor", "mirror", "form", "after_run")


def _config_value(key: str) -> str:
    if key == "lang":
        override = config.load_config().get("language", "")
        if isinstance(override, str) and override:
            return override
        return gettext("auto (%(locale)s)") % {"locale": i18n.current_locale()}
    if key == "editor":
        return config.load_editor() or gettext("default ($VISUAL / $EDITOR)")
    if key == "mirror":
        m = config.load_mirror()
        return m.pypi if m.enabled else "off"
    if key == "form":
        return config.load_form()
    return config.load_after_run()  # "after_run" — _CONFIG_KEYS guards the key set


def _config_set(key: str, value: str) -> None:
    if key == "lang":
        if value.lower() != "auto" and not i18n.is_supported(value):
            err_console.print(
                f"[red]{gettext('Unknown language: %(tag)s. Available: %(locales)s') % {'tag': escape(value), 'locales': ', '.join(i18n.available_locales())}}[/red]"
            )
            raise typer.Exit(EXIT_USAGE)
        i18n.set_language("" if value.lower() == "auto" else value)
    elif key == "editor":
        config.save_editor(value)
    elif key == "mirror":
        if value == "off":
            config.disable()
        elif value in config.PYPI_PRESETS:
            config.save_mirror(config.preset(value))
        else:
            choices = ", ".join([*config.PYPI_PRESETS, "off"])
            err_console.print(
                f"[red]{gettext('Unknown mirror: %(name)s. Choose from: %(names)s') % {'name': escape(value), 'names': choices}}[/red]"
            )
            raise typer.Exit(EXIT_USAGE)
    elif key == "form":
        if value not in config.FORM_STYLES:
            err_console.print(
                f"[red]{gettext('Unknown form style: %(value)s. Choose from: tui, plain') % {'value': escape(value)}}[/red]"
            )
            raise typer.Exit(EXIT_USAGE)
        config.save_form(value)
    else:  # after_run
        if value not in config.AFTER_RUN_MODES:
            err_console.print(
                f"[red]{gettext('Unknown after-run behavior: %(value)s. Choose from: exit, stay') % {'value': escape(value)}}[/red]"
            )
            raise typer.Exit(EXIT_USAGE)
        config.save_after_run(value)


@app.command(
    "config",
    help=gettext("Read or set skit's settings (language, editor, mirror, form style, after-run)."),
    epilog=gettext(
        "Examples:  skit config  ·  skit config lang zh-TW  ·  skit config after_run stay"
    ),
)
def config_cmd(
    key: str = typer.Argument(
        None, help=gettext("Setting name: lang / editor / mirror / form / after_run")
    ),
    value: str = typer.Argument(
        None, help=gettext('New value (omit to read; lang also accepts "auto")')
    ),
    as_json: bool = typer.Option(False, "--json", help=gettext("Output as JSON")),
) -> None:
    """git-config grammar: bare `skit config` lists everything; `config KEY` reads one;
    `config KEY VALUE` writes one. The guided experience lives in the TUI (press ,)."""
    if key is None:
        if as_json:
            console.print_json(
                json.dumps({k: _config_value(k) for k in _CONFIG_KEYS}, ensure_ascii=False)
            )
            return
        for k in _CONFIG_KEYS:
            console.print(f"  {k:<8}{escape(_config_value(k))}")
        return
    if key not in _CONFIG_KEYS:
        err_console.print(
            f"[red]{gettext('Unknown setting: %(key)s. Available: %(keys)s') % {'key': escape(key), 'keys': ', '.join(_CONFIG_KEYS)}}[/red]"
        )
        raise typer.Exit(EXIT_USAGE)
    if value is None:
        console.print(escape(_config_value(key)))
        return
    _config_set(key, value)
    console.print(f"[green]{key} = {escape(_config_value(key))}[/green]")


# --------------------------------------------------------------------------
# first run (mirror offer for blocked networks; interactive TTY only)
# --------------------------------------------------------------------------


def _prompt_uv_binary(default: str) -> str:
    """Prompt for the uv-binary mirror URL, insisting on https:// (the binary is
    downloaded, chmod +x'd, and executed — an http:// mirror is a MITM->RCE vector)."""
    while True:
        value = Prompt.ask(gettext("uv binary mirror URL"), default=default, console=console)
        if value.startswith("https://"):
            return value
        err_console.print(
            "[red]"
            + gettext(
                "The uv binary is downloaded and executed, so its mirror URL must use https:// (got: %(url)s)."
            )
            % {"url": escape(value)}
            + _RED_CLOSE
        )


def _mirror_wizard() -> None:
    m = config.load_mirror()
    if not m.enabled:
        default = "off"
    else:
        default = next((k for k, v in config.PYPI_PRESETS.items() if v == m.pypi), "custom")
    choice = Prompt.ask(
        gettext("Mirror for faster installs in mainland China"),
        choices=[*config.PYPI_PRESETS, "custom", "off"],
        default=default,
        console=console,
    )
    if choice == "off":
        config.disable()
    elif choice == "custom":
        config.save_mirror(
            config.MirrorConfig(
                enabled=True,
                pypi=Prompt.ask(
                    gettext("PyPI index URL"),
                    default=m.pypi or config.PYPI_PRESETS["tsinghua"],
                    console=console,
                ),
                python_install=Prompt.ask(
                    gettext("Python-install mirror URL"),
                    default=m.python_install or config.PYTHON_INSTALL_MIRROR,
                    console=console,
                ),
                uv_binary=_prompt_uv_binary(m.uv_binary or config.UV_BINARY_MIRROR),
            )
        )
    else:
        config.save_mirror(config.preset(choice))


def _maybe_first_run_setup() -> None:
    """On the first bare `skit` run, offer mirror setup if the network to PyPI/GitHub looks
    blocked. Interactive TTY only; runs once (a [mirror] section marks the offer as done)."""
    if config.mirror_configured() or not _is_interactive():
        return
    if config.looks_blocked():
        console.print(gettext("Network to PyPI / GitHub looks slow or blocked."))
        if Confirm.ask(
            gettext("Configure mirrors for faster installs (mainland China)?"),
            default=True,
            console=console,
        ):
            _mirror_wizard()
    if not config.mirror_configured():
        config.save_mirror(config.load_mirror())  # persist a marker so we don't probe every run


if __name__ == "__main__":
    sys.exit(app())
