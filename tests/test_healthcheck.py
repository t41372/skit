"""The one health pipeline both faces consume: healthcheck.collect / entry_drifted.

doctor and the TUI Health screen previously swept separately and disagreed. These tests
pin the collector's contract directly (every category populated, and the
double-report exclusions), so the two renderers can never drift apart again.
"""

from __future__ import annotations

import shutil
from pathlib import Path

import pytest

from skit import config, healthcheck, store
from skit.langs.python import metawriter
from skit.params import ParamDecl


@pytest.fixture(autouse=True)
def _isolated(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path / "config"))
    monkeypatch.setenv("SKIT_LANG", "en")


def _py(tmp_path: Path, body: str, name: str) -> store.Entry:
    p = tmp_path / f"{name}.py"
    p.write_text(body, encoding="utf-8")
    return store.add_python(p, name=name)


def _shell(tmp_path: Path, name: str) -> store.Entry:
    p = tmp_path / f"{name}.sh"
    p.write_text("#!/usr/bin/env bash\necho hi\n", encoding="utf-8")
    return store.add_script(p, kind="shell", name=name)


def _prompt(tmp_path: Path, text: str, name: str) -> store.Entry:
    p = tmp_path / f"{name}.prompt.md"
    p.write_text(text, encoding="utf-8")
    return store.add_prompt(p, name=name)


_DRIFTED = metawriter.write_params(
    "CITY = 'x'\nprint(CITY)\n",
    [
        ParamDecl(name="CITY", binding="const", type="str"),
        ParamDecl(name="GONE", binding="const", type="str"),
    ],
)


# ---------------------------------------------------------------- entry_drifted


def test_entry_drifted_true_for_managed_placeholder_gone_from_prompt(tmp_path):
    entry = _prompt(tmp_path, "Do {{a}} {{gone}}\n", "pr")
    assert entry.meta.params == ["a", "gone"]
    entry.script_path.write_text("Do {{a}}\n", encoding="utf-8")  # gone left the body
    assert healthcheck.entry_drifted(store.resolve("pr")) is True


def test_entry_drifted_false_when_prompt_body_unreadable(tmp_path, monkeypatch):
    # An unreadable body belongs to the target/preflight sweeps, not drift — the
    # read guard must swallow the OSError and report no drift.
    entry = _prompt(tmp_path, "Do {{a}} {{gone}}\n", "pr")
    target = entry.script_path
    real = Path.read_text

    def boom(self, *a, **k):
        if self == target:
            raise OSError("unreadable")
        return real(self, *a, **k)

    monkeypatch.setattr(Path, "read_text", boom)
    assert healthcheck.entry_drifted(store.resolve("pr")) is False


def test_entry_drifted_false_for_insertion_off_prompt(tmp_path):
    entry = _prompt(tmp_path, "Do {{a}} {{gone}}\n", "pr")
    store.write_prompt_interpolate(entry.slug, False)
    entry.script_path.write_text("Do {{a}}\n", encoding="utf-8")
    assert healthcheck.entry_drifted(store.resolve("pr")) is False  # nothing is filled


# ---------------------------------------------------------------- collect


def test_collect_reports_every_category_and_excludes_double_reports(tmp_path, monkeypatch):
    # (a) target-missing
    gone = _py(tmp_path, "print(1)\n", "gone")
    gone.script_path.unlink()
    # (b) drift: a python-managed entry AND a prompt whose managed placeholder left the body
    _py(tmp_path, _DRIFTED, "drift_py")
    drift_pr = _prompt(tmp_path, "Do {{a}} {{gone}}\n", "drift_pr")
    drift_pr.script_path.write_text("Do {{a}}\n", encoding="utf-8")
    # (c) needs_missing: a shell entry whose declared tool is off PATH
    _shell(tmp_path, "needs_sh")
    store.update_needs("needs_sh", ["ffmpeg"])
    # (d) launch_blocked: a shell entry whose interpreter binary is absent, and a prompt
    #     whose pinned runner binary is gone
    _shell(tmp_path, "blocked_sh")
    # A valid codex row (so the pin resolves) plus one malformed row → (e) invalid rows.
    config.save_config(
        {
            "prompt": {
                "runners_seeded": True,
                "runners": [
                    {"name": "codex", "argv": ["codex", "{{prompt}}"]},
                    {"name": "bad", "argv": ["no-hole-here"]},
                ],
            }
        }
    )
    blocked_pr = _prompt(tmp_path, "Do {{a}}\n", "blocked_pr")
    store.write_prompt_runner(blocked_pr.slug, "codex")

    # No interpreter binary, no runner binary, no declared tool resolves on PATH.
    monkeypatch.setattr("skit.langs.launch._which", lambda _name: None)
    monkeypatch.setattr(shutil, "which", lambda _name: None)

    report = healthcheck.collect(store.list_entries())

    names = lambda entries: {e.meta.name for e in entries}  # noqa: E731
    assert names(report.missing) == {"gone"}
    assert names(report.drifted) == {"drift_py", "drift_pr"}
    assert report.needs_missing.get("needs_sh") == ["ffmpeg"]
    # needs_entries carries the ENTRY object itself, not None (the TUI/doctor renders it)
    assert names(report.needs_entries) == {"needs_sh"}
    # launch_blocked names the two truly-blocked entries with a real reason...
    assert set(report.launch_blocked) == {"blocked_sh", "blocked_pr"}
    assert names(report.blocked_entries) == {"blocked_sh", "blocked_pr"}
    assert report.launch_blocked["blocked_sh"]  # non-empty reason string
    # ...and NEVER double-reports an entry already missing or needs-flagged.
    assert "gone" not in report.launch_blocked
    assert "needs_sh" not in report.launch_blocked
    # (e) the malformed runner row is surfaced, the valid one is not.
    assert report.invalid_runner_rows == ["bad"]


def test_collect_double_report_exclusion_continues_not_breaks(tmp_path, monkeypatch):
    # The preflight loop skips entries already reported above (missing/needs/no-spec) with
    # `continue` — just that one entry, so LATER entries are still swept. A `break` mutant
    # would abandon the whole rest of the list at the first excluded entry. Ordering makes
    # it observable: list_entries sorts by slug, so an excluded "aaa" precedes a blocked
    # "zzz" — `break` at aaa would leave zzz unreported.
    early = _py(tmp_path, "print(1)\n", "aaa_excluded")
    early.script_path.unlink()  # target-missing -> excluded via `continue`
    _shell(tmp_path, "zzz_blocked")  # sorts AFTER; interpreter absent -> should be blocked
    monkeypatch.setattr("skit.langs.launch._which", lambda _name: None)
    monkeypatch.setattr(shutil, "which", lambda _name: None)

    report = healthcheck.collect(store.list_entries())

    assert "aaa_excluded" in {e.meta.name for e in report.missing}
    # The later entry was still reached and reported — a `break` would have skipped it.
    assert "zzz_blocked" in report.launch_blocked
    assert "zzz_blocked" in {e.meta.name for e in report.blocked_entries}


def test_collect_clean_library_reports_nothing(tmp_path):
    _py(tmp_path, "print(1)\n", "ok")
    report = healthcheck.collect(store.list_entries())
    assert not report.missing
    assert not report.drifted
    assert not report.needs_missing
    assert not report.launch_blocked
    assert not report.invalid_runner_rows
