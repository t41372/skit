"""Concrete launch strategies (one per family; see base.LaunchStrategy).

The bodies here moved verbatim from launcher.py's _build_python/_build_exe/_build_shell
so behavior is pinned by the existing launcher tests; launcher.py keeps the public
build/describe/preflight/run surface and dispatches through the registry.
"""

from __future__ import annotations

import os
import re
import shlex
import shutil
import subprocess
import sys
from pathlib import Path
from typing import TYPE_CHECKING

from ..i18n import gettext
from ..paths import private_bin_dir
from .base import (
    ArgvLaunch,
    LaunchError,
    LaunchPayload,
    NotExecutableError,
    ShellLaunch,
    TargetMissingError,
)

if TYPE_CHECKING:
    from ..models import Entry


def find_uv() -> str | None:
    """Detection order: PATH -> skit's private bin (A9/§5.6)."""
    found = shutil.which("uv")
    if found:
        return found
    private = private_bin_dir() / "uv"
    if private.exists():
        return str(private)
    private_exe = private_bin_dir() / "uv.exe"
    if private_exe.exists():
        return str(private_exe)
    return None


def ensure_uv() -> str:
    """Find uv or auto-download a managed copy (first-run experience: zero user action). Raises
    LaunchError on failure."""
    found = find_uv()
    if found:
        return found
    from .. import uvman

    try:
        return uvman.ensure_uv_downloaded()
    except uvman.UvDownloadError as exc:
        raise LaunchError(
            f"{gettext('uv not found. Install it (https://docs.astral.sh/uv/) or run skit doctor for guidance.')} ({exc})"
        ) from exc


def _check_script_exists(script: Path) -> None:
    if not script.exists():
        raise TargetMissingError(
            gettext("The script file doesn't exist: %(path)s") % {"path": str(script)}
        )


def _check_exe_exists(source: str) -> None:
    path = Path(source)
    if not path.exists():
        raise TargetMissingError(
            gettext("The executable doesn't exist: %(path)s") % {"path": source}
        )
    if sys.platform != "win32" and path.is_file() and not os.access(path, os.X_OK):
        raise NotExecutableError(
            gettext("%(path)s exists but isn't executable (chmod +x it?).") % {"path": source}
        )


def join_for_display(argv: list[str]) -> str:
    if sys.platform == "win32":
        return subprocess.list2cmdline(argv)
    return shlex.join(argv)


class UvLaunch:
    """python entries: always `uv run --no-project --script <path>` (C2)."""

    def _argv_tail(self, entry: Entry) -> list[str]:
        """The flags after `uv run --no-project`, shared by build and describe.

        C2: unconditional isolation. Without --no-project, `uv run --script` attaches a
        block-less script to whatever uv project encloses the cwd (empirically verified) —
        and copy-mode entries default to workdir="invoke", so "run it from inside any
        project directory" was a live hijack path. Scripts with a PEP 723 block and
        reference-mode --with deps are unaffected by the flag.
        """
        cmd: list[str] = []
        # In reference mode, dependencies are recorded in meta (the original file can't take a
        # PEP 723 block), so pass them via --with/--python.
        if entry.meta.requires_python:
            cmd += ["--python", entry.meta.requires_python]
        for dep in entry.meta.dependencies or []:
            cmd += ["--with", dep]
        return cmd

    def build(
        self,
        entry: Entry,
        extra: list[str],
        values: dict[str, str] | None,
        script_override: Path | None,
    ) -> LaunchPayload:
        # Check the cheap, local condition (does the script exist?) before the potentially-
        # network-bound one (is uv installed, or does it need downloading?) — mirrors
        # preflight's ordering, and spares a user with a missing script a pointless uv
        # download/error first.
        script = script_override or entry.script_path
        _check_script_exists(script)
        uv = ensure_uv()
        cmd = [uv, "run", "--no-project", *self._argv_tail(entry)]
        return ArgvLaunch([*cmd, "--script", str(script), *extra])

    def describe(
        self,
        entry: Entry,
        extra: list[str],
        values: dict[str, str] | None,
        script_override: Path | None,
    ) -> str:
        uv = find_uv() or "uv"  # when uv isn't installed yet the literal "uv" stands in
        cmd = [uv, "run", "--no-project", *self._argv_tail(entry)]
        script = script_override or entry.script_path
        return join_for_display([*cmd, "--script", str(script), *extra])

    def target(self, entry: Entry) -> Path | None:
        return entry.script_path

    def preflight(self, entry: Entry) -> None:
        _check_script_exists(entry.script_path)


