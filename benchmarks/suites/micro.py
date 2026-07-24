"""micro — pyperf in-process benchmarks (store/analyzers/launch/render) plus one-shot
cold import+parse samples, reported separately from warm loops (never averaged).

pyperf purifies worker environments, so every script invocation carries
`--inherit-environ` for the SKIT_*/BENCH_* vars, and every script asserts its dataset
env at startup — a missing dataset dies loudly instead of benchmarking an empty (or
worse, the developer's real) library."""

from __future__ import annotations

import subprocess
from pathlib import Path
from typing import TYPE_CHECKING

from ..fixtures import sources
from ..parsers import median, p95, pyperf_benchmarks, stddev
from ..results import Metric, Skip, SuiteOutput
from ._env import PYPERF_INHERIT, RunCtx, bench_env

if TYPE_CHECKING:
    from collections.abc import Mapping

    from ..pipeline import SuitePlan

_MICRO_DIR = Path(__file__).parent.parent / "micro"
_SOURCE_LINES = (20, 200, 2000)
_COLD_SAMPLES = 5

_COLD_PROBE = """\
import os, time
t0 = time.perf_counter()
from skit.langs.registry import spec_for
spec = spec_for(os.environ["BENCH_KIND"])
with open(os.environ["BENCH_SOURCE"], encoding="utf-8") as f:
    text = f.read()
if spec is None or spec.analyzer is None:
    print("SKIP")
else:
    spec.analyzer.analyze(text)
    print((time.perf_counter() - t0) * 1000)
"""


def run(ctx: RunCtx, plan: SuitePlan) -> SuiteOutput:
    output = SuiteOutput(suite="micro")
    sources_dir = _materialize_sources(ctx)

    for n in plan.ns:
        env = bench_env(ctx, ctx.datasets[n].root)
        env["BENCH_N"] = str(n)
        _run_script(ctx, plan, "bench_store.py", env, output, f"store_n{n}")

    # Analyzer availability is a property of the installed grammars: record what can't
    # run instead of letting it silently vanish from the numbers. Under compare, the
    # harness itself runs on the side's (possibly older) skit — the availability
    # probe must degrade like everything else, not kill the run.
    available = []
    try:
        from skit.langs.registry import spec_for
    except ImportError as exc:
        if not plan.compare_mode:
            raise
        output.skipped.append(
            Skip(suite="micro", case="analyzers", reason=f"harness import failed: {exc}")
        )
    else:
        for lang in sources.LANGS:
            spec = spec_for(lang)
            if spec is None or spec.analyzer is None:
                output.skipped.append(
                    Skip(
                        suite="micro",
                        case=f"analyze.{lang}",
                        reason="analyzer unavailable (grammar failed to import)",
                    )
                )
            else:
                available.append(lang)

    if available:
        env = bench_env(ctx, ctx.datasets[plan.ns[0]].root)
        env["BENCH_SOURCES_DIR"] = str(sources_dir)
        _run_script(ctx, plan, "bench_analyzers.py", env, output, "analyzers")
        for lang in available:
            _cold_parse(ctx, plan, lang, sources_dir, output)

    launch_n = max(plan.ns)
    env = bench_env(ctx, ctx.datasets[launch_n].root)
    _run_script(ctx, plan, "bench_launch.py", env, output, "launch")
    env = bench_env(ctx, ctx.datasets[0].root)
    _run_script(ctx, plan, "bench_render.py", env, output, "render")
    return output


def _materialize_sources(ctx: RunCtx) -> Path:
    sources_dir = ctx.workdir / "sources"
    sources_dir.mkdir(parents=True, exist_ok=True)
    for lang in sources.LANGS:
        for lines in _SOURCE_LINES:
            path = sources_dir / f"{lang}_{lines}.{sources.EXTENSIONS[lang]}"
            path.write_text(sources.generate(lang, lines), encoding="utf-8")
    return sources_dir


def _run_script(
    ctx: RunCtx,
    plan: SuitePlan,
    script: str,
    env: Mapping[str, str],
    output: SuiteOutput,
    label: str,
) -> None:
    out_json = ctx.workdir / f"pyperf_{label}.json"
    argv = [
        ctx.python,
        str(_MICRO_DIR / script),
        "-o",
        str(out_json),
        "--inherit-environ",
        ",".join(PYPERF_INHERIT),
    ]
    if plan.fast:
        argv.append("--fast")
    proc = subprocess.run(  # noqa: S603 — fixed-shape pyperf argv
        argv,
        cwd=ctx.workdir,
        env=dict(env),
        capture_output=True,
        text=True,
        check=False,
    )
    if proc.returncode != 0:
        # Under benchmark-compare (plan.compare_mode) a side whose skit predates a
        # micro script's API is recorded — with the actual error, never a canned
        # label — so an A/B against old refs degrades per-script instead of dying. A
        # normal run keeps the hard failure policy: a crash is a bug, not a skip.
        if plan.compare_mode:
            output.skipped.append(
                Skip(
                    suite="micro",
                    case=script,
                    reason=f"exit {proc.returncode}: {proc.stderr.strip()[-300:] or 'no stderr'}",
                )
            )
            return
        raise RuntimeError(
            f"{script} failed ({proc.returncode}):\n{proc.stdout[-1000:]}\n{proc.stderr[-2000:]}"
        )
    benches = pyperf_benchmarks(out_json.read_text(encoding="utf-8"))
    for bench in benches:
        us = [v * 1e6 for v in bench.values_s]
        output.metrics[f"micro.{bench.name}.median_us"] = Metric(
            value=median(us), unit="us", n=len(us), p95=p95(us), stddev=stddev(us)
        )
    output.raw[label] = {b.name: b.values_s for b in benches}


def _cold_parse(
    ctx: RunCtx, plan: SuitePlan, lang: str, sources_dir: Path, output: SuiteOutput
) -> None:
    source = sources_dir / f"{lang}_200.{sources.EXTENSIONS[lang]}"
    env = bench_env(ctx, ctx.datasets[0].root)
    env["BENCH_KIND"] = lang
    env["BENCH_SOURCE"] = str(source)
    samples: list[float] = []
    for _ in range(_COLD_SAMPLES):
        proc = subprocess.run(  # noqa: S603 — fixed-shape probe argv
            [ctx.python, "-c", _COLD_PROBE],
            cwd=ctx.workdir,
            env=env,
            capture_output=True,
            text=True,
            check=False,
        )
        if proc.returncode != 0:
            # Same compare-mode degradation as _run_script: an older side's probe
            # failure records what broke; a normal run treats it as a pipeline bug.
            if plan.compare_mode:
                output.skipped.append(
                    Skip(
                        suite="micro",
                        case=f"analyze_cold.{lang}",
                        reason=f"exit {proc.returncode}: "
                        f"{proc.stderr.strip()[-300:] or 'no stderr'}",
                    )
                )
                return
            raise RuntimeError(
                f"cold-parse probe for {lang} failed ({proc.returncode}): {proc.stderr[-500:]}"
            )
        text = proc.stdout.strip()
        if text == "SKIP":
            # The orchestrator already checked availability; a worker-side SKIP means
            # the environments disagree — that's a bug, not a skip.
            raise RuntimeError(f"cold-parse probe for {lang} lost its analyzer")
        samples.append(float(text))
    cold_raw = output.raw.setdefault("analyze_cold", {})
    cold_raw[lang] = {"samples_ms": samples}
    output.metrics[f"micro.analyze_cold.{lang}.median_ms"] = Metric(
        value=median(samples), unit="ms", n=len(samples), p95=p95(samples), stddev=stddev(samples)
    )
