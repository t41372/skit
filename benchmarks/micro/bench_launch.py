"""pyperf micro: launch command assembly (describe_command — pure assembly, no uv
lookup, no side effects) for one entry per representative kind.

Self-contained (skit + pyperf + stdlib; pyperf re-execs this file). The dataset
arrives via SKIT_*_DIR; same refusal rule as bench_store."""

from __future__ import annotations

import os
import sys

if not os.environ.get("SKIT_DATA_DIR"):
    sys.exit("bench_launch: SKIT_DATA_DIR not set — refusing to benchmark the default library")

import pyperf

from skit import launcher, store


def main() -> None:
    entries = store.list_entries()
    runner = pyperf.Runner()
    benched = 0
    for kind in ("python", "shell", "command"):
        entry = next((e for e in entries if e.meta.kind == kind), None)
        if entry is None:
            continue
        values = dict.fromkeys(entry.meta.params or [], "value")
        runner.bench_func(f"launch.describe.{kind}", launcher.describe_command, entry, [], values)
        benched += 1
    if not benched:
        sys.exit("bench_launch: dataset has no python/shell/command entries")


if __name__ == "__main__":
    main()
