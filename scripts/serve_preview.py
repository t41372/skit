"""Dev preview: serve the TUI main menu in a browser via textual-serve (C10's GUI fallback check).

Development/demo only — not shipped with the package.
"""

from __future__ import annotations

import os

from textual_serve.server import Server


def main() -> None:
    os.environ.setdefault("SKIT_DATA_DIR", "/tmp/skit-demo/data")
    os.environ.setdefault("SKIT_STATE_DIR", "/tmp/skit-demo/state")
    server = Server(
        "uv run skit",
        host="0.0.0.0",
        port=int(os.environ.get("PORT", "8000")),
        title="skit — main menu preview",
    )
    server.serve()


if __name__ == "__main__":
    main()
