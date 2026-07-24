"""The result model — THE schema of a benchmark run (docs/design/benchmarks.md).

There is deliberately no parallel schema.json: these dataclasses are the single source
of truth for what a results file contains, `from_json` is the validator, and
benchmarks/README.md documents the shape for humans. A second schema document would be
a divergence waiting to happen.
"""

from __future__ import annotations

import json
import math
from dataclasses import asdict, dataclass, field
from typing import Any

SCHEMA_VERSION = 1


class ResultsError(ValueError):
    """A results document failed validation. The message names the offending path."""


@dataclass(frozen=True)
class Metric:
    """One measured number. `value` is the headline (median for statistical metrics);
    p95/stddev ride along when the metric is statistical, stay None when it is a
    single deterministic observation (a count, a byte size)."""

    value: float
    unit: str
    n: int
    p95: float | None = None
    stddev: float | None = None


@dataclass(frozen=True)
class Skip:
    """A suite's deliberate, reasoned decision not to run a case — recorded, counted
    (pipeline.skipped_count), and budgeted on the reference platform. A crash is NOT a
    skip; crashes fail the run."""

    suite: str
    case: str
    reason: str


@dataclass(frozen=True)
class GitInfo:
    commit: str
    dirty: bool


@dataclass(frozen=True)
class HostInfo:
    os: str
    kernel: str
    cpu: str
    cpu_count: int
    mem_total_mib: int
    platform_key: str
    ci_runner: str | None
    ci_image_version: str | None = None


@dataclass(frozen=True)
class Meta:
    generated_at: str
    profile: str
    git: GitInfo
    skit_version: str
    host: HostInfo
    python: str
    uv: str
    textual: str
    pyperf: str


@dataclass(frozen=True)
class SuiteOutput:
    """What one suite hands the pipeline: its metrics, its skips, its raw payloads,
    and how long it took. Written as `<suite>.json` into the run's suites/ directory."""

    suite: str
    metrics: dict[str, Metric] = field(default_factory=dict)
    skipped: list[Skip] = field(default_factory=list)
    raw: dict[str, Any] = field(default_factory=dict)
    duration_s: float = 0.0

    def to_json(self) -> str:
        return json.dumps(asdict(self), sort_keys=True, indent=2) + "\n"

    @classmethod
    def skip_all(cls, suite: str, reason: str) -> SuiteOutput:
        """A suite that cannot run at all: one whole-suite skip, both suite fields
        filled from one argument so they can never disagree."""
        return cls(suite=suite, skipped=[Skip(suite=suite, case="all", reason=reason)])

    @classmethod
    def from_json(cls, text: str) -> SuiteOutput:
        doc = _load_object(text, "suite output")
        suite = _string(doc, "suite")
        metrics = _metrics(doc.get("metrics", {}))
        skipped = _skips(doc.get("skipped", []))
        raw = doc.get("raw", {})
        if not isinstance(raw, dict):
            raise ResultsError("raw: expected an object")
        duration = doc.get("duration_s", 0.0)
        if not _is_number(duration):
            raise ResultsError("duration_s: expected a finite number")
        return cls(
            suite=suite,
            metrics=metrics,
            skipped=skipped,
            raw=raw,
            duration_s=float(duration),
        )


@dataclass(frozen=True)
class Results:
    """A whole run: environment manifest, flat dotted-ID metrics, recorded skips, and
    the raw per-suite payloads (full samples — the artifact of record)."""

    meta: Meta
    metrics: dict[str, Metric]
    skipped: list[Skip]
    raw: dict[str, Any]
    schema_version: int = SCHEMA_VERSION

    def to_json(self) -> str:
        return json.dumps(asdict(self), sort_keys=True, indent=2) + "\n"

    @classmethod
    def from_json(cls, text: str) -> Results:
        doc = _load_object(text, "results")
        version = doc.get("schema_version")
        if version != SCHEMA_VERSION:
            raise ResultsError(f"schema_version: expected {SCHEMA_VERSION}, got {version!r}")
        meta = _meta(doc.get("meta"))
        metrics = _metrics(doc.get("metrics", {}))
        skipped = _skips(doc.get("skipped", []))
        raw = doc.get("raw", {})
        if not isinstance(raw, dict):
            raise ResultsError("raw: expected an object")
        return cls(meta=meta, metrics=metrics, skipped=skipped, raw=raw)


def python_major_minor(version: str) -> str:
    """ "3.13.7" → "3.13" — the granularity budget provenance compares at."""
    return ".".join(version.split(".")[:2])


def meta_from_dict(node: Any) -> Meta:
    """Validate and build a Meta from parsed JSON (run.json's `meta` block)."""
    return _meta(node)


# ---------------------------------------------------------------- validation helpers


def _load_object(text: str, what: str) -> dict[str, Any]:
    try:
        doc = json.loads(text)
    except json.JSONDecodeError as exc:
        raise ResultsError(f"{what}: not valid JSON ({exc})") from exc
    if not isinstance(doc, dict):
        raise ResultsError(f"{what}: expected a JSON object")
    return doc


