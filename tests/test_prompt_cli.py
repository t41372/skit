"""The prompt kind's CLI surfaces: add lanes, run resolution, params ops, show,
the `skit runner` tree, and doctor's prompt sweeps (docs/design/prompt.md)."""

from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path

import pytest
from typer.testing import CliRunner

from skit import argstate, cli, config, i18n, store
from skit.langs.prompt import analyzer as prompt_analyzer

runner = CliRunner()


_HELP_SURFACE_PROBE = """
import json

from typer.testing import CliRunner

from skit import cli

commands = {
    "main": ["--help"],
    "list": ["list", "--help"],
    "show": ["show", "--help"],
    "remove": ["remove", "--help"],
    "rename": ["rename", "--help"],
    "describe": ["describe", "--help"],
    "params": ["params", "--help"],
    "deps": ["deps", "--help"],
    "doctor": ["doctor", "--help"],
}
runner = CliRunner()
payload = {}
for name, args in commands.items():
    result = runner.invoke(cli.app, args)
    payload[name] = {"exit_code": result.exit_code, "output": result.output}
print(json.dumps(payload, ensure_ascii=False))
"""


def _help_surfaces_in_fresh_locale(locale):
    """Import cli.app after selecting the locale, exactly like the real console script.

    Typer freezes command help while cli.py is imported, so changing SKIT_LANG around an
    already-imported CliRunner cannot test localized help.  A child process is the product
    boundary and also keeps this assertion independent of the developer's saved language.
    """
    env = os.environ.copy()
    env["SKIT_LANG"] = locale
    completed = subprocess.run(
        [sys.executable, "-c", _HELP_SURFACE_PROBE],
        env=env,
        capture_output=True,
        text=True,
        check=False,
    )
    assert completed.returncode == 0, completed.stderr
    return json.loads(completed.stdout)


def _json(result):
    """The --json contract: stdout is EXACTLY one JSON document (SKILL.md's stable
    contract), so parse the WHOLE output — never slice from the first `{`, which would
    mask a human line leaking onto stdout. These own-op writes emit no stderr, so the
    mixed .output equals stdout here and parsing it whole is the strict purity check."""
    return json.loads(result.output)


def _write(tmp_path: Path, text: str, name: str = "p.prompt.md") -> Path:
    path = tmp_path / name
    path.write_text(text, encoding="utf-8")
    return path


@pytest.fixture
def spawn_spy(monkeypatch):
    """Intercept the actual process spawn (run_entry) and capture the launch kwargs."""
    calls: dict[str, object] = {}

    def fake(
        entry,
        extra_args=None,
        *,
        values=None,
        invoke_cwd=None,
        script_override=None,
        env_overlay=None,
        runner=None,
        prepared=None,
    ):
        calls["entry"] = entry
        calls["extra"] = list(extra_args or [])
        calls["values"] = dict(values or {})
        calls["runner"] = runner
        calls["prepared"] = prepared
        return calls.get("code", 0)

    monkeypatch.setattr(cli.launcher, "run_entry", fake)
    monkeypatch.setattr("skit.langs.launch._which", lambda name: f"/bin/{name}")
    return calls


# --------------------------------------------------------------------------
# add
# --------------------------------------------------------------------------


def test_add_prompt_read_oserror_is_a_clean_store_error(tmp_path, monkeypatch):
    """A present prompt that becomes unreadable during onboarding must not traceback."""
    source = _write(tmp_path, "Review this\n")

    def denied(_path):
        raise PermissionError(13, "permission denied", str(source))

    monkeypatch.setattr("skit.langs.prompt.text.read", denied)
    result = runner.invoke(cli.app, ["add", str(source), "--prompt", "--no-input"])

    assert result.exit_code == 1
    assert "Can't read" in result.output
    assert str(source) in result.output.replace("\n", "")
    assert "permission denied" in result.output
    assert store.list_entries() == []


def test_localized_starter_is_minimal_and_never_creates_its_own_field():
    try:
        for locale, title in (
            ("en", "# New prompt"),
            ("zh_CN", "# 新提示词"),
            ("zh_TW", "# 新提示詞"),
        ):
            i18n.init(locale)
            starter = cli._starter_prompt()
            assert starter == title + "\n\n"
            assert prompt_analyzer.placeholder_names(starter) == []
            # A normal partial edit may leave the harmless title in place, but only
            # the user's own token becomes a field.
            assert prompt_analyzer.placeholder_names(starter + "Review {{目标}}\n") == ["目标"]
    finally:
        i18n.init("en")


def test_add_prompt_file_no_input_manages_everything(tmp_path):
    src = _write(tmp_path, "# Review\n\nCheck {{target}} for {{focus}}\n")
    result = runner.invoke(cli.app, ["add", str(src), "--no-input"])
    assert result.exit_code == 0, result.output
    entry = store.resolve("p")
    assert entry.meta.kind == "prompt"
    assert entry.meta.params == ["target", "focus"]
    assert entry.meta.runner == ""
    assert "Managed parameters: target, focus" in result.output


def test_add_prompt_secret_summary_states_both_sides_of_boundary(tmp_path):
    src = _write(tmp_path, "Use {{api_key}}\n")
    result = runner.invoke(cli.app, ["add", str(src), "--no-input"])
    assert result.exit_code == 0, result.output
    assert "never saved by skit: api_key" in result.output
    assert "selected agent receives those values as plaintext" in result.output
    assert "may log or sync them" in result.output


def test_add_prompt_interactive_tick_subset_and_runner_pick(tmp_path):
    src = _write(tmp_path, "{{a}} {{b}} {{c}}\n")
    result = (
        runner.invoke(
            cli.app,
            ["add", str(src), "-n", "picky"],
            input="1,3\nclaude\n",
            env={"SKIT_FORCE_INTERACTIVE": "1"},
        )
        if False
        else runner.invoke(cli.app, ["add", str(src), "-n", "picky", "--no-input"])
    )
    # CliRunner has no TTY, so the interactive tick path is driven via monkeypatched
    # interactivity in the dedicated tests below; this pins the non-interactive default.
    assert result.exit_code == 0, result.output
    assert store.resolve("picky").meta.params == ["a", "b", "c"]


def test_add_prompt_interactive_selection(tmp_path, monkeypatch):
    config.save_form("plain")  # the line-prompt path (form=tui hosts the review panel)
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    answers = iter(["", "1,3", "-"])
    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(lambda *a, **k: next(answers)))
    src = _write(tmp_path, "{{a}} {{b}} {{c}}\n")
    result = runner.invoke(cli.app, ["add", str(src), "-n", "picky"])
    assert result.exit_code == 0, result.output
    entry = store.resolve("picky")
    assert entry.meta.params == ["a", "c"]
    assert entry.meta.runner == ""  # "-" = no pin
    assert argstate.load_last_runner() == ""  # skipping is not a pick


def test_add_prompt_plain_identity_defaults_drop_compound_suffix(tmp_path, monkeypatch):
    config.save_form("plain")
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)

    def accept_defaults(question, **kwargs):
        if "Run this prompt" in question:
            return "-"
        return kwargs.get("default", "")

    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(accept_defaults))
    src = _write(tmp_path, "# Review pull requests\n", name="review.prompt.md")
    result = runner.invoke(cli.app, ["add", str(src)])
    assert result.exit_code == 0, result.output
    entry = store.resolve("review")
    assert entry.meta.name == "review"
    assert entry.meta.description == "Review pull requests"


def test_add_prompt_plain_identity_accepts_user_overrides(tmp_path, monkeypatch):
    config.save_form("plain")
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    answers = iter(["pr-review", "Team review prompt", "-"])
    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(lambda *a, **k: next(answers)))
    src = _write(tmp_path, "Review this change\n", name="review.prompt.md")
    result = runner.invoke(cli.app, ["add", str(src)])
    assert result.exit_code == 0, result.output
    entry = store.resolve("pr-review")
    assert entry.meta.description == "Team review prompt"


def test_add_prompt_interactive_runner_pick_pins_and_remembers(tmp_path, monkeypatch):
    config.save_form("plain")  # the line-prompt path (form=tui hosts the review panel)
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    answers = iter(["", "all", "codex"])
    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(lambda *a, **k: next(answers)))
    src = _write(tmp_path, "{{a}}\n")
    result = runner.invoke(cli.app, ["add", str(src), "-n", "pinned"])
    assert result.exit_code == 0, result.output
    assert store.resolve("pinned").meta.runner == "codex"
    assert argstate.load_last_runner() == "codex"
    assert "Runs with codex" in result.output


def test_add_prompt_interactive_tui_form_opens_review_panel(tmp_path, monkeypatch):
    """In a real terminal with form=tui, `skit add x.prompt.md` hosts the SAME review
    panel the TUI's `a` opens for prompts — exact python-lane parity; the flags ride
    along as prefills and the panel's slug feeds the printed summary."""
    src = _write(tmp_path, "Do {{a}}\n")
    seen: dict[str, object] = {}

    def fake_panel(path, **kwargs):
        seen["path"] = path
        seen.update(kwargs)
        return store.add_prompt(path, name="panelled").slug

    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    monkeypatch.setattr("skit.tui_add.run_prompt_review", fake_panel)
    result = runner.invoke(
        cli.app, ["add", str(src), "--runner", "claude", "--no-interpolate", "--ref"]
    )
    assert result.exit_code == 0, result.output
    assert seen["runner"] == "claude"
    assert seen["interpolate"] is False
    assert seen["reference"] is True
    assert "panelled" in result.output  # the summary reflects the panel's entry


def test_add_prompt_interactive_panel_cancel_exits_130(tmp_path, monkeypatch):
    """Esc in the panel = the form-cancel contract: exit 130, nothing added."""
    src = _write(tmp_path, "Do {{a}}\n")
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    monkeypatch.setattr("skit.tui_add.run_prompt_review", lambda path, **kw: None)
    result = runner.invoke(cli.app, ["add", str(src)])
    assert result.exit_code == 130
    assert "Cancelled" in result.output
    assert store.list_entries() == []


