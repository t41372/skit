"""Platform-aware editable argv text behavior."""

from types import SimpleNamespace

from skit import argv_text


def test_windows_split_ignores_separator_only_tail(monkeypatch):
    monkeypatch.setattr(argv_text, "sys", SimpleNamespace(platform="win32"))
    assert argv_text.split(" \t ") == []
    assert argv_text.split("agent.exe \t ") == ["agent.exe"]


def test_join_quotes_with_the_running_platforms_convention(monkeypatch):
    """The quoting half of the pair, pinned per platform because a caller outside the TUI
    depends on it now: the `params --add` hint is a command the user pastes into the shell
    they are sitting in, and shlex's single quotes are word-splitting noise to cmd.exe."""
    monkeypatch.setattr(argv_text, "sys", SimpleNamespace(platform="win32"))
    assert argv_text.join(["my tool"]) == '"my tool"'
    assert argv_text.split(argv_text.join(["my tool"])) == ["my tool"]

    monkeypatch.setattr(argv_text, "sys", SimpleNamespace(platform="linux"))
    assert argv_text.join(["my tool"]) == "'my tool'"
    assert argv_text.split(argv_text.join(["my tool"])) == ["my tool"]