def _is_number(value: Any) -> bool:
    """Finite numbers only: json.loads happily parses bare NaN/Infinity, and a NaN
    metric would silently pass every `value > max` budget comparison."""
    return isinstance(value, int | float) and not isinstance(value, bool) and math.isfinite(value)


def _string(doc: dict[Any, Any], key: str, *, path: str = "") -> str:
    value = doc.get(key)
    where = f"{path}{key}"
    if not isinstance(value, str) or not value:
        raise ResultsError(f"{where}: expected a non-empty string, got {value!r}")
    return value


def _metrics(node: Any) -> dict[str, Metric]:
    if not isinstance(node, dict):
        raise ResultsError("metrics: expected an object")
    out: dict[str, Metric] = {}
    for metric_id, body in node.items():
        if not isinstance(body, dict):
            raise ResultsError(f"metrics.{metric_id}: expected an object")
        value = body.get("value")
        if not _is_number(value):
            raise ResultsError(
                f"metrics.{metric_id}.value: expected a finite number, got {value!r}"
            )
        unit = body.get("unit")
        if not isinstance(unit, str) or not unit:
            raise ResultsError(f"metrics.{metric_id}.unit: expected a non-empty string")
        n = body.get("n")
        if not isinstance(n, int) or isinstance(n, bool) or n < 1:
            raise ResultsError(f"metrics.{metric_id}.n: expected a positive integer, got {n!r}")
        p95 = body.get("p95")
        if p95 is not None and not _is_number(p95):
            raise ResultsError(f"metrics.{metric_id}.p95: expected a finite number or null")
        stddev = body.get("stddev")
        if stddev is not None and not _is_number(stddev):
            raise ResultsError(f"metrics.{metric_id}.stddev: expected a finite number or null")
        out[metric_id] = Metric(
            value=float(value),
            unit=unit,
            n=n,
            p95=None if p95 is None else float(p95),
            stddev=None if stddev is None else float(stddev),
        )
    return out


def _skips(node: Any) -> list[Skip]:
    if not isinstance(node, list):
        raise ResultsError("skipped: expected an array")
    out: list[Skip] = []
    for i, body in enumerate(node):
        if not isinstance(body, dict):
            raise ResultsError(f"skipped[{i}]: expected an object")
        out.append(
            Skip(
                suite=_string(body, "suite", path=f"skipped[{i}]."),
                case=_string(body, "case", path=f"skipped[{i}]."),
                reason=_string(body, "reason", path=f"skipped[{i}]."),
            )
        )
    return out


def _meta(node: Any) -> Meta:
    if not isinstance(node, dict):
        raise ResultsError("meta: expected an object")
    git_node = node.get("git")
    if not isinstance(git_node, dict):
        raise ResultsError("meta.git: expected an object")
    dirty = git_node.get("dirty")
    if not isinstance(dirty, bool):
        raise ResultsError("meta.git.dirty: expected a boolean")
    git = GitInfo(commit=_string(git_node, "commit", path="meta.git."), dirty=dirty)
    host_node = node.get("host")
    if not isinstance(host_node, dict):
        raise ResultsError("meta.host: expected an object")
    cpu_count = host_node.get("cpu_count")
    if not isinstance(cpu_count, int) or isinstance(cpu_count, bool) or cpu_count < 1:
        raise ResultsError("meta.host.cpu_count: expected a positive integer")
    mem = host_node.get("mem_total_mib")
    if not isinstance(mem, int) or isinstance(mem, bool) or mem < 0:
        raise ResultsError("meta.host.mem_total_mib: expected a non-negative integer")
    ci_runner = host_node.get("ci_runner")
    if ci_runner is not None and not isinstance(ci_runner, str):
        raise ResultsError("meta.host.ci_runner: expected a string or null")
    ci_image_version = host_node.get("ci_image_version")
    if ci_image_version is not None and not isinstance(ci_image_version, str):
        raise ResultsError("meta.host.ci_image_version: expected a string or null")
    pyperf = node.get("pyperf", "unknown")
    if not isinstance(pyperf, str) or not pyperf:
        raise ResultsError("meta.pyperf: expected a non-empty string")
    host = HostInfo(
        os=_string(host_node, "os", path="meta.host."),
        kernel=_string(host_node, "kernel", path="meta.host."),
        cpu=_string(host_node, "cpu", path="meta.host."),
        cpu_count=cpu_count,
        mem_total_mib=mem,
        platform_key=_string(host_node, "platform_key", path="meta.host."),
        ci_runner=ci_runner,
        ci_image_version=ci_image_version,
    )
    return Meta(
        generated_at=_string(node, "generated_at", path="meta."),
        profile=_string(node, "profile", path="meta."),
        git=git,
        skit_version=_string(node, "skit_version", path="meta."),
        host=host,
        python=_string(node, "python", path="meta."),
        uv=_string(node, "uv", path="meta."),
        textual=_string(node, "textual", path="meta."),
        # Schema v1 artifacts produced before harness provenance was added remain
        # readable; comparisons surface "unknown" against a recorded version.
        pyperf=pyperf,
    )
