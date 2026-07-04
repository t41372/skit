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

_BLOCK_RE = re.compile(
    r"(?m)^# /// script\s*$\n(?P<body>(?:^#(?:| .*)$\n)*?)^# ///\s*$\n?",
)


def parse_block(text: str) -> dict[str, Any] | None:
    """Return the PEP 723 metadata dict (dependencies / requires-python); None if no block."""
    m = _BLOCK_RE.search(text)
    if not m:
        return None
    lines = [
        line[2:] if line.startswith("# ") else line[1:] for line in m.group("body").splitlines()
    ]
    try:
        return tomllib.loads("\n".join(lines))
    except tomllib.TOMLDecodeError:
        return None


def has_block(text: str) -> bool:
    return _BLOCK_RE.search(text) is not None


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
    return sorted(m for m in found if m not in stdlib and not m.startswith("_"))


def build_block(dependencies: list[str], requires_python: str = "") -> str:
    """Generate the PEP 723 block text (including the trailing newline)."""
    lines = ["# /// script"]
    if requires_python:
        lines.append(f'# requires-python = "{requires_python}"')
    if dependencies:
        lines.append("# dependencies = [")
        lines.extend(f'#     "{dep}",' for dep in dependencies)
        lines.append("# ]")
    else:
        lines.append("# dependencies = []")
    lines.append("# ///")
    return "\n".join(lines) + "\n"


def set_dependencies(text: str, dependencies: list[str], requires_python: str = "") -> str:
    """Update the dependencies / requires-python lines in an existing block, keeping the rest
    (such as [tool.skit]). With no block this behaves like inject_block. Still comment-only (A5)."""
    m = _BLOCK_RE.search(text)
    if not m:
        return inject_block(text, dependencies, requires_python)
    body_lines = m.group("body").splitlines()
    kept: list[str] = []
    in_deps_array = False
    for line in body_lines:
        stripped = line[2:] if line.startswith("# ") else line[1:]
        if in_deps_array:
            if stripped.strip() == "]":
                in_deps_array = False
            continue
        s = stripped.strip()
        if s.startswith("requires-python"):
            continue
        if s.startswith("dependencies"):
            # Multi-line array detection: `[` opened but not closed on the same line
            # (including a `[  # comment` trailing-comment form).
            if "[" in s and "]" not in s:
                in_deps_array = True
            continue
        kept.append(line)
    new_head: list[str] = []
    if requires_python:
        new_head.append(f'# requires-python = "{requires_python}"')
    if dependencies:
        new_head.append("# dependencies = [")
        new_head.extend(f'#     "{dep}",' for dep in dependencies)
        new_head.append("# ]")
    else:
        new_head.append("# dependencies = []")
    new_body = "\n".join(new_head + kept)
    if new_body:
        new_body += "\n"
    new_block = "# /// script\n" + new_body + "# ///\n"
    return text[: m.start()] + new_block + text[m.end() :]


def inject_block(text: str, dependencies: list[str], requires_python: str = "") -> str:
    """Insert the block at the top of the script (after shebang / coding declarations).
    If a block already exists, return the text unchanged."""
    if has_block(text):
        return text
    lines = text.splitlines(keepends=True)
    insert_at = 0
    if lines and lines[0].startswith("#!"):
        insert_at = 1
    if len(lines) > insert_at and re.match(r"^#.*coding[:=]", lines[insert_at]):
        insert_at += 1
    block = build_block(dependencies, requires_python)
    prefix = "".join(lines[:insert_at])
    suffix = "".join(lines[insert_at:])
    sep = "\n" if suffix and not suffix.startswith("\n") else ""
    return prefix + block + sep + suffix
