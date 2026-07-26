"""imports — the deterministic fast-path census (module counts + heavyweight
presence, the enforced ratchets) and the `-X importtime` artifact."""

from __future__ import annotations

import json
import subprocess
from typing import TYPE_CHECKING

from ..parsers import census, importtime_top
from ..results import Metric, SuiteOutput
from ._env import PROBE_TIMEOUT_S, RunCtx, bench_env

if TYPE_CHECKING:
    from ..pipeline import SuitePlan

# Run the REAL CLI path, then dump the module census where the suite can read it.
# SystemExit is the normal Typer exit.
#
# The entry point is spelled the way pyproject's [project.scripts] spells it, and
# tests/test_benchmarks_tooling.py holds the two together. That is not pedantry: this
# probe used to import `skit.cli:app` directly with a comment claiming it was "exactly
# what the console script does", and when the console script became a dispatcher that
# answers --version WITHOUT importing the CLI, the census went on measuring the old
# path — reporting 279 modules and has_typer=1 for an invocation that really loads 150
# and no typer at all. Wrong-but-plausible numbers, from a probe nobody had reason to
# re-read.
CONSOLE_SCRIPT = "skit.__main__:main"
_ENTRY_MODULE, _ENTRY_ATTR = CONSOLE_SCRIPT.split(":")

_CENSUS_PROBE = f"""\
import json, os, sys
sys.argv = ["skit"] + json.loads(os.environ["BENCH_ARGS"])
from {_ENTRY_MODULE} import {_ENTRY_ATTR} as entry
code = None
try:
    entry()
except SystemExit as exc:
    code = exc.code
if code not in (None, 0):
    # SystemExit.code may be an int or a message string; re-raise either as-is.
    raise SystemExit(code)
with open(os.environ["BENCH_OUT"], "w", encoding="utf-8") as f:
    json.dump(sorted(sys.modules), f)
"""


def _census(ctx: RunCtx, env: dict[str, str], name: str, args: list[str]) -> list[str]:
    """Run one census probe under `env`; return the modules its process ended up with."""
    out_file = ctx.workdir / f"census_{name}.json"
    probe_env = dict(env)
    probe_env["BENCH_ARGS"] = json.dumps(args)
    probe_env["BENCH_OUT"] = str(out_file)
    subprocess.run(  # noqa: S603 — fixed-shape probe argv
        [ctx.python, "-c", _CENSUS_PROBE],
        cwd=ctx.workdir,
        env=probe_env,
        timeout=PROBE_TIMEOUT_S,
        check=True,
        stdout=subprocess.DEVNULL,
    )
    return json.loads(out_file.read_text(encoding="utf-8"))


def run(ctx: RunCtx, plan: SuitePlan) -> SuiteOutput:
    output = SuiteOutput(suite="imports")
    # `--version` never reads the library, so it carries no N: one probe, unsuffixed.
    # `list` does, so its census is measured per N. Resolving an entry's kind no longer
    # imports that language's grammar (capabilities resolve lazily) — the populated
    # tiers are where a REINTRODUCED grammar import would show, which is what the
    # enforced has_tree_sitter=0 row at n100 keys on. An empty-library census cannot
    # see it: no kind is ever resolved there.
    probes = [("version", ["--version"], ctx.datasets[plan.ns[0]].root)]
    probes += [(f"list_json.n{n}", ["list", "--json"], ctx.datasets[n].root) for n in plan.ns]
    for name, args, dataset_root in probes:
        modules = _census(ctx, bench_env(ctx, dataset_root), name, args)
        result = census(modules)
        output.metrics[f"imports.{name}.modules"] = Metric(
            value=float(result.modules), unit="count", n=1
        )
        for flag in ("has_typer", "has_rich", "has_textual", "has_tree_sitter"):
            output.metrics[f"imports.{name}.{flag}"] = Metric(
                value=float(getattr(result, flag)), unit="bool", n=1
            )
        output.raw[f"census_{name}"] = modules
    env = bench_env(ctx, ctx.datasets[plan.ns[0]].root)

    # Deliberately `skit.cli`, not the dispatcher: this artifact answers "where does the
    # CLI's import time go", which is what any REAL command pays. The dispatcher's own
    # graph is what the version census above measures, and it is nearly empty by design.
    timed = subprocess.run(  # noqa: S603 — fixed-shape probe argv
        [ctx.python, "-X", "importtime", "-c", "import skit.cli"],
        cwd=ctx.workdir,
        env=dict(env),
        timeout=PROBE_TIMEOUT_S,
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
