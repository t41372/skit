"""CLI entry point (Typer). Running `skit` with no subcommand opens the TUI main menu.

Every user-visible string goes through i18n.gettext()/ngettext(). Help strings are resolved at import time,
so i18n is lazily initialized when this module is imported (see i18n.py for the detection chain).
"""

from __future__ import annotations

import sys
from pathlib import Path

import typer
from rich.console import Console
from rich.markup import escape
from rich.prompt import Prompt
from rich.table import Table

from . import (
    __version__,
    analyzer,
    argstate,
    i18n,
    launcher,
    metawriter,
    pep723,
    reconcile,
    shim,
    store,
)
from .i18n import gettext, ngettext

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


@app.callback(invoke_without_command=True)
def main(
    ctx: typer.Context,
    version: bool = typer.Option(False, "--version", "-V", help=gettext("Show version")),
) -> None:
    if version:
        console.print(f"skit {__version__}")
        raise typer.Exit()
    if ctx.invoked_subcommand is None:
        from .tui import run_menu

        raise typer.Exit(run_menu())


def _resolve_python_metadata(
    text: str, deps_opt: str | None, python_opt: str | None, no_input: bool
) -> tuple[list[str], str]:
    """Decide the (dependencies, requires_python) to fill in.

    - Script already has a PEP 723 block: don't ask, don't fill (the block is the source of truth).
    - Explicit --deps / --python: use them directly, no prompting.
    - Interactive: only ask when the AST reveals likely third-party imports; ask nothing when
      there are no dependencies at all.
    """
    if pep723.has_block(text):
        meta = pep723.parse_block(text) or {}
        deps = meta.get("dependencies", [])
        if deps:
            console.print(gettext("PEP 723 metadata found: %(deps)s") % {"deps": ", ".join(deps)})
        return [], ""
    if deps_opt is not None or python_opt is not None:
        deps = [d.strip() for d in (deps_opt or "").split(",") if d.strip()]
        return deps, python_opt or ""
    suggested = pep723.suggest_dependencies(text)
    if not suggested:
        return [], ""  # No dependencies: nothing to ask
    if no_input or not sys.stdin.isatty():
        return suggested, ""  # Non-interactive: accept the suggestions as-is
    answer = Prompt.ask(
        gettext("Dependencies (comma separated; leave empty for none)"),
        default=", ".join(suggested),
        console=console,
    )
    deps = [d.strip() for d in answer.split(",") if d.strip()]
    py = Prompt.ask(
        gettext("Python version (leave empty for automatic)"), default="", console=console
    )
    return deps, py.strip()


def _spec_from_candidate(c: analyzer.Candidate) -> metawriter.ParamSpec:
    return metawriter.ParamSpec(
        name=c.name,
        kind=c.kind,
        type=c.type,
        default=c.default,
        prompt=c.prompt,
        order=c.order,
        secret=c.secret,
    )


def _parse_selection(answer: str, count: int) -> list[int]:
    """Parse an onboarding selection: 'all' / 'none' (or empty) / '1,3,5'.

    Invalid numbers are ignored.
    """
    answer = answer.strip().lower()
    if answer in ("none", ""):
        return []
    if answer == "all":
        return list(range(count))
    picked: list[int] = []
    for raw_part in answer.split(","):
        part = raw_part.strip()
        if part.isdigit() and 1 <= int(part) <= count and (int(part) - 1) not in picked:
            picked.append(int(part) - 1)
    return picked


