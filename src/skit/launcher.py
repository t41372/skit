"""Launcher: assemble the run command and execute it straight through the terminal (C2/C5/C6).

The per-kind assembly lives in langs/launch.py strategies (UvLaunch/DirectLaunch/
TemplateLaunch), resolved through langs.registry; this module keeps the kind-agnostic
surface: workdir resolution, the env overlay, process spawn, and exit-code shaping.
The terminal is handed entirely to the child process (stdin/stdout/stderr pass through);
the TUI caller is responsible for suspend/resume.
"""

from __future__ import annotations

import os
import shutil
import subprocess
from collections.abc import Mapping
from dataclasses import dataclass
from pathlib import Path
from typing import TYPE_CHECKING

from . import config
from .i18n import gettext
from .langs import base as _base
from .langs import launch as _launch
from .langs.base import LaunchPayload, ShellLaunch
from .langs.registry import spec_for
from .models import Entry

if TYPE_CHECKING:
    from .config import PromptRunner

# Public re-exports: the exception family is part of launcher's stable surface
# (flows/cli/tui catch launcher.LaunchError) even though it now lives in langs/base.
LaunchError = _base.LaunchError
TargetMissingError = _base.TargetMissingError
NotExecutableError = _base.NotExecutableError  # raised here too (needs check)


@dataclass(frozen=True)
class PreparedLaunch:
    """One fully built launch snapshot, ready to spawn without re-reading inputs."""

    payload: LaunchPayload
    cwd: Path
    safe_display: str | None = None
    prompt_runner: PromptRunner | None = None


def find_uv() -> str | None:
    """Delegate to the canonical finder in langs.launch (kept on launcher's public
    surface for doctor/health; a *dynamic* delegate, not an assignment re-export, so a
    test patching skit.langs.launch.find_uv patches every consumer coherently)."""
    return _launch.find_uv()


def ensure_uv() -> str:
    """Dynamic delegate to langs.launch.ensure_uv (same reasoning as find_uv)."""
    return _launch.ensure_uv()


def _resolve_workdir(entry: Entry, invoke_cwd: Path) -> Path:
    policy = entry.meta.workdir
    if policy == "origin":
        src = entry.meta.source
        origin_dir = Path(src).parent if src else invoke_cwd
        if entry.meta.mode == "copy" and not origin_dir.is_dir():
            # Copy mode exists to decouple the entry from its original location, so a vanished
            # origin must not block a run when the store copy is intact — this also recovers
            # entries persisted with workdir="origin" before store.add_python's copy-mode default
            # changed to "invoke". Reference-mode entries are not decoupled from their origin (the
            # script check already fails first with a clearer message if it's gone), so they keep
            # resolving to the origin dir unconditionally.
            return invoke_cwd
        return origin_dir
    if policy == "store":
        return entry.dir
    if policy == "invoke":
        return invoke_cwd
    return Path(policy)  # absolute path


def _payload(
    entry: Entry,
    extra_args: list[str] | None,
    values: dict[str, str] | None,
    script_override: Path | None,
    runner: PromptRunner | None = None,
) -> LaunchPayload:
    spec = spec_for(entry.meta.kind)
    if spec is None:
        raise LaunchError(gettext("Unknown entry kind: %(kind)s") % {"kind": entry.meta.kind})
    return spec.launch.build(entry, extra_args or [], values, script_override, runner=runner)


def build_command(
    entry: Entry,
    extra_args: list[str] | None = None,
    values: dict[str, str] | None = None,
    *,
    script_override: Path | None = None,
    runner: PromptRunner | None = None,
) -> list[str] | str:
    """Return an argv list (python/exe) or a shell string (command).

    values: fill-ins for the named placeholders of a command template (missing values raise
    LaunchError).
    script_override: the temporary script path after shim injection (python entries only; A5 leaves
    the original copy untouched).
    """
    payload = _payload(entry, extra_args, values, script_override, runner)
    if isinstance(payload, ShellLaunch):
        return payload.command
    return payload.argv


