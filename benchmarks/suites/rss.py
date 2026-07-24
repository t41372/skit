"""rss — peak resident set of CLI commands via getrusage(RUSAGE_CHILDREN), one fresh
harness process per sample so maxima can't bleed between samples."""

from __future__ import annotations

import json
import platform
import subprocess
import sys
from typing import TYPE_CHECKING

from ..parsers import maxrss_kib, median
from ..results import Metric, SuiteOutput
from ._env import RunCtx, bench_env

if TYPE_CHECKING:
    from collections.abc import Mapping

    from ..pipeline import SuitePlan

# The whole harness: fork exactly one child, wait, report its peak RSS. Spawned fresh
# per sample — RUSAGE_CHILDREN aggregates the max over ALL waited children, so a
# reused harness would let a large earlier child mask a smaller later one.
_HARNESS = (
    "import json, resource, subprocess, sys\n"
    "p = subprocess.run(sys.argv[1:], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)\n"
    "ru = resource.getrusage(resource.RUSAGE_CHILDREN)\n"
    "print(json.dumps({'maxrss': ru.ru_maxrss, 'rc': p.returncode}))\n"
)


def run(ctx: RunCtx, plan: SuitePlan) -> SuiteOutput:
    if sys.platform == "win32":
        return SuiteOutput.skip_all("rss", "no resource module on Windows")
    output = SuiteOutput(suite="rss")
    cases: list[tuple[str, tuple[str, ...], int]] = [("rss.version", (ctx.skit, "--version"), 0)]
    cases += [(f"rss.list_json.n{n}", (ctx.skit, "list", "--json"), n) for n in plan.ns]
    for name, argv, n in cases:
        env = bench_env(ctx, ctx.datasets[n].root)
        peaks = [_sample(ctx, argv, env) for _ in range(plan.samples)]
        output.metrics[f"{name}.peak_kib"] = Metric(
            value=median([float(p) for p in peaks]), unit="KiB", n=len(peaks)
        )
        output.metrics[f"{name}.peak_max_kib"] = Metric(
            value=float(max(peaks)), unit="KiB", n=len(peaks)
        )
        output.raw[name] = {"samples_kib": peaks}
    return output


def _sample(ctx: RunCtx, argv: tuple[str, ...], env: Mapping[str, str]) -> int:
    proc = subprocess.run(  # noqa: S603 — fixed-shape harness argv
        [ctx.python, "-c", _HARNESS, *argv],
        cwd=ctx.workdir,
        env=dict(env),
        capture_output=True,
        text=True,
        check=True,
    )
    doc = json.loads(proc.stdout)
    if doc["rc"] != 0:
        raise RuntimeError(f"rss target {argv} exited {doc['rc']}")
    return maxrss_kib(int(doc["maxrss"]), platform.system())
