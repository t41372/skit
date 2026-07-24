"""syscalls — strace-counted file operations behind `skit list --json` (the direct
per-entry-I/O evidence for the future summary-index PR) and the network-syscall count
expected to be zero on warm read-only paths. Linux + strace only; skips recorded.

The metric IDs are unsuffixed (`syscalls.list_json.*`) and defined at exactly one
library size — the plan carries a single N (1000) by design."""

from __future__ import annotations

import subprocess
import sys
from typing import TYPE_CHECKING

from ..parsers import FILE_OP_SYSCALLS, NETWORK_SYSCALLS, count_group, strace_counts
from ..results import Metric, SuiteOutput
from ._env import RunCtx, bench_env

if TYPE_CHECKING:
    from ..pipeline import SuitePlan


def run(ctx: RunCtx, plan: SuitePlan) -> SuiteOutput:
    if sys.platform != "linux":
        return SuiteOutput.skip_all("syscalls", "not Linux")
    if ctx.strace is None:
        return SuiteOutput.skip_all("syscalls", "strace not found")
    if len(plan.ns) != 1:
        raise RuntimeError("syscalls metric IDs are unsuffixed: the plan must carry one N")
    n = plan.ns[0]
    env = bench_env(ctx, ctx.datasets[n].root)
    table_file = ctx.workdir / f"strace_n{n}.txt"
    subprocess.run(  # noqa: S603 — fixed-shape strace argv
        [ctx.strace, "-f", "-c", "-o", str(table_file), ctx.skit, "list", "--json"],
        cwd=ctx.workdir,
        env=dict(env),
        check=True,
        stdout=subprocess.DEVNULL,
    )
    counts = strace_counts(table_file.read_text(encoding="utf-8"))
    output = SuiteOutput(suite="syscalls")
    output.metrics["syscalls.list_json.file_ops"] = Metric(
        value=float(count_group(counts, FILE_OP_SYSCALLS)), unit="count", n=1
    )
    output.metrics["syscalls.list_json.network"] = Metric(
        value=float(count_group(counts, NETWORK_SYSCALLS)), unit="count", n=1
    )
    output.raw[f"n{n}"] = counts
    return output