def _onboard_params(text: str, script_name: str, no_input: bool) -> list[metawriter.ParamSpec]:
    """Parameter onboarding at add time (A4: which constant counts as a parameter is a UX call,
    so let the user choose).

    - If argparse/click/typer is detected: suggest the L1 pass-through + preset flow instead of
      injection-based management.
    - Non-interactive (--no-input / no tty): don't guess, don't select, return empty (honesty
      beats being clever).
    """
    result = analyzer.analyze(text)
    if result.uses_cli_framework:
        console.print(
            "[dim]"
            + gettext(
                "This script already parses its own arguments (%(names)s). Pass them straight through instead: skit run %(name)s -- <args>"
            )
            % {"names": ", ".join(result.frameworks), "name": script_name}
            + "[/dim]"
        )
        return []
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
    secret_mark = gettext(" (secret)")
    for i, c in enumerate(result.candidates, start=1):
        mark = secret_mark if c.secret else ""
        if c.kind == "const":
            console.print(
                "  "
                + gettext("%(num)s. %(name)s (%(type)s) = %(value)s%(secret)s")
                % {
                    "num": i,
                    "name": c.name,
                    "type": c.type,
                    "value": repr(c.default),
                    "secret": mark,
                }
            )
        else:
            console.print(
                "  "
                + gettext("%(num)s. input() #%(ordinal)s: %(prompt)s%(secret)s")
                % {"num": i, "ordinal": c.order + 1, "prompt": repr(c.prompt), "secret": mark}
            )
    answer = Prompt.ask(
        gettext("Which ones should skit manage? (e.g. 1,3 / all / none)"),
        default="all",
        console=console,
    )
    picked = _parse_selection(answer, len(result.candidates))
    return [_spec_from_candidate(result.candidates[i]) for i in picked]


@app.command()
def add(
    path: str = typer.Argument(None, help=gettext("Path to a Python script or executable")),
    name: str = typer.Option(
        None, "--name", "-n", help=gettext("Name / alias (defaults to the file name)")
    ),
    description: str = typer.Option(
        None,
        "--description",
        "-d",
        help=gettext("Description (defaults to the first line of the docstring)"),
    ),
    ref: bool = typer.Option(
        False,
        "--ref",
        help=gettext("Reference mode: link to the original file instead of copying it"),
    ),
    exe: bool = typer.Option(False, "--exe", help=gettext("Register as an executable entry")),
    cmd: str = typer.Option(
        None, "--cmd", help=gettext("Register a command template, e.g. --cmd 'ffmpeg -i {input}'")
    ),
    deps: str = typer.Option(
        None,
        "--deps",
        help=gettext("Dependencies, comma separated (skips the interactive question)"),
    ),
    python: str = typer.Option(
        None, "--python", help=gettext('Python version constraint, e.g. ">=3.11"')
    ),
    no_input: bool = typer.Option(
        False, "--no-input", help=gettext("Never prompt; accept the detected suggestions")
    ),
) -> None:
    """Add a script / executable / command to skit."""
    try:
        if cmd is not None:
            if not name:
                err_console.print(f"[red]{gettext('A --cmd entry needs a --name')}[/red]")
                raise typer.Exit(2)
            entry = store.add_command(cmd, name=name, description=description or "")
            if entry.meta.params:
                console.print(
                    gettext(
                        "Detected parameters: %(names)s (you'll be prompted on each run; your last values are remembered)"
                    )
                    % {"names": ", ".join(entry.meta.params)}
                )
        elif exe:
            if not path:
                err_console.print(f"[red]{gettext('--exe requires a path')}[/red]")
                raise typer.Exit(2)
            entry = store.add_exe(Path(path), name=name, description=description or "")
        else:
            if not path:
                err_console.print(
                    f"[red]{gettext('Provide a script path, or use --cmd to register a command template')}[/red]"
                )
                raise typer.Exit(2)
            p = Path(path)
            if p.suffix.lower() != ".py":
                err_console.print(
                    f"[yellow]{gettext("%(file)s isn't a .py file — pass --exe if it's an executable") % {'file': p.name}}[/yellow]"
                )
                raise typer.Exit(2)
            text = p.expanduser().resolve().read_text(encoding="utf-8", errors="replace")
            final_deps, final_py = _resolve_python_metadata(text, deps, python, no_input)
            entry = store.add_python(
                p,
                name=name,
                mode="reference" if ref else "copy",
                description=description,
                dependencies=final_deps or None,
                requires_python=final_py,
            )
            if final_deps:
                console.print(
                    gettext("Dependencies recorded: %(deps)s") % {"deps": ", ".join(final_deps)}
                )
            # Layer 2 onboarding: detect candidate parameters, then write the chosen definitions
            # into the copy's [tool.skit]. A5: comments only; A7: reference mode never writes the
            # original file — so don't ask, just skip and say so.
            if entry.meta.mode == "reference":
                console.print(
                    f"[dim]{gettext('Reference mode never touches the original file, so parameter setup was skipped.')}[/dim]"
                )
            else:
                params_specs = _onboard_params(text, entry.meta.name, no_input)
                if params_specs:
                    copy_path = entry.dir / "script.py"
                    current = copy_path.read_text(encoding="utf-8")
                    copy_path.write_text(
                        metawriter.write_params(current, params_specs), encoding="utf-8"
                    )
                    console.print(
                        "[green]"
                        + gettext(
                            "Parameter definitions written to the script's [tool.skit] block: %(names)s"
                        )
                        % {"names": ", ".join(s.name for s in params_specs)}
                        + "[/green]"
                    )
                    secrets = [s.name for s in params_specs if s.secret]
                    if secrets:
                        console.print(
                            f"[dim]{gettext('Secret parameter values are never saved to disk: %(names)s') % {'names': ', '.join(secrets)}}[/dim]"
                        )
    except store.StoreError as exc:
        err_console.print(f"[red]{exc}[/red]")
        raise typer.Exit(1) from exc
    mode_note = (
        gettext("(%(mode)s mode)") % {"mode": entry.meta.mode}
        if entry.meta.kind == "python"
        else ""
    )
    console.print(
        f"[green]{gettext('Added: %(name)s') % {'name': entry.meta.name}}[/green] {mode_note}"
    )
    if entry.meta.description:
        console.print(f"  {gettext('Description: %(desc)s') % {'desc': entry.meta.description}}")
    console.print(f"  {gettext('Run it: skit run %(name)s') % {'name': entry.meta.name}}")