def test_add_prompt_unknown_runner_refused_before_the_panel(tmp_path, monkeypatch):
    """An explicit --runner the panel can't honor is refused up front, never dropped —
    the same rule the line-prompt lane enforces."""
    src = _write(tmp_path, "Do {{a}}\n")
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    hit: dict[str, int] = {}
    monkeypatch.setattr(
        "skit.tui_add.run_prompt_review", lambda *a, **kw: hit.setdefault("panel", 1)
    )
    result = runner.invoke(cli.app, ["add", str(src), "--runner", "ghost"])
    assert result.exit_code == 2
    assert "Unknown runner" in result.output
    assert "panel" not in hit


def test_add_prompt_term_dumb_keeps_line_prompts(tmp_path, monkeypatch):
    """TERM=dumb can't host a Textual panel — same opt-out as the python lane."""
    src = _write(tmp_path, "Do {{a}}\n")
    monkeypatch.setenv("TERM", "dumb")
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    answers = iter(["", "all", "-"])  # description; manage everything; no runner pin
    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(lambda *a, **k: next(answers)))
    hit: dict[str, int] = {}
    monkeypatch.setattr(
        "skit.tui_add.run_prompt_review", lambda *a, **kw: hit.setdefault("panel", 1)
    )
    result = runner.invoke(cli.app, ["add", str(src), "-n", "dumbly"])
    assert result.exit_code == 0, result.output
    assert "panel" not in hit
    assert store.resolve("dumbly").meta.params == ["a"]


def test_add_prompt_missing_file_is_clean_on_the_panel_face(tmp_path, monkeypatch):
    """A typo'd path on the TUI-gate lane must be the python lane's clean StoreError,
    never a raw FileNotFoundError out of the panel's own file read."""
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    monkeypatch.setattr(
        "skit.tui_add.run_prompt_review",
        lambda *a, **kw: pytest.fail("the panel must not open for a missing file"),
    )
    result = runner.invoke(cli.app, ["add", str(tmp_path / "typo.prompt.md")])
    assert result.exit_code == 1
    assert "File not found" in result.output


def test_add_prompt_runner_flag_non_interactive(tmp_path):
    argstate.save_last_runner("opencode")
    src = _write(tmp_path, "{{a}}\n")
    result = runner.invoke(
        cli.app, ["add", str(src), "-n", "auto", "--runner", " claude ", "--no-input"]
    )
    assert result.exit_code == 0, result.output
    assert store.resolve("auto").meta.runner == "claude"
    assert argstate.load_last_runner() == "opencode"  # an add-time pin is not a picker choice


def test_add_prompt_unknown_runner_flag_is_usage_error(tmp_path):
    src = _write(tmp_path, "{{a}}\n")
    result = runner.invoke(cli.app, ["add", str(src), "-n", "x", "--runner", "ghost", "--no-input"])
    assert result.exit_code == 2
    assert "Unknown runner" in result.output


def test_add_runner_flag_without_prompt_is_refused(tmp_path):
    py = tmp_path / "s.py"
    py.write_text("print(1)\n")
    result = runner.invoke(cli.app, ["add", str(py), "--runner", "claude", "--no-input"])
    assert result.exit_code == 2
    assert "--runner only applies to prompt entries" in result.output


def test_add_prompt_conflicts_with_other_kind_flags(tmp_path):
    src = _write(tmp_path, "{{a}}\n")
    for flags in (["--exe"], ["--kind", "shell"], ["--edit"], ["--cmd", "echo {x}"]):
        result = runner.invoke(cli.app, ["add", str(src), "--prompt", *flags])
        assert result.exit_code == 2, flags
        assert "drop --edit/--exe/--kind/--cmd" in result.output


def test_add_prompt_flag_forces_the_kind_on_any_extension(tmp_path):
    src = tmp_path / "notes.txt"
    src.write_text("Do {{thing}}\n", encoding="utf-8")
    result = runner.invoke(cli.app, ["add", str(src), "--prompt", "--no-input"])
    assert result.exit_code == 0, result.output
    assert store.resolve("notes").meta.kind == "prompt"


def test_add_bare_md_no_input_requires_explicit_prompt(tmp_path):
    src = _write(tmp_path, "hello {{x}}\n", name="notes.md")
    result = runner.invoke(cli.app, ["add", str(src), "--no-input"])
    assert result.exit_code == 2
    assert "--prompt" in result.output


def test_missing_bare_md_is_refused_before_the_prompt_confirmation(tmp_path, monkeypatch):
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)

    def asked(*_args, **_kwargs):
        pytest.fail("a path that does not exist must never reach the prompt-kind question")

    monkeypatch.setattr(cli.Confirm, "ask", staticmethod(asked))
    missing = tmp_path / "missing.md"
    result = runner.invoke(cli.app, ["add", str(missing)])
    assert result.exit_code == 1
    assert "File not found:" in result.output
    assert "missing.md" in result.output


def test_executable_lane_preserves_the_existing_non_file_contract(tmp_path):
    directory = tmp_path / "tool-dir"
    directory.mkdir()
    result = runner.invoke(cli.app, ["add", str(directory), "--exe", "--no-input"])
    assert result.exit_code == 0, result.output
    entry = store.resolve("tool-dir")
    assert entry.meta.kind == "exe"
    assert entry.meta.source_hash == ""


def test_add_bare_md_interactive_ask_yes_and_no(tmp_path, monkeypatch):
    config.save_form("plain")  # the line-prompt path (form=tui hosts the review panel)
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    monkeypatch.setattr(cli.Confirm, "ask", staticmethod(lambda *a, **k: True))

    def accept_identity_defaults(question, **kwargs):
        return "-" if "Run this prompt" in question else kwargs.get("default", "")

    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(accept_identity_defaults))
    src = _write(tmp_path, "hello {{x}}\n", name="notes.md")
    result = runner.invoke(cli.app, ["add", str(src)])
    assert result.exit_code == 0, result.output
    assert store.resolve("notes").meta.kind == "prompt"

    monkeypatch.setattr(cli.Confirm, "ask", staticmethod(lambda *a, **k: False))
    other = _write(tmp_path, "x\n", name="other.md")
    result = runner.invoke(cli.app, ["add", str(other)])
    assert result.exit_code == 130
    assert "nothing was added" in result.output.lower()


def test_add_prompt_from_stdin_needs_a_name(tmp_path):
    result = runner.invoke(cli.app, ["add", "-", "--prompt"], input="body {{x}}\n")
    assert result.exit_code == 2
    assert "--name" in result.output


def test_add_prompt_from_stdin(tmp_path):
    result = runner.invoke(
        cli.app,
        ["add", "-", "--prompt", "-n", "clip", "--runner", "amp"],
        input="Summarize {{url}} briefly.\n",
    )
    assert result.exit_code == 0, result.output
    entry = store.resolve("clip")
    assert entry.meta.kind == "prompt"
    assert entry.meta.params == ["url"]
    assert entry.meta.runner == "amp"
    assert entry.script_path.read_text() == "Summarize {{url}} briefly.\n"


def test_add_kind_prompt_from_stdin_uses_the_prompt_contract(tmp_path):
    result = runner.invoke(
        cli.app,
        [
            "add",
            "-",
            "--kind",
            "prompt",
            "-n",
            "kind-clip",
            "--runner",
            "amp",
            "--no-interpolate",
        ],
        input="Keep {{url}} literal.\r\n",
    )

    assert result.exit_code == 0, result.output
    entry = store.resolve("kind-clip")
    assert entry.meta.kind == "prompt"
    assert entry.meta.runner == "amp"
    assert entry.meta.interpolate is False
    assert entry.meta.params is None
    assert entry.meta.workdir == "invoke"
    assert entry.script_path.read_bytes() == b"Keep {{url}} literal.\r\n"


def test_add_prompt_from_stdin_empty_body(tmp_path):
    result = runner.invoke(cli.app, ["add", "-", "--prompt", "-n", "e"], input="  \n")
    assert result.exit_code == 1
    assert "Nothing arrived on stdin" in result.output


def test_add_prompt_editor_lane_routes_to_stdin_when_not_interactive(tmp_path):
    # `skit add --prompt` with no path, no TTY: the body arrives on stdin.
    result = runner.invoke(cli.app, ["add", "--prompt", "-n", "drafted"], input="Draft {{a}}\n")
    assert result.exit_code == 0, result.output
    assert store.resolve("drafted").meta.params == ["a"]


def test_add_prompt_editor_lane_interactive(tmp_path, monkeypatch):
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    answers = iter(["all", "-"])  # manage everything; no runner pin
    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(lambda *a, **k: next(answers)))

    def fake_editor(path: Path) -> None:
        path.write_text("Edited body {{v}}\n", encoding="utf-8")

    monkeypatch.setattr(cli.editor, "open_in_editor", fake_editor)
    result = runner.invoke(cli.app, ["add", "--prompt", "-n", "note"])
    assert result.exit_code == 0, result.output
    entry = store.resolve("note")
    assert entry.script_path.read_text() == "Edited body {{v}}\n"
    assert entry.meta.params == ["v"]


def test_add_prompt_editor_lane_untouched_starter_adds_nothing(tmp_path, monkeypatch):
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    monkeypatch.setattr(cli.editor, "open_in_editor", lambda path: None)
    result = runner.invoke(cli.app, ["add", "--prompt", "-n", "empty"])
    assert result.exit_code == 0
    assert "Nothing was written" in result.output
    assert not store.list_entries()


def test_add_prompt_editor_lane_asks_for_a_name(tmp_path, monkeypatch):
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(lambda *a, **k: ""))
    result = runner.invoke(cli.app, ["add", "--prompt"])
    assert result.exit_code == 2
    assert "A name is required" in result.output


def _never(*_a, **_k):
    raise AssertionError("the editor must not be launched here")


