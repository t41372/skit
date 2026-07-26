"""CodSpeed benchmarks: store read paths over a populated library.

`store.resolve()`, `store.list_entries()` and `store.list_summaries()` are on skit's
hottest path — every CLI invocation and every TUI paint reads the library. The library
is redirected to a temp directory via SKIT_*_DIR (the same isolation skit's own test
suite uses), so no developer/CI real library is ever touched.

**Two library shapes, on purpose.** These read paths cost what they cost because of what
the index holds, and what the index holds depends on the KIND MIX:

- `*_commands`: 200 command templates. Cheap to build (no source files) but degenerate —
  a command is always reference mode, so every row carries the mode marker and the
  target key. This is the worst case for index size, by construction.
- `*_mixed`: the seeded generator's own library (`benchmarks.datasets`), whose kind mix
  is a documented contract and is ~84% copy mode. This is what a real library looks
  like, and the only shape whose numbers should be read as "what users pay".

Measuring only the first would have reported a worst case as the typical one; measuring
only the second would have hidden the worst case. Both are reported, and a change that
moves them apart is telling you something about the row shape, not about the read path.
"""

from __future__ import annotations

from pathlib import Path

import pytest

from benchmarks import datasets
from skit import store

_N = 200  # a realistically sized library; the O(N^2) add cost is paid once in setup


@pytest.fixture
def command_library(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> list[str]:
    """200 command templates, built through the PUBLIC store API so the on-disk format
    cannot drift. 100% reference mode — the index worst case."""
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path / "config"))
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    return [store.add_command(f"echo {i} {{arg}}", name=f"cmd-{i:04d}").slug for i in range(_N)]


@pytest.fixture
def mixed_library(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> list[str]:
    """The seeded generator's library: the documented kind mix, ~84% copy mode, half the
    entries carrying last-run state. Deterministic, and the same fixture the pyperf
    pipeline measures against — so the two suites cannot disagree about what a realistic
    library is."""
    root = tmp_path / "dataset"
    datasets.generate(root, _N)
    for key, value in datasets.skit_dirs(root).items():
        monkeypatch.setenv(key, value)
    return sorted(entry.slug for entry in store.list_entries())


# --------------------------------------------------------------------------
# command-only (the index worst case; these series predate the mixed ones)
# --------------------------------------------------------------------------


def test_list_entries(benchmark, command_library: list[str]) -> None:
    entries = benchmark(store.list_entries)
    assert len(entries) == _N


def test_list_summaries(benchmark, command_library: list[str]) -> None:
    """What `skit list` and the CLI listing surfaces actually call: served from
    registry.toml, where list_entries opens one meta.toml per entry. The two are
    benchmarked side by side on purpose — the gap between them IS the optimization,
    and a change that closes it is a regression in the index."""
    summaries = benchmark(store.list_summaries)
    assert len(summaries) == _N


def test_resolve(benchmark, command_library: list[str]) -> None:
    """ONE resolve series per fixture, not first/last. Resolve-by-slug is a dict hit
    after the registry parse, so row position cannot matter; the by-name linear scan
    it could fall back to measures 0.6% of the call at N=200 (parse dominates). The
    first/last pair this replaces was two names for the same number — noise in the
    regression surface the pipeline rules make mandatory evidence."""
    target = command_library[-1]
    entry = benchmark(store.resolve, target)
    assert entry.slug == target


# --------------------------------------------------------------------------
# mixed kinds (what a real library costs)
# --------------------------------------------------------------------------


def test_list_entries_mixed(benchmark, mixed_library: list[str]) -> None:
    entries = benchmark(store.list_entries)
    assert len(entries) == _N


def test_list_summaries_mixed(benchmark, mixed_library: list[str]) -> None:
    summaries = benchmark(store.list_summaries)
    assert len(summaries) == _N


def test_resolve_mixed(benchmark, mixed_library: list[str]) -> None:
    target = mixed_library[-1]
    entry = benchmark(store.resolve, target)
    assert entry.slug == target
