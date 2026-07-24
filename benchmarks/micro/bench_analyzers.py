"""pyperf micro: warm per-language analyze() across generated source sizes.

Self-contained (skit + pyperf + stdlib; pyperf re-execs this file). Sources are
materialized by the orchestrator; BENCH_SOURCES_DIR points here. A language whose
grammar failed to import is simply absent from this script's output — the
orchestrator records the skip, so the absence is visible, never silent."""

from __future__ import annotations

import os
import sys
from pathlib import Path

if not os.environ.get("BENCH_SOURCES_DIR"):
    sys.exit("bench_analyzers: BENCH_SOURCES_DIR not set")

import pyperf

from skit.langs.registry import spec_for

_LINES = (20, 200, 2000)
_EXT = {"python": "py", "shell": "sh", "js": "js", "ts": "ts"}


def main() -> None:
    sources_dir = Path(os.environ["BENCH_SOURCES_DIR"])
    runner = pyperf.Runner()
    for lang, ext in _EXT.items():
        spec = spec_for(lang)
        if spec is None or spec.analyzer is None:
            continue
        for lines in _LINES:
            text = (sources_dir / f"{lang}_{lines}.{ext}").read_text(encoding="utf-8")
            runner.bench_func(f"analyze.{lang}.l{lines}", spec.analyzer.analyze, text)


if __name__ == "__main__":
    main()