def describe_command(
    entry: Entry,
    extra_args: list[str] | None = None,
    values: dict[str, str] | None = None,
    *,
    script_override: Path | None = None,
    runner: PromptRunner | None = None,
) -> str:
    """A purely descriptive command line for transparency output and --dry-run: no uv
    lookup or download, no existence checks, no side effects. Mirrors build_command's
    shape; when uv isn't installed yet the literal "uv" stands in."""
    spec = spec_for(entry.meta.kind)
    if spec is None:
        # A kind written by a newer skit: nothing to assemble, but describe must not raise —
        # show the template (the only launch material meta itself carries), usually "".
        return entry.meta.template
    return spec.launch.describe(entry, extra_args or [], values, script_override, runner=runner)


def target_missing(entry: Entry) -> bool:
    """Whether entry's launch target is already known to be gone from disk: the source path for
    exe/reference entries, the stored copy for copy-mode python. Command entries have no file
    target and never report missing."""
    spec = spec_for(entry.meta.kind)
    if spec is None:
        return False  # unknown kind: nothing this version can check
    target = spec.launch.target(entry)
    return target is not None and not target.exists()


def missing_marker(entry: Entry) -> str | None:
    """A human-readable "target is missing" message for entry, or None when it's healthy or has no
    file target (command entries). Callers decide how to style/render it (TUI table, CLI list).
    exe entries are always reference-mode, so script_path is exactly their source path."""
    if not target_missing(entry):
        return None
    return gettext("⚠ missing: %(path)s") % {"path": str(entry.script_path)}


def _check_workdir(cwd: Path) -> None:
    if not cwd.is_dir():
        raise LaunchError(
            gettext("The working directory doesn't exist: %(path)s") % {"path": str(cwd)}
        )


def _check_needs(entry: Entry) -> None:
    """`needs = ["jq", …]`: every named external command must be on PATH before launch.
    Exit-code contract: 126 (the target exists but its prerequisites can't run) — the
    same NotExecutableError family a non-executable exe raises."""
    missing = [tool for tool in entry.meta.needs or [] if shutil.which(tool) is None]
    if missing:
        raise NotExecutableError(
            gettext("Missing required command(s): %(names)s — install them and retry.")
            % {"names": ", ".join(missing)}
        )


def missing_needs(entry: Entry) -> list[str]:
    """The subset of entry's declared external commands not on PATH (doctor/health
    sweep — same check preflight enforces, surfaced as a report instead of an error)."""
    return [tool for tool in entry.meta.needs or [] if shutil.which(tool) is None]


def preflight(
    entry: Entry,
    invoke_cwd: Path | None = None,
    *,
    runner: PromptRunner | None = None,
) -> None:
    """Validate the launch target, selected runtime, declared needs, and workdir.

    ``runner`` is the prompt kind's resolved per-run override.  When omitted, prompt
    entries validate their pin, preserving the health and form-free rerun contract.
    The function remains side-effect-free (no download, install, or process spawn),
    so a TUI can call it after selection but before suspending the terminal.
    """
    spec = spec_for(entry.meta.kind)
    if spec is not None:
        spec.launch.preflight(entry, runner=runner)
    _check_needs(entry)
    _check_workdir(_resolve_workdir(entry, invoke_cwd or Path.cwd()))


