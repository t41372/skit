"""PEP 723 inline script metadata: parse, generate, inject (comment-only edits, A5).

- parse_block(): read the `# /// script` block from a script.
- suggest_dependencies(): walk imports via AST, diff against stdlib, suggest third-party deps.
- inject_block(): insert the completed block into the script text (after shebang / encoding lines).
  The injected content is pure comments and changes no semantics, so a copy-mode copy just runs.

This module is headless; it imports neither CLI nor TUI.
"""

from __future__ import annotations

import ast
import re
import sys
import tomllib
from typing import Any


def _block_re(leader: str) -> re.Pattern[str]:
    """The inline-metadata block regex for a given line-comment `leader` ("#" for Python/shell,
    "//" for JS/TS). One engine, two comment dialects: `# /// script … # ///` and
    `// /// script … // ///`. Neither "#" nor "//" carries a regex metacharacter, so the leader is
    spliced in literally — `_block_re("#").pattern` is therefore byte-identical to the historical
    frozen pattern (pinned by a regression test), which is what keeps the Python golden corpus intact.

    The closer's trailing whitespace is restricted to horizontal whitespace ([^\\S\\n], i.e. space /
    tab / \\r for CRLF) so it cannot cross a line boundary and swallow blank lines that follow the
    block — `\\s*$` would otherwise match greedily across newlines and absorb them into the block,
    deleting them on rewrite (metawriter/pep723 both rebuild the text from m.end()).
    """
    return re.compile(
        rf"(?m)^{leader} /// script\s*$\n(?P<body>(?:^{leader}(?:| .*)$\n)*?)^{leader} ///[^\S\n]*$\n?",
    )


_BLOCK_RE = _block_re("#")

# Import name -> PyPI distribution name, for the common cases where they differ. Without this an
# `import PIL` becomes a `PIL` dependency, which uv can't resolve (the package is `Pillow`). Curated
# and stdlib-only (a static dict, no network / importlib.metadata probing): we only rewrite names we
# are sure about and leave everything else untouched.
_IMPORT_TO_PACKAGE = {
    "PIL": "Pillow",
    "cv2": "opencv-python",
    "yaml": "PyYAML",
    "bs4": "beautifulsoup4",
    "sklearn": "scikit-learn",
    "skimage": "scikit-image",
    "dotenv": "python-dotenv",
    "dateutil": "python-dateutil",
    "serial": "pyserial",
    "jwt": "PyJWT",
    "docx": "python-docx",
    "pptx": "python-pptx",
    "fitz": "PyMuPDF",
    "OpenSSL": "pyOpenSSL",
    "Crypto": "pycryptodome",
    "Cryptodome": "pycryptodomex",
    "git": "GitPython",
    "attr": "attrs",
    "slugify": "python-slugify",
    "usb": "pyusb",
    "win32com": "pywin32",
    "win32api": "pywin32",
}


def _strip_comment_prefix(line: str, leader: str = "#") -> str:
    """_block_re guarantees `leader` or `leader ...` lines; both branches agree on a bare leader."""
    prefix = leader + " "
    if line.startswith(prefix):  # pragma: no mutate — TOML tolerates the extra leading space
        return line[len(prefix) :]
    # pragma: no mutate — only reached on a bare leader; [len(leader):] == [len(prefix):] == ""
    return line[len(leader) :]


def parse_block(text: str, leader: str = "#") -> dict[str, Any] | None:
    """Return the inline-metadata dict (dependencies / requires-python); None if no block."""
    m = _block_re(leader).search(text)
    if not m:
        return None
    lines = [_strip_comment_prefix(line, leader) for line in m.group("body").splitlines()]
    try:
        return tomllib.loads("\n".join(lines))
    except tomllib.TOMLDecodeError:
        return None


def has_block(text: str, leader: str = "#") -> bool:
    return _block_re(leader).search(text) is not None


def suggest_dependencies(text: str) -> list[str]:
    """Scan imports, return the top-level module names that look third-party (sorted, deduped).

    Returns an empty list on syntax errors.
    """
    try:
        tree = ast.parse(text)
    except SyntaxError:
        return []
    found: set[str] = set()
    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            for alias in node.names:
                found.add(alias.name.split(".")[0])
        elif isinstance(node, ast.ImportFrom) and node.module and node.level == 0:
            found.add(node.module.split(".")[0])
    stdlib = sys.stdlib_module_names
    third_party = (m for m in found if m not in stdlib and not m.startswith("_"))
    # Map known import names to their real PyPI distribution names, then dedupe again in case two
    # imports collapse to the same package (e.g. `Crypto` and its submodules -> pycryptodome).
    # A name PEP 508 refuses (`import café` — a legal Python identifier, an illegal
    # distribution name) is not a suggestion: the non-interactive add accepts
    # suggestions as-is, and validate-then-write applies to skit's own fabrications
    # hardest of all.
    return sorted(
        n
        for n in {_IMPORT_TO_PACKAGE.get(m, m) for m in third_party}
        if requirement_error(n) is None
    )


