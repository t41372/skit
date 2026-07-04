"""MetaWriter: write parameter definitions into the `[tool.skit]` table of a PEP 723 block (A5/A6).

Strict discipline:
- **Plain-text operations, never through the AST** — never reorder or reformat a single line of the
  user's code.
- Only touch the `# /// script … # ///` comment block; create one (with empty dependencies) if none
  exists.
- Inside the block, only the `[tool.skit]` section is replaced; everything else (dependencies, other
  tool tables) is preserved verbatim.
- Definitions travel with the file (portable, hand-editable); values and presets live in central
  state (argstate).

Shape of `[tool.skit]` (the TOML once comment prefixes are stripped):

    [tool.skit]
    schema = 1

    [[tool.skit.params]]
    name = "API_KEY"
    kind = "const"          # const | input
    type = "str"            # str | int | float | bool
    default = "xxx"         # const: the source value (absent for input)
    prompt = "API key: "    # input: the original prompt; const may set a custom form prompt
    order = 0               # input: the call-order key (B1); omitted for const
    secret = true           # C3: the value never lands in a state file
"""

from __future__ import annotations

import re
from dataclasses import dataclass, field
from typing import Any

from . import pep723

SCT_SCHEMA = 1

_BLOCK_RE = re.compile(
    r"(?m)^# /// script\s*$\n(?P<body>(?:^#(?:| .*)$\n)*?)^# ///\s*$\n?",
)


@dataclass
class ParamSpec:
    """One `[[tool.skit.params]]`. Field-aligned with analyzer.Candidate and inter-convertible."""

    name: str
    kind: str = "const"  # "const" | "input"
    type: str = "str"
    default: str | int | float | bool | None = None
    prompt: str = ""
    order: int = -1
    secret: bool = False

    def to_dict(self) -> dict[str, str | int | float | bool]:
        d: dict[str, str | int | float | bool] = {
            "name": self.name,
            "kind": self.kind,
            "type": self.type,
        }
        if self.default is not None:
            d["default"] = self.default
        if self.prompt:
            d["prompt"] = self.prompt
        if self.order >= 0:
            d["order"] = self.order
        if self.secret:
            d["secret"] = True
        return d

    @classmethod
    def from_dict(cls, d: dict[str, Any]) -> ParamSpec:
        return cls(
            name=str(d.get("name", "")),
            kind=str(d.get("kind", "const")),
            type=str(d.get("type", "str")),
            default=d.get("default"),
            prompt=str(d.get("prompt", "")),
            order=int(d.get("order", -1)),
            secret=bool(d.get("secret", False)),
        )


@dataclass
class SctMeta:
    params: list[ParamSpec] = field(default_factory=list)


def _toml_str(value: str) -> str:
    r"""A TOML basic string. Control characters must be escaped: if a prompt contains something like
    \n and is emitted verbatim, it splits the string across comment lines, the rewritten block fails
    to parse, and all definitions are lost."""
    out = value.replace("\\", "\\\\").replace('"', '\\"')
    out = out.replace("\n", "\\n").replace("\r", "\\r").replace("\t", "\\t")
    # A TOML basic string forbids unescaped U+0000..U+001F and U+007F.
    out = "".join(f"\\u{ord(ch):04X}" if ch < " " or ch == "\x7f" else ch for ch in out)
    return '"' + out + '"'


def _toml_value(value: str | int | float | bool) -> str:
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, (int, float)):
        return repr(value)
    return _toml_str(value)


def render_skit_toml(params: list[ParamSpec]) -> str:
    """Generate the comment-stripped [tool.skit] TOML text (without the comment prefix)."""
    lines = ["[tool.skit]", f"schema = {SCT_SCHEMA}"]
    for p in params:
        lines.append("")
        lines.append("[[tool.skit.params]]")
        for key, val in p.to_dict().items():
            lines.append(f"{key} = {_toml_value(val)}")
    return "\n".join(lines) + "\n"


def _commentify(toml_text: str) -> list[str]:
    return [("# " + ln).rstrip() for ln in toml_text.splitlines()]


def _strip_skit_section(body_lines: list[str]) -> list[str]:
    """Remove any existing [tool.skit] section (and its [[tool.skit.params]]) from the block body,
    keeping the rest."""
    out: list[str] = []
    skipping = False
    for line in body_lines:
        stripped = line[2:] if line.startswith("# ") else line[1:] if line.startswith("#") else line
        stripped = stripped.strip()
        if stripped.startswith("["):
            in_skit = stripped.startswith("[tool.skit]") or stripped.startswith("[[tool.skit.")
            skipping = in_skit
        if not skipping:
            out.append(line)
    # Drop trailing empty comment lines.
    while out and out[-1].strip() in ("#", ""):
        out.pop()
    return out


def write_params(text: str, params: list[ParamSpec]) -> str:
    """Write (or replace) the script's [tool.skit] with the parameter definitions. Empty params
    means removing the section.

    - Existing PEP 723 block: replace/append [tool.skit] inside the block, other lines verbatim.
    - No block and non-empty params: create a block with empty dependencies (same insertion point as
      pep723.inject_block).
    """
    m = _BLOCK_RE.search(text)
    if m is None:
        if not params:
            return text
        base = pep723.inject_block(text, [])  # create a block with empty dependencies
        # inject_block has inserted the block; recurse once through the "block exists" path.
        if _BLOCK_RE.search(base) is None:  # pragma: no cover — inject_block guarantees a block
            raise RuntimeError("inject_block failed to create a PEP 723 block")
        return write_params(base, params)
    body_lines = m.group("body").splitlines()
    kept = _strip_skit_section(body_lines)
    new_body_lines = list(kept)
    if params:
        if new_body_lines:
            new_body_lines.append("#")
        new_body_lines.extend(_commentify(render_skit_toml(params)))
    new_block = "# /// script\n"
    if new_body_lines:
        new_block += "\n".join(new_body_lines) + "\n"
    new_block += "# ///\n"
    return text[: m.start()] + new_block + text[m.end() :]


def read_params(text: str) -> list[ParamSpec]:
    """Read the script's [tool.skit] parameter definitions; empty list if no block/section/parse."""
    meta = pep723.parse_block(text)
    if not meta:
        return []
    skit = meta.get("tool", {}).get("skit", {})
    raw = skit.get("params", [])
    return [ParamSpec.from_dict(d) for d in raw if isinstance(d, dict) and d.get("name")]
