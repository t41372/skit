"""Language-neutral source rewriting: the one true injection primitive.

An **absolute-byte-offset splice** that every language adapter feeds. Python's `ast` adapter
produces 1-based (lineno, col) locations (col is already a UTF-8 byte offset within the line);
`line_start_table` + `linecol_to_byte` turn those into the absolute byte offsets a `ByteSpan`
carries. tree-sitter natively yields `start_byte`/`end_byte`, so it builds `ByteSpan`s directly.
Both then go through the same `apply_byte_spans` core, which encodes once, splices bottom-up, and
decodes once — so there is a single place multibyte/CRLF alignment can be right (or wrong), pinned
by the golden corpus for Python and, later, the shell corpus.

`write_injected` also lives here: writing an injected result (which may carry plaintext secret
values) to a private OS-temp file is the same discipline for every language; only the temp-file
suffix differs (python `.py`, shell `.sh`, …).
"""

from __future__ import annotations

import os
import re
import tempfile
from dataclasses import dataclass
from pathlib import Path

_UTF8 = "utf-8"  # pragma: no mutate — "utf-8"/"UTF-8" codec alias

# The exact set of newline sequences CPython's tokenizer/AST count as a line break: \r\n, \r, \n
# (in that preference order, so a CRLF pair is one line break, not two). str.splitlines() breaks on
# a much larger set (\v \f \x1c \x1d \x1e \x85 U+2028 U+2029 too), which desyncs any code that
# indexes AST linenos into its output — see _physical_lines.
_NEWLINE_RE = re.compile(r"\r\n|\r|\n")


def _physical_lines(text: str) -> list[str]:
    """Split text into the same physical lines (keeping line endings) that AST linenos count.

    A drop-in replacement for `text.splitlines(keepends=True)` for this exact purpose: that method
    also splits on \\v, \\f, \\x1c-\\x1e, NEL (\\x85), and U+2028/U+2029 — none of which end a
    physical line as far as the tokenizer/AST are concerned. When one of those characters appears
    anywhere in the source (even inside a string literal, e.g. `MSG = "hi\\u2028there"`), splitlines
    silently produces *more* entries than the AST's line count, so `lines[lineno - 1]` for every
    node at or after that point no longer names the node's real line — the byte-slice write lands on
    the wrong physical line. Depending on where that lands, the result is either a SyntaxError in the
    injected temp copy, or — worse — a silently-corrupted preamble insertion that never takes effect
    (the queued input() value is dropped with no error at all).
    """
    if not text:
        return []
    lines: list[str] = []
    pos = 0
    for m in _NEWLINE_RE.finditer(text):
        lines.append(text[pos : m.end()])
        pos = m.end()
    if pos < len(text):
        lines.append(text[pos:])
    return lines


@dataclass(frozen=True)
class ByteSpan:
    """A splice request: replace the absolute UTF-8 byte range [start, end) with new_text.

    A zero-width span (start == end) is a pure insertion at that offset (nothing removed) — how a
    preamble line is inserted at the start of a target line.
    """

    start: int  # absolute UTF-8 byte offset, [start, end)
    end: int
    new_text: str


def apply_byte_spans(text: str, spans: list[ByteSpan]) -> str:
    """Apply splices bottom-up (highest start first) so an earlier splice never shifts a later
    one's offsets. Spans are guaranteed non-overlapping by callers: for Python, a const target's RHS
    must be a literal (same decision as analyzer) so it cannot contain an input() call, and an input
    replacement only ever covers the `input` callee name itself (a fixed 5-byte identifier), never
    an argument — so const spans, other calls' callee spans, and a given call's own argument spans
    never collide.

    Offsets are **UTF-8 byte** offsets, not character offsets (ast's col_offset / end_col_offset
    are byte offsets too; tree-sitter's start_byte/end_byte are native byte offsets). When a line
    contains multibyte characters (e.g. CJK), slicing the str directly misaligns; we encode the
    whole text once, splice at the byte level, and decode once (a real bug caught by corpus
    17_unicode_cjk).
    """
    data = text.encode(_UTF8)
    for span in sorted(spans, key=lambda s: s.start, reverse=True):
        data = data[: span.start] + span.new_text.encode(_UTF8) + data[span.end :]
    return data.decode(_UTF8)


