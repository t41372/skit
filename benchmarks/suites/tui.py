"""tui — first-idle / search-responsiveness / peak-RSS proxies via a fresh headless
probe process per sample (see tui_probe.py for the measured spans and assertions)."""

from __future__ import annotations

import json
import subprocess
from pathlib import Path
from typing import TYPE_CHECKING

from ..datasets import SEARCH_PROBE_CHAR
from ..parsers import median, p95, stddev, vmhwm_kib
from ..results import Metric, Skip, SuiteOutput
from ._env import RunCtx, bench_env

if TYPE_CHECKING:
    from ..pipeline import SuitePlan

_PROBE = Path(__file__).parent / "tui_probe.py"


def run(ctx: RunCtx, plan: SuitePlan) -> SuiteOutput:
    output = SuiteOutput(suite="tui")
    # Peak-RSS needs /proc/self/status: a PRE-SPAWN decision, recorded once. A parse
    # failure on a host where the file exists is a crash, never a skip (the design's
    # failure taxonomy) — so only existence is checked here.
    proc_status = Path("/proc/self/status").exists()
    if not proc_status:
        output.skipped.append(
            Skip(suite="tui", case="peak_rss", reason="no /proc/self/status on this host")
        )
    import_samples: list[float] = []
    for n in plan.ns:
        env = bench_env(ctx, ctx.datasets[n].root)
        first_idle: list[float] = []
        search: list[float] = []
        peaks: list[float] = []
        for i in range(plan.samples):
            out_file = ctx.workdir / f"tui_{n}_{i}.json"
            subprocess.run(  # noqa: S603 — fixed-shape probe argv
                [
                    ctx.python,
                    str(_PROBE),
                    "--entries",
                    str(n),
                    "--probe-char",
                    SEARCH_PROBE_CHAR,
                    "--out",
                    str(out_file),
                ],
                cwd=ctx.workdir,
                env=dict(env),
                check=True,
                stdout=subprocess.DEVNULL,
            )
            doc = json.loads(out_file.read_text(encoding="utf-8"))
            first_idle.append(doc["first_idle_ms"])
            search.append(doc["search_ms"])
            if n == 0:
                import_samples.append(doc["import_ms"])
            if proc_status:
                peaks.append(float(vmhwm_kib(doc["status_text"])))
        output.metrics[f"tui.first_idle.n{n}.median_ms"] = _stat(first_idle)
        output.metrics[f"tui.search.n{n}.median_ms"] = _stat(search)
        if peaks:
            output.metrics[f"tui.rss.n{n}.peak_kib"] = Metric(
                value=median(peaks), unit="KiB", n=len(peaks)
            )
        output.raw[f"n{n}"] = {"first_idle_ms": first_idle, "search_ms": search}
    if import_samples:
        output.metrics["tui.import.median_ms"] = _stat(import_samples)
    return output


def _stat(values: list[float]) -> Metric:
    return Metric(
        value=median(values),
        unit="ms",
        n=len(values),
        p95=p95(values),
        stddev=stddev(values),
    )