def test_add_prompt_editor_lane_name_taken_refuses_before_the_editor(tmp_path, monkeypatch):
    """The prompt editor lane checks the name conflict BEFORE $EDITOR opens (the same
    draft-preservation rule as the script lane) — the editor is never launched."""
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    _write(tmp_path, "existing {{a}}\n", "e.prompt.md")
    store.add_prompt(tmp_path / "e.prompt.md", name="taken")
    monkeypatch.setattr(cli.editor, "open_in_editor", _never)  # must NOT be launched
    result = runner.invoke(cli.app, ["add", "--prompt", "-n", "taken"])
    assert result.exit_code == 1
    assert "already taken" in result.output
    # _never raises AssertionError if the editor opens — a clean SystemExit proves refusal
    # happened before the editor.
    assert not isinstance(result.exception, AssertionError)


def test_add_prompt_editor_lane_post_edit_failure_keeps_the_draft(tmp_path, monkeypatch):
    """A failure AFTER the prompt edit keeps the temp draft (the user's only copy) and names
    where it lives — nothing is added."""
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    seen: dict[str, Path] = {}

    def fake_editor(path: Path) -> None:
        seen["path"] = path
        path.write_text("Drafted prompt {{v}}\n", encoding="utf-8")

    def onboard_boom(*_a, **_k):
        raise store.StoreError("disk full")

    monkeypatch.setattr(cli.editor, "open_in_editor", fake_editor)
    monkeypatch.setattr(cli, "_onboard_prompt", onboard_boom)
    result = runner.invoke(cli.app, ["add", "--prompt", "-n", "keptprompt"])
    try:
        assert result.exit_code == 1
        assert "Your draft was kept at" in result.output
        assert seen["path"].exists()  # the draft survived the failure
        assert store.list_entries() == []  # nothing added
    finally:
        seen["path"].unlink(missing_ok=True)


def test_add_prompt_editor_lane_deleted_draft_is_a_clean_honest_failure(tmp_path, monkeypatch):
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    seen: dict[str, Path] = {}

    def delete(path: Path):
        seen["path"] = path
        path.unlink()

    monkeypatch.setattr(cli.editor, "open_in_editor", delete)
    result = runner.invoke(cli.app, ["add", "--prompt", "-n", "gone"])
    assert result.exit_code == 1
    assert "Can't read" in result.output
    assert "The draft is no longer at" in result.output
    assert "Your draft was kept at" not in result.output
    assert not seen["path"].exists()


def test_add_prompt_ref_mode_keeps_original_and_pins_invoke(tmp_path):
    src = _write(tmp_path, "Ref {{x}}\n")
    result = runner.invoke(cli.app, ["add", str(src), "--ref", "--no-input"])
    assert result.exit_code == 0, result.output
    entry = store.resolve("p")
    assert entry.meta.mode == "reference"
    assert entry.meta.workdir == "invoke"
    assert entry.script_path == src


def test_add_prompt_no_path_with_ref_is_refused(tmp_path):
    result = runner.invoke(cli.app, ["add", "--prompt", "--ref", "-n", "x"], input="b\n")
    assert result.exit_code == 2


# --------------------------------------------------------------------------
# run
# --------------------------------------------------------------------------


def _added(tmp_path, text="Do {{a}}\n", name="p", pin=""):
    entry = store.add_prompt(_write(tmp_path, text, name=f"{name}.prompt.md"), name=name)
    if pin:
        entry = store.write_prompt_runner(entry.slug, pin)
    return entry


@pytest.mark.parametrize(
    ("locale", "expected"),
    [
        (
            "en",
            {
                "main": "scripts, prompts, programs, and commands",
                "list": "registered entry",
                "show": "one entry",
                "remove": "registered entry",
                "rename": "Rename an entry",
                "describe": "entry's description",
                "params": "an entry's managed or declared parameters",
                "deps": "an entry's package dependencies",
                "doctor": "entry library",
            },
        ),
        (
            "zh-TW",
            {
                "main": "腳本、提示詞、程式和命令",
                "list": "已登記的條目",
                "show": "一個條目",
                "remove": "已登記的條目",
                "rename": "重新命名條目",
                "describe": "條目的說明",
                "params": "條目的納管參數或宣告參數",
                "deps": "條目的套件依賴",
                "doctor": "工具庫",
            },
        ),
    ],
)
def test_umbrella_cli_help_uses_entry_taxonomy_in_the_requested_locale(locale, expected):
    surfaces = _help_surfaces_in_fresh_locale(locale)
    for command, phrase in expected.items():
        result = surfaces[command]
        assert result["exit_code"] == 0, result["output"]
        assert phrase in " ".join(result["output"].split())


def test_prompt_only_library_uses_entry_taxonomy_on_dynamic_cli_surfaces(tmp_path, monkeypatch):
    _added(tmp_path, text="Review this\n")
    monkeypatch.setattr(cli.launcher, "find_uv", lambda: "/bin/uv")

    result = runner.invoke(cli.app, ["doctor"])
    assert result.exit_code == 0, result.output
    assert "1 entry registered" in result.output
    assert "script registered" not in result.output


def test_empty_library_does_not_claim_it_only_accepts_scripts():
    result = runner.invoke(cli.app, ["list"])
    assert result.exit_code == 0, result.output
    assert "No entries yet" in result.output
    assert "No scripts yet" not in result.output


def test_run_prompt_no_input_without_pin_is_126(tmp_path):
    _added(tmp_path)
    result = runner.invoke(cli.app, ["run", "p", "--set", "a=1", "--no-input"])
    assert result.exit_code == 126
    assert "No runner selected" in result.output


def test_run_no_input_is_provably_unaffected_by_last_picked_state(tmp_path):
    # The last-picked name is a PICKER DEFAULT only (design risk #10): a --no-input run
    # with no pin must still refuse with 126, whatever state remembers.
    _added(tmp_path)
    argstate.save_last_runner("claude")
    result = runner.invoke(cli.app, ["run", "p", "--set", "a=1", "--no-input"])
    assert result.exit_code == 126
    assert "No runner selected" in result.output


def test_run_prompt_runner_flag_threads_through(tmp_path, spawn_spy):
    _added(tmp_path)
    result = runner.invoke(
        cli.app, ["run", "p", "--runner", " claude ", "--set", "a=1", "--no-input"]
    )
    assert result.exit_code == 0, result.output
    assert spawn_spy["runner"] == config.find_prompt_runner("claude")
    assert spawn_spy["values"] == {"a": "1"}
    assert argstate.load_last_runner() == "claude"  # --runner is a pick


def test_run_prompt_unicode_placeholder_threads_through_set(tmp_path, spawn_spy):
    _added(tmp_path, text="审查 {{目标}}\n")
    result = runner.invoke(
        cli.app, ["run", "p", "--runner", "claude", "--set", "目标=src/app.py", "--no-input"]
    )
    assert result.exit_code == 0, result.output
    assert spawn_spy["values"] == {"目标": "src/app.py"}


def test_run_prompt_pin_resolves_without_touching_last_picked(tmp_path, spawn_spy):
    _added(tmp_path, pin="codex")
    result = runner.invoke(cli.app, ["run", "p", "--set", "a=1", "--no-input"])
    assert result.exit_code == 0, result.output
    assert spawn_spy["runner"] == config.find_prompt_runner("codex")
    assert argstate.load_last_runner() == ""  # using a pin is not a pick


def test_run_prompt_unknown_runner_is_126_listing_names(tmp_path):
    _added(tmp_path)
    result = runner.invoke(cli.app, ["run", "p", "--runner", "ghost", "--set", "a=1", "--no-input"])
    assert result.exit_code == 126
    assert "ghost" in result.output
    assert "claude" in result.output


def test_run_prompt_pinned_but_removed_runner_is_126(tmp_path):
    _added(tmp_path, pin="mine")
    config.save_prompt_runners([])  # the pin's row is gone
    result = runner.invoke(cli.app, ["run", "p", "--set", "a=1", "--no-input"])
    assert result.exit_code == 126
    assert "mine" in result.output


def test_run_unpinned_prompt_with_empty_runner_list_teaches_a_copyable_recovery(tmp_path):
    _added(tmp_path)
    config.save_prompt_runners([])
    result = runner.invoke(cli.app, ["run", "p", "--set", "a=1", "--no-input"])
    assert result.exit_code == 126
    assert "No agents are configured" in result.output
    assert "skit runner add mycli -- mycli run {{prompt}}" in " ".join(result.output.split())


def test_run_runner_flag_on_non_prompt_is_usage_error(tmp_path):
    store.add_command("echo {m}", name="cmd")
    result = runner.invoke(
        cli.app, ["run", "cmd", "--runner", "claude", "--set", "m=1", "--no-input"]
    )
    assert result.exit_code == 2
    assert "--runner only applies to prompt entries" in result.output


def test_run_prompt_interactive_ask_prefilled_from_last_picked(tmp_path, spawn_spy, monkeypatch):
    _added(tmp_path)
    argstate.save_last_runner("opencode")
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    seen: dict[str, object] = {}

    def fake_ask(*a, **k):
        seen["default"] = k.get("default")
        return "amp"

    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(fake_ask))
    result = runner.invoke(cli.app, ["run", "p", "--set", "a=1", "--plain"])
    assert result.exit_code == 0, result.output
    assert seen["default"] == "opencode"
    assert spawn_spy["runner"] == config.find_prompt_runner("amp")
    assert argstate.load_last_runner() == "amp"
    assert "amp -x runs this prompt once" in result.output


