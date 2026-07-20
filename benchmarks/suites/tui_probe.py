"""The TUI probe: one fresh process per sample, measuring three spans against the
library the environment points it at (docs/design/benchmarks.md, "tui").

Standalone by design — spawned as a plain file, imports only skit/textual/stdlib,
never the benchmarks package. The suite parses this probe's JSON (including the raw
/proc/self/status text; parsing lives in the covered parse layer, not here).

Spans:
1. import — `import skit.tui` (the cold import share).
2. first idle — MenuApp() → run_test(120x40) → first pilot.pause() returns (mount +
   initial _reload + message queue drained).
3. search — the table owns focus after mount and plain letters are action keys, so:
   press "/" (focus search), settle, THEN measure press("x") → pause().

The probe asserts its measurements are real: the row count must equal the expected
entry count (a wrong-environment probe must die, not benchmark an empty library), the
search input must hold "x", and at N > 0 the filter must strictly drop rows (the
dataset generator guarantees an x-free entry). A silent no-op dies; it is never
recorded as a fast search.
"""

from __future__ import annotations

import argparse
import asyncio
import json
import os
import sys
import time
from pathlib import Path


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--entries", type=int, required=True)
    ap.add_argument("--out", required=True)
    args = ap.parse_args()

    if not os.environ.get("SKIT_DATA_DIR"):
        sys.exit("tui_probe: SKIT_DATA_DIR not set — refusing to probe the default library")

    t_import = time.perf_counter()
    from skit.tui import MenuApp

    import_ms = (time.perf_counter() - t_import) * 1000

    from textual.widgets import DataTable, Input

    async def drive() -> dict[str, float]:
        t_app = time.perf_counter()
        app = MenuApp()
        async with app.run_test(size=(120, 40)) as pilot:
            await pilot.pause()
            first_idle_ms = (time.perf_counter() - t_app) * 1000
            table = app.query_one(DataTable)
            if table.row_count != args.entries:
                sys.exit(
                    f"tui_probe: expected {args.entries} rows, saw {table.row_count} — "
                    "wrong library behind SKIT_DATA_DIR?"
                )
            await pilot.press("slash")
            await pilot.pause()
            t_search = time.perf_counter()
            await pilot.press("x")
            await pilot.pause()
            search_ms = (time.perf_counter() - t_search) * 1000
            search = app.query_one("#search", Input)
            if search.value != "x":
                sys.exit("tui_probe: search input did not receive the keystroke")
            if args.entries > 0 and table.row_count >= args.entries:
                sys.exit(
                    "tui_probe: filter dropped no rows — dead filter or the dataset "
                    "lost its x-free entry invariant"
                )
        return {"first_idle_ms": first_idle_ms, "search_ms": search_ms}

    spans = asyncio.run(drive())

    status = Path("/proc/self/status")
    status_text = status.read_text(encoding="utf-8") if status.exists() else None
    Path(args.out).write_text(
        json.dumps(
            {
                "import_ms": import_ms,
                "first_idle_ms": spans["first_idle_ms"],
                "search_ms": spans["search_ms"],
                "status_text": status_text,
            }
        ),
        encoding="utf-8",
    )


if __name__ == "__main__":
    main()
