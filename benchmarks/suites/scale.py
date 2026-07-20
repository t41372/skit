"""scale — the same CLI reads against growing libraries; the per-entry slope is the
evidence the future summary-index PR will be judged on."""

from __future__ import annotations

from typing import TYPE_CHECKING

from ..hyperfine import Case
from ..results import Skip, SuiteOutput
from ._env import RunCtx, bench_env, run_hyperfine

if TYPE_CHECKING:
    from ..pipeline import SuitePlan


def run(ctx: RunCtx, plan: SuitePlan) -> SuiteOutput:
    if ctx.hyperfine is None:
        return SuiteOutput(
            suite="scale",
            skipped=[Skip(suite="scale", case="all", reason="hyperfine not found")],
        )
    output = SuiteOutput(suite="scale")
    for n in plan.ns:
        manifest = ctx.datasets[n]
        env = bench_env(ctx, manifest.root)
        cases = [
            Case(f"scale.list.n{n}", (ctx.skit, "list")),
            Case(f"scale.list_json.n{n}", (ctx.skit, "list", "--json")),
        ]
        if n > 0:
            cases.append(Case(f"scale.show.n{n}", (ctx.skit, "show", manifest.mid_slug, "--json")))
        if plan.doctor:
            cases.append(Case(f"scale.doctor_json.n{n}", (ctx.skit, "doctor", "--json")))
        metrics, raw = run_hyperfine(
            ctx,
            cases,
            warmup=plan.warmup,
            min_runs=plan.min_runs,
            env=env,
            export_name=f"scale_n{n}",
        )
        output.metrics.update(metrics)
        output.raw[f"n{n}"] = raw
    return output
