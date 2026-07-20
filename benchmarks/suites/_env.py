"""Binary discovery and process spawning for the suites — genuinely spawn-and-wait.

The decisions live in covered modules: the environment contract in
`benchmarks/envspec.py`, hyperfine argv/parsing/metric-keying in
`benchmarks/hyperfine.py`, dataset reuse rules in `benchmarks/datasets.py`. What
remains here is finding tools and waiting on children. The harness cwd lives OUTSIDE
any uv project (a system temp dir), so `uv run --script` lanes can never attach to the
skit checkout.
"""

from __future__ import annotations

import shutil
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import TYPE_CHECKING

from ..envspec import PYPERF_INHERIT, build_env
from ..hyperfine import Case, build_argv, metrics_from_export, parse_export

if TYPE_CHECKING:
    from collections.abc import Mapping

    from ..datasets import Manifest
    from ..results import Metric

__all__ = ["PYPERF_INHERIT", "RunCtx", "bench_env", "discover", "run_hyperfine"]


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
    """The constructed env for this run's children (envspec.build_env, applied to the
    discovered binaries and this run's workdir)."""
    return build_env(
        skit=ctx.skit,
        uv=ctx.uv,
        node=ctx.node,
        workdir=ctx.workdir,
        dataset_root=dataset_root,
    )


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
    return metrics_from_export(times_by_case), {"times_s": times_by_case}


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