@app.command("list")
def list_cmd(
    as_json: bool = typer.Option(False, "--json", help=gettext("Output as JSON")),
) -> None:
    """List every registered script."""
    entries = store.list_entries()
    if as_json:
        import json

        console.print_json(
            json.dumps(
                [
                    {
                        "name": e.meta.name,
                        "slug": e.slug,
                        "kind": e.meta.kind,
                        "mode": e.meta.mode,
                        "description": e.meta.description,
                    }
                    for e in entries
                ],
                ensure_ascii=False,
            )
        )
        return
    if not entries:
        console.print(gettext("No scripts yet. Add one with: skit add <path>"))
        return
    table = Table(show_header=True, header_style="bold")
    table.add_column(gettext("Name"))
    table.add_column(gettext("Kind"))
    table.add_column(gettext("Description"))
    for e in entries:
        table.add_row(e.meta.name, e.meta.kind, e.meta.description or "—")
    console.print(table)


@app.command()
def remove(
    name: str = typer.Argument(..., help=gettext("Script name or slug")),
    yes: bool = typer.Option(False, "--yes", "-y", help=gettext("Skip confirmation")),
) -> None:
    """Remove a script (copy mode deletes the copy in the store; the original is untouched)."""
    try:
        entry = store.resolve(name)
    except store.NotFoundError as exc:
        err_console.print(f"[red]{exc}[/red]")
        raise typer.Exit(1) from exc
    if not yes:
        typer.confirm(gettext('Remove "%(name)s"?') % {"name": entry.meta.name}, abort=True)
    removed = store.remove(name)
    console.print(f"[green]{gettext('Removed: %(name)s') % {'name': removed}}[/green]")


def _entry_param_specs(entry: store.Entry) -> list[metawriter.ParamSpec]:
    """The [tool.skit] parameter definitions for a python entry (other kinds return empty)."""
    if entry.meta.kind != "python" or not entry.script_path.exists():
        return []
    return metawriter.read_params(entry.script_path.read_text(encoding="utf-8", errors="replace"))


