"""ParamsIO for JS/TS: the `[tool.skit]` definitions carried in the file body's `// ///` block.

The comment-block engine is language-blind (`langs/python/metawriter`) — the only difference between
Python/shell (`# /// script … # ///`) and JS/TS (`// /// script … // ///`) is the line-comment
leader, so this module is a two-line binding of that engine to `"//"`. Byte fidelity for both
dialects is pinned by the golden corpus and by the frozen-regex regression assert.
"""

from __future__ import annotations

from typing import TYPE_CHECKING

from ..python import metawriter

if TYPE_CHECKING:
    from ...params import ParamDecl

_LEADER = "//"


def read_params(text: str) -> list[ParamDecl]:
    """The JS/TS entry's [tool.skit] parameter definitions; empty list if no block/section/parse."""
    return metawriter.read_params(text, leader=_LEADER)


def write_params(text: str, params: list[ParamDecl]) -> str:
    """Write (or replace) the JS/TS entry's [tool.skit] in `// ///` form. Empty params removes it."""
    return metawriter.write_params(text, params, leader=_LEADER)
