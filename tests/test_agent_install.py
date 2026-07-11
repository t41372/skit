"""`skit agent install` — consent-first installer for the bundled Agent Skill.

Non-invasive contract (AGENTS.md #6): an explicit TARGET/--to is consent by flag;
bare interactive mode detects existing agent directories and asks; bare
non-interactive mode refuses with a usage error rather than guessing.
"""

from __future__ import annotations

from pathlib import Path

import pytest
from typer.testing import CliRunner

from skit import agentskill, cli

runner = CliRunner()


@pytest.fixture
def fake_home(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Path:
    home = tmp_path / "home"
    home.mkdir()
    monkeypatch.setenv("HOME", str(home))
    monkeypatch.setenv("USERPROFILE", str(home))  # Path.home() on Windows
    return home


@pytest.fixture
def fake_cwd(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Path:
    cwd = tmp_path / "project"
    cwd.mkdir()
    # Tests must never chdir (see conftest), and monkeypatching os.getcwd would break
    # mutmut's mid-test stats bookkeeping — redirect the CLI's dedicated seam instead.
    # Path.home() stays live so this composes with fake_home's HOME redirect.
    monkeypatch.setattr(agentskill, "default_roots", lambda: (Path.home(), cwd))
    return cwd


SKILL_MARKER = "---\nname: skit\n"


# --------------------------------------------------------------------------
# headless helpers (agentskill.py)
# --------------------------------------------------------------------------


def test_skill_text_is_the_bundled_skill():
    text = agentskill.skill_text()
    assert text.startswith(SKILL_MARKER)


def test_detect_targets_reports_only_existing_marker_dirs(tmp_path):
    home = tmp_path / "h"
    cwd = tmp_path / "c"
    (home / ".claude").mkdir(parents=True)
    (cwd / ".agents").mkdir(parents=True)
    found = agentskill.detect_targets(home=home, cwd=cwd)
    assert [(t.name, t.scope) for t in found] == [("claude", "user"), ("agents", "project")]
    assert found[0].skills_dir == home / ".claude" / "skills"
    assert found[1].skills_dir == cwd / ".agents" / "skills"


def test_detect_targets_empty_when_nothing_exists(tmp_path):
    assert agentskill.detect_targets(home=tmp_path / "h", cwd=tmp_path / "c") == []


def test_named_target_user_and_project_scopes(tmp_path):
    home, cwd = tmp_path / "h", tmp_path / "c"
    claude = agentskill.named_target("claude", project=False, home=home, cwd=cwd)
    assert claude is not None
    assert claude.name == "claude"
    assert claude.scope == "user"
    assert claude.skills_dir == home / ".claude" / "skills"
    codex = agentskill.named_target("codex", project=True, home=home, cwd=cwd)
    assert codex is not None
    assert codex.name == "codex"
    assert codex.scope == "project"
    assert codex.skills_dir == cwd / ".codex" / "skills"


def test_named_target_agents_is_always_project_scoped(tmp_path):
    home, cwd = tmp_path / "h", tmp_path / "c"
    for project in (False, True):
        t = agentskill.named_target("agents", project=project, home=home, cwd=cwd)
        assert t is not None
        assert t.name == "agents"
        assert t.scope == "project"
        assert t.skills_dir == cwd / ".agents" / "skills"


def test_named_target_unknown_is_none(tmp_path):
    home, cwd = tmp_path / "h", tmp_path / "c"
    assert agentskill.named_target("cursor", project=False, home=home, cwd=cwd) is None
    assert agentskill.named_target("cursor", project=True, home=home, cwd=cwd) is None


def test_install_into_writes_and_upgrades(tmp_path):
    skills_dir = tmp_path / "skills"
    out = agentskill.install_into(skills_dir, agentskill.skill_text())
    assert out == skills_dir / "skit" / "SKILL.md"
    assert out.read_text(encoding="utf-8") == agentskill.skill_text()
    out.write_text("stale", encoding="utf-8")
    again = agentskill.install_into(skills_dir, agentskill.skill_text())
    assert again == out
    assert out.read_text(encoding="utf-8") == agentskill.skill_text()  # reinstall = upgrade


# --------------------------------------------------------------------------
# CLI: explicit consent paths
# --------------------------------------------------------------------------


def test_cli_install_to_explicit_dir(tmp_path, monkeypatch):
    monkeypatch.setattr(cli.console, "_width", 400)  # keep the long path on one line
    dest = tmp_path / "anywhere"
    result = runner.invoke(cli.app, ["agent", "install", "--to", str(dest)])
    assert result.exit_code == 0, result.output
    assert (dest / "skit" / "SKILL.md").read_text(encoding="utf-8") == agentskill.skill_text()
    assert (
        f"Installed the skit Agent Skill: {dest / 'skit' / 'SKILL.md'}"
        in result.output.splitlines()
    )


def test_cli_install_broken_package_fails_loudly(tmp_path, monkeypatch):
    """A bundled skill missing from the package is a packaging bug: it must surface
    loudly, not be soft-reported as a destination problem (the OSError wrap around
    install_into is for write errors only — skill_text resolves before it)."""

    def broken() -> str:
        raise FileNotFoundError("SKILL.md missing from the wheel")

    monkeypatch.setattr(agentskill, "skill_text", broken)
    result = runner.invoke(cli.app, ["agent", "install", "--to", str(tmp_path / "d")])
    assert result.exit_code != 0
    assert "Could not write the skill there" not in result.output
    assert isinstance(result.exception, FileNotFoundError)


def test_cli_install_to_a_file_fails_cleanly(tmp_path, monkeypatch):
    monkeypatch.setattr(cli.err_console, "_width", 400)
    blocker = tmp_path / "afile"
    blocker.write_text("not a directory", encoding="utf-8")
    result = runner.invoke(cli.app, ["agent", "install", "--to", str(blocker)])
    assert result.exit_code == 1
    assert any(
        line.startswith("Could not write the skill there: ") for line in result.output.splitlines()
    )
    assert "Traceback" not in result.output


def test_cli_install_to_with_project_is_a_conflict(tmp_path):
    result = runner.invoke(cli.app, ["agent", "install", "--to", str(tmp_path / "x"), "--project"])
    assert result.exit_code == 2
    assert not (tmp_path / "x").exists()


def test_cli_install_to_expands_tilde(fake_home):
    result = runner.invoke(cli.app, ["agent", "install", "--to", "~/myskills"])
    assert result.exit_code == 0, result.output
    assert (fake_home / "myskills" / "skit" / "SKILL.md").is_file()


def test_cli_install_named_target_user_scope(fake_home):
    result = runner.invoke(cli.app, ["agent", "install", "claude"])
    assert result.exit_code == 0, result.output
    assert (fake_home / ".claude" / "skills" / "skit" / "SKILL.md").is_file()


def test_cli_install_named_target_project_scope(fake_cwd):
    result = runner.invoke(cli.app, ["agent", "install", "codex", "--project"])
    assert result.exit_code == 0, result.output
    assert (fake_cwd / ".codex" / "skills" / "skit" / "SKILL.md").is_file()


def test_cli_install_unknown_target_exits_2(fake_home):
    result = runner.invoke(cli.app, ["agent", "install", "cursor"])
    assert result.exit_code == 2
    assert "cursor" in result.output
    assert not (fake_home / ".claude").exists()


def test_cli_install_target_and_to_conflict_exits_2(tmp_path):
    result = runner.invoke(cli.app, ["agent", "install", "claude", "--to", str(tmp_path / "x")])
    assert result.exit_code == 2
    assert not (tmp_path / "x").exists()


# --------------------------------------------------------------------------
# CLI: bare mode — never guess
# --------------------------------------------------------------------------


def test_cli_bare_non_interactive_refuses(fake_home):
    result = runner.invoke(cli.app, ["agent", "install"])  # CliRunner stdin is not a tty
    assert result.exit_code == 2
    assert list(fake_home.iterdir()) == []  # nothing was written anywhere


def test_cli_bare_interactive_no_candidates_exits_1(fake_home, fake_cwd, monkeypatch):
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    result = runner.invoke(cli.app, ["agent", "install"])
    assert result.exit_code == 1
    assert "--to" in result.output


def test_cli_bare_interactive_picks_and_confirms(fake_home, fake_cwd, monkeypatch):
    (fake_home / ".claude").mkdir()
    (fake_cwd / ".agents").mkdir()
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(lambda *a, **k: "2"))
    monkeypatch.setattr(cli.Confirm, "ask", staticmethod(lambda *a, **k: True))
    result = runner.invoke(cli.app, ["agent", "install"])
    assert result.exit_code == 0, result.output
    assert (fake_cwd / ".agents" / "skills" / "skit" / "SKILL.md").is_file()
    assert not (fake_home / ".claude" / "skills").exists()  # only the picked one


def test_cli_bare_interactive_backing_out_writes_nothing(fake_home, fake_cwd, monkeypatch):
    (fake_home / ".claude").mkdir()
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(lambda *a, **k: "1"))
    monkeypatch.setattr(cli.Confirm, "ask", staticmethod(lambda *a, **k: False))
    result = runner.invoke(cli.app, ["agent", "install"])
    assert result.exit_code == 0, result.output
    assert "Cancelled — nothing was written." in result.output
    assert not (fake_home / ".claude" / "skills").exists()


