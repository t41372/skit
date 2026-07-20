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
    """Read the script's `getopts` surface. None when the script doesn't parse or has no
    getopts call at all; a getopts whose optstring is DYNAMIC (`getopts "$OPTS" opt`) is a
    detected-but-unmodelable parser and degrades honestly (`ok=False, "dynamic"`) — the
    same distinction the python and JS readers draw, so the form says "couldn't model
    this script's own arguments" instead of silently pretending there is no CLI."""
    data = text.encode("utf-8")  # pragma: no mutate — utf-8 equivalence
    root = Parser(_LANGUAGE).parse(data).root_node
    if root.has_error:
        return None
    found = _find_getopts_optstring(root)
    if found is None:
        return None
    optstring, literal = found
    if not literal:
        return ArgSpec(ok=False, reason="dynamic")
    return ArgSpec(fields=_parse_optstring(optstring))


def _find_getopts_optstring(root: Node) -> tuple[str, bool] | None:
    """(optstring, is-literal) for the first `getopts` command, or None when there is no
    getopts (or it has no optstring argument at all). The optstring is getopts' FIRST
    argument; the second is the loop variable. A non-literal first argument returns
    ("", False) — detected, but dynamic."""
    for node in _walk(root):
        if node.type != "command":
            continue
        name = node.child_by_field_name("name")
        if name is None or _text(name) != "getopts":
            continue
        args = _arguments(node)
        if not args:
            return None  # `getopts` with no optstring — nothing to read
        literal = _literal_text(args[0])
        if literal is None:
            return ("", False)
        return (literal, True)
    return None


def _parse_optstring(optstring: str) -> list[ParamDecl]:
    """One field per option letter: `letter:` ⇒ a str value flag, a bare letter ⇒ a store_true
    boolean. A leading `:` (silent mode) and any non-alphanumeric character are skipped; repeated
    letters keep the first."""
    # No-op strip, kept for intent: a leading ':' is skipped by the non-alnum branch below anyway,
    # so this can never change the emitted fields. Isolated onto its own line so the (equivalent)
    # ':' comparison can be pragma'd without hiding the killable `[1:]` slice on the next line.
    silent = optstring.startswith(":")  # pragma: no mutate — leading-':' strip is a semantic no-op
    body = optstring[1:] if silent else optstring
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
    # binding "none" / delivery "flag" are the ParamDecl defaults; passing them explicitly would
    # only add equivalent drop-kwarg mutants. The behaviour-bearing fields stay explicit.
    decl = ParamDecl(
        name=letter,
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