def test_run_prompt_inline_stale_pin_prefills_last_configured_pick(
    tmp_path, spawn_spy, monkeypatch
):
    _added(tmp_path, pin="removed")
    config.save_prompt_runners(
        [
            config.PromptRunner("first", ("first", "{{prompt}}")),
            config.PromptRunner("remembered", ("remembered", "{{prompt}}")),
        ]
    )
    argstate.save_last_runner("remembered")
    config.save_form("tui")
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    seen: dict[str, object] = {}

    def fake_collect(entry, plan, prefill, runners=None, runner_default=""):
        seen["default"] = runner_default
        return {"a": "1"}, "remembered", False

    monkeypatch.setattr("skit.inlineform.collect", fake_collect)
    result = runner.invoke(cli.app, ["run", "p"])
    assert result.exit_code == 0, result.output
    assert seen["default"] == "remembered"
    assert spawn_spy["runner"] == config.find_prompt_runner("remembered")


def test_run_prompt_dry_run_prints_the_resolved_argv(tmp_path):
    _added(tmp_path, text="Say {{a}}!\n")
    result = runner.invoke(
        cli.app,
        ["run", "p", "--runner", "claude", "--set", "a=hello world", "--no-input", "--dry-run"],
    )
    assert result.exit_code == 0, result.output
    assert "claude" in result.output
    assert "hello world" in result.output


def test_run_prompt_dry_run_missing_body_is_127_before_output(tmp_path):
    entry = _added(tmp_path, text="Say it\n", pin="claude")
    entry.script_path.unlink()
    result = runner.invoke(cli.app, ["run", "p", "--no-input", "--dry-run"])
    assert result.exit_code == 127
    assert "doesn't exist" in result.output
    assert "→" not in result.output


def test_normal_prompt_transparency_omits_body_but_keeps_agent_flags(tmp_path, spawn_spy):
    body = "PRIVATE-DOCUMENT-START\n" + ("detail " * 2_000) + "{{a}}\n"
    _added(tmp_path, text=body, pin="claude")
    result = runner.invoke(
        cli.app,
        ["run", "p", "--set", "a=done", "--no-input", "--", "--model", "opus"],
    )
    assert result.exit_code == 0, result.output
    assert "PRIVATE-DOCUMENT-START" not in result.output
    assert "rendered prompt omitted" in result.output
    assert "claude" in result.output
    assert "--model" in result.output
    assert "opus" in result.output
    assert spawn_spy["values"] == {"a": "done"}


def test_overlong_prompt_refuses_before_normal_transparency(tmp_path, spawn_spy):
    from skit.langs.prompt import render

    marker = "MUST-NOT-REACH-SCROLLBACK"
    _added(tmp_path, text=marker + ("x" * (render.ARGV_LIMIT + 1)), pin="claude")
    result = runner.invoke(cli.app, ["run", "p", "--no-input"])
    assert result.exit_code == 125
    assert marker not in result.output
    assert "over this platform" in result.output
    assert "entry" not in spawn_spy


def test_dry_run_refuses_nul_without_looking_up_agent_binary(tmp_path, monkeypatch):
    from skit.langs import launch as langs_launch

    _added(tmp_path, text="before\x00after", pin="claude")
    monkeypatch.setattr(
        langs_launch,
        "_which",
        lambda _name: pytest.fail("dry-run must not look up the runner on PATH"),
    )
    result = runner.invoke(cli.app, ["run", "p", "--no-input", "--dry-run"])
    assert result.exit_code == 125
    assert "NUL byte" in result.output


def test_dry_run_refuses_overlong_prompt_without_printing_it(tmp_path):
    from skit.langs.prompt import render

    marker = "DRY-RUN-TOO-LONG"
    _added(tmp_path, text=marker + ("x" * (render.ARGV_LIMIT + 1)), pin="claude")
    result = runner.invoke(cli.app, ["run", "p", "--no-input", "--dry-run"])
    assert result.exit_code == 125
    assert marker not in result.output
    assert "over this platform" in result.output


def test_dry_run_prints_the_same_prompt_snapshot_it_validated(tmp_path, monkeypatch):
    from skit.langs import launch as langs_launch
    from skit.langs.prompt import render

    _added(tmp_path, text="stored body\n", pin="claude")
    monkeypatch.setattr(render, "ARGV_LIMIT", 64)
    marker = "UNVALIDATED-SECOND-SNAPSHOT"
    bodies = iter(["VALIDATED-SNAPSHOT", marker + ("x" * (render.ARGV_LIMIT + 1))])
    reads = 0

    def changing_body(_path):
        nonlocal reads
        reads += 1
        return next(bodies)

    monkeypatch.setattr(langs_launch.PromptLaunch, "_read_body", staticmethod(changing_body))
    result = runner.invoke(cli.app, ["run", "p", "--no-input", "--dry-run"])

    assert result.exit_code == 0, result.output
    assert "VALIDATED-SNAPSHOT" in result.output
    assert marker not in result.output
    assert reads == 1


def test_run_prompt_extra_args_pass_through_after_dashes(tmp_path, spawn_spy):
    _added(tmp_path, pin="claude")
    result = runner.invoke(
        cli.app,
        ["run", "p", "--set", "a=1", "--no-input", "--", "--model", "opus"],
    )
    assert result.exit_code == 0, result.output
    assert spawn_spy["extra"] == ["--model", "opus"]


def test_prompt_extra_agent_args_do_not_fill_required_placeholders(tmp_path, spawn_spy):
    _added(tmp_path, pin="claude")
    result = runner.invoke(cli.app, ["run", "p", "--no-input", "--", "--model", "opus"])
    assert result.exit_code == 125
    assert "a is required" in result.output
    assert "entry" not in spawn_spy


def test_extra_argv_does_not_hide_a_filled_flag_type_error(tmp_path, spawn_spy):
    source = tmp_path / "count.py"
    source.write_text(
        "import argparse\n"
        "p = argparse.ArgumentParser()\n"
        "p.add_argument('--count', type=int, required=True)\n"
        "p.parse_args()\n",
        encoding="utf-8",
    )
    store.add_python(source, name="count")
    result = runner.invoke(
        cli.app,
        ["run", "count", "--set", "count=nope", "--no-input", "--", "--count", "2"],
    )
    assert result.exit_code == 125
    assert "whole number" in result.output
    assert "entry" not in spawn_spy


def test_run_prompt_secret_placeholder_masked_in_dry_run(tmp_path):
    _added(tmp_path, text="Use {{api_key}}\n", name="sec")
    result = runner.invoke(
        cli.app,
        ["run", "sec", "--runner", "claude", "--set", "api_key=hunter2", "--no-input", "--dry-run"],
    )
    assert result.exit_code == 0, result.output
    assert "hunter2" not in result.output
    assert "•••" in result.output
    assert "receives" not in result.output  # dry-run sends nothing


def test_real_prompt_run_warns_before_sending_a_nonempty_secret(tmp_path, spawn_spy):
    _added(tmp_path, text="Use {{api_key}}\n", name="sec", pin="claude")
    result = runner.invoke(cli.app, ["run", "sec", "--set", "api_key=hunter2", "--no-input"])
    assert result.exit_code == 0, result.output
    assert "never saved by skit" in result.output
    assert "selected agent as plaintext" in result.output
    assert "may log or sync" in result.output
    assert "hunter2" not in result.output
    assert spawn_spy["values"] == {"api_key": "hunter2"}


def test_missing_runner_binary_refuses_before_any_delivery_output(tmp_path, monkeypatch):
    from skit.langs import launch as langs_launch

    missing = config.PromptRunner("missing", ("definitely-not-installed", "{{prompt}}"))
    config.save_prompt_runners([missing])
    _added(tmp_path, text="Use {{api_key}}\n", name="sec", pin="missing")
    monkeypatch.setattr(langs_launch, "_which", lambda _name: None)

    result = runner.invoke(
        cli.app,
        ["run", "sec", "--set", "api_key=hunter2", "--no-input"],
    )

    assert result.exit_code == 126
    assert "definitely-not-installed" in result.output
    assert "selected agent as plaintext" not in result.output
    assert "→" not in result.output
    assert "hunter2" not in result.output


def test_real_run_spawns_the_same_prompt_snapshot_it_validated(tmp_path, monkeypatch, spawn_spy):
    from skit.langs.base import ArgvLaunch

    _added(tmp_path, text="stored body\n", pin="claude")
    bodies = iter(["ONE-PREPARED-BODY", "SECOND-BODY-MUST-NOT-BE-READ"])
    reads = 0

    def changing_body(_path):
        nonlocal reads
        reads += 1
        return next(bodies)

    monkeypatch.setattr("skit.langs.launch.PromptLaunch._read_body", staticmethod(changing_body))
    result = runner.invoke(cli.app, ["run", "p", "--no-input"])

    assert result.exit_code == 0, result.output
    assert reads == 1
    prepared = spawn_spy["prepared"]
    assert isinstance(prepared, cli.launcher.PreparedLaunch)
    assert isinstance(prepared.payload, ArgvLaunch)
    assert "ONE-PREPARED-BODY" in prepared.payload.argv
    assert all("SECOND-BODY" not in token for token in prepared.payload.argv)


def test_real_run_transparency_and_amp_note_use_the_prepared_runner_row(
    tmp_path, monkeypatch, spawn_spy
):
    amp_seed = next(row for row in config.PROMPT_RUNNER_SEEDS if row.name == "amp")
    config.save_prompt_runners([amp_seed])
    _added(tmp_path, text="Do it\n", pin="amp")
    real_prepare = cli.launcher.prepare_entry

    def prepare_then_replace_runner(*args, **kwargs):
        prepared = real_prepare(*args, **kwargs)
        config.save_prompt_runners(
            [config.PromptRunner("amp", ("replacement-agent", "{{prompt}}"))]
        )
        return prepared

    monkeypatch.setattr(cli.launcher, "prepare_entry", prepare_then_replace_runner)
    result = runner.invoke(cli.app, ["run", "p", "--no-input"])

    assert result.exit_code == 0, result.output
    assert "amp -x runs this prompt once" in result.output
    assert "amp -x" in result.output
    assert "replacement-agent" not in result.output
    prepared = spawn_spy["prepared"]
    assert isinstance(prepared, cli.launcher.PreparedLaunch)
    assert prepared.prompt_runner == amp_seed


