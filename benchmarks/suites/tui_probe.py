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
   press "/" (focus search), settle, THEN measure press(<probe char>) → pause().

The probe asserts its measurements are real: the row count must equal the expected
entry count (a wrong-environment probe must die, not benchmark an empty library);
after filtering, the search input must hold the probe char, the visible rows must
strictly drop (the dataset generator guarantees a probe-char-free entry), and — for
libraries of 3+ entries — some rows must SURVIVE too (the generator guarantees a
matching long description), so the span measures a real filtered repaint, not the
degenerate filter-to-zero path. A silent no-op dies; it is never recorded as a fast
search.
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
    ap.add_argument("--probe-char", required=True)
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
            await pilot.press(args.probe_char)
            await pilot.pause()
            search_ms = (time.perf_counter() - t_search) * 1000
            search = app.query_one("#search", Input)
            if search.value != args.probe_char:
                sys.exit("tui_probe: search input did not receive the keystroke")
            if args.entries > 0 and table.row_count >= args.entries:
                sys.exit(
                    "tui_probe: filter dropped no rows — dead filter or the dataset "
                    "lost its probe-char-free entry invariant"
                )
            if args.entries >= 3 and table.row_count == 0:
                sys.exit(
                    "tui_probe: filter emptied the table — the dataset lost its "
                    "matching-entry invariant, so the span degenerated to filter-to-zero"
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
