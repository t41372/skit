"""Mutation-kill tests for skit/agentskill.py.

skill_text() reads the SKILL.md that ships inside the installed `skit` package. The bundled
copy is the single source `skit agent install` writes and the CLI renders, so its real,
observable content is what every consumer depends on.
"""

from __future__ import annotations

from pathlib import Path

from skit import agentskill


def test_skill_text_returns_the_bundled_skill_content() -> None:
    """skill_text() resolves the packaged SKILL.md and returns its real bytes as text — the
    frontmatter (name/license) and the first heading are load-bearing content the CLI hands to
    agents, so a wrong anchor (reading the wrong file / failing to resolve) is observable here."""
    text = agentskill.skill_text()
    # Frontmatter contract (agentskills.io): the skill is named "skit" under an MIT license.
    assert text.startswith("---\n")
    assert "\nname: skit\n" in text
    assert "\nlicense: MIT\n" in text
    # Body: the document's own H1 heading, unique to the real SKILL.md.
    assert "# skit — the user's entry library" in text


def test_skill_text_matches_the_file_on_disk() -> None:
    """The returned text is byte-for-byte the packaged SKILL.md (src/skit/skills/skit/SKILL.md),
    not some other resource — pins that the anchor and path segments resolve to exactly that file."""
    packaged = Path(agentskill.__file__).resolve().parent / "skills" / "skit" / "SKILL.md"
    assert agentskill.skill_text() == packaged.read_text(encoding="utf-8")
