"""Pure run-plan logic: profile → suite plans, merge + derived metrics, the rendered
summary, and the history-export conversion. Suites spawn processes; this module makes
every decision about what runs and what the numbers mean — gate code, covered."""

from __future__ import annotations

import json
import math
from dataclasses import dataclass
from typing import TYPE_CHECKING, Any

from .budgets import Report, evaluate, render_report
from .results import Meta, Metric, Results, Skip, SuiteOutput, meta_from_dict

if TYPE_CHECKING:
    from pathlib import Path

    from .budgets import Budget

PROFILES = ("pr", "full", "compare")


class PipelineError(RuntimeError):
    """The run plan or the merge is inconsistent — always a pipeline bug, never noise."""


@dataclass(frozen=True)
class SuitePlan:
    suite: str
    ns: tuple[int, ...] = ()
    warmup: int = 3
    min_runs: int = 15
    samples: int = 5
    fast: bool = True
    closure: bool = False
    js_lane: bool = False
    doctor: bool = False
    # A/B side run (benchmark-compare): suites that import skit internals degrade
    # per-case with recorded skips instead of failing the run. Carried on the plan —
    # the one source of truth for execution mode — never on ambient env.
    compare_mode: bool = False


def build_plan(profile: str) -> tuple[SuitePlan, ...]:
    """The suites x profiles table from docs/design/benchmarks.md, as data."""
    if profile == "pr":
        return (
            SuitePlan("imports", ns=(0,)),
            SuitePlan("footprint", closure=False),
            SuitePlan("rss", ns=(0, 1000), samples=5),
            SuitePlan("startup", ns=(0,), warmup=3, min_runs=15),
            SuitePlan("scale", ns=(0, 100, 1000), warmup=3, min_runs=15),
            SuitePlan("run_overhead", warmup=3, min_runs=15, js_lane=False),
            SuitePlan("micro", ns=(0, 100, 1000), fast=True),
            SuitePlan("tui", ns=(0, 100, 1000), samples=5),
        )
    if profile == "full":
        return (
            SuitePlan("imports", ns=(0,)),
            SuitePlan("footprint", closure=True),
            SuitePlan("rss", ns=(0, 1000), samples=10),
            SuitePlan("startup", ns=(0,), warmup=5, min_runs=40),
            SuitePlan("scale", ns=(0, 10, 100, 1000), warmup=5, min_runs=40, doctor=True),
            SuitePlan("run_overhead", warmup=5, min_runs=40, js_lane=True),
            SuitePlan("micro", ns=(0, 100, 1000), fast=False),
            SuitePlan("tui", ns=(0, 100, 1000), samples=10),
            SuitePlan("syscalls", ns=(1000,)),
        )
    if profile == "compare":
        # The A/B profile benchmark-compare.yml runs once per side: pr minus
        # footprint. footprint builds the wheel from the HARNESS checkout, so under
        # A/B it would measure the invoking ref twice — wrong-but-plausible data,
        # exactly the failure class this pipeline exists to prevent.
        return (
            SuitePlan("imports", ns=(0,), compare_mode=True),
            SuitePlan("rss", ns=(0, 1000), samples=5, compare_mode=True),
            SuitePlan("startup", ns=(0,), warmup=3, min_runs=15, compare_mode=True),
            SuitePlan("scale", ns=(0, 100, 1000), warmup=3, min_runs=15, compare_mode=True),
            SuitePlan("run_overhead", warmup=3, min_runs=15, js_lane=False, compare_mode=True),
            SuitePlan("micro", ns=(0, 100, 1000), fast=True, compare_mode=True),
            SuitePlan("tui", ns=(0, 100, 1000), samples=5, compare_mode=True),
        )
    raise PipelineError(f"unknown profile {profile!r} (expected one of {PROFILES})")


def dataset_ns(plans: tuple[SuitePlan, ...]) -> tuple[int, ...]:
    """Every library size any planned suite needs — generated once, shared."""
    return tuple(sorted({n for plan in plans for n in plan.ns}))


# ---------------------------------------------------------------- merge + derive


