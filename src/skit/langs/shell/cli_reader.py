"""Static getopts reader: turn `while getopts "ab:c:" opt` into flag fields.

The shell analogue of the argparse / parseArgs / param() readers: a shell script that parses its
own options with the `getopts` builtin doesn't get injection — skit reads the LITERAL optstring
statically and assembles real single-dash flags (`-a`, `-b value`).

Honesty rules (mirrors the other readers):
- Only a LITERAL optstring is trusted. A dynamic one (built from a variable or a command
  substitution) can't be read statically ⇒ None, so callers fall back to the other form sources.
- A leading `:` in the optstring is getopts' "silent error" mode, not an option — skipped.
- A letter followed by `:` takes a value (a str flag); a bare letter is a boolean store_true flag.
- Case-arm help text (`case $opt in a) …`) is not extracted in v1.

Uses the analyzer's tree-sitter handle (node walk, no query strings), so it lives behind the same
grammar import guard as the rest of shell's analysis — a broken grammar wheel degrades it to None
along with the analyzer/injector.
"""

from __future__ import annotations

from typing import TYPE_CHECKING

from tree_sitter import Parser

from ...params import ParamDecl, is_secret_name
from ..python.argspec import ArgSpec
from .analyzer import _LANGUAGE, _arguments, _literal_text, _text, _walk

if TYPE_CHECKING:
    from tree_sitter import Node


def read_cli(text: str) -> ArgSpec | None:
    """Read the script's `getopts` surface. None when the script doesn't parse, has no getopts
    call, or builds its optstring dynamically — every one a "no readable surface" case."""
    root = Parser(_LANGUAGE).parse(text.encode("utf-8")).root_node
    if root.has_error:
        return None
    optstring = _find_getopts_optstring(root)
    if optstring is None:
        return None
    return ArgSpec(fields=_parse_optstring(optstring))


def _find_getopts_optstring(root: Node) -> str | None:
    """The literal optstring of the first `getopts` command, or None (no getopts, or its optstring
    is dynamic / absent). The optstring is getopts' FIRST argument; the second is the loop variable."""
    for node in _walk(root):
        if node.type != "command":
            continue
        name = node.child_by_field_name("name")
        if name is None or _text(name) != "getopts":
            continue
        args = _arguments(node)
        if not args:
            return None  # `getopts` with no optstring — nothing to read
        return _literal_text(args[0])  # None when the optstring isn't a plain literal (dynamic)
    return None


def _parse_optstring(optstring: str) -> list[ParamDecl]:
    """One field per option letter: `letter:` ⇒ a str value flag, a bare letter ⇒ a store_true
    boolean. A leading `:` (silent mode) and any non-alphanumeric character are skipped; repeated
    letters keep the first."""
    body = optstring[1:] if optstring.startswith(":") else optstring
    fields: list[ParamDecl] = []
    seen: set[str] = set()
    i = 0
    n = len(body)
    while i < n:
        ch = body[i]
        if not ch.isalnum():
            i += 1
            continue  # only single alphanumeric option letters are modeled
        takes_value = i + 1 < n and body[i + 1] == ":"
        if ch not in seen:
            seen.add(ch)
            fields.append(_option(ch, takes_value))
        i += 2 if takes_value else 1
    return fields


def _option(letter: str, takes_value: bool) -> ParamDecl:
    decl = ParamDecl(
        name=letter,
        binding="none",
        delivery="flag",
        flag=f"-{letter}",
        secret=is_secret_name(letter),
    )
    if takes_value:
        decl.type = "str"
    else:
        decl.type = "bool"
        decl.action = "store_true"
        decl.default = False
    return decl
