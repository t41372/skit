"""The bundled Agent Skill: spec compliance, packaging, and anti-drift.

Three invariants keep skills/skit/SKILL.md honest:
1. The frontmatter satisfies the Agent Skills spec (agentskills.io) so every
   compatible agent can load it.
2. The repo-root copy (what `npx skills add t41372/skit` discovers) is byte-identical
   to the packaged copy (what `skit agent install` writes) — a symlink would break
   Windows checkouts, so a test enforces the sync instead.
3. Every `skit …` invocation the skill teaches resolves against the real command
   tree: a renamed command or dropped flag fails here before it can strand an agent.
"""

from __future__ import annotations

import re
import shlex
from importlib import resources
from pathlib import Path
from typing import Any

import typer.main

from skit import cli

ROOT = Path(__file__).resolve().parent.parent
ROOT_SKILL = ROOT / "skills" / "skit" / "SKILL.md"
PACKAGED_SKILL = ROOT / "src" / "skit" / "skills" / "skit" / "SKILL.md"

# agentskills.io spec: lowercase alphanumerics and single hyphens, ≤64 chars.
NAME_RE = re.compile(r"^[a-z0-9]+(-[a-z0-9]+)*$")


def _frontmatter(text: str) -> dict[str, str]:
    """The skill's YAML frontmatter. skit's own skill deliberately keeps it to plain
    `key: value` lines (no nesting), so a line parser is enough — no yaml dependency."""
    assert text.startswith("---\n")
    block = text.split("---\n", 2)[1]
    out: dict[str, str] = {}
    for line in block.splitlines():
        key, sep, value = line.partition(":")
        assert sep, f"frontmatter line without a colon: {line!r}"
        out[key.strip()] = value.strip()
    return out


def test_root_and_packaged_copies_are_identical():
    # To update the skill: edit skills/skit/SKILL.md, then
    #   cp skills/skit/SKILL.md src/skit/skills/skit/SKILL.md
    assert ROOT_SKILL.read_bytes() == PACKAGED_SKILL.read_bytes()


def test_skill_ships_inside_the_package():
    res = resources.files("skit").joinpath("skills", "skit", "SKILL.md")
    assert res.is_file()
    assert res.read_text(encoding="utf-8") == ROOT_SKILL.read_text(encoding="utf-8")


def test_frontmatter_satisfies_the_agent_skills_spec():
    fm = _frontmatter(ROOT_SKILL.read_text(encoding="utf-8"))
    assert fm["name"] == "skit"
    assert fm["name"] == ROOT_SKILL.parent.name  # spec: name must match the directory
    assert NAME_RE.fullmatch(fm["name"])
    assert len(fm["name"]) <= 64
    assert 1 <= len(fm["description"]) <= 1024
    assert "compatibility" not in fm or 1 <= len(fm["compatibility"]) <= 500
    assert fm["license"] == "MIT"


def test_skill_stays_within_the_progressive_disclosure_budget():
    # The spec recommends keeping SKILL.md under 500 lines; agents load the whole body
    # on activation, so bloat here is a per-use context tax.
    assert len(ROOT_SKILL.read_text(encoding="utf-8").splitlines()) < 500


def _skill_command_lines() -> list[str]:
    lines: list[str] = []
    in_block = False
    for line in ROOT_SKILL.read_text(encoding="utf-8").splitlines():
        if line.strip().startswith("```"):
            in_block = not in_block
            continue
        stripped = line.strip()
        if in_block and stripped.startswith("skit "):
            lines.append(stripped)
    return lines


def _click_root() -> Any:
    # typer may vendor its own click layer, so resolve the tree by duck typing
    # (`.commands` on groups, `.params`/`.opts` on commands) rather than isinstance.
    cmd = typer.main.get_command(cli.app)
    assert hasattr(cmd, "commands")
    return cmd


def _resolve(root: Any, tokens: list[str]) -> tuple[Any, list[str]]:
    node: Any = root
    rest = list(tokens)
    while rest and rest[0] in getattr(node, "commands", {}):
        node = node.commands[rest[0]]
        rest = rest[1:]
    assert node is not root, f"unknown skit subcommand in SKILL.md: {tokens[:2]}"
    return node, rest


def test_every_command_the_skill_teaches_exists():
    root = _click_root()
    lines = _skill_command_lines()
    assert len(lines) >= 15  # the skill actually teaches the surface, not a stub
    for line in lines:
        tokens = shlex.split(line, comments=True)[1:]  # drop the leading "skit"
        command, rest = _resolve(root, tokens)
        allowed: set[str] = set()
        for param in command.params:
            allowed.update(param.opts)
            allowed.update(param.secondary_opts)
        for tok in rest:
            if tok == "--":
                break  # passthrough args belong to the user's script, not to skit
            if tok.startswith("--"):
                flag = tok.split("=", 1)[0]
                assert flag in allowed, f"SKILL.md uses unknown flag {flag!r} in: {line}"
            elif len(tok) == 2 and tok.startswith("-") and tok[1].isalpha():
                assert tok in allowed, f"SKILL.md uses unknown flag {tok!r} in: {line}"


def test_the_skill_never_mentions_json_free_surfaces_wrongly():
    # Every `--json` the skill shows must be real: a command that silently ignores an
    # unknown flag doesn't exist in click/typer (it errors), but the cheap guarantee
    # here is that we never document --json on a command that lacks it.
    root = _click_root()
    for line in _skill_command_lines():
        if "--json" not in line:
            continue
        tokens = shlex.split(line, comments=True)[1:]
        command, _ = _resolve(root, tokens)
        allowed = {opt for param in command.params for opt in param.opts}
        assert "--json" in allowed, f"--json documented but not offered: {line}"