def _collect_command_values(
    entry: store.Entry, no_input: bool, preset: str | None
) -> dict[str, str]:
    """Fill placeholder values for a command entry.

    Default resolution: preset > last-used value. Interactive mode confirms each one;
    non-interactive reuses the recorded values as-is.
    """
    params = entry.meta.params or []
    if not params:
        return {}
    state = argstate.load_state(entry.slug)
    defaults = dict(state["values"])
    if preset:
        defaults.update(state["presets"].get(preset, {}))
    values: dict[str, str] = {}
    interactive = not no_input and sys.stdin.isatty()
    for p in params:
        default = defaults.get(p, "")
        if interactive:
            values[p] = Prompt.ask(f"  {p}", default=default or None, console=console) or ""
        elif default:
            # Non-interactive: only carry recorded values; leave missing ones for the launcher to
            # report via launch-err-missing-values — never silently assemble a broken command.
            values[p] = default
    return values


def _collect_param_form(
    entry: store.Entry,
    specs: list[metawriter.ParamSpec],
    no_input: bool,
    preset: str | None,
) -> dict[str, str]:
    """Pre-run parameter form (CLI version).

    Resolution order: this run's input > preset > last-used > the definition's default.
    """
    prefill = argstate.resolve_defaults(specs, entry.slug, preset)
    interactive = not no_input and sys.stdin.isatty()
    if not interactive:
        return prefill
    console.print(
        gettext("Parameters for %(name)s (press Enter to keep the value shown):")
        % {"name": entry.meta.name}
    )
    values: dict[str, str] = {}
    for s in specs:
        label = s.prompt or s.name
        default = prefill.get(s.name)
        if s.secret:
            answer = Prompt.ask(f"  {label}", password=True, console=console)
            values[s.name] = answer if answer else (default or "")
        else:
            values[s.name] = Prompt.ask(f"  {label}", default=default, console=console) or ""
    return values


def _reconciled_specs(
    entry: store.Entry, specs: list[metawriter.ParamSpec], text: str
) -> list[metawriter.ParamSpec]:
    """Pre-run reconciliation: warn on drift (old/new comparison), drop missing definitions from
    this form (injecting them would just throw the value into a black hole); changed ones only
    warn but are still injected.
    """
    report = reconcile.reconcile(text, specs)
    if report.has_drift:
        for line in reconcile.drift_lines(report, entry.meta.name):
            # The copy contains literal brackets like [tool.skit]; escape them so rich doesn't
            # swallow them as markup.
            err_console.print(f"[yellow]{escape(line)}[/yellow]")
    return report.usable


def _validate_preset(entry: store.Entry, preset: str | None) -> None:
    if not preset:
        return
    presets = argstate.load_state(entry.slug)["presets"]
    if preset not in presets:
        err_console.print(
            "[red]"
            + gettext('Unknown preset "%(preset)s". Available: %(presets)s')
            % {"preset": preset, "presets": ", ".join(sorted(presets)) or "—"}
            + "[/red]"
        )
        raise typer.Exit(2)


