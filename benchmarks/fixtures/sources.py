"""Seeded per-language source generators for the analyzer benchmarks.

Committed generators, not committed blobs: the micro suite materializes these into a
scratch directory at run time, so the corpus scales (20/200/2000 lines) without a
megabyte of checked-in filler. Deterministic: same (language, lines, seed) → same
bytes. The exact line count is honored — analyzer cost curves are plotted against it.
"""

from __future__ import annotations

import random

LANGS = ("python", "shell", "js", "ts")
EXTENSIONS = {"python": "py", "shell": "sh", "js": "js", "ts": "ts"}

_WORDS = ("alpha", "bravo", "delta", "gamma", "kilo", "lima", "omega", "sigma")


def generate(lang: str, lines: int, seed: int = 20260720) -> str:
    """A syntactically valid source of exactly `lines` lines (plus final newline),
    carrying the constructs each language's analyzer actually looks at — parameters,
    env-defaults, argument parsing — so warm-parse timings exercise real paths."""
    if lang not in LANGS:
        raise ValueError(f"unknown language {lang!r} (expected one of {LANGS})")
    if lines < 8:
        raise ValueError("need at least 8 lines for the fixed scaffold")
    rng = random.Random(f"{seed}:{lang}:{lines}")  # noqa: S311 — deterministic fixtures, not crypto
    body = {"python": _python, "shell": _shell, "js": _js, "ts": _ts}[lang](lines, rng)
    if len(body) != lines:
        raise AssertionError(f"generator produced {len(body)} lines, wanted {lines}")
    return "\n".join(body) + "\n"


def _pad(body: list[str], lines: int, comment: str) -> list[str]:
    while len(body) < lines:
        body.append(f"{comment} filler line {len(body) + 1}")
    return body


def _python(lines: int, rng: random.Random) -> list[str]:
    body = [
        "import argparse",
        "",
        "parser = argparse.ArgumentParser(description='generated bench source')",
        f"parser.add_argument('--{rng.choice(_WORDS)}', type=int, default={rng.randrange(9)})",
        "parser.add_argument('--verbose', action='store_true')",
        "args = parser.parse_args()",
    ]
    while len(body) < lines - 2:
        word = rng.choice(_WORDS)
        body += [f"def fn_{len(body)}(x: int) -> int:", f"    return x + {len(word)}"]
    return _pad(body, lines, "#")


def _shell(lines: int, rng: random.Random) -> list[str]:
    body = ["#!/usr/bin/env bash", "set -euo pipefail"]
    body.extend(
        f'{word.upper()}="${{{word.upper()}:-{rng.randrange(99)}}}"'
        for word in _WORDS[: rng.randrange(3, 6)]
    )
    while len(body) < lines - 1:
        body.append(f'echo "step {len(body)}: ${_WORDS[len(body) % len(_WORDS)].upper()}"')
    return _pad(body, lines, "#")


def _js(lines: int, rng: random.Random) -> list[str]:
    body = [
        "const args = process.argv.slice(2);",
        f"let {rng.choice(_WORDS)} = {rng.randrange(9)};",
    ]
    # Emit only WHOLE function chunks, then pad — truncating a chunk would cut its
    # closing brace and hand the analyzer an error-recovery parse tree, silently
    # under-reporting analyze times ~5x (this actually happened; the validity test
    # in tests/test_benchmarks_tooling.py now pins it).
    while len(body) + 3 <= lines:
        body += [f"function fn{len(body)}(x) {{", f"  return x + {len(body)};", "}"]
    return _pad(body, lines, "//")


def _ts(lines: int, rng: random.Random) -> list[str]:
    body = [
        "const args: string[] = process.argv.slice(2);",
        f"let {rng.choice(_WORDS)}: number = {rng.randrange(9)};",
    ]
    # Whole chunks only — see _js.
    while len(body) + 3 <= lines:
        body += [
            f"function fn{len(body)}(x: number): number {{",
            f"  return x + {len(body)};",
            "}",
        ]
    return _pad(body, lines, "//")
