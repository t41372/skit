"""startup — process-cold, filesystem-warm launch cost against an empty library."""

from __future__ import annotations

from typing import TYPE_CHECKING

from ..hyperfine import Case
from ..results import SuiteOutput
from ._env import RunCtx, bench_env, run_hyperfine

if TYPE_CHECKING:
    from ..pipeline import SuitePlan


def run(ctx: RunCtx, plan: SuitePlan) -> SuiteOutput:
    if ctx.hyperfine is None:
        return SuiteOutput.skip_all("startup", "hyperfine not found")
    env = bench_env(ctx, ctx.datasets[0].root)
    cases = [
        Case("startup.python", (ctx.python, "-c", "pass")),
        Case("startup.import_skit", (ctx.python, "-c", "import skit")),
        Case("startup.import_skit_cli", (ctx.python, "-c", "import skit.cli")),
        Case("startup.version", (ctx.skit, "--version")),
        Case("startup.help", (ctx.skit, "--help")),
        Case("startup.list", (ctx.skit, "list")),
        Case("startup.list_json", (ctx.skit, "list", "--json")),
    ]
    metrics, raw = run_hyperfine(
        ctx,
        cases,
        warmup=plan.warmup,
        min_runs=plan.min_runs,
        env=env,
        export_name="startup",
    )
    return SuiteOutput(suite="startup", metrics=metrics, raw=raw)