def _next_nonspace(text: str, pos: int) -> str:
    """The first non-whitespace character at or after pos, or "" when none is left."""
    for ch in text[pos:]:
        if not ch.isspace():
            return ch
    return ""


def requires_python_error(value: str) -> str | None:
    """A localized refusal when `value` can't be a requires-python constraint, else
    None. Validation lives at the intakes, never the launch path (validate-then-write):
    an unparseable constraint written into a block bricks every subsequent run with
    uv's raw error — the deferred failure no skit surface would ever have named."""
    from packaging.specifiers import InvalidSpecifier, SpecifierSet

    from .i18n import gettext

    try:
        SpecifierSet(value)
    except InvalidSpecifier:
        return gettext(
            '%(value)s isn\'t a Python version constraint (e.g. ">=3.11" or ">=3.12,<3.13").'
        ) % {"value": value}
    return None


def requirement_error(value: str) -> str | None:
    """The dependency twin of requires_python_error: a PEP 508 check for one
    requirement string. npm dependencies are NOT routed here — their grammar belongs
    to the npm installer, which names its own errors at materialization."""
    from packaging.requirements import InvalidRequirement, Requirement

    from .i18n import gettext

    try:
        Requirement(value)
    except InvalidRequirement:
        return gettext(
            '%(value)s isn\'t a package requirement (e.g. "requests" or "rich>=13,<16").'
        ) % {"value": value}
    return None


def split_requirements(text: str) -> list[str]:
    """Split a comma-separated requirement list without shredding PEP 508 internals.

    A single requirement may itself contain commas — in a version-specifier list
    (``requests>=2,<3``), an extras bracket (``pkg[security,socks]``), a parenthesized
    specifier (``foo (>=1.0,<2.0)``), or a quoted marker value
    (``x; sys_platform in "linux,darwin"``). A naive ``split(",")`` turns
    ``requests>=2,<3`` into the two bogus items ``requests>=2`` and ``<3``.

    A comma separates two requirements only when it sits outside brackets/quotes AND
    what follows starts a new requirement: PEP 508 names begin with a letter or digit,
    while a continued specifier clause always begins with an operator (``<`` ``>``
    ``=`` ``!`` ``~``). A trailing comma (nothing follows) also terminates. Known
    limitation: a direct-URL reference whose URL itself contains a comma is not
    supported (rare enough that guessing would be worse).
    """
    parts: list[str] = []
    buf: list[str] = []
    depth = 0
    quote = ""  # pragma: no mutate — falsy-equivalent sentinel (""/None): only read via truthiness
    for i, ch in enumerate(text):
        if quote:
            buf.append(ch)
            if ch == quote:
                quote = ""  # pragma: no mutate — falsy-equivalent sentinel, as above
            continue
        if ch in ('"', "'"):
            quote = ch
        elif ch in "([":
            depth += 1
        elif ch in ")]":
            depth -= 1
        elif ch == "," and depth == 0:
            nxt = _next_nonspace(text, i + 1)
            if not nxt or nxt.isalnum():
                parts.append("".join(buf))
                buf = []
                continue
        buf.append(ch)
    parts.append("".join(buf))
    return [p.strip() for p in parts if p.strip()]


def _toml_str(value: str) -> str:
    r"""A TOML basic string for the PEP 723 comment block. A raw double quote or backslash
    would terminate/mangle the string — a PEP 508 marker like `; python_version >= "3.8"`
    carries embedded quotes — and any newline-class character would break out of the single
    comment line the block is built from, so the rewritten block fails to re-parse and the
    dependency list is silently lost. (Kept local: metawriter imports pep723, so pep723 must
    not import back for the shared _toml_str.)"""
    out = value.replace("\\", "\\\\").replace('"', '\\"')
    out = out.replace("\n", "\\n").replace("\r", "\\r").replace("\t", "\\t")

    def _escape(ch: str) -> str:
        if ch < " " or ch in ("\x7f", "\x85", "\u2028", "\u2029"):  # pragma: no mutate
            return f"\\u{ord(ch):04X}"
        return ch

    return '"' + "".join(_escape(ch) for ch in out) + '"'


def build_block(dependencies: list[str], requires_python: str = "", leader: str = "#") -> str:
    """Generate the inline-metadata block text (including the trailing newline)."""
    lines = [f"{leader} /// script"]
    if requires_python:
        lines.append(f"{leader} requires-python = {_toml_str(requires_python)}")
    if dependencies:
        lines.append(f"{leader} dependencies = [")
        lines.extend(f"{leader}     {_toml_str(dep)}," for dep in dependencies)
        lines.append(f"{leader} ]")
    else:
        lines.append(f"{leader} dependencies = []")
    lines.append(f"{leader} ///")
    return "\n".join(lines) + "\n"


