"""Thin argparse front door — every decision lives in the covered modules; this file
only wires arguments to functions and functions to exit codes.

    uv run python -m benchmarks datasets --n 1000 --out .bench/datasets/n1000
    uv run python -m benchmarks run --profile pr --out .bench
    uv run python -m benchmarks summarize .bench
    uv run python -m benchmarks check .bench/results.json [--propose] [--require-enforced]
    uv run python -m benchmarks compare base.json head.json
    uv run python -m benchmarks export-gha .bench/results.json --out gha.json
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

from .budgets import BudgetsError, evaluate, load_budgets, propose, render_report
from .compare import compare as compare_results
from .compare import render_markdown as render_compare
from .datasets import DEFAULT_SEED, DEFAULT_STATE_FRACTION, DatasetError, generate
from .parsers import ParseError
from .pipeline import PROFILES, PipelineError, export_gha, summarize_dir
from .results import Results, ResultsError

_REPO_ROOT = Path(__file__).resolve().parent.parent
_DEFAULT_BUDGETS = Path(__file__).resolve().parent / "budgets.toml"


def _build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="benchmarks", description="skit's performance evaluation pipeline"
    )
    sub = parser.add_subparsers(dest="command", required=True)

    p = sub.add_parser("datasets", help="generate a deterministic benchmark library")
    p.add_argument("--n", type=int, required=True)
    p.add_argument("--out", type=Path, required=True)
    p.add_argument("--seed", type=int, default=DEFAULT_SEED)
    p.add_argument("--state-fraction", type=float, default=DEFAULT_STATE_FRACTION)

    p = sub.add_parser("run", help="run a profile end to end")
    p.add_argument("--profile", choices=PROFILES, required=True)
    p.add_argument("--out", type=Path, default=Path(".bench"))
    p.add_argument("--budgets", type=Path, default=_DEFAULT_BUDGETS)

    p = sub.add_parser("summarize", help="re-merge a run directory into results.json/md")
    p.add_argument("bench_dir", type=Path)
    p.add_argument("--budgets", type=Path, default=_DEFAULT_BUDGETS)

    p = sub.add_parser("check", help="evaluate results against the budget contract")
    p.add_argument("results", type=Path)
    p.add_argument("--budgets", type=Path, default=_DEFAULT_BUDGETS)
    p.add_argument("--propose", action="store_true", help="print refreshed budgets.toml")
    p.add_argument(
        "--require-enforced",
        action="store_true",
        help="also fail when zero applicable enforced rows were evaluated (CI passes this)",
    )

    p = sub.add_parser("compare", help="A/B delta table between two results files")
    p.add_argument("base", type=Path)
    p.add_argument("head", type=Path)

    p = sub.add_parser("export-gha", help="headline metrics as customSmallerIsBetter JSON")
    p.add_argument("results", type=Path)
    p.add_argument("--out", type=Path, default=None)
    return parser


def _load_results(path: Path) -> Results:
    return Results.from_json(path.read_text(encoding="utf-8"))


def main(argv: list[str] | None = None) -> int:
    args = _build_parser().parse_args(argv)
    if args.command == "datasets":
        manifest = generate(args.out, args.n, seed=args.seed, state_fraction=args.state_fraction)
        print(f"generated {manifest.n} entries in {manifest.root}")
        return 0
    if args.command == "run":
        from .suites._run import execute

        budgets = load_budgets(args.budgets.read_text(encoding="utf-8"))
        results = execute(args.profile, args.out, _REPO_ROOT, budgets)
        print(f"results: {args.out / 'results.json'} ({len(results.metrics)} metrics)")
        return 0
    if args.command == "summarize":
        budgets = load_budgets(args.budgets.read_text(encoding="utf-8"))
        results = summarize_dir(args.bench_dir, budgets)
        print(f"results: {args.bench_dir / 'results.json'} ({len(results.metrics)} metrics)")
        return 0
    if args.command == "check":
        results = _load_results(args.results)
        budgets = load_budgets(args.budgets.read_text(encoding="utf-8"))
        if args.propose:
            print(propose(budgets, results), end="")
            return 0
        report = evaluate(budgets, results)
        print(render_report(report), end="")
        if report.failures:
            return 1
        if args.require_enforced and report.enforced_evaluated == 0:
            print("check: zero applicable enforced rows were evaluated", file=sys.stderr)
            return 1
        return 0
    if args.command == "compare":
        base, head = _load_results(args.base), _load_results(args.head)
        print(render_compare(base, head, compare_results(base, head)), end="")
        return 0
    # export-gha
    rows = export_gha(_load_results(args.results))
    text = json.dumps(rows, indent=2) + "\n"
    if args.out is None:
        print(text, end="")
    else:
        args.out.write_text(text, encoding="utf-8")
        print(f"wrote {len(rows)} rows to {args.out}")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (
        ResultsError,
        BudgetsError,
        PipelineError,
        DatasetError,
        ParseError,
        RuntimeError,
    ) as exc:
        print(f"benchmarks: {exc}", file=sys.stderr)
        sys.exit(1)