# --------------------------------------------------------------------------
# params
# --------------------------------------------------------------------------


def test_params_read_view_shows_unmanaged_and_gone(tmp_path):
    entry = _added(tmp_path, text="{{a}} {{b}}\n")
    store.write_prompt_managed(entry.slug, ["a"])
    entry.script_path.write_text("{{b}} {{c}} only\n", encoding="utf-8")
    result = runner.invoke(cli.app, ["params", "p"])
    assert result.exit_code == 0, result.output
    assert "Prompt placeholders" in result.output
    assert "Detected but not yet managed: b, c" in result.output
    assert "No longer in the prompt" in result.output
    assert "a" in result.output


def test_params_json_carries_runner_and_unmanaged(tmp_path):
    entry = _added(tmp_path, text="{{a}} {{b}}\n", pin="claude")
    store.write_prompt_managed(entry.slug, ["a"])
    result = runner.invoke(cli.app, ["params", "p", "--json"])
    assert result.exit_code == 0, result.output
    payload = json.loads(result.output)
    assert payload["placeholders"] == ["a"]
    assert payload["unmanaged"] == ["b"]
    assert payload["runner"] == "claude"


def test_params_add_manages_a_body_placeholder(tmp_path):
    entry = _added(tmp_path, text="{{a}} {{b}}\n")
    store.write_prompt_managed(entry.slug, ["a"])
    result = runner.invoke(cli.app, ["params", "p", "--add", "b"])
    assert result.exit_code == 0, result.output
    reloaded = store.resolve("p")
    assert reloaded.meta.params == ["a", "b"]  # body order
    rows = store.read_parameters("p")
    assert [(d.name, d.delivery) for d in rows] == [("b", "placeholder")]


def test_params_rm_unmanages_even_without_a_declared_row(tmp_path):
    _added(tmp_path, text="{{a}} {{b}}\n")
    result = runner.invoke(cli.app, ["params", "p", "--rm", "b"])
    assert result.exit_code == 0, result.output
    assert store.resolve("p").meta.params == ["a"]
    assert "not-declared" not in result.output
    assert "isn't declared" not in result.output


def test_params_add_unknown_name_becomes_env_rider(tmp_path):
    _added(tmp_path, text="{{a}}\n")
    result = runner.invoke(cli.app, ["params", "p", "--add", "EXTRA"])
    assert result.exit_code == 0, result.output
    assert store.resolve("p").meta.params == ["a"]  # not a body hole — not managed
    assert [d.delivery for d in store.read_parameters("p")] == ["env"]


def test_params_deliver_placeholder_is_allowed_on_prompts(tmp_path):
    entry = _added(tmp_path, text="{{a}}\n")
    store.write_parameters(entry.slug, [ParamDeclFactory("a")])
    result = runner.invoke(cli.app, ["params", "p", "--deliver", "a=placeholder"])
    assert result.exit_code == 0, result.output
    assert store.read_parameters("p")[0].delivery == "placeholder"


def ParamDeclFactory(name: str):
    from skit.params import ParamDecl

    return ParamDecl(name=name, delivery="env")


def test_params_runner_pin_and_clear(tmp_path):
    _added(tmp_path)
    argstate.save_last_runner("opencode")
    result = runner.invoke(cli.app, ["params", "p", "--runner", "claude"])
    assert result.exit_code == 0, result.output
    assert store.resolve("p").meta.runner == "claude"
    assert argstate.load_last_runner() == "opencode"  # a settings pin is not a run pick
    result = runner.invoke(cli.app, ["params", "p", "--runner", ""])
    assert result.exit_code == 0, result.output
    assert store.resolve("p").meta.runner == ""
    assert argstate.load_last_runner() == "opencode"
    assert "asks at run time" in result.output


def test_params_runner_pin_with_json_emits_the_read_view(tmp_path):
    """An own-op with --json emits the final read view instead of silently dropping the
    flag (the deps-write precedent): the human confirmation prints, THEN the read-view
    JSON — every own-op branch of `skit params`, not just the schema edits it first fixed."""
    _added(tmp_path)
    result = runner.invoke(cli.app, ["params", "p", "--runner", "claude", "--json"])
    assert result.exit_code == 0, result.output
    assert store.resolve("p").meta.runner == "claude"  # the pin was written
    payload = _json(result)  # stdout is exactly one JSON document
    assert payload["runner"] == "claude"  # the pin shows in the emitted read view


def test_params_workdir_with_json_emits_the_read_view(tmp_path):
    """The workdir/interpreter/template own-op honors --json the same way: the policy is
    written, then a valid read-view JSON is emitted on stdout — not a silent drop."""
    _added(tmp_path)
    result = runner.invoke(cli.app, ["params", "p", "--workdir", "origin", "--json"])
    assert result.exit_code == 0, result.output
    assert store.resolve("p").meta.workdir == "origin"  # the policy was written
    payload = _json(result)  # stdout is exactly one JSON document
    assert "params" in payload  # the entry's read view
    assert payload["runner"] is None  # unpinned prompt renders a null runner


def test_params_interpolate_with_json_emits_the_read_view(tmp_path):
    """The interpolate own-op honors --json too — the fourth own-op face of the rule."""
    _added(tmp_path)
    result = runner.invoke(cli.app, ["params", "p", "--no-interpolate", "--json"])
    assert result.exit_code == 0, result.output
    assert store.resolve("p").meta.interpolate is False  # the switch flipped
    payload = _json(result)  # stdout is exactly one JSON document
    assert payload["interpolate"] is False  # the new state shows in the read view


def test_params_runner_pin_validates_the_name(tmp_path):
    _added(tmp_path)
    result = runner.invoke(cli.app, ["params", "p", "--runner", "ghost"])
    assert result.exit_code == 1
    assert "isn't configured" in result.output
    assert store.resolve("p").meta.runner == ""


def test_params_runner_pin_refused_on_non_prompt(tmp_path):
    store.add_command("echo {m}", name="cmd")
    result = runner.invoke(cli.app, ["params", "cmd", "--runner", "claude"])
    assert result.exit_code == 1
    assert "--runner only applies to prompt entries" in result.output


# --------------------------------------------------------------------------
# show
# --------------------------------------------------------------------------


def test_show_json_prompt_additions(tmp_path):
    _added(tmp_path, pin="claude")
    result = runner.invoke(cli.app, ["show", "p", "--json"])
    payload = json.loads(result.output)
    assert payload["kind"] == "prompt"
    assert payload["runner"] == "claude"
    assert "claude" in payload["runners_available"]
    assert payload["workdir"] == "invoke"
    assert [f["key"] for f in payload["fields"]] == ["a"]
    assert [f["source"] for f in payload["fields"]] == ["placeholder"]


def test_show_json_non_prompt_has_no_runner_keys(tmp_path):
    store.add_command("echo {m}", name="cmd")
    payload = json.loads(runner.invoke(cli.app, ["show", "cmd", "--json"]).output)
    assert "runner" not in payload
    assert "runners_available" not in payload


def test_show_human_prints_the_runner_line(tmp_path):
    _added(tmp_path, pin="claude")
    result = runner.invoke(cli.app, ["show", "p"])
    assert "Runner: claude" in result.output
    store.write_prompt_runner("p", "")
    result = runner.invoke(cli.app, ["show", "p"])
    assert "asks at run time" in result.output


def test_show_human_no_fields_names_prompt_and_command_receivers(tmp_path):
    _added(tmp_path, text="No fields\n", name="plain")
    prompt_view = runner.invoke(cli.app, ["show", "plain"])
    assert "arguments after -- go to the selected agent" in prompt_view.output
    assert "pass straight through to the script" not in prompt_view.output

    store.add_command("echo ready", name="cmd")
    command_view = runner.invoke(cli.app, ["show", "cmd"])
    assert "arguments after -- are appended to the command" in command_view.output


# --------------------------------------------------------------------------
# skit runner …
# --------------------------------------------------------------------------


def test_runner_list_materializes_the_seeds(tmp_path):
    assert not config.prompt_runners_seeded()
    result = runner.invoke(cli.app, ["runner", "list"])
    assert result.exit_code == 0, result.output
    assert config.prompt_runners_seeded()  # first management need seeded the config
    for name in ("claude", "codex", "opencode", "amp", "antigravity"):
        assert name in result.output
    assert "amp -x" in result.output
    assert "does not open an interactive session" in " ".join(result.output.split())


def test_runner_list_json(tmp_path):
    payload = json.loads(runner.invoke(cli.app, ["runner", "list", "--json"]).output)
    assert {"name": "claude", "argv": ["claude", "--", "{{prompt}}"]} in payload
    assert {"name": "opencode", "argv": ["opencode", "--prompt={{prompt}}"]} in payload


def test_runner_list_all_json_exposes_stable_raw_indexes_and_reasons(tmp_path):
    config.save_config(
        {
            "prompt": {
                "runners_seeded": True,
                "runners": [
                    {"name": "good", "argv": ["good", "{{prompt}}"]},
                    {"name": "broken", "argv": ["broken"]},
                    "not-a-table",
                ],
            }
        }
    )
    result = runner.invoke(cli.app, ["runner", "list", "--all", "--json"])
    assert result.exit_code == 0, result.output
    payload = json.loads(result.output)
    assert payload == [
        {
            "row": 0,
            "name": "good",
            "argv": ["good", "{{prompt}}"],
            "reason": None,
            "descriptor": "good",
            "valid": True,
        },
        {
            "row": 1,
            "name": "broken",
            "argv": ["broken"],
            "reason": "prompt-slot-count",
            "descriptor": "broken",
            "valid": False,
        },
        {
            "row": 2,
            "name": None,
            "argv": None,
            "reason": "row-not-table",
            "descriptor": "not-a-table",
            "valid": False,
        },
    ]