def _structural_bracket_delta(s: str) -> int:
    """Net count of structural `[` minus `]` in a line of TOML content (already stripped of its
    leading PEP 723 `#`/`# ` comment marker).

    A naive `s.count("[") - s.count("]")` over the raw text is wrong whenever a bracket lives
    inside a TOML string value (e.g. a dependency string `"foo]bar"`) or inside an inline `#`
    comment (e.g. `# pin later [`) — those brackets are data/prose, not array syntax, and must
    not perturb the array-nesting depth used to find the real closing `]`.

    This walks the line char-by-char, tracking whether we are inside a quoted string (TOML basic
    `"..."` strings, where `\\` escapes the next char, or literal `'...'` strings, which have no
    escapes) and stops counting entirely once an unquoted `#` starts an inline comment. Only
    brackets seen outside a string and outside a comment are counted, which is exactly what makes
    them structural TOML array syntax.
    """
    delta = 0
    quote: str | None = None
    i = 0
    n = len(s)
    while i < n:
        ch = s[i]
        if quote:
            if quote == '"' and ch == "\\":
                i += 2  # pragma: no mutate — skip the escaped char; only reachable via `"` strings
                continue
            if ch == quote:
                quote = None
            i += 1
            continue
        if ch in ("'", '"'):
            quote = ch
        elif ch == "#":
            break  # rest of the line is an inline TOML comment; nothing after this is structural
        elif ch == "[":
            delta += 1
        elif ch == "]":
            delta -= 1
        i += 1
    return delta


def set_dependencies(
    text: str, dependencies: list[str], requires_python: str = "", leader: str = "#"
) -> str:
    """Update the dependencies / requires-python lines in an existing block, keeping the rest
    (such as [tool.skit]). With no block this behaves like inject_block. Still comment-only (A5)."""
    m = _block_re(leader).search(text)
    if not m:
        return inject_block(text, dependencies, requires_python, leader)
    body_lines = m.group("body").splitlines()
    kept: list[str] = []
    in_deps_array = False  # pragma: no mutate — only read via truthiness (`if in_deps_array`)
    depth = 0
    for line in body_lines:
        stripped = _strip_comment_prefix(line, leader)
        if in_deps_array:
            # Track bracket nesting depth rather than requiring the closer to be alone on its line:
            # a hand-edited array may close on the same line as the last element (`"requests"]`) or
            # carry a trailing comment (`] # pin`). Only structural brackets count, so
            # `"pkg[extra]"` requirement strings, a stray `]` inside a string (`"foo]bar"`), and a
            # comment containing a bracket (`# pin later [`) can't desync the depth.
            depth += _structural_bracket_delta(stripped)
            if depth <= 0:
                in_deps_array = False  # pragma: no mutate — only read via truthiness
            continue
        s = stripped.strip()
        if s.startswith("requires-python"):
            continue
        if s.startswith("dependencies"):
            # Multi-line array detection: `[` opened but not closed on the same line
            # (including a `[  # comment` trailing-comment form). Structural-only counting here
            # too, for the same reasons as the in-array depth tracking above.
            net = _structural_bracket_delta(s)
            if net > 0:
                in_deps_array = True
                depth = net
            continue
        kept.append(line)
    new_head: list[str] = []
    if requires_python:
        new_head.append(f"{leader} requires-python = {_toml_str(requires_python)}")
    if dependencies:
        new_head.append(f"{leader} dependencies = [")
        new_head.extend(f"{leader}     {_toml_str(dep)}," for dep in dependencies)
        new_head.append(f"{leader} ]")
    else:
        new_head.append(f"{leader} dependencies = []")
    new_body = "\n".join(new_head + kept) + "\n"
    new_block = f"{leader} /// script\n" + new_body + f"{leader} ///\n"
    return text[: m.start()] + new_block + text[m.end() :]


def inject_block(
    text: str, dependencies: list[str], requires_python: str = "", leader: str = "#"
) -> str:
    """Insert the block at the top of the script (after shebang / coding declarations).
    If a block already exists, return the text unchanged.

    The shebang is always `#!` regardless of the comment leader (`#!/usr/bin/env node` is legal
    in a `.mjs` file), so only the block-comment leader varies; the coding-declaration skip is a
    Python-only line shape (`# -*- coding: … -*-`) and is a harmless no-op for a `//` leader, whose
    lines never start with `#`."""
    if has_block(text, leader):
        return text
    lines = text.splitlines(keepends=True)
    insert_at = 0
    if lines and lines[0].startswith("#!"):
        insert_at = 1
    if len(lines) > insert_at and re.match(r"^#.*coding[:=]", lines[insert_at]):
        insert_at += 1
    block = build_block(dependencies, requires_python, leader)
    prefix = "".join(lines[:insert_at])
    suffix = "".join(lines[insert_at:])
    sep = "\n" if suffix and not suffix.startswith("\n") else ""
    return prefix + block + sep + suffix
