"""The constructed-environment contract (docs/design/benchmarks.md) — one place.

Benchmarked processes never inherit the ambient environment: this module builds the
env dict (dataset-pointed SKIT dirs, scratch HOME/XDG, composed PATH, pinned locale
and terminal geometry, per-session UV cache) and every suite passes it to its
children. The harness cwd lives OUTSIDE any uv project (a system temp dir), so
`uv run --script` lanes can never attach to the skit checkout.
"""

from __future__ import annotations

import shutil
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import TYPE_CHECKING

from ..hyperfine import Case, build_argv, metric_from_times, parse_export
from ..results import Metric

if TYPE_CHECKING:
    from collections.abc import Mapping

    from ..datasets import Manifest

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


@dataclass(frozen=True)
class RunCtx:
    """Everything a suite needs: discovered binaries, the external workdir, the
    generated datasets, and where outputs land."""

    repo_root: Path
    out_dir: Path  # run outputs: suites/*.json, artifacts/
    workdir: Path  # EXTERNAL temp dir: cwd, scratch HOME/XDG, UV cache
    datasets: dict[int, Manifest]
    fixtures_dir: Path
    python: str
    skit: str
    uv: str | None
    bash: str | None
    node: str | None
    hyperfine: str | None
    strace: str | None


def discover(
    repo_root: Path, out_dir: Path, workdir: Path, datasets: dict[int, Manifest]
) -> RunCtx:
    """Resolve the benchmarked binaries: the venv's own skit/python (never `uv run`,
    whose overhead would pollute every number), and the external tools off the ambient
    PATH. Missing optional tools stay None — suites record skips, never crash."""
    venv_bin = Path(sys.executable).parent
    skit_bin = venv_bin / "skit"
    if not skit_bin.exists():
        raise RuntimeError(
            f"no skit console script next to {sys.executable} — run via `uv run python -m benchmarks`"
        )
    return RunCtx(
        repo_root=repo_root,
        out_dir=out_dir,
        workdir=workdir,
        datasets=datasets,
        fixtures_dir=repo_root / "benchmarks" / "fixtures",
        python=sys.executable,
        skit=str(skit_bin),
        uv=shutil.which("uv"),
        bash=shutil.which("bash"),
        node=shutil.which("node"),
        hyperfine=shutil.which("hyperfine"),
        strace=shutil.which("strace"),
    )


def bench_env(ctx: RunCtx, dataset_root: Path | None) -> dict[str, str]:
    """The constructed env dict — built, not scrubbed: composed PATH (venv, uv, node,
    system), dataset-pointed SKIT dirs, scratch HOME/XDG, per-session UV cache, pinned
    locale/terminal. Ambient PYTHONPATH, color vars, UV_* mirrors never leak in."""
    from ..datasets import skit_dirs

    path_parts: list[str] = [str(Path(ctx.skit).parent)]
    path_parts.extend(str(Path(tool).parent) for tool in (ctx.uv, ctx.node) if tool)
    path_parts += ["/usr/bin", "/bin"]
    seen: dict[str, None] = {}
    for part in path_parts:
        seen.setdefault(part, None)

    home = ctx.workdir / "home"
    home.mkdir(parents=True, exist_ok=True)
    env: dict[str, str] = {
        "PATH": ":".join(seen),
        "HOME": str(home),
        "XDG_DATA_HOME": str(ctx.workdir / "xdg-data"),
        "XDG_STATE_HOME": str(ctx.workdir / "xdg-state"),
        "XDG_CONFIG_HOME": str(ctx.workdir / "xdg-config"),
        "XDG_CACHE_HOME": str(ctx.workdir / "xdg-cache"),
        "UV_CACHE_DIR": str(ctx.workdir / "uv-cache"),
        "SKIT_LANG": "en",
        "PYTHONUTF8": "1",
        "LC_ALL": "C.UTF-8",
        "TERM": "dumb",
        "COLUMNS": "100",
        "LINES": "40",
    }
    if dataset_root is None:
        dataset_root = ctx.workdir / "empty-library"
        dataset_root.mkdir(parents=True, exist_ok=True)
    elif not (dataset_root / "manifest.json").exists():
        # Wrong-but-plausible defense: a dataset root that doesn't hold a generated
        # library would make every child benchmark an empty one. Die here, loudly.
        raise RuntimeError(f"{dataset_root} is not a generated dataset (no manifest.json)")
    env.update(skit_dirs(dataset_root))
    return env


def run_hyperfine(
    ctx: RunCtx,
    cases: list[Case],
    *,
    warmup: int,
    min_runs: int,
    env: Mapping[str, str],
    export_name: str,
) -> tuple[dict[str, Metric], dict[str, object]]:
    """Spawn hyperfine on `cases`, parse its export, return per-case Metrics keyed
    `<case>.median_ms` plus the raw times for the artifact."""
    if ctx.hyperfine is None:
        raise RuntimeError("hyperfine not found")  # callers skip-record before this
    export = ctx.workdir / f"{export_name}.json"
    argv = build_argv(
        cases,
        warmup=warmup,
        min_runs=min_runs,
        export_json=str(export),
        hyperfine_bin=ctx.hyperfine,
    )
    proc = subprocess.run(  # noqa: S603 — fixed-shape argv built by hyperfine.build_argv
        argv,
        cwd=ctx.workdir,
        env=dict(env),
        check=False,
    )
    if proc.returncode != 0:
        # hyperfine swallows the failing command's stderr; an opaque exit code helps
        # nobody. Re-run each case once, captured, and report what actually broke.
        raise RuntimeError(
            f"hyperfine batch {export_name!r} failed ({proc.returncode}); "
            f"single-shot diagnosis: {diagnose_cases(ctx, cases, env)}"
        )
    times_by_case = parse_export(export.read_text(encoding="utf-8"))
    metrics = {
        f"{name}.median_ms": metric_from_times(times) for name, times in times_by_case.items()
    }
    return metrics, {"times_s": times_by_case}


def diagnose_cases(ctx: RunCtx, cases: list[Case], env: Mapping[str, str]) -> str:
    reports = []
    for case in cases:
        probe = subprocess.run(  # noqa: S603 — re-running the exact benchmarked argv
            list(case.argv),
            cwd=ctx.workdir,
            env=dict(env),
            capture_output=True,
            text=True,
            check=False,
        )
        detail = "" if probe.returncode == 0 else f" stderr: {probe.stderr.strip()[-500:]!r}"
        reports.append(f"[{case.name} rc={probe.returncode}{detail}]")
    return " ".join(reports)
