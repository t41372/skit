"""Launcher: assemble the run command and execute it straight through the terminal (C2/C5/C6).

- python entries: always `uv run --no-project --script <path>` (C2). `--script` alone does NOT
  isolate a block-less script — uv attaches it to any enclosing project's environment (verified
  empirically), so --no-project is unconditional; PEP 723 blocks and --with deps still resolve.
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


class TargetMissingError(LaunchError):
    """The launch target (script file / executable) is gone from disk.

    A distinct type so `skit run` can map it to exit 127 (command not found, docker
    convention) while other skit-side failures map to 125 — scripts that themselves
    exit 1 stay distinguishable from skit failing to launch them at all."""


class NotExecutableError(LaunchError):
    """The exe target exists but has no execute permission (exit 126, docker convention)."""


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


def _build_python(entry: Entry, extra: list[str], script_override: Path | None) -> list[str]:
    # Check the cheap, local condition (does the script exist?) before the potentially-network-
    # bound one (is uv installed, or does it need downloading?) — mirrors preflight's ordering, and
    # spares a user with a missing script a pointless uv download/error first.
    script = script_override or entry.script_path
    _check_script_exists(script)
    uv = ensure_uv()
    # C2: unconditional isolation. Without --no-project, `uv run --script` attaches a
    # block-less script to whatever uv project encloses the cwd (empirically verified) —
    # and copy-mode entries default to workdir="invoke", so "run it from inside any
    # project directory" was a live hijack path. Scripts with a PEP 723 block and
    # reference-mode --with deps are unaffected by the flag.
    cmd = [uv, "run", "--no-project"]
    # In reference mode, dependencies are recorded in meta (the original file can't take a PEP 723
    # block), so pass them via --with/--python.
    if entry.meta.requires_python:
        cmd += ["--python", entry.meta.requires_python]
    for dep in entry.meta.dependencies or []:
        cmd += ["--with", dep]
    return [*cmd, "--script", str(script), *extra]


def _build_exe(entry: Entry, extra: list[str]) -> list[str]:
    exe = entry.meta.source
    _check_exe_exists(exe)
    return [exe, *extra]


def _quote_for_shell(value: str) -> str:
    """Quote a single substituted value for the platform shell _build_shell executes under,
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


def _build_shell(entry: Entry, extra: list[str], values: dict[str, str] | None) -> str:
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
        return _quote_for_shell(vals[name])

    cmd = _TEMPLATE_TOKEN_RE.sub(repl, template)
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


def describe_command(
    entry: Entry,
    extra_args: list[str] | None = None,
    values: dict[str, str] | None = None,
    *,
    script_override: Path | None = None,
) -> str:
    """A purely descriptive command line for transparency output and --dry-run: no uv
    lookup or download, no existence checks, no side effects. Mirrors build_command's
    shape; when uv isn't installed yet the literal "uv" stands in."""
    extra = extra_args or []
    kind = entry.meta.kind
    if kind == "python":
        uv = find_uv() or "uv"
        cmd = [uv, "run", "--no-project"]  # mirrors _build_python's unconditional C2 isolation
        if entry.meta.requires_python:
            cmd += ["--python", entry.meta.requires_python]
        for dep in entry.meta.dependencies or []:
            cmd += ["--with", dep]
        script = script_override or entry.script_path
        return _join_for_display([*cmd, "--script", str(script), *extra])
    if kind == "exe":
        return _join_for_display([entry.meta.source, *extra])
    try:
        built = _build_shell(entry, extra, values)
    except LaunchError:
        built = entry.meta.template
    return built


def _join_for_display(argv: list[str]) -> str:
    if sys.platform == "win32":
        return subprocess.list2cmdline(argv)
    return shlex.join(argv)


def target_missing(entry: Entry) -> bool:
    """Whether entry's launch target is already known to be gone from disk: the source path for
    exe/reference entries, the stored copy for copy-mode python. Command entries have no file
    target and never report missing."""
    if entry.meta.kind == "exe":
        return not Path(entry.meta.source).exists()
    if entry.meta.kind == "python":
        return not entry.script_path.exists()
    return False  # command entries have no file target


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


def preflight(entry: Entry, invoke_cwd: Path | None = None) -> None:
    """Validate what can be checked before any values/params are collected: the launch target
    (script/exe) and the working directory. Raises LaunchError with the same messages the actual
    build/run would eventually raise, but does none of the actual work (no uv lookup/download, no
    process spawn) — so the TUI can call this before suspending the terminal."""
    if entry.meta.kind == "python":
        _check_script_exists(entry.script_path)
    elif entry.meta.kind == "exe":
        _check_exe_exists(entry.meta.source)
    _check_workdir(_resolve_workdir(entry, invoke_cwd or Path.cwd()))


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
    _check_workdir(cwd)
    # Overlay skit's mirror settings onto uv's environment — a no-op unless the user enabled them,
    # and never clobbering a variable the user set themselves (see config.mirror_env).
    env = {**os.environ, **config.mirror_env(os.environ)}
    if isinstance(cmd, str):
        # A command entry is by definition "a shell command the user registered"; shell=True is a
        # feature, not a hole. The template was written by the user via `skit add`, so the trust
        # boundary is the same as the user's own shell history.
        proc = subprocess.run(cmd, shell=True, cwd=cwd, check=False, env=env)  # noqa: S602  # pragma: no mutate — check=None is falsy-equivalent to False; omitting it matches subprocess.run's own default
    else:
        proc = subprocess.run(cmd, cwd=cwd, check=False, env=env)  # noqa: S603 — argv from a user entry  # pragma: no mutate — check=None is falsy-equivalent to False; omitting it matches subprocess.run's own default
    return _normalize_exit_code(proc.returncode)


def _normalize_exit_code(returncode: int) -> int:
    """Map subprocess.run's signal-death reporting (a negative returncode -N for "killed by signal
    N") onto the conventional shell exit status 128+N, matching what a user would see running the
    same command directly in a POSIX shell. Left as a raw negative number, it would be silently
    mangled by sys.exit (which reduces any status to a byte via `& 0xFF`, e.g. -11 -> 245) while
    also being printed to the user as a confusing negative code."""
    return returncode if returncode >= 0 else 128 - returncode
