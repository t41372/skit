"""Host/environment manifest — the provenance block of every results file.

Budget predicates key on this module's output (`platform_key`, `ci_runner`), which
makes it gate code: everything here is pure functions over injected values, covered at
the repo's 100% floor. The one seam wiring those functions to the real host lives at
the bottom, marked `# pragma: no cover`.
"""

from __future__ import annotations

import os
import platform
import subprocess
import sys
from collections.abc import Mapping
from importlib.metadata import PackageNotFoundError, version
from pathlib import Path
from typing import TYPE_CHECKING

from .results import GitInfo, HostInfo, Meta

if TYPE_CHECKING:
    pass

# The workflows export this (no GitHub-provided variable carries the `runs-on` label);
# absent or empty locally, which envinfo reports as None = "not CI".
CI_RUNNER_VAR = "BENCH_CI_RUNNER"

_MACHINE_ALIASES = {"amd64": "x86_64", "arm64": "aarch64"}


def platform_key(system: str, machine: str) -> str:
    """ "Linux" + "x86_64" → "linux-x86_64" — the value budget `platform` predicates
    match against. Machine aliases are folded so macOS "arm64" and Linux "aarch64"
    compare equal."""
    normalized = machine.lower()
    return f"{system.lower()}-{_MACHINE_ALIASES.get(normalized, normalized)}"


def ci_runner(env: Mapping[str, str]) -> str | None:
    """The runner label CI exported, or None off-CI. Empty string collapses to None:
    an empty export is "unset", not a runner named ''."""
    return env.get(CI_RUNNER_VAR) or None


def cpu_model(cpuinfo_text: str, fallback: str) -> str:
    """First "model name" from /proc/cpuinfo-shaped text; `fallback` (typically the
    machine arch) when the field is absent (non-x86, non-Linux)."""
    for line in cpuinfo_text.splitlines():
        key, sep, value = line.partition(":")
        if sep and key.strip() == "model name" and value.strip():
            return value.strip()
    return fallback


def mem_total_mib(page_size: int, phys_pages: int) -> int:
    return page_size * phys_pages // (1024 * 1024)


def git_dirty(porcelain: str) -> bool:
    """`git status --porcelain` output → whether the tree carries local changes."""
    return bool(porcelain.strip())


def build_host(
    *,
    system: str,
    machine: str,
    kernel: str,
    cpu: str,
    cpu_count: int,
    mem_mib: int,
    env: Mapping[str, str],
) -> HostInfo:
    return HostInfo(
        os=system,
        kernel=kernel,
        cpu=cpu,
        cpu_count=cpu_count,
        mem_total_mib=mem_mib,
        platform_key=platform_key(system, machine),
        ci_runner=ci_runner(env),
    )


def build_meta(
    *,
    profile: str,
    generated_at: str,
    commit: str,
    dirty: bool,
    host: HostInfo,
    python_version: str,
    uv_version: str,
    skit_version: str,
    textual_version: str,
) -> Meta:
    return Meta(
        generated_at=generated_at,
        profile=profile,
        git=GitInfo(commit=commit, dirty=dirty),
        skit_version=skit_version,
        host=host,
        python=python_version,
        uv=uv_version,
        textual=textual_version,
    )


def dist_version(name: str) -> str:
    """Installed distribution version, or "unknown" when the name isn't installed."""
    try:
        return version(name)
    except PackageNotFoundError:
        return "unknown"


def uv_version_from_output(output: str) -> str:
    """`uv --version` → "0.11.26" (second word; the line is "uv X.Y.Z (…)")."""
    words = output.split()
    return words[1] if len(words) > 1 else "unknown"


# ---------------------------------------------------------------- the real-host seam


def collect_meta(profile: str, repo_root: Path) -> Meta:  # pragma: no cover — real-host seam
    """Assemble the manifest from the actual machine: the only untested lines in this
    module, and they only *read* the host and delegate to the pure builders above."""
    import skit
    from skit.models import now_iso

    uname = platform.uname()
    try:
        cpuinfo = Path("/proc/cpuinfo").read_text(encoding="utf-8")
    except OSError:
        cpuinfo = ""
    sysconf = getattr(os, "sysconf", None)  # absent on Windows
    try:
        mem = mem_total_mib(sysconf("SC_PAGE_SIZE"), sysconf("SC_PHYS_PAGES")) if sysconf else 0
    except (ValueError, OSError):
        mem = 0
    host = build_host(
        system=uname.system,
        machine=uname.machine,
        kernel=uname.release,
        cpu=cpu_model(cpuinfo, uname.machine),
        cpu_count=os.cpu_count() or 1,
        mem_mib=mem,
        env=os.environ,
    )
    commit = subprocess.run(
        ["git", "rev-parse", "HEAD"],  # noqa: S607 — fixed program name, dev tooling
        cwd=repo_root,
        capture_output=True,
        text=True,
        check=True,
    ).stdout.strip()
    porcelain = subprocess.run(
        ["git", "status", "--porcelain"],  # noqa: S607 — fixed program name, dev tooling
        cwd=repo_root,
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    uv_out = subprocess.run(
        ["uv", "--version"],  # noqa: S607 — fixed program name, dev tooling
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    return build_meta(
        profile=profile,
        generated_at=now_iso(),
        commit=commit,
        dirty=git_dirty(porcelain),
        host=host,
        python_version=f"{sys.version_info.major}.{sys.version_info.minor}.{sys.version_info.micro}",
        uv_version=uv_version_from_output(uv_out),
        skit_version=skit.__version__,
        textual_version=dist_version("textual"),
    )
