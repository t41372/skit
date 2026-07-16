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

from ... import pep723
from ...params import ParamDecl

SCT_SCHEMA = 1


def _toml_str(value: str) -> str:
    r"""A TOML basic string. Control characters must be escaped: if a prompt contains something like
    \n and is emitted verbatim, it splits the string across comment lines, the rewritten block fails
    to parse, and all definitions are lost."""
    out = value.replace("\\", "\\\\").replace('"', '\\"')
    out = out.replace("\n", "\\n").replace("\r", "\\r").replace("\t", "\\t")

    # A TOML basic string forbids unescaped U+0000..U+001F and U+007F; escape those.
    # ALSO escape U+0085/U+2028/U+2029: TOML permits them literally, but _commentify splits
    # the rewritten block with str.splitlines(), which breaks on those three too — an
    # unescaped one would shred the comment body and drop every managed param definition.
    def _escape(ch: str) -> str:
        if ch < " " or ch in ("\x7f", "\x85", "\u2028", "\u2029"):  # pragma: no mutate
            return f"\\u{ord(ch):04X}"
        return ch

    out = "".join(_escape(ch) for ch in out)
    return '"' + out + '"'


def _toml_value(value: str | int | float | bool) -> str:
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, (int, float)):
        return repr(value)
    return _toml_str(value)


def render_skit_toml(params: list[ParamDecl]) -> str:
    """Generate the comment-stripped [tool.skit] TOML text (without the comment prefix)."""
    lines = ["[tool.skit]", f"schema = {SCT_SCHEMA}"]
    for p in params:
        lines.append("")
        lines.append("[[tool.skit.params]]")
        for key, val in p.to_block_dict().items():
            lines.append(f"{key} = {_toml_value(val)}")
    return "\n".join(lines) + "\n"


def _commentify(toml_text: str, leader: str = "#") -> list[str]:
    return [(leader + " " + ln).rstrip() for ln in toml_text.splitlines()]


def _strip_comment_prefix(line: str, leader: str = "#") -> str:
    prefix = leader + " "
    if line.startswith(prefix):  # pragma: no mutate — prefix length re-stripped next
        return line[len(prefix) :]
    if line.startswith(leader):
        return line[len(leader) :]
    return line


def _strip_skit_section(body_lines: list[str], leader: str = "#") -> list[str]:
    """Remove any existing [tool.skit] section (and its [[tool.skit.params]]) from the block body,
    keeping the rest."""
    out: list[str] = []
    skipping = False  # pragma: no mutate — only read via truthiness (`if not skipping`)
    for line in body_lines:
        stripped = _strip_comment_prefix(line, leader).strip()
        if stripped.startswith("["):
            in_skit = stripped.startswith("[tool.skit]") or stripped.startswith("[[tool.skit.")
            skipping = in_skit
        if not skipping:
            out.append(line)
    # Drop trailing empty comment lines.
    while out and out[-1].strip() in ("#", ""):
        out.pop()
    return out


def _drop_synthetic_separator(base: str, original: str, leader: str = "#") -> str:
    """Undo inject_block()'s optional blank-line separator when write_params() is about to fill the
    freshly-created block with params in the same operation.

    inject_block() inserts a blank line between the closer and the following code purely for
    readability when it is used on its own (e.g. `skit deps` adding a bare dependencies-only
    block). But when write_params() creates the block itself (this is the "no block yet, non-empty
    params" path), that blank line would be the ONLY thing ever added outside the
    "# /// … # ///" block — breaking the comment-only-edits contract (A5) and the golden-corpus
    byte-fidelity invariant. Detected structurally (not by re-deriving inject_block's shebang/coding
    insertion logic): strip exactly one blank line, and only when it truly is synthetic (i.e.
    `original`'s own content did not already start with a blank line at that point — inject_block
    itself skips the separator in that case, to avoid a double blank line).
    """
    block_only = pep723.build_block([], leader=leader)
    idx = base.index(block_only)
    prefix = base[:idx]
    rest = base[idx + len(block_only) :]
    suffix = original[len(prefix) :]
    if rest == "\n" + suffix:
        return prefix + block_only + suffix
    return base


def write_params(text: str, params: list[ParamDecl], leader: str = "#") -> str:
    """Write (or replace) the script's [tool.skit] with the parameter definitions. Empty params
    means removing the section.

    - Existing inline-metadata block: replace/append [tool.skit] inside the block, others verbatim.
    - No block and non-empty params: create a block with empty dependencies (same insertion point as
      pep723.inject_block).

    `leader` selects the comment dialect: "#" (Python/shell) or "//" (JS/TS). The whole engine is
    otherwise language-blind — it never touches a line outside the `leader /// … leader ///` block.
    """
    m = pep723._block_re(leader).search(text)
    if m is None:
        if not params:
            return text
        base = pep723.inject_block(
            text, [], leader=leader
        )  # pragma: no mutate — build_block only checks truthiness of dependencies
        # inject_block has inserted the block; recurse once through the "block exists" path.
        if pep723._block_re(leader).search(base) is None:  # pragma: no cover — guaranteed by inject
            raise RuntimeError("inject_block failed to create an inline-metadata block")
        base = _drop_synthetic_separator(base, text, leader)
        return write_params(base, params, leader)
    body_lines = m.group("body").splitlines()
    kept = _strip_skit_section(body_lines, leader)
    new_body_lines = list(kept)
    if params:
        if new_body_lines:
            new_body_lines.append(leader)
        new_body_lines.extend(_commentify(render_skit_toml(params), leader))
    new_block = f"{leader} /// script\n"
    if new_body_lines:
        new_block += "\n".join(new_body_lines) + "\n"
    new_block += f"{leader} ///\n"
    return text[: m.start()] + new_block + text[m.end() :]


def read_params(text: str, leader: str = "#") -> list[ParamDecl]:
    """Read the script's [tool.skit] parameter definitions; empty list if no block/section/parse."""
    meta = pep723.parse_block(text, leader)
    if not meta:
        return []
    # All valid TOML, all defended: `tool`, `tool.skit`, and `tool.skit.params` could each be a
    # scalar (e.g. `tool = 5`) rather than the table/array shape this module writes. read_params is
    # contracted to be total ("empty list if no block/section/parse"), so a malformed shape must
    # fall back to [] rather than raising AttributeError/TypeError out of the `.get` chain or the
    # iteration below.
    tool = meta.get("tool")
    skit = tool.get("skit") if isinstance(tool, dict) else None
    raw = skit.get("params") if isinstance(skit, dict) else None
    if not isinstance(raw, list):
        return []
    return [ParamDecl.from_block_dict(d) for d in raw if isinstance(d, dict) and d.get("name")]
