"""The run loop: prepare datasets, discover binaries, execute each planned suite in
order, persist outputs, summarize. A suite that raises fails the whole run — that is
the design's failure policy (a crash is never a skip)."""

from __future__ import annotations

import dataclasses
import json
import shutil
import tempfile
import time
from importlib import import_module
from pathlib import Path
from typing import TYPE_CHECKING

from ..datasets import Manifest, check_reusable, generate
from ..envinfo import collect_meta
from ..pipeline import build_plan, dataset_ns, summarize_dir
from ._env import discover

if TYPE_CHECKING:
    from ..budgets import Budget
    from ..results import Results, SuiteOutput


def prepare_datasets(bench_dir: Path, ns: tuple[int, ...]) -> dict[int, Manifest]:
    """Generate (or reuse) the shared libraries. The reuse rules live in
    datasets.check_reusable (covered): every generation input, including the version
    of the skit that WROTE the store, must match — else the run refuses."""
    out: dict[int, Manifest] = {}
    for n in ns:
        root = bench_dir / "datasets" / f"n{n}"
        if (root / "manifest.json").exists():
            manifest = Manifest.load(root)
            check_reusable(manifest, n)
            out[n] = manifest
        else:
            if root.exists():
                # No manifest = a crashed previous generation inside our own work
                # area; cleaning it is safe and required (generate refuses non-empty).
                shutil.rmtree(root)
            out[n] = generate(root, n)
    return out


def execute(
    profile: str,
    bench_dir: Path,
    repo_root: Path,
    budgets: list[Budget] | None,
    *,
    measured_root: Path | None = None,
) -> Results:
    """Run a profile end to end. `repo_root` is the harness checkout (fixtures, wheel
    builds); `measured_root` is where the benchmarked skit's git identity lives — the
    compare workflow points it at each side's checkout so the results name what they
    measured, never the harness ref."""
    t0 = time.monotonic()
    plans = build_plan(profile)
    bench_dir.mkdir(parents=True, exist_ok=True)
    # A crashed run must not leave a previous run's outputs for summarize/check to
    # merge with fresh partial data — that would stamp old provenance on new numbers.
    for stale in ("run.json", "results.json", "results.md"):
        (bench_dir / stale).unlink(missing_ok=True)
    datasets = prepare_datasets(bench_dir, dataset_ns(plans))
    suites_dir = bench_dir / "suites"
    suites_dir.mkdir(exist_ok=True)
    for stale_suite in suites_dir.glob("*.json"):
        stale_suite.unlink()  # a fresh run must not summarize a previous run's leftovers

    workdir = Path(tempfile.mkdtemp(prefix="skit-bench-"))  # OUTSIDE any uv project
    try:
        ctx = discover(repo_root, bench_dir, workdir, datasets)
        for plan in plans:
            module = import_module(f"benchmarks.suites.{plan.suite}")
            t_suite = time.monotonic()
            output: SuiteOutput = module.run(ctx, plan)
            if output.suite != plan.suite:
                raise RuntimeError(f"suite {plan.suite!r} returned output labeled {output.suite!r}")
            output = dataclasses.replace(output, duration_s=time.monotonic() - t_suite)
            (suites_dir / f"{plan.suite}.json").write_text(output.to_json(), encoding="utf-8")
    finally:
        shutil.rmtree(workdir, ignore_errors=True)

    meta = collect_meta(profile, measured_root or repo_root)
    (bench_dir / "run.json").write_text(
        json.dumps(
            {"meta": dataclasses.asdict(meta), "total_duration_s": time.monotonic() - t0},
            sort_keys=True,
            indent=2,
        )
        + "\n",
        encoding="utf-8",
    )
    return summarize_dir(bench_dir, budgets)
