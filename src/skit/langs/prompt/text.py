"""The one text boundary for prompt payloads.

Prompt bodies are later sent as one exact argv value.  A replacement-character decode
would silently change the user's payload before delivery, so every prompt-specific
reader goes through this strict UTF-8 boundary and either receives the real text or a
path-and-byte-offset error it can map into its own Store/Launch/UI domain.
"""

from __future__ import annotations

from pathlib import Path

from ...i18n import gettext


class PromptEncodingError(UnicodeError):
    """A prompt body is not UTF-8; ``offset`` is the first invalid byte's index."""

    def __init__(self, path: Path, offset: int) -> None:
        self.path: Path = path
        self.offset: int = offset
        super().__init__(
            gettext("Prompt %(path)s isn't valid UTF-8 (invalid byte at offset %(offset)d).")
            % {"path": str(path), "offset": offset}
        )


def decode(data: bytes, path: Path) -> str:
    """Decode bytes strictly while retaining the source path in any refusal."""
    try:
        return data.decode("utf-8")
    except UnicodeDecodeError as exc:
        raise PromptEncodingError(path, exc.start) from exc


def read(path: Path) -> str:
    """Read without universal-newline translation, then apply strict UTF-8."""
    return decode(path.read_bytes(), path)
