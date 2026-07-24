"""run_overhead — skit's launch cost over the raw interpreter, per lane, with the
uv-script lane matching skit's EXACT argv (`uv run --no-project --script`; see
src/skit/langs/launch.py). Lane C legitimately includes skit's post-run state
persistence (two fsync'd constant-size writes). Uses its own dedicated 3-entry
library, so resolve cost never rides on the scale grid's N."""

from __future__ import annotations

from typing import TYPE_CHECKING

from ..datasets import generate_runover
from ..hyperfine import Case
from ..results import Skip, SuiteOutput
from ._env import RunCtx, bench_env, run_hyperfine

if TYPE_CHECKING:
    from ..pipeline import SuitePlan


def run(ctx: RunCtx, plan: SuitePlan) -> SuiteOutput:
    if ctx.hyperfine is None:
        return SuiteOutput.skip_all("run_overhead", "hyperfine not found")
    if ctx.uv is None:
        return SuiteOutput.skip_all("run_overhead", "uv not found")
    library = generate_runover(ctx.workdir / "runover", ctx.fixtures_dir)
    env = bench_env(ctx, library.root)
    src = library.root / "srcfiles"
    output = SuiteOutput(suite="run_overhead")

    cases = [
        Case("run_overhead.python.python", (ctx.python, str(src / "noop.py"))),
        Case(
            "run_overhead.python.uv_script",
            (ctx.uv, "run", "--no-project", "--script", str(src / "noop.py")),
        ),
        Case("run_overhead.python.skit", (ctx.skit, "run", "noop-py", "--no-input")),
    ]
    if ctx.bash is None:
        output.skipped.append(Skip(suite="run_overhead", case="shell", reason="bash not found"))
    else:
        cases += [
            Case("run_overhead.shell.bash", (ctx.bash, str(src / "noop.sh"))),
            Case("run_overhead.shell.skit", (ctx.skit, "run", "noop-sh", "--no-input")),
        ]
    if plan.js_lane:
        if ctx.node is None:
            output.skipped.append(Skip(suite="run_overhead", case="js", reason="node not found"))
        else:
            cases += [
                Case("run_overhead.js.node", (ctx.node, str(src / "noop.js"))),
                Case("run_overhead.js.skit", (ctx.skit, "run", "noop-js", "--no-input")),
            ]
    metrics, raw = run_hyperfine(
        ctx,
        cases,
        warmup=plan.warmup,
        min_runs=plan.min_runs,
        env=env,
        export_name="run_overhead",
    )
    output.metrics.update(metrics)
    output.raw.update(raw)
    return output