def test_agent_pick_target_renders_the_menu_exactly(monkeypatch):
    # Short synthetic paths so nothing wraps: the menu text is part of the contract
    # a mouse-less user reads, so it is pinned exactly (mutation-hardening).
    targets = [
        agentskill.Target(name="claude", scope="user", base=Path("/tmp/h/.claude")),
        agentskill.Target(name="agents", scope="project", base=Path("/tmp/p/.agents")),
    ]
    seen: dict[str, object] = {}

    def fake_prompt(question, *, choices, default, console):
        assert console is cli.console  # the picker must talk through skit's console
        seen["prompt"] = question
        seen["choices"] = choices
        seen["default"] = default
        return "2"

    def fake_confirm(question, *, default, console):
        assert console is cli.console
        seen["confirm"] = question
        seen["confirm_default"] = default
        return True

    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(fake_prompt))
    monkeypatch.setattr(cli.Confirm, "ask", staticmethod(fake_confirm))
    with cli.console.capture() as cap:
        picked = cli._agent_pick_target(targets)
    assert picked is targets[1]  # the numbered choice maps 1-based onto the list
    # Paths rendered via the Target objects so the pin also holds on Windows separators.
    assert cap.get() == (
        "Agent directories on this machine:\n"
        f"  1. claude (user)  →  {targets[0].skills_dir}\n"
        f"  2. agents (project)  →  {targets[1].skills_dir}\n"
    )
    assert seen["prompt"] == "Install where?"
    assert seen["choices"] == ["1", "2"]
    assert seen["default"] == "1"
    assert seen["confirm"] == f"Write the skill into {targets[1].skills_dir}?"
    assert seen["confirm_default"] is True


def test_agent_pick_target_backing_out_returns_none(monkeypatch):
    targets = [agentskill.Target(name="claude", scope="user", base=Path("/tmp/h/.claude"))]
    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(lambda *a, **k: "1"))
    monkeypatch.setattr(cli.Confirm, "ask", staticmethod(lambda *a, **k: False))
    with cli.console.capture():
        assert cli._agent_pick_target(targets) is None