def merge(meta: Meta, outputs: list[SuiteOutput], total_duration_s: float) -> Results:
    """Union the suite outputs into one Results: duplicate metric IDs are a pipeline
    bug (fail loudly), per-suite durations and the skip count become metrics, and the
    derived metrics are computed from whatever inputs actually arrived."""
    metrics: dict[str, Metric] = {}
    skipped: list[Skip] = []
    raw: dict[str, Any] = {}
    for output in outputs:
        for metric_id, metric in output.metrics.items():
            if metric_id.startswith("pipeline."):
                raise PipelineError(f"reserved pipeline metric id {metric_id!r}")
            if metric_id in metrics:
                raise PipelineError(f"duplicate metric id {metric_id!r}")
            metrics[metric_id] = metric
        skipped.extend(output.skipped)
        if output.suite in raw:
            raise PipelineError(f"duplicate suite output {output.suite!r}")
        raw[output.suite] = output.raw
        metrics[f"pipeline.suite.{output.suite}.duration_s"] = Metric(
            value=round(output.duration_s, 3), unit="s", n=1
        )
    metrics["pipeline.duration_s"] = Metric(value=round(total_duration_s, 3), unit="s", n=1)
    metrics["pipeline.skipped_count"] = Metric(value=float(len(skipped)), unit="count", n=1)
    for metric_id, metric in derive(metrics).items():
        if metric_id in metrics:
            raise PipelineError(f"derived metric id {metric_id!r} already present")
        metrics[metric_id] = metric
    return Results(meta=meta, metrics=metrics, skipped=skipped, raw=raw)


@dataclass(frozen=True)
class Derivation:
    """One cross-suite delta. `strict` pairs are minted unconditionally by a single
    suite (both inputs appear together or not at all), so a HALF-present pair can
    only mean a renamed/broken producer — the loud-failure channel that keeps a
    metric rename from silently dropping a headline number. Non-strict pairs
    (scale's endpoints) depend on the plan's N grid and may legitimately lose one
    side to a grid change."""

    target: str
    minuend: str
    subtrahend: str
    unit: str
    strict: bool


DERIVATIONS: tuple[Derivation, ...] = (
    Derivation(
        "startup.version.over_python_ms",
        "startup.version.median_ms",
        "startup.python.median_ms",
        "ms",
        strict=True,
    ),
    # (ms delta over 1000 entries) → µs per entry: ÷1000 entries x 1000 µs/ms cancel.
    Derivation(
        "scale.list_json.per_entry_us",
        "scale.list_json.n1000.median_ms",
        "scale.list_json.n0.median_ms",
        "us",
        strict=False,
    ),
    Derivation(
        "run_overhead.python.overhead_ms",
        "run_overhead.python.skit.median_ms",
        "run_overhead.python.uv_script.median_ms",
        "ms",
        strict=True,
    ),
    Derivation(
        "run_overhead.shell.overhead_ms",
        "run_overhead.shell.skit.median_ms",
        "run_overhead.shell.bash.median_ms",
        "ms",
        strict=True,
    ),
    Derivation(
        "run_overhead.js.overhead_ms",
        "run_overhead.js.skit.median_ms",
        "run_overhead.js.node.median_ms",
        "ms",
        strict=True,
    ),
)


def derive(metrics: dict[str, Metric]) -> dict[str, Metric]:
    """Cross-suite deltas from DERIVATIONS. Both inputs present → derived; both
    absent (suite/lane skipped) → the derivation is omitted and its budget reports
    metric-missing; exactly ONE present on a strict pair → a renamed/broken producer,
    which fails loudly here instead of quietly dropping a headline metric."""
    out: dict[str, Metric] = {}
    for d in DERIVATIONS:
        a, b = metrics.get(d.minuend), metrics.get(d.subtrahend)
        if a is not None and b is not None:
            out[d.target] = Metric(value=round(a.value - b.value, 4), unit=d.unit, n=1)
        elif d.strict and (a is not None) != (b is not None):
            present, absent = (
                (d.minuend, d.subtrahend) if a is not None else (d.subtrahend, d.minuend)
            )
            raise PipelineError(
                f"derivation {d.target!r}: input pair half-present ({present} without "
                f"{absent}) — was a suite's metric ID renamed?"
            )
    return out


# ---------------------------------------------------------------- rendering / export