def test_runner_list_all_preserves_anonymous_argv_and_localizes_human_status(tmp_path, monkeypatch):
    command = ["valuable-agent", "--model", "x", "{{prompt}}"]
    config.save_config(
        {
            "prompt": {
                "runners_seeded": True,
                "runners": [
                    {"name": "   ", "argv": command},
                    {"name": "broken", "argv": ["broken"]},
                ],
            }
        }
    )
    payload = json.loads(runner.invoke(cli.app, ["runner", "list", "--all", "--json"]).output)
    assert payload[0]["name"] is None
    assert payload[0]["argv"] == command
    assert payload[0]["reason"] == "name"  # JSON keeps the stable machine code

    monkeypatch.setattr(config, "gettext", lambda message: f"XX{message}XX")
    human = runner.invoke(cli.app, ["runner", "list", "--all"])
    flat = " ".join(human.output.split())
    assert "valuable-agent" in flat
    assert "--model x" in flat
    assert "'{{prompt}}'" in flat
    assert "XXA name is" in flat
    assert "required.XX" in flat
    assert "XXThe command needs" in flat
    assert "prompt lands.XX" in flat
    assert "prompt-slot-count" not in flat


def test_runner_list_empty_state(tmp_path):
    config.save_prompt_runners([])
    result = runner.invoke(cli.app, ["runner", "list"])
    assert result.exit_code == 0
    assert "No agents are configured" in result.output
    assert "skit runner add mycli -- mycli run {{prompt}}" in " ".join(result.output.split())
    all_rows = runner.invoke(cli.app, ["runner", "list", "--all"])
    assert all_rows.exit_code == 0
    assert "No agents are configured" in all_rows.output


def test_runner_list_without_amp_omits_the_one_shot_note(tmp_path):
    config.save_prompt_runners([config.PromptRunner("mycli", ("mycli", "run", "{{prompt}}"))])
    result = runner.invoke(cli.app, ["runner", "list"])
    assert result.exit_code == 0, result.output
    assert "mycli" in result.output
    assert "one-shot" not in result.output


def test_runner_add_with_flag_bearing_argv(tmp_path):
    result = runner.invoke(
        cli.app,
        ["runner", "add", " sonnet ", "claude", "--model", "sonnet", "{{prompt}}"],
    )
    assert result.exit_code == 0, result.output
    assert config.find_prompt_runner("sonnet") == config.PromptRunner(
        "sonnet", ("claude", "--model", "sonnet", "{{prompt}}")
    )
    assert config.load_config()["prompt"]["runners"][-1]["name"] == "sonnet"
    assert config.find_prompt_runner(" sonnet ") is None


def test_runner_add_preserves_bad_rows_and_force_repairs_matching_name(tmp_path):
    anonymous = "not-a-table"
    config.save_config(
        {
            "prompt": {
                "runners_seeded": True,
                "runners": [
                    {"name": "typo", "argv": ["old"]},
                    anonymous,
                ],
            }
        }
    )
    added = runner.invoke(cli.app, ["runner", "add", "new", "new", "{{prompt}}"])
    assert added.exit_code == 0, added.output
    assert config.load_config()["prompt"]["runners"][:2] == [
        {"name": "typo", "argv": ["old"]},
        anonymous,
    ]

    refused = runner.invoke(cli.app, ["runner", "add", "typo", "fixed", "{{prompt}}"])
    assert refused.exit_code == 1
    repaired = runner.invoke(
        cli.app,
        ["runner", "add", "typo", "--force", "--", "fixed", "{{prompt}}"],
    )
    assert repaired.exit_code == 0, repaired.output
    assert config.load_config()["prompt"]["runners"] == [
        {"name": "typo", "argv": ["fixed", "{{prompt}}"]},
        anonymous,
        {"name": "new", "argv": ["new", "{{prompt}}"]},
    ]


def test_runner_add_blank_name_is_refused_before_seeding(tmp_path):
    result = runner.invoke(cli.app, ["runner", "add", "   ", "x", "{{prompt}}"])
    assert result.exit_code == 2
    assert "A name is required" in result.output
    assert not config.prompt_runners_seeded()


def test_runner_add_validation_errors(tmp_path):
    cases = {
        ("noslot", "claude"): "exactly once",
        ("bin", "{{prompt}}"): "first word",
        ("stray", "x", "{{other}}"): "only the {{prompt}} slot",
    }
    for argv, needle in cases.items():
        result = runner.invoke(cli.app, ["runner", "add", *argv])
        assert result.exit_code == 2, argv
        assert needle in result.output
    result = runner.invoke(cli.app, ["runner", "add", "bare"])
    assert result.exit_code == 2
    assert "needs a command" in result.output


def test_runner_add_duplicate_name_refused(tmp_path):
    result = runner.invoke(cli.app, ["runner", "add", "claude", "x", "{{prompt}}"])
    assert result.exit_code == 1
    assert "already exists" in result.output


@pytest.mark.parametrize(
    ("prompt_value", "needle"),
    [
        ("broken", "isn't a table"),
        ({"runners": "broken"}, "isn't a list"),
    ],
)
def test_runner_add_reports_malformed_config_container(tmp_path, prompt_value, needle):
    config.save_config({"prompt": prompt_value})
    result = runner.invoke(cli.app, ["runner", "add", "new", "new", "{{prompt}}"])
    assert result.exit_code == 1
    assert needle in result.output
    assert config.load_config()["prompt"] == prompt_value


def test_runner_remove_and_unknown(tmp_path):
    # -y skips confirmation; an unknown name is refused before any confirmation prompt.
    assert runner.invoke(cli.app, ["runner", "remove", " amp ", "-y"]).exit_code == 0
    assert config.find_prompt_runner("amp") is None
    result = runner.invoke(cli.app, ["runner", "remove", "amp", "-y"])
    assert result.exit_code == 1
    assert "Unknown runner" in result.output


def test_runner_remove_blank_name_is_usage_error_before_seeding(tmp_path):
    result = runner.invoke(cli.app, ["runner", "remove", "   ", "--yes"])
    assert result.exit_code == 2
    assert "A name is required" in result.output
    assert not config.prompt_runners_seeded()


@pytest.mark.parametrize(
    ("args", "needle"),
    [
        ([], "exactly one"),
        (["amp", "--row", "0"], "exactly one"),
        (["--row", "not-an-index"], "non-negative index"),
        (["--row", "-1"], "non-negative index"),
    ],
)
def test_runner_remove_rejects_ambiguous_or_invalid_targets_before_writing(tmp_path, args, needle):
    config.save_prompt_runners([])
    before = config.load_config()
    result = runner.invoke(cli.app, ["runner", "remove", *args, "--yes"])
    assert result.exit_code == 2
    assert needle in result.output
    assert config.load_config() == before


def test_removing_every_runner_stays_empty(tmp_path):
    for name in ("claude", "codex", "opencode", "amp", "antigravity"):
        assert runner.invoke(cli.app, ["runner", "remove", name, "--yes"]).exit_code == 0
    assert config.load_prompt_runners() == []
    # The seeds must NOT resurrect (the runners_seeded marker).
    assert runner.invoke(cli.app, ["runner", "list"]).output.count("claude") == 0


def test_runner_remove_confirms_unless_yes(tmp_path, monkeypatch):
    """Deleting a configured agent is not a one-keystroke act: without -y the CLI asks
    (typer.confirm, abort=True) and a "y" answer goes through."""
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    result = runner.invoke(cli.app, ["runner", "remove", "amp"], input="y\n")
    assert result.exit_code == 0, result.output
    assert 'Remove the agent "amp"?' in result.output
    assert config.find_prompt_runner("amp") is None  # confirmed → gone
    assert "Runner amp removed." in result.output


def test_runner_remove_abort_keeps_the_runner(tmp_path, monkeypatch):
    """Answering "n" (or EOF) aborts: exit 1, nothing removed — the confirm really guards."""
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    result = runner.invoke(cli.app, ["runner", "remove", "amp"], input="n\n")
    assert result.exit_code == 1  # typer.confirm(abort=True) → Abort → exit 1
    assert config.find_prompt_runner("amp") is not None  # still configured
    assert "Runner amp removed." not in result.output


def test_runner_remove_warns_and_preserves_affected_prompt_pins(tmp_path):
    _added(tmp_path, pin="amp")
    result = runner.invoke(cli.app, ["runner", "remove", "amp", "--yes"])
    assert result.exit_code == 0, result.output
    assert "1 prompt pins this runner" in result.output
    assert store.resolve("p").meta.runner == "amp"
    assert config.find_prompt_runner("amp") is None


def test_runner_remove_raw_row_is_targeted_and_requires_yes_noninteractively(tmp_path):
    config.save_config(
        {
            "prompt": {
                "runners_seeded": True,
                "runners": [
                    {"name": "good", "argv": ["good", "{{prompt}}"]},
                    {"name": "broken", "argv": ["broken"]},
                    "untouched",
                ],
            }
        }
    )
    refused = runner.invoke(cli.app, ["runner", "remove", "--row", "1", "--no-input"])
    assert refused.exit_code == 2
    assert "pass --yes" in refused.output
    assert len(config.load_config()["prompt"]["runners"]) == 3

    removed = runner.invoke(cli.app, ["runner", "remove", "--row", "1", "--yes"])
    assert removed.exit_code == 0, removed.output
    assert "Malformed runner row 1 removed" in removed.output
    assert "Runner broken removed" not in removed.output
    assert config.load_config()["prompt"]["runners"] == [
        {"name": "good", "argv": ["good", "{{prompt}}"]},
        "untouched",
    ]
    unknown = runner.invoke(cli.app, ["runner", "remove", "--row", "9", "--yes"])
    assert unknown.exit_code == 1
    assert "runner list --all" in unknown.output


