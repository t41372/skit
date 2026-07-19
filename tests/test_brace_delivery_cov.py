"""Brace-escape delivery across the flows assembler and the live FieldRow preview, and
the Health screen's launch-blocked / malformed-runner rows.

The contract under test: a placeholder-source value travels byte-identical (`{{x}}` kept),
an inject/flag-source value halves the escape pair (`{{x}}` → `{x}`), and the form's live
preview shows exactly what delivery will produce. Plus: both health faces share one
collector, so the TUI screen renders the same launch_blocked / invalid-runner issues
doctor does.
"""

from __future__ import annotations

import contextlib
from pathlib import Path

import pytest
from textual.widgets import Input, OptionList, Static

from skit import config, flows, launcher, store, tui
from skit.langs.python import metawriter
from skit.params import ParamDecl
from skit.tui_form import FieldRow, RunFormScreen
from skit.tui_health import HealthScreen

CWD = Path("/work/dir")


@pytest.fixture(autouse=True)
def tmp_store(tmp_path, monkeypatch):
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path / "config"))
    monkeypatch.setenv("SKIT_LANG", "en")


@contextlib.contextmanager
def _noop_suspend():
    yield


@pytest.fixture
def quiet_run(monkeypatch):
    config.save_after_run("stay")
    monkeypatch.setattr(launcher, "run_entry", lambda *a, **k: 0)
    monkeypatch.setattr(tui.MenuApp, "suspend", lambda self: _noop_suspend())


# ---------------------------------------------------------------- flows.assemble delivery


def test_assemble_placeholder_keeps_braces_inject_halves():
    plan = flows.FormPlan(
        source="inject",
        fields=[
            flows.FormField(key="ph", label="ph", source="placeholder"),
            flows.FormField(key="inj", label="inj", source="inject"),
        ],
    )
    out = flows.assemble(plan, {"ph": "{{x}}", "inj": "{{x}}"}, [], cwd=CWD)
    assert out.command_values["ph"] == "{{x}}"  # placeholder delivery: byte-identical
    assert out.inject_values["inj"] == "{x}"  # inject delivery: escape pair halved


# ---------------------------------------------------------------- live FieldRow preview


async def test_fieldrow_placeholder_value_keeps_braces_no_preview_line(tmp_path, quiet_run):
    src = tmp_path / "p.prompt.md"
    src.write_text("Do {{loc}}\n", encoding="utf-8")
    store.add_prompt(src, name="p")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 34)) as pilot:
        app.action_run()
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, RunFormScreen)
        row = next(r for r in screen.query(FieldRow) if r.field.key == "loc")
        assert row.field.source == "placeholder"
        row.query_one(Input).value = "{{cwd}}"
        await pilot.pause()
        preview = row.query_one(".field-preview", Static)
        # Placeholder delivery keeps {{cwd}} byte-identical → the preview shows no lie.
        assert preview.display is False


async def test_fieldrow_inject_value_halves_braces_in_preview(tmp_path, quiet_run):
    text = metawriter.write_params(
        'CITY = "x"\nprint(CITY)\n', [ParamDecl(name="CITY", binding="const", type="str")]
    )
    p = tmp_path / "job.py"
    p.write_text(text, encoding="utf-8")
    store.add_python(p, name="job")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 34)) as pilot:
        app.action_run()
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, RunFormScreen)
        row = next(r for r in screen.query(FieldRow) if r.field.key == "CITY")
        assert row.field.source == "inject"
        row.query_one(Input).value = "{{cwd}}"
        await pilot.pause()
        preview = row.query_one(".field-preview", Static)
        assert preview.display is True
        assert "{cwd}" in str(preview.render())  # inject delivery halves the pair
        assert "{{cwd}}" not in str(preview.render())


# ---------------------------------------------------------------- Health screen: both faces


def _shell(tmp_path, name):
    p = tmp_path / f"{name}.sh"
    p.write_text("#!/usr/bin/env bash\necho hi\n", encoding="utf-8")
    return store.add_script(p, kind="shell", name=name)


async def test_health_screen_lists_blocked_and_malformed_runner_rows(tmp_path, monkeypatch):
    _shell(tmp_path, "blocked")
    config.save_config(
        {"prompt": {"runners_seeded": True, "runners": [{"name": "bad", "argv": ["no-hole"]}]}}
    )
    monkeypatch.setattr("skit.langs.launch._which", lambda _name: None)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(HealthScreen())
        await pilot.pause()
        screen = app.screen
        text = "\n".join(str(w.render()) for w in screen.query(Static))
        issues = screen.query_one("#hc-issues", OptionList)
        prompts = [str(issues.get_option_at_index(i).prompt) for i in range(issues.option_count)]
    # The blocked shell entry appears as an issue naming the refusal...
    assert any("blocked" in p and "refuse to start" in p for p in prompts)
    # ...and the malformed runner row gets its own warning line (the TUI face doctor's
    # invalid_prompt_runners feeds — same collector).
    assert "Malformed agent (runner) rows in config" in text
    assert "bad" in text
