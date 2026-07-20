"""Platform-aware editable argv text behavior."""

from types import SimpleNamespace

from skit import argv_text


def test_windows_split_ignores_separator_only_tail(monkeypatch):
    monkeypatch.setattr(argv_text, "sys", SimpleNamespace(platform="win32"))
    assert argv_text.split(" \t ") == []
    assert argv_text.split("agent.exe \t ") == ["agent.exe"]