def test_runner_remove_raw_duplicate_has_no_false_pin_warning_or_key_removed_claim(tmp_path):
    config.save_config(
        {
            "prompt": {
                "runners_seeded": True,
                "runners": [
                    {"name": "same", "argv": ["first", "{{prompt}}"]},
                    {"name": "same", "argv": ["second", "{{prompt}}"]},
                ],
            }
        }
    )
    _added(tmp_path, pin="same")

    result = runner.invoke(cli.app, ["runner", "remove", "--row", "1", "--yes"])

    assert result.exit_code == 0, result.output
    assert "pins this runner" not in result.output
    assert "Runner same removed" not in result.output
    assert "Malformed runner row 1 removed" in result.output
    assert config.find_prompt_runner("same") == config.PromptRunner("same", ("first", "{{prompt}}"))
    assert store.resolve("p").meta.runner == "same"


def test_runner_remove_raw_valid_row_requires_stable_name_path(tmp_path):
    rows = [
        {"name": "same", "argv": ["first", "{{prompt}}"]},
        {"name": "same", "argv": ["second", "{{prompt}}"]},
    ]
    config.save_config({"prompt": {"runners_seeded": True, "runners": rows}})

    result = runner.invoke(cli.app, ["runner", "remove", "--row", "0", "--yes"])

    assert result.exit_code == 2
    assert 'skit runner remove "same"' in " ".join(result.output.split())
    assert config.load_config()["prompt"]["runners"] == rows


def test_runner_remove_raw_row_refuses_if_index_shifted_during_confirmation(tmp_path, monkeypatch):
    original = [
        {"name": "good", "argv": ["good", "{{prompt}}"]},
        {"name": "target", "argv": ["target"]},
        {"name": "other", "argv": ["other", "{{prompt}}"]},
    ]
    config.save_config({"prompt": {"runners_seeded": True, "runners": original}})
    inserted = {"name": "inserted", "argv": ["inserted", "{{prompt}}"]}

    def shift_before_confirm(*_args, **_kwargs):
        doc = config.load_config()
        doc["prompt"]["runners"].insert(0, inserted)
        config.save_config(doc)
        return True

    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    monkeypatch.setattr(cli.typer, "confirm", shift_before_confirm)
    result = runner.invoke(cli.app, ["runner", "remove", "--row", "1"])

    assert result.exit_code == 1
    assert "changed before it could be removed" in result.output
    assert config.load_config()["prompt"]["runners"] == [inserted, *original]


def test_runner_remove_name_refuses_if_key_is_replaced_during_confirmation(tmp_path, monkeypatch):
    config.save_prompt_runners([config.PromptRunner("victim", ("old", "{{prompt}}"))])
    replacement = config.PromptRunner("victim", ("new", "--important", "{{prompt}}"))

    def replace_during_confirm(*_args, **_kwargs):
        config.set_prompt_runner(replacement, replace_existing=True)
        return True

    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    monkeypatch.setattr(cli.typer, "confirm", replace_during_confirm)
    result = runner.invoke(cli.app, ["runner", "remove", "victim"])

    assert result.exit_code == 1
    assert "changed before it could be removed" in result.output
    assert config.find_prompt_runner("victim") == replacement


def test_runner_remove_container_repairs_only_targeted_prompt_value(tmp_path):
    config.save_config({"language": "zh-TW", "prompt": "garbage"})
    inspected = json.loads(runner.invoke(cli.app, ["runner", "list", "--all", "--json"]).output)
    assert inspected[0]["row"] is None
    assert inspected[0]["reason"] == "prompt-section-not-table"

    result = runner.invoke(cli.app, ["runner", "remove", "--row", "container", "--yes"])
    assert result.exit_code == 0, result.output
    assert "Malformed prompt runner container removed" in result.output
    assert "Runner container removed" not in result.output
    assert config.load_config() == {
        "language": "zh-TW",
        "prompt": {"runners_seeded": True, "runners": []},
    }


# --------------------------------------------------------------------------
# doctor
# --------------------------------------------------------------------------


def test_doctor_reports_prompt_drift_and_bad_runner_rows(tmp_path):
    entry = _added(tmp_path, text="{{a}}\n")
    entry.script_path.write_text("no holes\n", encoding="utf-8")
    config.save_config(
        {
            "prompt": {
                "runners_seeded": True,
                "runners": [{"name": "broken", "argv": ["x"]}],
            }
        }
    )
    result = runner.invoke(cli.app, ["doctor", "--json"])
    payload = json.loads(result.output)
    assert "p" in payload["drift"]
    assert payload["runner_rows_invalid"] == ["broken"]
    human = runner.invoke(cli.app, ["doctor"])
    assert "broken" in human.output
    assert "Inspect and repair with: skit runner list --all" in " ".join(human.output.split())


def test_doctor_healthy_prompt_reports_no_drift(tmp_path):
    _added(tmp_path, text="{{a}}\n")
    payload = json.loads(runner.invoke(cli.app, ["doctor", "--json"]).output)
    assert payload["drift"] == []
    assert payload["runner_rows_invalid"] == []


# --------------------------------------------------------------------------
# completion
# --------------------------------------------------------------------------


def test_complete_runner_names(tmp_path, monkeypatch):
    assert "claude" in cli._complete_runner("cl")
    assert cli._complete_runner("zz") == []
    monkeypatch.setattr(
        cli.config, "load_prompt_runners", lambda: (_ for _ in ()).throw(RuntimeError)
    )
    assert cli._complete_runner("") == []  # completion must never crash the shell


# --------------------------------------------------------------------------
# edges: unreadable bodies, store failures, refusal lanes
# --------------------------------------------------------------------------


def test_add_prompt_unreadable_file_is_a_store_error(tmp_path):
    trap = tmp_path / "dir.prompt.md"
    trap.mkdir()  # read_text raises IsADirectoryError (an OSError) while it "exists"
    result = runner.invoke(cli.app, ["add", str(trap), "--no-input"])
    assert result.exit_code == 1
    assert "Not a file" in result.output


def test_add_runner_flag_refused_on_cmd_edit_exe_lanes(tmp_path):
    for args in (
        ["add", "--cmd", "echo {x}", "-n", "c", "--runner", "claude"],
        ["add", "--edit", "--runner", "claude"],
        ["add", "x", "--exe", "--runner", "claude"],
    ):
        result = runner.invoke(cli.app, args)
        assert result.exit_code == 2, args
        # Two honest refusal voices: the path lane's prompt-specific message, or the
        # lane matrix's "can't apply here" — either way exit 2, nothing added.
        assert (
            "--runner only applies to prompt entries" in result.output
            or "--runner can't apply here" in result.output
        ), args


def test_add_no_interpolate_refused_up_front_on_non_prompt_path_lane(tmp_path):
    """--no-interpolate is prompt-only: on a PATH add it is refused UP FRONT (before kind
    inference) whenever the kind is already known — via --exe or an explicit non-prompt
    --kind — never silently dropped."""
    prog = tmp_path / "tool"
    prog.write_text("#!/bin/sh\necho hi\n", encoding="utf-8")
    for extra in (["--exe"], ["--kind", "shell"]):
        result = runner.invoke(
            cli.app, ["add", str(prog), *extra, "--no-interpolate", "-n", "t", "--no-input"]
        )
        assert result.exit_code == 2, extra
        assert "--no-interpolate only applies to prompt entries" in result.output
        assert not store.list_entries()


def test_add_prompt_editor_lane_reports_store_errors(tmp_path, monkeypatch):
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    answers = iter(["taken", "all", "-", "all", "-"])
    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(lambda *a, **k: next(answers)))
    monkeypatch.setattr(cli.editor, "open_in_editor", lambda path: path.write_text("body {{x}}\n"))
    store.add_command("echo hi", name="taken")  # the editor lane's add will collide
    result = runner.invoke(cli.app, ["add", "--prompt"])  # name asked interactively
    assert result.exit_code == 1
    assert "already taken" in result.output


def test_add_prompt_stdin_lane_reports_store_errors(tmp_path):
    store.add_command("echo hi", name="taken")
    result = runner.invoke(cli.app, ["add", "-", "--prompt", "-n", "taken"], input="b\n")
    assert result.exit_code == 1
    assert "already taken" in result.output


def test_params_view_survives_an_unreadable_reference_body(tmp_path):
    src = _write(tmp_path, "{{a}}\n")
    store.add_prompt(src, mode="reference")
    src.unlink()  # the original vanished: fresh scan degrades to "no candidates"
    result = runner.invoke(cli.app, ["params", "p"])
    assert result.exit_code == 0, result.output
    assert "a = " in result.output  # the managed record still lists


def test_params_runner_pin_reports_store_errors(tmp_path, monkeypatch):
    _added(tmp_path)

    def boom(slug, name):
        raise store.StoreError("disk on fire")

    monkeypatch.setattr(cli.store, "write_prompt_runner", boom)
    result = runner.invoke(cli.app, ["params", "p", "--runner", "claude"])
    assert result.exit_code == 1
    assert "disk on fire" in result.output


def test_doctor_skips_a_prompt_whose_body_is_gone(tmp_path):
    entry = _added(tmp_path, text="{{a}}\n")
    entry.script_path.unlink()
    payload = json.loads(runner.invoke(cli.app, ["doctor", "--json"]).output)
    assert payload["drift"] == []  # missing is missing's problem, not drift's
    assert "p" in payload["missing"]


# --------------------------------------------------------------------------
# the interpolate switch + flood caps (CLI surfaces)
# --------------------------------------------------------------------------


def test_add_no_interpolate(tmp_path):
    src = _write(tmp_path, "{{a}} {{b}}\n")
    result = runner.invoke(cli.app, ["add", str(src), "--no-interpolate", "--no-input"])
    assert result.exit_code == 0, result.output
    entry = store.resolve("p")
    assert entry.meta.interpolate is False
    assert entry.meta.params is None
    assert "insertion is off" in result.output.lower()


def test_add_no_interpolate_refused_off_the_prompt_lanes(tmp_path):
    py = tmp_path / "s.py"
    py.write_text("print(1)\n")
    result = runner.invoke(cli.app, ["add", str(py), "--no-interpolate", "--no-input"])
    assert result.exit_code == 2
    assert "--no-interpolate only applies to prompt entries" in result.output
    result = runner.invoke(cli.app, ["add", "--cmd", "echo hi", "-n", "c", "--no-interpolate"])
    assert result.exit_code == 2


