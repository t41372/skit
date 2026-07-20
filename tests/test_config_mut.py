"""Mutation-survivor coverage for config.py (companion to test_config.py / test_config_cmd.py).

Two clusters the base suite left under-pinned:

* the corrupt-config warning `_load_config_for_save` prints — its exact wording and its
  `%(path)s` / `%(backup)s` substitution, on both the backup-succeeded and backup-failed branches;
* the read-modify-write savers `save_bash_path` / `save_js_runner` — the `dict.pop(key, None)`
  no-default-error contract when the key/section is already absent, and that `save_js_runner`
  reads the *existing* `[js]` section (not a fresh one) so sibling keys survive a runner write.
"""

from __future__ import annotations

from pathlib import Path

import pytest

from skit import atomic, config, i18n


@pytest.fixture(autouse=True)
def cfg_dir(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Path:
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path))
    # The warning-text assertions compare against the English source msgids; pin the locale so a
    # translation left active by an earlier test can't turn them into false failures.
    i18n.init("en")
    return tmp_path


def _write_corrupt(cfg_dir: Path) -> None:
    (cfg_dir / "config.toml").write_text("this is = = not [valid toml", encoding="utf-8")


# --- corrupt-config warning: exact wording + path/backup interpolation --------------------------


def test_corrupt_config_backup_warning_is_verbatim(
    cfg_dir: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    """Backup-succeeded branch: the stderr warning is the exact English sentence, with the real
    config path and the real .bak path interpolated (not a mutated/miscased string, not str(None))."""
    _write_corrupt(cfg_dir)
    config.save_editor("vim")  # any read-modify-write saver routes through _load_config_for_save
    err = capsys.readouterr().err
    path = cfg_dir / "config.toml"
    backup = cfg_dir / "config.toml.bak"
    assert backup.is_file()  # the branch under test really did take the backup path
    expected = (
        f"{path} is corrupt and could not be parsed. It has been backed up to "
        f"{backup} before this change; recover any lost settings from that file."
    )
    assert expected in err


def test_corrupt_config_no_backup_warning_is_verbatim(
    cfg_dir: Path, capsys: pytest.CaptureFixture[str], monkeypatch: pytest.MonkeyPatch
) -> None:
    """Backup-failed branch (copy2 raises → backup_path is None): the *other* exact English sentence
    is printed, again with the real path interpolated, and no .bak file is left behind."""
    _write_corrupt(cfg_dir)

    def boom(*_a: object, **_k: object) -> None:
        raise OSError("read-only config dir")

    monkeypatch.setattr(atomic.shutil, "copy2", boom)
    config.save_editor("vim")
    err = capsys.readouterr().err
    path = cfg_dir / "config.toml"
    assert not (cfg_dir / "config.toml.bak").exists()  # the backup genuinely failed
    expected = (
        f"{path} is corrupt and could not be parsed, and it could not be backed up "
        "either; the settings it contained will be lost when this change is saved."
    )
    assert expected in err


# --- save_bash_path: pop() must tolerate an already-absent key/section ---------------------------


def test_save_bash_path_clear_tolerates_missing_bash_path_key(cfg_dir: Path) -> None:
    """Clearing bash_path when the [shell] section has no bash_path key must be a no-op, not a
    KeyError — so `section.pop("bash_path", None)` keeps its default, and the sibling key survives."""
    config.save_config({"shell": {"other": "keep"}})
    config.save_bash_path("")  # would raise KeyError if pop lost its None default
    assert config.load_config()["shell"] == {"other": "keep"}


def test_save_bash_path_clear_tolerates_missing_shell_section(cfg_dir: Path) -> None:
    """Clearing bash_path when there is no [shell] section at all must not raise: the empty section
    drops through to `doc.pop("shell", None)`, whose None default absorbs the absent key."""
    config.save_config({"language": "en"})
    config.save_bash_path("")  # would raise KeyError if doc.pop lost its None default
    doc = config.load_config()
    assert doc == {"language": "en"}
    assert "shell" not in doc


# --- save_js_runner: pop() default + reading the *existing* [js] section -------------------------


def test_save_js_runner_clear_tolerates_missing_runner_key(cfg_dir: Path) -> None:
    """Clearing the runner when [js] has no runner key must be a no-op, not a KeyError."""
    config.save_config({"js": {"other": "keep"}})
    config.save_js_runner("")  # would raise KeyError if pop lost its None default
    assert config.load_config()["js"] == {"other": "keep"}


def test_save_js_runner_clear_tolerates_missing_js_section(cfg_dir: Path) -> None:
    """Clearing the runner when there is no [js] section must not raise: the empty section drops
    through to `doc.pop("js", None)`, whose None default absorbs the absent key."""
    config.save_config({"language": "en"})
    config.save_js_runner("")  # would raise KeyError if doc.pop lost its None default
    doc = config.load_config()
    assert doc == {"language": "en"}
    assert "js" not in doc


def test_save_js_runner_preserves_sibling_js_keys(cfg_dir: Path) -> None:
    """Writing a runner must merge into the *existing* [js] table (read via doc.get("js")), so any
    other key already in that section survives rather than being dropped by a fresh {}."""
    config.save_config({"js": {"other": "keep"}})
    config.save_js_runner("deno")
    section = config.load_config()["js"]
    assert section["runner"] == "deno"
    assert section["other"] == "keep"  # dropped if the existing section wasn't read back