def line_start_table(text: str) -> list[int]:
    """Absolute UTF-8 byte offset of each physical line's start.

    Uses the EXACT newline set the CPython tokenizer/AST count (\\r\\n|\\r|\\n, via _physical_lines);
    str.splitlines' larger set is the documented trap (see _physical_lines). Returns one entry per
    physical line plus a trailing sentinel equal to the total byte length (the start of the line
    that would follow the last one), so an insertion index one past the final line is still valid.
    """
    table = [0]
    offset = 0
    for line in _physical_lines(text):
        offset += len(line.encode(_UTF8))
        table.append(offset)
    return table


def linecol_to_byte(table: list[int], lineno: int, col: int) -> int:
    """Absolute byte offset of a 1-based line / byte-column position.

    ast linenos are 1-based (so index `table` at `lineno - 1`); ast col_offsets are ALREADY UTF-8
    byte offsets within the line, so they add directly to the line's start offset.
    """
    return table[lineno - 1] + col


def write_injected(
    entry_dir: Path, content: str, *, suffix: str, prefer_entry_dir: bool = False
) -> Path:
    """Write the injected result to a unique temp file and return its path.

    The file is written to the OS temp directory, not entry_dir — the persistent script store
    (3b): entry_dir sits right next to script.py and holds only script.py + meta.toml (see
    store.add_python's own contract for that invariant), and nothing depends on the injected copy
    living there specifically — the run's cwd is resolved independently by
    launcher._resolve_workdir, and `uv run --script <path>` doesn't require the script to sit next
    to anything else. Writing a plaintext-secret-bearing file (const substitutions / queue literals)
    into entry_dir instead would mean a SIGKILL/OOM/power-loss before the caller's
    `finally: unlink()` runs leaves it there forever, since nothing skit owns ever sweeps entry_dir;
    the OS temp directory, by contrast, is periodically reaped by the platform itself.

    entry_dir is kept as a fallback (defense in depth) for the rare case the OS temp directory isn't
    writable, so a run never fails outright just because TMPDIR is misconfigured.

    `prefer_entry_dir=True` INVERTS that choice: the copy is written into entry_dir (OS tmp as the
    fallback). A copy-mode JS/TS entry with managed npm deps must run from entry_dir — module
    resolution walks up from the copy's own path, and only adjacency finds entry_dir/node_modules.
    The crash-leftover concern above is answered for that path by RunnerLaunch's sweep of aged
    `.injected-*` files, which runs on every deps-managed launch.

    - Unique filename (.injected-XXXX<suffix>): concurrent runs of the same script don't clobber.
      suffix is language-specific (python ".py", shell ".sh") so the interpreter still recognizes
      the temp copy by extension.
    - 0o600 permissions: the content may contain secret values (const substitutions / queue
      literals), so don't let other local users read it (an extension of C3; the caller must still
      delete the file in a finally).
    """
    dirs = (entry_dir, None) if prefer_entry_dir else (None, entry_dir)
    try:
        fd, tmp = tempfile.mkstemp(dir=dirs[0], prefix=".injected-", suffix=suffix)
    except OSError:
        fd, tmp = tempfile.mkstemp(dir=dirs[1], prefix=".injected-", suffix=suffix)
    try:
        os.chmod(
            tmp, 0o600
        )  # mkstemp is already 0600; state the intent explicitly (no-op on Windows)
    except BaseException:
        os.close(fd)  # chmod raised before fdopen took ownership of fd; close it ourselves
        os.unlink(tmp)
        raise
    try:
        with os.fdopen(fd, "w", encoding=_UTF8) as f:
            f.write(content)
    except BaseException:
        # fdopen already owns fd here (and the `with` closes it, whether the write succeeded or
        # raised inside the block, or fdopen itself raised before returning) — closing it again
        # would raise "Bad file descriptor" on an already-closed fd.
        os.unlink(tmp)
        raise
    return Path(tmp)
