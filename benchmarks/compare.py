"""A/B delta report between two results files — the evidence format optimization PRs
cite. Warn-only by design: `compare` renders, it never gates (hosted-runner wall clock
is advisory; see docs/design/benchmarks.md)."""

from __future__ import annotations

from dataclasses import dataclass

from .results import Results, python_major_minor

# |Δ| must clear BOTH: 5% of base and a per-unit absolute noise floor. The floor is
# NOT one physical duration converted between units: macro (hyperfine, ms-scale)
# numbers carry hosted-runner process noise, so sub-2ms wiggle is meaningless there —
# while micro (pyperf, µs-scale) numbers are calibrated in-process loops whose values
# sit in the 1-600 µs range, where a 2000 µs floor would swallow every realistic
# regression (a 3x micro slowdown must land in "Notable", not "Within noise").
RELATIVE_THRESHOLD = 0.05
TIME_FLOOR_BY_UNIT = {"s": 0.002, "ms": 2.0, "us": 1.0}


@dataclass(frozen=True)
class Delta:
    metric: str
    unit: str
    base: float
    head: float

    @property
    def diff(self) -> float:
        return self.head - self.base

    @property
    def pct(self) -> float | None:
        if self.base == 0:
            return None
        return self.diff / self.base * 100

    @property
    def notable(self) -> bool:
        floor = TIME_FLOOR_BY_UNIT.get(self.unit, 0.0)
        if abs(self.diff) <= floor:
            return False
        if self.base == 0:
            return self.head != 0
        return abs(self.diff) > RELATIVE_THRESHOLD * abs(self.base)


@dataclass(frozen=True)
class Comparison:
    deltas: list[Delta]
    only_base: list[str]
    only_head: list[str]
    # Provenance fields on which the two sides differ ("profile: full vs pr", ...) —
    # non-empty means the delta table compares apples to oranges and the report says
    # so up top instead of rendering a clean-looking A/B.
    incomparable: list[str]

    @property
    def notable(self) -> list[Delta]:
        return [d for d in self.deltas if d.notable]


def compare(base: Results, head: Results) -> Comparison:
    """`pipeline.*` self-timings (suite durations, skip counts) are excluded: they
    measure the harness, not skit, and their wobble would intermittently pollute the
    "Notable" section of A/B evidence. Provenance mismatches (profile, platform,
    python) are detected here and rendered as a loud warning — a full-vs-pr or
    cross-platform diff must never look like a legitimate comparison."""
    incomparable = [
        f"{label}: {a} vs {b}"
        for label, a, b in (
            ("profile", base.meta.profile, head.meta.profile),
            ("platform", base.meta.host.platform_key, head.meta.host.platform_key),
            (
                "runner image",
                base.meta.host.ci_image_version,
                head.meta.host.ci_image_version,
            ),
            (
                "python",
                python_major_minor(base.meta.python),
                python_major_minor(head.meta.python),
            ),
            ("pyperf", base.meta.pyperf, head.meta.pyperf),
        )
        if a != b
    ]
    deltas: list[Delta] = []
    for metric_id in sorted(set(base.metrics) & set(head.metrics)):
        if metric_id.startswith("pipeline."):
            continue
        b, h = base.metrics[metric_id], head.metrics[metric_id]
        if b.unit != h.unit:
            incomparable.append(f"unit {metric_id}: {b.unit} vs {h.unit}")
            continue
        deltas.append(Delta(metric=metric_id, unit=h.unit, base=b.value, head=h.value))
    return Comparison(
        deltas=deltas,
        only_base=sorted(
            m for m in set(base.metrics) - set(head.metrics) if not m.startswith("pipeline.")
        ),
        only_head=sorted(
            m for m in set(head.metrics) - set(base.metrics) if not m.startswith("pipeline.")
        ),
        incomparable=incomparable,
    )


def render_markdown(base: Results, head: Results, comparison: Comparison) -> str:
    lines = [
        "## Benchmark comparison",
        "",
        f"Base: `{base.meta.git.commit[:12]}` ({base.meta.skit_version}) · "
        f"Head: `{head.meta.git.commit[:12]}` ({head.meta.skit_version}) · "
        f"profile {head.meta.profile} · {head.meta.host.platform_key}",
        "",
        "Warn-only: notable = |Δ| > max(5%, per-unit floor: 2 ms macro / 1 µs micro). "
        "Hosted-runner numbers are advisory.",
        "",
    ]
    if comparison.incomparable:
        lines += [
            "> [!WARNING]",
            "> **The sides are not directly comparable** — "
            + "; ".join(comparison.incomparable)
            + ". Deltas below mix apples and oranges.",
            "",
        ]
    notable = comparison.notable
    lines.append(f"### Notable ({len(notable)})" if notable else "### Notable (none)")
    if notable:
        lines += _table(notable)
    rest = [d for d in comparison.deltas if not d.notable]
    if rest:
        lines += ["", f"<details><summary>Within noise ({len(rest)})</summary>", ""]
        lines += _table(rest)
        lines += ["", "</details>"]
    for title, ids in (
        ("Only in base", comparison.only_base),
        ("Only in head", comparison.only_head),
    ):
        if ids:
            lines += ["", f"### {title}", ""]
            lines += [f"- `{metric_id}`" for metric_id in ids]
    return "\n".join(lines) + "\n"


def _table(deltas: list[Delta]) -> list[str]:
    rows = ["| Metric | Base | Head | Δ | Δ% |", "| --- | ---: | ---: | ---: | ---: |"]
    for d in deltas:
        pct = "—" if d.pct is None else f"{d.pct:+.1f}%"
        rows.append(
            f"| `{d.metric}` | {d.base:g} {d.unit} | {d.head:g} {d.unit} | {d.diff:+g} | {pct} |"
        )
    return rows
