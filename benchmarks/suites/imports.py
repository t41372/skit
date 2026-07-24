"""imports — the deterministic fast-path census (module counts + heavyweight
presence, the enforced ratchets) and the `-X importtime` artifact."""

from __future__ import annotations

import json
import subprocess
from typing import TYPE_CHECKING

from ..parsers import census, importtime_top
from ..results import Metric, SuiteOutput
from ._env import RunCtx, bench_env

if TYPE_CHECKING:
    from ..pipeline import SuitePlan

# Run the REAL CLI path (exactly what the console script does), then dump the module
# census where the suite can read it. SystemExit is the normal Typer exit.
_CENSUS_PROBE = """\
import json, os, sys
sys.argv = ["skit"] + json.loads(os.environ["BENCH_ARGS"])
from skit.cli import app
code = None
try:
    app()
except SystemExit as exc:
    code = exc.code
if code not in (None, 0):
    # SystemExit.code may be an int or a message string; re-raise either as-is.
    raise SystemExit(code)
with open(os.environ["BENCH_OUT"], "w", encoding="utf-8") as f:
    json.dump(sorted(sys.modules), f)
"""


def run(ctx: RunCtx, plan: SuitePlan) -> SuiteOutput:
    output = SuiteOutput(suite="imports")
    env = bench_env(ctx, ctx.datasets[0].root)
    for probe, args in (("version", ["--version"]), ("list_json", ["list", "--json"])):
        out_file = ctx.workdir / f"census_{probe}.json"
        probe_env = dict(env)
        probe_env["BENCH_ARGS"] = json.dumps(args)
        probe_env["BENCH_OUT"] = str(out_file)
        subprocess.run(  # noqa: S603 — fixed-shape probe argv
            [ctx.python, "-c", _CENSUS_PROBE],
            cwd=ctx.workdir,
            env=probe_env,
            check=True,
            stdout=subprocess.DEVNULL,
        )
        modules = json.loads(out_file.read_text(encoding="utf-8"))
        result = census(modules)
        output.metrics[f"imports.{probe}.modules"] = Metric(
            value=float(result.modules), unit="count", n=1
        )
        for flag in ("has_typer", "has_rich", "has_textual", "has_tree_sitter"):
            output.metrics[f"imports.{probe}.{flag}"] = Metric(
                value=float(getattr(result, flag)), unit="bool", n=1
            )
        output.raw[f"census_{probe}"] = modules

    timed = subprocess.run(  # noqa: S603 — fixed-shape probe argv
        [ctx.python, "-X", "importtime", "-c", "import skit.cli"],
        cwd=ctx.workdir,
        env=dict(env),
        check=True,
        capture_output=True,
        text=True,
    )
    artifacts = ctx.out_dir / "artifacts"
    artifacts.mkdir(parents=True, exist_ok=True)
    (artifacts / "importtime.txt").write_text(timed.stderr, encoding="utf-8")
    output.raw["importtime_top"] = [
        {"module": t.module, "self_us": t.self_us, "cumulative_us": t.cumulative_us}
        for t in importtime_top(timed.stderr)
    ]
    return output