def test_add_no_interpolate_through_stdin_lane(tmp_path):
    result = runner.invoke(
        cli.app,
        ["add", "-", "--prompt", "-n", "clip", "--no-interpolate"],
        input="Body {{x}}\n",
    )
    assert result.exit_code == 0, result.output
    assert store.resolve("clip").meta.interpolate is False


def test_add_interactive_off_answer_disables_insertion(tmp_path, monkeypatch):
    config.save_form("plain")  # the line-prompt path (form=tui hosts the review panel)
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    answers = iter(["", "off", "-"])
    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(lambda *a, **k: next(answers)))
    src = _write(tmp_path, "{{a}} {{b}}\n")
    result = runner.invoke(cli.app, ["add", str(src), "-n", "quiet"])
    assert result.exit_code == 0, result.output
    entry = store.resolve("quiet")
    assert entry.meta.interpolate is False
    assert entry.meta.params is None


def test_add_flood_cap_manages_nothing_and_says_so(tmp_path):
    from skit.langs.prompt.analyzer import AUTO_MANAGE_LIMIT

    many = " ".join("{{h" + str(i) + "}}" for i in range(AUTO_MANAGE_LIMIT + 5))
    src = _write(tmp_path, many + "\n")
    result = runner.invoke(cli.app, ["add", str(src), "--no-input"])
    assert result.exit_code == 0, result.output
    assert store.resolve("p").meta.params is None
    assert "too many to manage automatically" in result.output


def test_add_interactive_flood_defaults_to_none_and_caps_the_listing(tmp_path, monkeypatch):
    from skit.langs.prompt.analyzer import AUTO_MANAGE_LIMIT, LIST_PREVIEW_LIMIT

    config.save_form("plain")  # the line-prompt path (form=tui hosts the review panel)
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    seen: dict[str, object] = {}

    def fake_ask(*a, **k):
        if "Manage which" in a[0]:
            seen["default"] = k.get("default")
            return k.get("default")
        return "-"

    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(fake_ask))
    many = " ".join("{{h" + str(i) + "}}" for i in range(AUTO_MANAGE_LIMIT + 5))
    src = _write(tmp_path, many + "\n")
    result = runner.invoke(cli.app, ["add", str(src), "-n", "big"])
    assert result.exit_code == 0, result.output
    assert seen["default"] == "none"  # flood flips the interactive default
    assert f"…and {AUTO_MANAGE_LIMIT + 5 - LIST_PREVIEW_LIMIT} more" in result.output
    assert store.resolve("big").meta.params is None


def test_add_interactive_explicit_all_beats_the_flood_cap(tmp_path, monkeypatch):
    from skit.langs.prompt.analyzer import AUTO_MANAGE_LIMIT

    config.save_form("plain")  # the line-prompt path (form=tui hosts the review panel)
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    answers = iter(["", "all", "-"])
    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(lambda *a, **k: next(answers)))
    many = " ".join("{{h" + str(i) + "}}" for i in range(AUTO_MANAGE_LIMIT + 2))
    src = _write(tmp_path, many + "\n")
    result = runner.invoke(cli.app, ["add", str(src), "-n", "all-in"])
    assert result.exit_code == 0, result.output
    assert len(store.resolve("all-in").meta.params or []) == AUTO_MANAGE_LIMIT + 2


def test_params_interpolate_off_and_on(tmp_path):
    _added(tmp_path)
    result = runner.invoke(cli.app, ["params", "p", "--no-interpolate"])
    assert result.exit_code == 0, result.output
    assert store.resolve("p").meta.interpolate is False
    view = runner.invoke(cli.app, ["params", "p"])
    assert "Variable insertion is off" in view.output
    payload = json.loads(runner.invoke(cli.app, ["params", "p", "--json"]).output)
    assert payload["interpolate"] is False
    assert payload["unmanaged"] == []  # no scanning while off
    result = runner.invoke(cli.app, ["params", "p", "--interpolate"])
    assert result.exit_code == 0, result.output
    assert store.resolve("p").meta.interpolate is True
    assert store.resolve("p").meta.params == ["a"]  # survived the round trip


def test_params_interpolate_reports_store_errors(tmp_path, monkeypatch):
    _added(tmp_path)

    def boom(slug, on):
        raise store.StoreError("disk on fire")

    monkeypatch.setattr(cli.store, "write_prompt_interpolate", boom)
    result = runner.invoke(cli.app, ["params", "p", "--no-interpolate"])
    assert result.exit_code == 1
    assert "disk on fire" in result.output


def test_params_interpolate_refused_on_non_prompt(tmp_path):
    store.add_command("echo {m}", name="cmd")
    result = runner.invoke(cli.app, ["params", "cmd", "--no-interpolate"])
    assert result.exit_code == 1
    assert "--interpolate only applies to prompt entries" in result.output


@pytest.mark.parametrize(
    ("extra", "tail"), [(1, "and 1 more candidate"), (7, "and 7 more candidates")]
)
def test_params_unmanaged_listing_is_flood_capped_and_localizable(tmp_path, extra, tail):
    from skit.langs.prompt.analyzer import LIST_PREVIEW_LIMIT

    entry = _added(tmp_path)
    names = [f"u{i}" for i in range(LIST_PREVIEW_LIMIT + extra)]
    many = " ".join("{{" + name + "}}" for name in names)
    entry.script_path.write_text("{{a}} " + many + "\n", encoding="utf-8")
    result = runner.invoke(cli.app, ["params", "p"])
    assert result.exit_code == 0, result.output
    flat = " ".join(result.output.split())
    assert tail in flat
    assert names[LIST_PREVIEW_LIMIT - 1] in flat
    assert names[LIST_PREVIEW_LIMIT] not in flat

    payload = json.loads(runner.invoke(cli.app, ["params", "p", "--json"]).output)
    assert payload["unmanaged"] == names  # machine contract is full data, never a preview


def test_params_unmanaged_tail_passes_through_the_i18n_boundary(tmp_path, monkeypatch):
    from skit.langs.prompt.analyzer import LIST_PREVIEW_LIMIT

    monkeypatch.setenv("SKIT_LANG", "x-pseudo")
    entry = _added(tmp_path)
    names = [f"u{i}" for i in range(LIST_PREVIEW_LIMIT + 3)]
    entry.script_path.write_text(
        "{{a}} " + " ".join("{{" + name + "}}" for name in names), encoding="utf-8"
    )

    result = runner.invoke(cli.app, ["params", "p"])

    assert result.exit_code == 0, result.output
    assert "⟦" in result.output
    assert "möré" in result.output  # pseudo-transformed tail, not hard-coded English
    assert "and 3 more" not in result.output


def test_show_reports_the_interpolate_switch(tmp_path):
    _added(tmp_path)
    store.write_prompt_interpolate("p", False)
    payload = json.loads(runner.invoke(cli.app, ["show", "p", "--json"]).output)
    assert payload["interpolate"] is False
    human = runner.invoke(cli.app, ["show", "p"])
    assert "Variable insertion: off" in human.output


def test_doctor_skips_drift_for_an_insertion_off_prompt(tmp_path):
    entry = _added(tmp_path, text="{{a}}\n")
    entry.script_path.write_text("gone\n", encoding="utf-8")
    store.write_prompt_interpolate("p", False)
    payload = json.loads(runner.invoke(cli.app, ["doctor", "--json"]).output)
    assert payload["drift"] == []


def test_run_insertion_off_prompt_rejects_set_and_sends_verbatim(tmp_path, spawn_spy):
    _added(tmp_path, pin="claude")
    store.write_prompt_interpolate("p", False)
    result = runner.invoke(cli.app, ["run", "p", "--set", "a=1", "--no-input"])
    assert result.exit_code == 2  # no fields: --set has nothing to target
    result = runner.invoke(cli.app, ["run", "p", "--no-input"])
    assert result.exit_code == 0, result.output
    assert spawn_spy["values"] == {}


def test_params_schema_edits_refused_while_insertion_is_off(tmp_path):
    # Coherence with the read view: an off prompt must not be silently scanned and
    # given inert rows — the edit surface refuses and names the way back on.
    _added(tmp_path, text="{{a}} {{b}}\n")
    runner.invoke(cli.app, ["params", "p", "--no-interpolate"])
    for flags in (["--add", "b"], ["--rm", "a"], ["--deliver", "a=placeholder"]):
        result = runner.invoke(cli.app, ["params", "p", *flags])
        assert result.exit_code == 1, flags
        assert "Variable insertion is off" in result.output
    assert store.resolve("p").meta.params == ["a", "b"]  # nothing was mutated
    runner.invoke(cli.app, ["params", "p", "--interpolate"])
    assert runner.invoke(cli.app, ["params", "p", "--rm", "b"]).exit_code == 0


def test_add_interactive_flooded_numbers_address_the_previewed_names_only(tmp_path, monkeypatch):
    from skit.langs.prompt.analyzer import AUTO_MANAGE_LIMIT, LIST_PREVIEW_LIMIT

    config.save_form("plain")  # the line-prompt path (form=tui hosts the review panel)
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    beyond = str(LIST_PREVIEW_LIMIT + 3)  # an index whose name was never shown
    answers = iter(["", f"3,{beyond}", "-"])
    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(lambda *a, **k: next(answers)))
    many = " ".join("{{h" + str(i) + "}}" for i in range(AUTO_MANAGE_LIMIT + 5))
    src = _write(tmp_path, many + "\n")
    result = runner.invoke(cli.app, ["add", str(src), "-n", "blind"])
    assert result.exit_code == 0, result.output
    # Only the previewed index landed; the blind one was ignored, not guessed.
    assert store.resolve("blind").meta.params == ["h2"]
