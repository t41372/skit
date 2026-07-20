"""Mutation-kill tests for ``shebang_program`` in ``src/skit/langs/registry.py``.

These pin the low-level byte handling of ``registry.shebang_program``: it reads only
the first 512 bytes of the file, strips exactly the ``#!`` prefix, and decodes the
rest with ``errors="replace"`` so a malformed shebang stays total (never raises).
Each test exercises the real function against a real file on disk.
"""

from __future__ import annotations

from pathlib import Path

from skit.langs import registry as reg


def _write(tmp_path: Path, name: str, data: bytes) -> Path:
    p = tmp_path / name
    p.write_bytes(data)
    return p


def test_shebang_invalid_utf8_replaces_and_stays_total(tmp_path: Path):
    """A shebang carrying non-UTF-8 bytes decodes with U+FFFD and never raises.

    The 0xF6 (Latin-1 'ö') byte is invalid UTF-8. With ``errors="replace"`` it
    becomes the replacement char and a program name is still returned; with strict
    decoding (``errors=`` dropped/None) or an unknown handler name ("XXreplaceXX",
    "REPLACE") the decode raises instead — so this pins the exact ``errors="replace"``
    contract.
    """
    p = _write(tmp_path, "s", b"#!/usr/bin/pyth\xf6n arg\n")
    assert reg.shebang_program(p) == "pyth�n"  # � = U+FFFD replacement char


def test_shebang_strips_exactly_the_bang_prefix(tmp_path: Path):
    """Only the two ``#!`` bytes are stripped — not a third character.

    A no-space shebang (``#!python3``) makes the off-by-one visible: dropping one
    extra byte (``first[3:]``) would yield ``ython3``.
    """
    p = _write(tmp_path, "s", b"#!python3\n")
    assert reg.shebang_program(p) == "python3"


def test_shebang_reads_only_first_512_bytes(tmp_path: Path):
    """The first line is read with a 512-byte cap.

    A single 600-'x' token past ``#!`` is truncated to 510 chars by the 512-byte
    ``readline`` limit. Reading the whole line (``readline(None)`` -> 600) or reading
    513 bytes (-> 511) would give a different basename, so pinning exactly 510
    distinguishes the correct cap from both.
    """
    p = _write(tmp_path, "s", b"#!" + b"x" * 600 + b"\n")
    assert reg.shebang_program(p) == "x" * 510


# ---- shebang_program_from_line: the path-less half, called directly ---------------


def test_shebang_from_line_strips_exactly_the_bang_prefix():
    """``line[2:]`` drops exactly ``#!`` — one byte more (``line[3:]``) would read a
    no-space ``#!python3`` shebang as ``ython3``. Direct call, no file needed."""
    assert reg.shebang_program_from_line("#!python3") == "python3"


def test_shebang_from_line_skips_env_dash_flags():
    """``#!/usr/bin/env -S deno run`` names deno: the ``-S`` (and any leading-dash token)
    is skipped by ``not tok.startswith("-")``. Mutating the prefix to a never-matching
    string would take ``-S`` itself as the program."""
    assert reg.shebang_program_from_line("#!/usr/bin/env -S deno run --allow-net") == "deno"
