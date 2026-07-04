"""Launcher: assemble the run command and execute it straight through the terminal (C2/C5/C6).

- python entries: always `uv run --script <path>` (C2: explicit --script prevents hijacking by a
  neighboring pyproject).
- exe entries: run directly.
- command entries: template + placeholder fill-in, executed through the shell.
- The terminal is handed entirely to the child process (stdin/stdout/stderr pass through); the TUI
  caller is responsible for suspend/resume.
"""

from __future__ import annotations

import os
import re
import shlex
import shutil
import subprocess
import sys
from pathlib import Path

from . import config
from .i18n import gettext
from .models import Entry
from .paths import private_bin_dir


class LaunchError(Exception):
    pass


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
    from . import uvman

    try:
        return uvman.ensure_uv_downloaded()
    except uvman.UvDownloadError as exc:
        raise LaunchError(
            f"{gettext('uv not found. Install it (https://docs.astral.sh/uv/) or run skit doctor for guidance.')} ({exc})"
        ) from exc


def _resolve_workdir(entry: Entry, invoke_cwd: Path) -> Path:
    policy = entry.meta.workdir
    if policy == "origin":
        src = entry.meta.source
        return Path(src).parent if src else invoke_cwd
    if policy == "store":
        return entry.dir
    if policy == "invoke":
        return invoke_cwd
    return Path(policy)  # absolute path


def _build_python(entry: Entry, extra: list[str], script_override: Path | None) -> list[str]:
    uv = ensure_uv()
    script = script_override or entry.script_path
    if not script.exists():
        raise LaunchError(
            gettext("The script file doesn't exist: %(path)s") % {"path": str(script)}
        )
    cmd = [uv, "run"]
    if script_override is not None:
        cmd.append("--no-project")  # C2: force isolation on the injection path so we don't inhale
        # the user's project pyproject
    # In reference mode, dependencies are recorded in meta (the original file can't take a PEP 723
    # block), so pass them via --with/--python.
    if entry.meta.requires_python:
        cmd += ["--python", entry.meta.requires_python]
    for dep in entry.meta.dependencies or []:
        cmd += ["--with", dep]
    return [*cmd, "--script", str(script), *extra]


def _build_exe(entry: Entry, extra: list[str]) -> list[str]:
    exe = entry.meta.source
    if not Path(exe).exists():
        raise LaunchError(gettext("The executable doesn't exist: %(path)s") % {"path": exe})
    return [exe, *extra]


def _build_shell(entry: Entry, extra: list[str], values: dict[str, str] | None) -> str:
    cmd = entry.meta.template
    if entry.meta.params:
        vals = values or {}
        missing = [p for p in entry.meta.params if p not in vals]
        if missing:
            raise LaunchError(
                gettext("Missing parameter values: %(names)s") % {"names": ", ".join(missing)}
            )
        # Only replace named placeholders; the regex boundaries match store.extract_placeholders so
        # that an escape like {{name}} isn't substituted by mistake (str.replace would).
        cmd = re.sub(
            r"(?<!\{)\{([a-zA-Z_][a-zA-Z0-9_]*)\}(?!\})",
            lambda m: vals.get(m.group(1), m.group(0)),
            cmd,
        )
    # Restore {{ }} escapes: do this whether or not there were placeholders (escaping is part of the
    # template syntax).
    cmd = cmd.replace("{{", "{").replace("}}", "}")
    if extra:
        # shell=True execution: quoting must follow that platform's shell (POSIX uses shlex, Windows
        # cmd uses list2cmdline), or arguments containing $ or backticks would be expanded.
        if sys.platform == "win32":
            cmd = cmd + " " + subprocess.list2cmdline(extra)
        else:
            cmd = cmd + " " + shlex.join(extra)
    return cmd


def build_command(
    entry: Entry,
    extra_args: list[str] | None = None,
    values: dict[str, str] | None = None,
    *,
    script_override: Path | None = None,
) -> list[str] | str:
    """Return an argv list (python/exe) or a shell string (command).

    values: fill-ins for the named placeholders of a command template (missing values raise
    LaunchError).
    script_override: the temporary script path after shim injection (python entries only; A5 leaves
    the original copy untouched).
    """
    extra = extra_args or []
    kind = entry.meta.kind
    if kind == "python":
        return _build_python(entry, extra, script_override)
    if kind == "exe":
        return _build_exe(entry, extra)
    if kind == "command":
        return _build_shell(entry, extra, values)
    raise LaunchError(gettext("Unknown entry kind: %(kind)s") % {"kind": kind})


def run_entry(
    entry: Entry,
    extra_args: list[str] | None = None,
    *,
    values: dict[str, str] | None = None,
    invoke_cwd: Path | None = None,
    script_override: Path | None = None,
) -> int:
    """Run straight through the terminal and return the exit code.

    The TUI must be suspended before calling this.
    """
    cmd = build_command(entry, extra_args, values, script_override=script_override)
    cwd = _resolve_workdir(entry, invoke_cwd or Path.cwd())
    if not cwd.is_dir():
        raise LaunchError(
            gettext("The working directory doesn't exist: %(path)s") % {"path": str(cwd)}
        )
    # Overlay skit's mirror settings onto uv's environment — a no-op unless the user enabled them,
    # and never clobbering a variable the user set themselves (see config.mirror_env).
    env = {**os.environ, **config.mirror_env(os.environ)}
    if isinstance(cmd, str):
        # A command entry is by definition "a shell command the user registered"; shell=True is a
        # feature, not a hole. The template was written by the user via `skit add`, so the trust
        # boundary is the same as the user's own shell history.
        proc = subprocess.run(cmd, shell=True, cwd=cwd, check=False, env=env)  # noqa: S602
    else:
        proc = subprocess.run(cmd, cwd=cwd, check=False, env=env)  # noqa: S603 — argv from a user entry
    return proc.returncode