@app.command()
def run(
    name: str = typer.Argument(..., help=gettext("Script name or slug")),
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
    ),
    raw: bool = typer.Option(
        False,
        "--raw",
        help=gettext(
            "Skip the parameter form and injection and run the script as-is (escape hatch)"
        ),
    ),
) -> None:
    """Run a script (straight through the terminal)."""
    injected_path = None
    try:
        entry = store.resolve(name)
        _validate_preset(entry, preset)
        extra = list(args or [])
        # --raw escape hatch: skip the parameter form and injection, run the script as-is (a way
        # out if the injection engine ever misbehaves). This only affects the form/injection for
        # python entries; command entries still fill placeholders (that isn't injection).
        specs = [] if raw else _entry_param_specs(entry)
        text = ""
        if raw:
            console.print(
                f"[dim]{gettext('Raw mode: skipping the parameter form and injection.')}[/dim]"
            )
        if specs:
            # Reconcile the definitions against the script's current content (drop missing,
            # warn on changed) before showing the form.
            text = entry.script_path.read_text(encoding="utf-8", errors="replace")
            specs = _reconciled_specs(entry, specs, text)
        if specs:
            values = _collect_param_form(entry, specs, no_input, preset)
            # shim injection: the copy is never modified (A5); the injected artifact is a temp
            # file in the same directory.
            try:
                injected = shim.inject(text, specs, values)
            except shim.ShimError as exc:
                err_console.print(
                    f"[red]{gettext("Can't inject parameters into %(name)s: targets not found (%(detail)s). The script may have drifted from its [tool.skit] definitions — re-add it or edit the block.") % {'name': entry.meta.name, 'detail': str(exc)}}[/red]"
                )
                raise typer.Exit(1) from exc
            injected_path = shim.write_injected(entry.dir, injected)
            console.print(
                f"[dim]{gettext('Values are injected at run time; the stored copy of your script is never modified.')}[/dim]"
            )
        else:
            values = _collect_command_values(entry, no_input, preset)
        # python/exe: when no args are given, reuse the last ones (tell the user;
        # any new args override).
        if not extra and entry.meta.kind in ("python", "exe"):
            last_extra = argstate.load_last(entry.slug)["extra_args"]
            if last_extra:
                extra = last_extra
                console.print(
                    f"[dim]{gettext('Reusing your last arguments: %(args)s') % {'args': ' '.join(extra)}}[/dim]"
                )
        code = launcher.run_entry(entry, extra, values=values, script_override=injected_path)
    except (store.StoreError, launcher.LaunchError) as exc:
        err_console.print(f"[red]{exc}[/red]")
        raise typer.Exit(1) from exc
    finally:
        if injected_path is not None and injected_path.exists():
            injected_path.unlink(missing_ok=True)
    # Once the command assembles successfully, remember this run's input (used as next run's
    # default); secret values are structurally stripped (C3).
    secret_names = {s.name for s in specs if s.secret}
    argstate.save_last(
        entry.slug,
        values=values or None,
        extra_args=extra or None,
        secret_names=secret_names,
    )
    if code != 0:
        err_console.print(
            f"[yellow]{gettext('Script exited with code %(code)s') % {'code': code}}[/yellow]"
        )
    raise typer.Exit(code)


preset_app = typer.Typer(
    help=gettext("Manage named parameter presets for a script."), no_args_is_help=True
)
app.add_typer(preset_app, name="preset")


@preset_app.command("save")
def preset_save(
    name: str = typer.Argument(..., help=gettext("Script name or slug")),
    preset_name: str = typer.Argument(..., help=gettext("Preset name")),
) -> None:
    """Interactively fill a set of values and save them as a named preset (secret values are never
    persisted, C3)."""
    try:
        entry = store.resolve(name)
    except store.NotFoundError as exc:
        err_console.print(f"[red]{exc}[/red]")
        raise typer.Exit(1) from exc
    specs = _entry_param_specs(entry)
    if entry.meta.kind == "command":
        placeholders = entry.meta.params or []
        if not placeholders:
            err_console.print(
                f"[red]{gettext("%(name)s has no managed parameters or placeholders, so there's nothing to save.") % {'name': entry.meta.name}}[/red]"
            )
            raise typer.Exit(1)
        values: dict[str, str] = {}
        state = argstate.load_state(entry.slug)
        for p in placeholders:
            default = state["values"].get(p, "")
            values[p] = Prompt.ask(f"  {p}", default=default or None, console=console) or ""
        argstate.save_preset(entry.slug, preset_name, values)
        console.print(
            f"[green]{gettext('Preset "%(preset)s" saved for %(name)s.') % {'preset': preset_name, 'name': entry.meta.name}}[/green]"
        )
        return
    if not specs:
        err_console.print(
            f"[red]{gettext("%(name)s has no managed parameters or placeholders, so there's nothing to save.") % {'name': entry.meta.name}}[/red]"
        )
        raise typer.Exit(1)
    values = _collect_param_form(entry, specs, no_input=False, preset=None)
    secret_names = {s.name for s in specs if s.secret}
    if secret_names & values.keys():
        console.print(
            "[dim]"
            + gettext("Secret values are never stored in presets; skipped: %(names)s")
            % {"names": ", ".join(sorted(secret_names & values.keys()))}
            + "[/dim]"
        )
    argstate.save_preset(entry.slug, preset_name, values, secret_names=secret_names)
    console.print(
        f"[green]{gettext('Preset "%(preset)s" saved for %(name)s.') % {'preset': preset_name, 'name': entry.meta.name}}[/green]"
    )


