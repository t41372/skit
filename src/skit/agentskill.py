"""The bundled Agent Skill and where it can be installed (headless; the CLI renders).

skit ships an official Agent Skill (skills/skit/SKILL.md, agentskills.io format) that
teaches AI coding agents to drive the library through the CLI. This module owns the
headless half of `skit agent install`: locating the bundled copy inside the installed
package and computing which agent skill directories exist on this machine.

Non-invasive by design (AGENTS.md principle #6): detection only reports directories
whose parent tool is already present (`~/.claude` exists, `./.codex` exists, …) — skit
never conjures another tool's config tree uninvited. Writing anywhere requires either
an explicit target/--to (consent by flag) or an interactive confirmation.
"""

from __future__ import annotations

from dataclasses import dataclass
from importlib import resources
from pathlib import Path

SKILL_DIR_NAME = "skit"
SKILL_FILE_NAME = "SKILL.md"

# Known agent conventions: tool marker directory -> skills subdirectory lives inside it.
# claude: Claude Code (user ~/.claude/skills, project .claude/skills)
# codex:  OpenAI Codex CLI (user ~/.codex/skills, project .codex/skills)
# agents: the cross-agent project convention (.agents/skills), project-only.
USER_TARGETS = {"claude": ".claude", "codex": ".codex"}
PROJECT_TARGETS = {"claude": ".claude", "codex": ".codex", "agents": ".agents"}


@dataclass
class Target:
    """One place the skill can be installed: `<base>/skills/skit/SKILL.md`."""

    name: str  # "claude" | "codex" | "agents"
    scope: str  # "user" | "project"
    base: Path  # the tool's marker directory (e.g. ~/.claude)

    @property
    def skills_dir(self) -> Path:
        return self.base / "skills"


def default_roots() -> tuple[Path, Path]:
    """(home, cwd) for target resolution — one seam for the CLI and for tests. Tests
    redirect this instead of monkeypatching os.getcwd, which mutmut's stats
    bookkeeping also relies on mid-test."""
    return Path.home(), Path.cwd()


def skill_text() -> str:
    """The bundled SKILL.md, read from the installed package (single source: the repo's
    skills/skit/SKILL.md is a test-enforced byte-identical copy of this file)."""
    package = "skit"
    # files(None) infers this module's own package, and agentskill lives directly in `skit`, so
    # anchor→None resolves to the identical package root — an equivalent mutant. Scoped to this one
    # line so the killable anchor-string ("skit"→…) and joinpath mutants below stay mutation-tested.
    root = resources.files(package)  # pragma: no mutate
    res = root.joinpath("skills", SKILL_DIR_NAME, SKILL_FILE_NAME)
    # I/O kwarg mutants ("utf-8"→"UTF-8"/None) are equivalent aliases here; content is
    # pinned by test_skill_ships_inside_the_package (see docs/mutation-ledger.md).
    return res.read_text(encoding="utf-8")  # pragma: no mutate


def named_target(name: str, *, project: bool, home: Path, cwd: Path) -> Target | None:
    """Resolve an explicit target name ("claude" / "codex" / "agents") to a Target.
    None for an unknown name. `agents` is a project-level convention, so it resolves
    to the project scope regardless of the --project flag."""
    if name == "agents":
        return Target(name=name, scope="project", base=cwd / PROJECT_TARGETS[name])
    if project:
        marker = PROJECT_TARGETS.get(name)
        return Target(name=name, scope="project", base=cwd / marker) if marker else None
    marker = USER_TARGETS.get(name)
    return Target(name=name, scope="user", base=home / marker) if marker else None


def detect_targets(*, home: Path, cwd: Path) -> list[Target]:
    """Every known target whose marker directory already exists — the tool is in use,
    so offering to drop a skill inside it is not an intrusion. User scope first."""
    found: list[Target] = []
    for name, marker in USER_TARGETS.items():
        base = home / marker
        if base.is_dir():
            found.append(Target(name=name, scope="user", base=base))
    for name, marker in PROJECT_TARGETS.items():
        base = cwd / marker
        if base.is_dir():
            found.append(Target(name=name, scope="project", base=base))
    return found


def install_into(skills_dir: Path, text: str) -> Path:
    """Write `text` as `<skills_dir>/skit/SKILL.md` (idempotent: rewriting is how an
    upgrade lands). Returns the written file's path. Deliberately write-only — the
    caller resolves the bundled copy via skill_text() first, so a broken installation
    fails loudly there instead of masquerading as a destination write error here."""
    dest = skills_dir / SKILL_DIR_NAME
    dest.mkdir(parents=True, exist_ok=True)
    out = dest / SKILL_FILE_NAME
    # Same I/O kwarg equivalence; written content is pinned byte-for-byte by
    # test_install_into_writes_and_upgrades (see docs/mutation-ledger.md).
    out.write_text(text, encoding="utf-8")  # pragma: no mutate
    return out
