"""pyperf micro: store read paths at the library size BENCH_N points at.

Self-contained by design: skit + pyperf + stdlib imports only (pyperf re-execs this
file as a plain script in worker processes), dataset location arrives via the
SKIT_*_DIR environment. Refuses to run without it — an unset dataset would benchmark
the machine's default (possibly the developer's real) library and produce
plausible-looking garbage."""

from __future__ import annotations

import os
import sys

if not os.environ.get("SKIT_DATA_DIR"):
    sys.exit("bench_store: SKIT_DATA_DIR not set — refusing to benchmark the default library")
if not os.environ.get("BENCH_N"):
    sys.exit("bench_store: BENCH_N not set — metric names need the library size")

import pyperf

from skit import argstate, store


def main() -> None:
    n = os.environ["BENCH_N"]
    runner = pyperf.Runner()
    entries = store.list_entries()
    if int(n) > 0 and not entries:
        # A non-empty library that reads as empty means the env is pointing at the
        # wrong place — benchmarking it would produce plausible-looking garbage.
        sys.exit(f"bench_store: BENCH_N={n} but the library reads empty — wrong SKIT_*_DIR?")
    runner.bench_func(f"store.list_entries.n{n}", store.list_entries)
    if entries:
        first = entries[0].slug
        mid = entries[len(entries) // 2].slug
        last = entries[-1].slug
        runner.bench_func(f"store.resolve.first.n{n}", store.resolve, first)
        runner.bench_func(f"store.resolve.mid.n{n}", store.resolve, mid)
        runner.bench_func(f"store.resolve.last.n{n}", store.resolve, last)
        runner.bench_func(f"argstate.load_state.n{n}", argstate.load_state, mid)


if __name__ == "__main__":
    main()
