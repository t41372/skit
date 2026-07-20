"""hyperfine, minus the process: pure command-set building and export parsing.

The suites spawn the binary; this module decides the argv and reads the JSON, so the
decisions and the parsing sit under the coverage floor while the spawn stays a
one-liner in orchestration code.
"""

from __future__ import annotations

import json
import shlex
from dataclasses import dataclass

from .parsers import ParseError, median, p95, stddev
from .results import Metric

# The pinned binary the workflows install (upstream publishes no checksum assets, so
# the digest was computed once at pin time; the workflow verifies against it).
HYPERFINE_VERSION = "1.20.0"
HYPERFINE_SHA256 = "63ad53934062118f5b0be11785e0bb1603d4b91667d1921f2fd8df9a8712040a"
HYPERFINE_URL = (
    "https://github.com/sharkdp/hyperfine/releases/download/"
    f"v{HYPERFINE_VERSION}/hyperfine-v{HYPERFINE_VERSION}-x86_64-unknown-linux-gnu.tar.gz"
)


@dataclass(frozen=True)
class Case:
    """One benchmarked command. `argv` is a real argv — under `--shell=none` hyperfine
    word-splits the command string itself, so we shlex-join here and nothing ever goes
    through a shell."""

    name: str
    argv: tuple[str, ...]


def build_argv(
    cases: list[Case],
    *,
    warmup: int,
    min_runs: int,
    export_json: str,
    hyperfine_bin: str = "hyperfine",
) -> list[str]:
    if not cases:
        raise ValueError("no cases to benchmark")
    argv = [
        hyperfine_bin,
        "--shell=none",
        "--style",
        "basic",
        "--warmup",
        str(warmup),
        "--min-runs",
        str(min_runs),
        "--export-json",
        export_json,
    ]
    for case in cases:
        argv += ["--command-name", case.name, shlex.join(case.argv)]
    return argv


def parse_export(json_text: str) -> dict[str, list[float]]:
    """The export file → {case name: times in seconds}. Any non-zero exit code in the
    samples is a broken benchmark, not a datapoint (hyperfine itself aborts on the
    first failure by default; this guards the parse against `--ignore-failure` ever
    creeping in)."""
    try:
        doc = json.loads(json_text)
    except json.JSONDecodeError as exc:
        raise ParseError(f"hyperfine export is not JSON: {exc}") from exc
    results = doc.get("results") if isinstance(doc, dict) else None
    if not isinstance(results, list) or not results:
        raise ParseError("hyperfine export has no results")
    out: dict[str, list[float]] = {}
    for entry in results:
        name = entry.get("command")
        times = entry.get("times")
        if not isinstance(name, str) or not isinstance(times, list) or not times:
            raise ParseError("hyperfine result entry missing command/times")
        codes = entry.get("exit_codes", [])
        if any(code != 0 for code in codes):
            raise ParseError(f"hyperfine case {name!r} recorded non-zero exit codes")
        out[name] = [float(t) for t in times]
    return out


def metric_from_times(times_s: list[float]) -> Metric:
    """Times in seconds → the standard statistical Metric in milliseconds."""
    ms = [t * 1000 for t in times_s]
    return Metric(value=median(ms), unit="ms", n=len(ms), p95=p95(ms), stddev=stddev(ms))