class DirectLaunch:
    """exe entries: run the referenced file directly."""

    def build(
        self,
        entry: Entry,
        extra: list[str],
        values: dict[str, str] | None,
        script_override: Path | None,
    ) -> LaunchPayload:
        exe = entry.meta.source
        _check_exe_exists(exe)
        return ArgvLaunch([exe, *extra])

    def describe(
        self,
        entry: Entry,
        extra: list[str],
        values: dict[str, str] | None,
        script_override: Path | None,
    ) -> str:
        return join_for_display([entry.meta.source, *extra])

    def target(self, entry: Entry) -> Path | None:
        return Path(entry.meta.source)

    def preflight(self, entry: Entry) -> None:
        _check_exe_exists(entry.meta.source)


def quote_for_shell(value: str) -> str:
    """Quote a single substituted value for the platform shell TemplateLaunch executes under,
    mirroring how `extra` args are already quoted below (shlex on POSIX, list2cmdline on Windows) —
    otherwise a value with spaces or shell metacharacters reshapes the command's argument
    structure or, worse, injects extra shell syntax."""
    if sys.platform == "win32":
        return subprocess.list2cmdline([value])
    return shlex.quote(value)


# Matches, left to right: a `{{` escape, a `}}` escape, or a `{name}` placeholder (the same
# identifier rule as store.extract_placeholders). Substitution and escape-restoration run together
# in ONE pass over the ORIGINAL template via this pattern so replacement text is never re-scanned —
# doing it as two sequential passes (substitute placeholders, then str.replace "{{"/"}}") would
# corrupt any substituted value that itself contains "{{" or "}}".
_TEMPLATE_TOKEN_RE = re.compile(r"\{\{|\}\}|(?<!\{)\{([a-zA-Z_][a-zA-Z0-9_]*)\}(?!\})")


class TemplateLaunch:
    """command entries: template + placeholder fill-in, executed through the shell."""

    def _render(self, entry: Entry, extra: list[str], values: dict[str, str] | None) -> str:
        template = entry.meta.template
        vals = values or {}
        if entry.meta.params:
            missing = [p for p in entry.meta.params if p not in vals]
            if missing:
                raise LaunchError(
                    gettext("Missing parameter values: %(names)s") % {"names": ", ".join(missing)}
                )

        def repl(m: re.Match[str]) -> str:
            matched = m.group(0)
            if matched == "{{":
                return "{"
            if matched == "}}":
                return "}"
            name = m.group(1)
            if name is None or name not in vals:
                return matched
            return quote_for_shell(vals[name])

        cmd = _TEMPLATE_TOKEN_RE.sub(repl, template)
        if extra:
            # shell=True execution: quoting must follow that platform's shell (POSIX uses shlex,
            # Windows cmd uses list2cmdline), or arguments containing $ or backticks would be
            # expanded.
            if sys.platform == "win32":
                cmd = cmd + " " + subprocess.list2cmdline(extra)
            else:
                cmd = cmd + " " + shlex.join(extra)
        return cmd

    def build(
        self,
        entry: Entry,
        extra: list[str],
        values: dict[str, str] | None,
        script_override: Path | None,
    ) -> LaunchPayload:
        return ShellLaunch(self._render(entry, extra, values))

    def describe(
        self,
        entry: Entry,
        extra: list[str],
        values: dict[str, str] | None,
        script_override: Path | None,
    ) -> str:
        try:
            return self._render(entry, extra, values)
        except LaunchError:
            return entry.meta.template

    def target(self, entry: Entry) -> Path | None:
        return None  # command entries have no file target

    def preflight(self, entry: Entry) -> None:
        return None  # nothing to check before values are collected
