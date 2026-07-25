"""CodSpeed benchmarks: per-language warm analyze() over generated sources.

These exercise skit's parameter-detection hot path — the tree-sitter / AST parse
that runs on every add and reconcile. Sources are produced by the existing seeded
generator (`benchmarks/fixtures/sources.py`), so the inputs carry the real constructs
each analyzer looks at (parameters, env-defaults, argument parsing) and scale
deterministically with line count.

pytest-codspeed measures the callable passed to `benchmark(...)`; generation of the
source text happens once, outside the measured region.
"""

from __future__ import annotations

import pytest

from benchmarks.fixtures import sources
from skit.langs.registry import spec_for

# Analyzer-capable kinds and the size tiers plotted by the pyperf micro suite.
_LANGS = ("python", "shell", "js", "ts")
_LINES = (20, 200, 2000)


@pytest.mark.parametrize("lang", _LANGS)
@pytest.mark.parametrize("lines", _LINES)
def test_analyze(benchmark, lang: str, lines: int) -> None:
    spec = spec_for(lang)
    if spec is None or spec.analyzer is None:
        pytest.skip(f"{lang}: analyzer unavailable (grammar failed to import)")
    text = sources.generate(lang, lines)
    analyze = spec.analyzer.analyze
    result = benchmark(analyze, text)
    assert result is not None