HEADLINE_METRICS: tuple[str, ...] = (
    "startup.version.median_ms",
    "startup.version.over_python_ms",
    "startup.list_json.median_ms",
    "scale.list_json.n100.median_ms",
    "scale.list_json.n1000.median_ms",
    "scale.list_json.per_entry_us",
    "run_overhead.python.overhead_ms",
    "run_overhead.shell.overhead_ms",
    "tui.first_idle.n100.median_ms",
    "tui.first_idle.n1000.median_ms",
    "tui.search.n1000.median_ms",
    "rss.version.peak_kib",
    "rss.list_json.n1000.peak_kib",
    "imports.version.modules",
    "imports.list_json.modules",
    "footprint.wheel_bytes",
    "footprint.closure_bytes",
    "micro.store.list_entries.n1000.median_us",
    "syscalls.list_json.file_ops",
    "pipeline.duration_s",
)


def render_markdown(results: Results, report: Report | None = None) -> str:
    meta = results.meta
    dirty = " (dirty)" if meta.git.dirty else ""
    lines = [
        "## Benchmark results",
        "",
        f"skit {meta.skit_version} @ `{meta.git.commit[:12]}`{dirty} · "
        f"profile **{meta.profile}** · {meta.generated_at}",
        "",
        f"{meta.host.os} {meta.host.kernel} · {meta.host.cpu} x {meta.host.cpu_count} · "
        f"{meta.host.mem_total_mib} MiB · python {meta.python} · uv {meta.uv} · "
        f"textual {meta.textual} · runner "
        f"{meta.host.ci_runner if meta.host.ci_runner else 'local'}",
        "",
        "| Metric | Value | p95 | n |",
        "| --- | ---: | ---: | ---: |",
    ]
    for metric_id in HEADLINE_METRICS:
        metric = results.metrics.get(metric_id)
        if metric is None:
            continue
        p95 = f"{metric.p95:g}" if metric.p95 is not None else "—"
        lines.append(f"| `{metric_id}` | {metric.value:g} {metric.unit} | {p95} | {metric.n} |")
    if results.skipped:
        lines += ["", f"### Skipped ({len(results.skipped)})", ""]
        lines += [f"- `{s.suite}/{s.case}`: {s.reason}" for s in results.skipped]
    else:
        lines += ["", "No skipped cases."]
    if report is not None:
        lines += ["", "### Budgets", "", "```", render_report(report).rstrip(), "```"]
    return "\n".join(lines) + "\n"


def export_gha(results: Results) -> list[dict[str, Any]]:
    """github-action-benchmark `customSmallerIsBetter` rows for the headline metrics
    that this run actually produced. Names are the stable metric IDs."""
    rows = [
        {"name": metric_id, "unit": metric.unit, "value": metric.value}
        for metric_id in HEADLINE_METRICS
        if (metric := results.metrics.get(metric_id)) is not None
    ]
    if not rows:
        raise PipelineError("no headline metrics present — nothing to export")
    return rows


# ---------------------------------------------------------------- run-dir summarize


def summarize_dir(bench_dir: Path, budgets: list[Budget] | None = None) -> Results:
    """Merge a run directory (run.json + suites/*.json) into results.json/results.md.
    `run` calls this at the end of a run; the `summarize` subcommand re-runs it."""
    run_path = bench_dir / "run.json"
    if not run_path.exists():
        raise PipelineError(f"no run.json in {bench_dir} — did `run` complete?")
    try:
        run_doc = json.loads(run_path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        raise PipelineError(f"run.json is not valid JSON ({exc}) — crashed run?") from exc
    meta = meta_from_dict(run_doc.get("meta"))
    total = run_doc.get("total_duration_s")
    if not isinstance(total, int | float) or isinstance(total, bool) or not math.isfinite(total):
        raise PipelineError("run.json total_duration_s: expected a finite number")
    suite_files = sorted((bench_dir / "suites").glob("*.json"))
    if not suite_files:
        raise PipelineError(f"no suite outputs under {bench_dir / 'suites'}")
    outputs = [SuiteOutput.from_json(p.read_text(encoding="utf-8")) for p in suite_files]
    results = merge(meta, outputs, float(total))
    report = evaluate(budgets, results) if budgets is not None else None
    (bench_dir / "results.json").write_text(results.to_json(), encoding="utf-8")
    (bench_dir / "results.md").write_text(render_markdown(results, report), encoding="utf-8")
    return results