@preset_app.command("list")
def preset_list(
    name: str = typer.Argument(..., help=gettext("Script name or slug")),
) -> None:
    """List a script's named presets."""
    try:
        entry = store.resolve(name)
    except store.NotFoundError as exc:
        err_console.print(f"[red]{exc}[/red]")
        raise typer.Exit(1) from exc
    presets = argstate.load_state(entry.slug)["presets"]
    if not presets:
        console.print(
            gettext(
                "No presets for %(name)s yet. Create one with: skit preset save %(name)s <preset>"
            )
            % {"name": entry.meta.name}
        )
        return
    for pname, vals in sorted(presets.items()):
        pairs = ", ".join(f"{k}={v}" for k, v in vals.items())
        console.print(f"  [bold]{pname}[/bold]: {pairs}")


@preset_app.command("delete")
def preset_delete(
    name: str = typer.Argument(..., help=gettext("Script name or slug")),
    preset_name: str = typer.Argument(..., help=gettext("Preset name")),
) -> None:
    """Delete a named preset."""
    try:
        entry = store.resolve(name)
    except store.NotFoundError as exc:
        err_console.print(f"[red]{exc}[/red]")
        raise typer.Exit(1) from exc
    if argstate.delete_preset(entry.slug, preset_name):
        console.print(
            gettext('Preset "%(preset)s" deleted from %(name)s.')
            % {"preset": preset_name, "name": entry.meta.name}
        )
    else:
        err_console.print(
            "[red]"
            + gettext('Unknown preset "%(preset)s". Available: %(presets)s')
            % {
                "preset": preset_name,
                "presets": ", ".join(sorted(argstate.load_state(entry.slug)["presets"])) or "—",
            }
            + "[/red]"
        )
        raise typer.Exit(1)


@app.command()
def params(
    name: str = typer.Argument(..., help=gettext("Script name or slug")),
) -> None:
    """Show a script's managed parameters (definitions travel with the file, values live in central
    state; secret values are never shown or persisted)."""
    try:
        entry = store.resolve(name)
    except store.NotFoundError as exc:
        err_console.print(f"[red]{exc}[/red]")
        raise typer.Exit(1) from exc
    last = argstate.load_last(entry.slug)["values"]
    if entry.meta.kind == "command":
        placeholders = entry.meta.params or []
        if not placeholders:
            console.print(
                gettext(
                    "%(name)s has no managed parameters. Re-add the script or edit its [tool.skit] block to define some."
                )
                % {"name": entry.meta.name}
            )
            return
        console.print(gettext("Command template placeholders (you'll be asked on each run):"))
        for p in placeholders:
            shown = last.get(p, "—")
            console.print(f"  {p} = {shown}")
        return
    specs: list[metawriter.ParamSpec] = []
    if entry.meta.kind == "python" and entry.script_path.exists():
        specs = metawriter.read_params(
            entry.script_path.read_text(encoding="utf-8", errors="replace")
        )
    if not specs:
        console.print(
            gettext(
                "%(name)s has no managed parameters. Re-add the script or edit its [tool.skit] block to define some."
            )
            % {"name": entry.meta.name}
        )
        return
    table = Table(show_header=True, header_style="bold")
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
            s.name,
            s.kind,
            s.type,
            default_shown,
            gettext("yes") if s.secret else "—",
            str(last_shown),
        )
    console.print(table)


def _parse_prompt_opts(raw: list[str]) -> tuple[dict[str, str], list[str]]:
    """Parse --prompt NAME=text into a dict; malformed entries are collected as warnings."""
    prompts: dict[str, str] = {}
    bad: list[str] = []
    for item in raw:
        if "=" in item:
            name, _, text = item.partition("=")
            if name.strip():
                prompts[name.strip()] = text
                continue
        bad.append(item)
    return prompts, bad


