"""The pure parse/derive layer: everything that turns tool output into metric values.

Census dumps, `-X importtime` stderr, VmHWM lines, getrusage numbers, `strace -c`
tables, pyperf JSON — enforced metrics (the import ratchets) are computed here, so this
code is gate code and sits under the 100% coverage floor. Nothing in this module spawns
a process or touches the host.
"""

from __future__ import annotations

import json
import math
import re
from dataclasses import dataclass
from typing import Any


class ParseError(ValueError):
    """Tool output didn't have the shape this parser was promised."""


# ---------------------------------------------------------------- statistics


def median(values: list[float]) -> float:
    if not values:
        raise ParseError("median of no values")
    ordered = sorted(values)
    mid = len(ordered) // 2
    if len(ordered) % 2:
        return ordered[mid]
    return (ordered[mid - 1] + ordered[mid]) / 2


def p95(values: list[float]) -> float:
    """Nearest-rank 95th percentile (ceil(0.95·n)-th of the sorted samples) — simple,
    deterministic, and documented in benchmarks/README.md."""
    if not values:
        raise ParseError("p95 of no values")
    ordered = sorted(values)
    rank = math.ceil(0.95 * len(ordered))
    return ordered[rank - 1]


def stddev(values: list[float]) -> float:
    if not values:
        raise ParseError("stddev of no values")
    if len(values) == 1:
        return 0.0
    mean = sum(values) / len(values)
    return math.sqrt(sum((v - mean) ** 2 for v in values) / (len(values) - 1))


# ---------------------------------------------------------------- import census


@dataclass(frozen=True)
class Census:
    modules: int
    has_typer: bool
    has_rich: bool
    has_textual: bool
    has_tree_sitter: bool


def census(module_names: list[str]) -> Census:
    """The deterministic fast-path census: how many modules a command imported, and
    whether the heavyweights are among them. `tree_sitter` matches the package and
    every grammar wheel (`tree_sitter_bash`, …)."""
    names = set(module_names)

    def has(prefix: str) -> bool:
        return any(n == prefix or n.startswith(prefix + ".") for n in names)

    return Census(
        modules=len(names),
        has_typer=has("typer"),
        has_rich=has("rich"),
        has_textual=has("textual"),
        has_tree_sitter=has("tree_sitter") or any(n.startswith("tree_sitter_") for n in names),
    )


# ---------------------------------------------------------------- -X importtime


@dataclass(frozen=True)
class ImportTiming:
    module: str
    self_us: int
    cumulative_us: int


_IMPORTTIME_RE = re.compile(r"^import time:\s+(\d+)\s+\|\s+(\d+)\s+\|(\s+)(\S.*)$")


def importtime_top(stderr: str, top: int = 20) -> list[ImportTiming]:
    """Top offenders by cumulative µs from `-X importtime` stderr. All depths are
    ranked together (a child's cumulative time is part of its parent's — the artifact
    carries the full text for the tree view; this is the summary)."""
    rows: list[ImportTiming] = []
    for line in stderr.splitlines():
        m = _IMPORTTIME_RE.match(line)
        if m:
            rows.append(
                ImportTiming(
                    module=m.group(4).strip(),
                    self_us=int(m.group(1)),
                    cumulative_us=int(m.group(2)),
                )
            )
    rows.sort(key=lambda r: (-r.cumulative_us, r.module))
    return rows[:top]


# ---------------------------------------------------------------- memory


def vmhwm_kib(status_text: str) -> int:
    """Peak resident set from /proc/self/status ("VmHWM:   12345 kB")."""
    for line in status_text.splitlines():
        if line.startswith("VmHWM:"):
            fields = line.split()
            if len(fields) >= 2 and fields[1].isdigit():
                return int(fields[1])
            break
    raise ParseError("no VmHWM line in status text")


def maxrss_kib(ru_maxrss: int, sysname: str) -> int:
    """getrusage().ru_maxrss normalized to KiB: Linux reports KiB, macOS bytes."""
    if ru_maxrss < 0:
        raise ParseError(f"negative ru_maxrss: {ru_maxrss}")
    if sysname == "Darwin":
        return ru_maxrss // 1024
    return ru_maxrss


# ---------------------------------------------------------------- strace -c


# The file-op set the summary-index work will be judged on: opens, stats, reads.
FILE_OP_SYSCALLS = frozenset(
    {"open", "openat", "openat2", "stat", "lstat", "fstat", "newfstatat", "statx", "read"}
)
NETWORK_SYSCALLS = frozenset({"socket", "connect"})

_STRACE_ROW_RE = re.compile(r"^\s*[\d.]+\s+[\d.]+\s+\d+\s+(\d+)\s+(?:\d+\s+)?([a-z0-9_]+)\s*$")


def strace_counts(table: str) -> dict[str, int]:
    """`strace -c` summary table → {syscall: calls}. Header/separator/total lines are
    shaped differently and fall out of the row regex."""
    counts: dict[str, int] = {}
    for line in table.splitlines():
        m = _STRACE_ROW_RE.match(line)
        if m and m.group(2) != "total":
            counts[m.group(2)] = counts.get(m.group(2), 0) + int(m.group(1))
    if not counts:
        raise ParseError("no syscall rows found in strace -c output")
    return counts


def count_group(counts: dict[str, int], group: frozenset[str]) -> int:
    return sum(calls for name, calls in counts.items() if name in group)


# ---------------------------------------------------------------- pyperf JSON


@dataclass(frozen=True)
class PyperfBench:
    name: str
    values_s: list[float]


def pyperf_benchmarks(json_text: str) -> list[PyperfBench]:
    """pyperf's JSON: benchmarks[].runs[].values (seconds; calibration runs carry only
    warmups and are skipped). Raises when the shape is wrong or a benchmark ends up
    with no values — an empty benchmark is a broken run, not a zero."""
    try:
        doc = json.loads(json_text)
    except json.JSONDecodeError as exc:
        raise ParseError(f"pyperf output is not JSON: {exc}") from exc
    benches = doc.get("benchmarks") if isinstance(doc, dict) else None
    if not isinstance(benches, list) or not benches:
        raise ParseError("pyperf output has no benchmarks")
    out: list[PyperfBench] = []
    for bench in benches:
        name = _pyperf_name(bench, doc)
        values: list[float] = []
        for run in bench.get("runs", []):
            run_values = run.get("values")
            if isinstance(run_values, list):
                values.extend(float(v) for v in run_values)
        if not values:
            raise ParseError(f"pyperf benchmark {name!r} has no measured values")
        out.append(PyperfBench(name=name, values_s=values))
    return out


def _pyperf_name(bench: dict[str, Any], doc: dict[str, Any]) -> str:
    for source in (bench.get("metadata"), doc.get("metadata")):
        if isinstance(source, dict) and isinstance(source.get("name"), str):
            return source["name"]
    raise ParseError("pyperf benchmark has no name")
