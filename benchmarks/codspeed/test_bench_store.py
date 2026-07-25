"""CodSpeed benchmarks: store read paths over a populated library.

`store.list_entries()` and `store.resolve()` are on skit's hottest path — every CLI
invocation and every TUI paint reads the library. These benchmarks build an isolated
library through the PUBLIC store API (`add_command`, no source files needed) so the
on-disk format cannot drift, then measure the read paths at a realistic size.

The library is redirected to a temp directory via SKIT_DATA_DIR (the same isolation
skit's own test suite uses), so no developer/CI real library is ever touched.
"""

from __future__ import annotations

from pathlib import Path

import pytest

from skit import store

_N = 200  # a realistically sized library; the O(N^2) add cost is paid once in setup


@pytest.fixture
def library(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> list[str]:
    """Populate an isolated library and return the added slugs."""
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path / "config"))
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    slugs = [store.add_command(f"echo {i} {{arg}}", name=f"cmd-{i:04d}").slug for i in range(_N)]
    return slugs


def test_list_entries(benchmark, library: list[str]) -> None:
    entries = benchmark(store.list_entries)
    assert len(entries) == _N


def test_resolve_first(benchmark, library: list[str]) -> None:
    first = library[0]
    entry = benchmark(store.resolve, first)
    assert entry.slug == first


def test_resolve_last(benchmark, library: list[str]) -> None:
    last = library[-1]
    entry = benchmark(store.resolve, last)
    assert entry.slug == last
