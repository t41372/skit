"""Platform-aware editable argv text behavior."""

from types import SimpleNamespace

from skit import argv_text


def test_windows_split_ignores_separator_only_tail(monkeypatch):
    monkeypatch.setattr(argv_text, "sys", SimpleNamespace(platform="win32"))
    assert argv_text.split(" \t ") == []
    assert argv_text.split("agent.exe \t ") == ["agent.exe"]


def test_join_quotes_with_the_running_platforms_convention(monkeypatch):
    """The quoting half of the pair, pinned per platform: the runner picker and the path picker
    show a command line the user reads and edits in place, and shlex's single quotes are
    word-splitting noise to cmd.exe. (skit's own paste-able hints stopped quoting entirely in
    round 8 — they name the slug, which needs no convention — so this pair now serves the
    editable-argv surfaces alone.)"""
    monkeypatch.setattr(argv_text, "sys", SimpleNamespace(platform="win32"))
    assert argv_text.join(["my tool"]) == '"my tool"'
    assert argv_text.split(argv_text.join(["my tool"])) == ["my tool"]

    monkeypatch.setattr(argv_text, "sys", SimpleNamespace(platform="linux"))
    assert argv_text.join(["my tool"]) == "'my tool'"
    assert argv_text.split(argv_text.join(["my tool"])) == ["my tool"]
