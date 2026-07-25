"""CodSpeed benchmarks for skit's headless hot paths.

skit's cost centre when a user runs `skit add` is *analysis*: every supported
language parses the incoming script (tree-sitter for shell/JS/TS, the stdlib
`ast` for Python) and walks the tree to detect candidate parameters, CLI
frameworks, and inline metadata. These functions are pure, deterministic, and
CPU-bound — an ideal fit for CodSpeed's simulation instrument.

The benchmarks feed the analyzers the project's own golden corpus (the same
realistic scripts the correctness tests use), so the measured work matches what
skit does on real user input. Each corpus is concatenated into a single blob per
language to give the parser a representative, non-trivial amount of source to
chew through in one call.
"""

from __future__ import annotations

from pathlib import Path

import pytest
from pytest_codspeed import BenchmarkFixture

from skit import pep723
from skit.langs.fish import analyzer as fish_analyzer
from skit.langs.javascript import analyzer as js_analyzer
from skit.langs.python import analyzer as py_analyzer
from skit.langs.python import argspec as py_argspec
from skit.langs.shell import analyzer as sh_analyzer

CORPUS = Path(__file__).parent.parent / "tests" / "corpus"


def _read(path: Path) -> str:
    # newline="" preserves CRLF exactly, matching how skit reads scripts off disk.
    with path.open(encoding="utf-8", newline="") as f:
        return f.read()


def _concat(paths: list[Path]) -> str:
    # Join corpus files with a blank line so each stays an independent top-level
    # unit — realistic bulk input without merging two scripts into one statement.
    return "\n\n".join(_read(p) for p in paths)


PY_FILES = sorted(CORPUS.glob("*.py"))
SH_FILES = sorted((CORPUS / "shell").glob("*.sh"))
JS_FILES = sorted((CORPUS / "js").glob("*.mjs"))
TS_FILES = sorted((CORPUS / "ts").glob("*.ts"))
FISH_FILES = sorted((CORPUS / "fish").glob("*.fish"))

PY_SOURCE = _concat(PY_FILES)
SH_SOURCE = _concat(SH_FILES)
JS_SOURCE = _concat(JS_FILES)
TS_SOURCE = _concat(TS_FILES)
FISH_SOURCE = _concat(FISH_FILES) if FISH_FILES else ""


def test_python_analyze(benchmark: BenchmarkFixture) -> None:
    benchmark(py_analyzer.analyze, PY_SOURCE)


def test_python_read_cli(benchmark: BenchmarkFixture) -> None:
    # argparse/click/typer detection over the argparse corpus sample.
    argparse_src = _read(CORPUS / "22_argparse_framework.py")
    benchmark(py_argspec.read_cli, argparse_src)


def test_shell_analyze(benchmark: BenchmarkFixture) -> None:
    benchmark(sh_analyzer.analyze, SH_SOURCE)


def test_javascript_analyze(benchmark: BenchmarkFixture) -> None:
    benchmark(js_analyzer.analyze, JS_SOURCE, lang="js")


def test_typescript_analyze(benchmark: BenchmarkFixture) -> None:
    benchmark(js_analyzer.analyze, TS_SOURCE, lang="ts")


@pytest.mark.skipif(not FISH_SOURCE, reason="no fish corpus present")
def test_fish_analyze(benchmark: BenchmarkFixture) -> None:
    benchmark(fish_analyzer.analyze, FISH_SOURCE)


def test_pep723_parse_block(benchmark: BenchmarkFixture) -> None:
    src = _read(CORPUS / "30_pep723_tool_skit.py")
    benchmark(pep723.parse_block, src)


def test_pep723_suggest_dependencies(benchmark: BenchmarkFixture) -> None:
    # Import-walk + stdlib diff over the whole Python corpus (many import styles).
    benchmark(pep723.suggest_dependencies, PY_SOURCE)
