"""pyperf micro: the prompt render path (raw substitution, no shell, no quoting —
langs/prompt/render.py). Self-contained; needs no dataset."""

from __future__ import annotations

import pyperf

from skit.langs.prompt.render import render_body

_BODY = (
    "Review the repository at {{path}} and produce a report about {{topic}}.\n"
    "Constraints: keep it under {{limit}} words, cite files by path, and\n"
    "prefer bullet lists. Repeat the summary at the end.\n" * 10
)
_VALUES = {"path": "/workspace/repo", "topic": "error handling", "limit": "500"}
_MANAGED = ["path", "topic", "limit"]


def main() -> None:
    runner = pyperf.Runner()
    runner.bench_func("prompt.render_body", render_body, _BODY, _VALUES, _MANAGED)


if __name__ == "__main__":
    main()