def prepare_entry(
    entry: Entry,
    extra_args: list[str] | None = None,
    *,
    values: dict[str, str] | None = None,
    invoke_cwd: Path | None = None,
    script_override: Path | None = None,
    runner: PromptRunner | None = None,
) -> PreparedLaunch:
    """Build and validate exactly the payload that a later spawn will consume.

    This is stronger than ``preflight``: it resolves the executable and renders the
    final argv/body once. Delivery-boundary UI can therefore stay silent until this
    succeeds, then spawn the same immutable snapshot without a TOCTOU rebuild.
    """
    spec = spec_for(entry.meta.kind)
    if spec is None:
        raise LaunchError(gettext("Unknown entry kind: %(kind)s") % {"kind": entry.meta.kind})
    safe_display: str | None = None
    prompt_runner: PromptRunner | None = None
    if isinstance(spec.launch, _launch.PromptLaunch):
        # Prompt preflight and the former execute gate validate the body before
        # needs/binary failures. build_snapshot preserves that order while also
        # resolving the runner row only once for argv and transparency.
        payload, safe_display, prompt_runner = spec.launch.build_snapshot(
            entry,
            extra_args or [],
            values,
            script_override,
            runner=runner,
        )
        _check_needs(entry)
    else:
        # Preserve the established non-prompt run ordering: prerequisites before
        # strategy build (which may perform more expensive runtime/dependency work).
        _check_needs(entry)
        payload = spec.launch.build(
            entry,
            extra_args or [],
            values,
            script_override,
            runner=runner,
        )
    cwd = _resolve_workdir(entry, invoke_cwd or Path.cwd())
    _check_workdir(cwd)
    return PreparedLaunch(
        payload=payload,
        cwd=cwd,
        safe_display=safe_display,
        prompt_runner=prompt_runner,
    )


def run_entry(
    entry: Entry,
    extra_args: list[str] | None = None,
    *,
    values: dict[str, str] | None = None,
    invoke_cwd: Path | None = None,
    script_override: Path | None = None,
    env_overlay: Mapping[str, str] | None = None,
    runner: PromptRunner | None = None,
    prepared: PreparedLaunch | None = None,
) -> int:
    """Run straight through the terminal and return the exit code.

    env_overlay: env-delivered parameter values, applied LAST — an explicitly set
    parameter is a deliberate override, so it wins over both the ambient environment
    and skit's own mirror variables.

    The TUI must be suspended before calling this.
    """
    launch = prepared or prepare_entry(
        entry,
        extra_args,
        values=values,
        invoke_cwd=invoke_cwd,
        script_override=script_override,
        runner=runner,
    )
    # Overlay skit's mirror settings onto EVERY child's environment (uv reads the index
    # vars, npm/bun read the registry var — the overlay exists for both) — a no-op
    # unless the user enabled them, never clobbering a variable the user set themselves.
    env = {**os.environ, **config.mirror_env(os.environ), **(env_overlay or {})}
    # LaunchPayload is a closed two-member union, so isinstance/else is exhaustive (the
    # else narrows to ArgvLaunch) without the phantom no-match arm a `match` would add.
    if isinstance(launch.payload, ShellLaunch):
        # A command entry is by definition "a shell command the user registered"; shell=True is a
        # feature, not a hole. The template was written by the user via `skit add`, so the trust
        # boundary is the same as the user's own shell history.
        proc = subprocess.run(  # noqa: S602  # pragma: no mutate
            launch.payload.command, shell=True, cwd=launch.cwd, check=False, env=env
        )
    else:
        proc = subprocess.run(launch.payload.argv, cwd=launch.cwd, check=False, env=env)  # noqa: S603 — argv from a user entry  # pragma: no mutate — check=None is falsy-equivalent to False; omitting it matches subprocess.run's own default
    return _normalize_exit_code(proc.returncode)


def _normalize_exit_code(returncode: int) -> int:
    """Map subprocess.run's signal-death reporting (a negative returncode -N for "killed by signal
    N") onto the conventional shell exit status 128+N, matching what a user would see running the
    same command directly in a POSIX shell. Left as a raw negative number, it would be silently
    mangled by sys.exit (which reduces any status to a byte via `& 0xFF`, e.g. -11 -> 245) while
    also being printed to the user as a confusing negative code."""
    return returncode if returncode >= 0 else 128 - returncode
