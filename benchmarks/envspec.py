"""The constructed-environment contract, as pure computation — covered gate code.

Every benchmarked child's environment is BUILT here, never inherited: dataset-pointed
SKIT dirs (absolute), scratch HOME/XDG, composed PATH, pinned locale/terminal, a
per-session uv cache. `suites/_env.py` only discovers binaries and spawns; the
decisions live in this module because a wrong environment produces wrong-but-plausible
numbers — the worst failure class a benchmark suite has (docs/design/benchmarks.md).
"""

from __future__ import annotations

from pathlib import Path

from .datasets import skit_dirs

# What pyperf workers must inherit on top of their purified environment (pyperf drops
# everything but PATH/HOME/locale/PYTHONPATH), plus the per-script fixture vars.
PYPERF_INHERIT = (
    "SKIT_DATA_DIR",
    "SKIT_STATE_DIR",
    "SKIT_CONFIG_DIR",
    "SKIT_LANG",
    "PYTHONUTF8",
    "LC_ALL",
    "BENCH_N",
    "BENCH_SOURCES_DIR",
)


def build_env(
    *,
    skit: str,
    uv: str | None,
    node: str | None,
    workdir: Path,
    dataset_root: Path | None,
) -> dict[str, str]:
    """The constructed env dict — built, not scrubbed: composed PATH (venv, uv, node,
    system), dataset-pointed SKIT dirs, scratch HOME/XDG, per-session UV cache, pinned
    locale/terminal. Ambient PYTHONPATH, color vars, UV_* mirrors never leak in."""
    path_parts: list[str] = [str(Path(skit).parent)]
    path_parts.extend(str(Path(tool).parent) for tool in (uv, node) if tool)
    path_parts += ["/usr/bin", "/bin"]
    seen: dict[str, None] = {}
    for part in path_parts:
        seen.setdefault(part, None)

    home = workdir / "home"
    home.mkdir(parents=True, exist_ok=True)
    env: dict[str, str] = {
        "PATH": ":".join(seen),
        "HOME": str(home),
        "XDG_DATA_HOME": str(workdir / "xdg-data"),
        "XDG_STATE_HOME": str(workdir / "xdg-state"),
        "XDG_CONFIG_HOME": str(workdir / "xdg-config"),
        "XDG_CACHE_HOME": str(workdir / "xdg-cache"),
        "UV_CACHE_DIR": str(workdir / "uv-cache"),
        "SKIT_LANG": "en",
        "PYTHONUTF8": "1",
        "LC_ALL": "C.UTF-8",
        "TERM": "dumb",
        "COLUMNS": "100",
        "LINES": "40",
    }
    if dataset_root is None:
        dataset_root = workdir / "empty-library"
        dataset_root.mkdir(parents=True, exist_ok=True)
    elif not (dataset_root / "manifest.json").exists():
        # Wrong-but-plausible defense: a dataset root that doesn't hold a generated
        # library would make every child benchmark an empty one. Die here, loudly.
        raise RuntimeError(f"{dataset_root} is not a generated dataset (no manifest.json)")
    env.update(skit_dirs(dataset_root))
    return env