@app.command()
def edit(
    name: str = typer.Argument(..., help=gettext("Script name or slug")),
    resync: bool = typer.Option(
        False,
        "--resync",
        help=gettext("Prune definitions that no longer match the script and refresh changed types"),
    ),
    add: list[str] = typer.Option(
        None,
        "--add",
        help=gettext("Bring a currently detected candidate under management (repeatable)"),
    ),
    remove: list[str] = typer.Option(
        None, "--remove", help=gettext("Drop a managed parameter (repeatable)")
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
) -> None:
    """Edit a script's managed parameter definitions (rewrites the copy's [tool.skit] directly, so
    you never have to hand-edit TOML)."""
    try:
        entry = store.resolve(name)
    except store.NotFoundError as exc:
        err_console.print(f"[red]{exc}[/red]")
        raise typer.Exit(1) from exc
    if entry.meta.kind != "python":
        err_console.print(
            f"[red]{gettext("%(name)s isn't a Python script; only Python entries have managed parameters.") % {'name': entry.meta.name}}[/red]"
        )
        raise typer.Exit(1)
    if entry.meta.mode == "reference":
        # A7: in reference mode the definitions live with the original file, and skit never
        # writes the original.
        err_console.print(
            f"[red]{gettext('%(name)s is in reference mode, and skit never writes the original file. Edit the [tool.skit] block in the source directly.') % {'name': entry.meta.name}}[/red]"
        )
        raise typer.Exit(1)
    copy_path = entry.dir / "script.py"
    if not copy_path.exists():
        err_console.print(
            f"[red]{gettext('%(name)s has no stored copy to edit.') % {'name': entry.meta.name}}[/red]"
        )
        raise typer.Exit(1)
    text = copy_path.read_text(encoding="utf-8", errors="replace")
    current = metawriter.read_params(text)

    prompts, bad_prompts = _parse_prompt_opts(prompt or [])
    for item in bad_prompts:
        err_console.print(
            f"[yellow]{escape(gettext('Ignored a malformed --prompt value: %(item)s (expected NAME=text).') % {'item': item})}[/yellow]"
        )

    # No operation requested: show the current state (managed params + not-yet-managed candidates)
    # and hint at the available flags.
    no_ops = not (resync or add or remove or secret or no_secret or prompts)
    if no_ops:
        report = reconcile.reconcile(text, current)
        console.print(gettext("Managed parameters for %(name)s:") % {"name": entry.meta.name})
        if current:
            for s in current:
                mark = f" [{gettext('Secret').lower()}]" if s.secret else ""
                console.print(f"  {s.name} ({s.kind}:{s.type}){escape(mark)}")
        else:
            console.print(
                f"  {gettext('%(name)s has no managed parameters. Re-add the script or edit its [tool.skit] block to define some.') % {'name': entry.meta.name}}"
            )
        if report.new:
            names = ", ".join(c.name for c in report.new)
            console.print(
                gettext("Detected but not yet managed: %(names)s (use --add to manage them)")
                % {"names": names}
            )
        console.print(
            gettext(
                "Use --resync, --add, --remove, --secret/--no-secret, or --prompt NAME=text to make changes."
            )
        )
        raise typer.Exit(0)

    result = reconcile.edit_specs(
        text,
        current,
        resync=resync,
        add=add or [],
        remove=remove or [],
        secret=secret or [],
        no_secret=no_secret or [],
        prompts=prompts,
    )
    for w in result.warnings:
        err_console.print(f"[yellow]{escape(reconcile.render_warning(w))}[/yellow]")

    copy_path.write_text(metawriter.write_params(text, result.specs), encoding="utf-8")
    remaining = ", ".join(s.name for s in result.specs) or "—"
    console.print(
        f"[green]{gettext('Updated %(name)s. Managed parameters: %(names)s') % {'name': entry.meta.name, 'names': remaining}}[/green]"
    )


@app.command()
def deps(
    name: str = typer.Argument(..., help=gettext("Script name or slug")),
    set_deps: str = typer.Option(
        None,
        "--set",
        help=gettext("New dependency list, comma separated (an empty string clears it)"),
    ),
    python: str = typer.Option(
        None, "--python", help=gettext('Python version constraint, e.g. ">=3.11"')
    ),
) -> None:
    """View or update a script's recorded dependencies (copy mode syncs the PEP 723 block;
    reference mode only touches meta)."""
    try:
        entry = store.resolve(name)
    except store.NotFoundError as exc:
        err_console.print(f"[red]{exc}[/red]")
        raise typer.Exit(1) from exc
    if entry.meta.kind != "python":
        err_console.print(
            f"[red]{gettext('%(name)s is not a Python script entry.') % {'name': entry.meta.name}}[/red]"
        )
        raise typer.Exit(1)
    if set_deps is None and python is None:
        current = entry.meta.dependencies or []
        console.print(
            gettext("Dependencies of %(name)s: %(deps)s")
            % {"name": entry.meta.name, "deps": ", ".join(current) or "—"}
        )
        if entry.meta.requires_python:
            console.print(
                gettext("Python constraint: %(python)s") % {"python": entry.meta.requires_python}
            )
        return
    new_deps = (
        [d.strip() for d in set_deps.split(",") if d.strip()]
        if set_deps is not None
        else list(entry.meta.dependencies or [])
    )
    try:
        entry = store.update_dependencies(entry.slug, new_deps, requires_python=python)
    except store.StoreError as exc:
        err_console.print(f"[red]{exc}[/red]")
        raise typer.Exit(1) from exc
    console.print(
        f"[green]{gettext('Dependencies of %(name)s updated: %(deps)s') % {'name': entry.meta.name, 'deps': ', '.join(new_deps) or '—'}}[/green]"
    )


@app.command()
def doctor(
    rebuild: bool = typer.Option(
        False, "--rebuild", help=gettext("Rebuild the index from each script's meta.toml")
    ),
) -> None:
    """Environment self-check: whether uv is available and the store is intact.

    Pass --rebuild to regenerate the index from disk.
    """
    uv = launcher.find_uv()
    if uv:
        console.print(f"[green]{gettext('uv: %(path)s') % {'path': uv}}[/green]")
    else:
        console.print(
            f"[red]{gettext('uv: not found. Install it from https://docs.astral.sh/uv/getting-started/installation/')}[/red]"
        )
    if rebuild:
        count, problems = store.doctor_rebuild()
        console.print(
            f"[green]{ngettext('Index rebuilt: %(count)s entry', 'Index rebuilt: %(count)s entries', count) % {'count': count}}[/green]"
        )
        for p in problems:
            console.print(f"  [yellow]{p}[/yellow]")
    else:
        entries = store.list_entries()
        console.print(gettext("Registered scripts: %(count)s") % {"count": len(entries)})
        missing = [
            e.meta.name
            for e in entries
            if e.meta.mode == "reference" and e.meta.source and not Path(e.meta.source).exists()
        ]
        for m in missing:
            console.print(
                f"  [yellow]{gettext('%(name)s: reference source file is gone') % {'name': m}}[/yellow]"
            )
    raise typer.Exit(0 if uv else 1)


@app.command()
def lang(
    value: str = typer.Argument(
        None,
        help=gettext('A language tag such as en / zh-TW / zh-CN, or "auto" to clear the override'),
    ),
) -> None:
    """Show or set the interface language."""
    locales = i18n.available_locales()
    if value is None:
        console.print(gettext("Active language: %(locale)s") % {"locale": i18n.current_locale()})
        console.print(gettext("Available: %(locales)s") % {"locales": ", ".join(locales)})
        return
    if value.lower() == "auto":
        i18n.set_language("")
        console.print(gettext("Language override cleared — back to auto-detection."))
        console.print(gettext("Active language: %(locale)s") % {"locale": i18n.current_locale()})
        return
    if not i18n.is_supported(value):
        err_console.print(
            f"[red]{gettext('Unknown language: %(tag)s. Available: %(locales)s') % {'tag': value, 'locales': ', '.join(locales)}}[/red]"
        )
        raise typer.Exit(2)
    effective = i18n.set_language(value)
    console.print(gettext("Language set to %(locale)s") % {"locale": effective})


if __name__ == "__main__":
    sys.exit(app())
