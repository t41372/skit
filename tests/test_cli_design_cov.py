"""CLI coverage for curation (rename/describe), launch policy
(params --workdir/--interpreter/--template), stdin --kind, run --forget-args, the raw
no-op notice, show's interpreter key, doctor's launch_blocked, runner add --force, the
interpreted-add review routing, the line-mode script onboarding, and the prompt-run
inline runner picker.

Every test drives the real command tree and asserts the persisted state, the exit code,
or the exact user-facing wording — never that a line merely ran.
"""

from __future__ import annotations

import dataclasses
import io
import json
import sys
import types
from pathlib import Path

import pytest
from typer.testing import CliRunner

from skit import argstate, cli, config, i18n, store
from skit.langs.registry import spec_for

runner = CliRunner()


def _json(result):
    """The --json contract: stdout is EXACTLY one JSON document (SKILL.md's stable
    contract), so parse the WHOLE output — never slice from the first `{`, which would
    mask a human line leaking onto stdout."""
    return json.loads(result.output)


@pytest.fixture(autouse=True)
def _isolated(tmp_path, monkeypatch):
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path / "config"))
    monkeypatch.setenv("SKIT_LANG", "en")


@pytest.fixture
def spawn_spy(monkeypatch):
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
    # Prompt execution resolves the selected runner before crossing the delivery
    # boundary.  This fixture replaces the eventual process spawn, so keep that
    # earlier lookup hermetic as well instead of depending on the developer's PATH.
    monkeypatch.setattr("skit.langs.launch._which", lambda name: f"/bin/{name}")
    return calls


def _py(tmp_path, body="print('hi')\n", name="job"):
    p = tmp_path / f"{name}.py"
    p.write_text(body, encoding="utf-8")
    return store.add_python(p, name=name)


def _shell(tmp_path, body="#!/usr/bin/env bash\necho hi\n", name="sh"):
    p = tmp_path / f"{name}.sh"
    p.write_text(body, encoding="utf-8")
    return store.add_script(p, kind="shell", name=name)


def _prompt(tmp_path, text="Do {{a}}\n", name="p", pin=""):
    p = tmp_path / f"{name}.prompt.md"
    p.write_text(text, encoding="utf-8")
    entry = store.add_prompt(p, name=name)
    if pin:
        entry = store.write_prompt_runner(entry.slug, pin)
    return entry


def _spec(kind):
    s = spec_for(kind)
    assert s is not None
    return s


def _find_runner(name):
    r = config.find_prompt_runner(name)
    assert r is not None
    return r


def _flat(text: str) -> str:
    """Collapse rich's soft-wrap newlines/whitespace so a message split across the 80-col
    CliRunner width still matches as one string."""
    return " ".join(text.split())


# ============================================================ curation


def test_rename_success_keeps_presets_and_state(tmp_path):
    old = _py(tmp_path, name="old")
    argstate.save_preset(old.slug, "nightly", {"x": "1"})
    argstate.save_last(old.slug, extra_args=["--flag"])
    result = runner.invoke(cli.app, ["rename", "old", "new"])
    assert result.exit_code == 0, result.output
    assert "Renamed to new." in result.output
    entry = store.resolve("new")
    assert entry.meta.name == "new"
    assert entry.slug == old.slug  # slug is immutable
    # presets and remembered args survive under the same slug.
    state = argstate.load_state(entry.slug)
    assert list(state["presets"]) == ["nightly"]
    assert state["extra_args"] == ["--flag"]


def test_rename_not_found(tmp_path):
    result = runner.invoke(cli.app, ["rename", "ghost", "x"])
    assert result.exit_code == 1


def test_rename_conflict(tmp_path):
    _py(tmp_path, name="a")
    _py(tmp_path, name="b")
    result = runner.invoke(cli.app, ["rename", "a", "b"])
    assert result.exit_code == 1
    assert "already taken" in result.output
    assert store.resolve("a").meta.name == "a"  # untouched


def test_describe_set_and_clear(tmp_path):
    _py(tmp_path, name="a")
    set_res = runner.invoke(cli.app, ["describe", "a", "  A tidy helper  "])
    assert set_res.exit_code == 0, set_res.output
    assert "Description updated for a." in set_res.output
    assert store.resolve("a").meta.description == "A tidy helper"
    clr_res = runner.invoke(cli.app, ["describe", "a", ""])
    assert clr_res.exit_code == 0, clr_res.output
    assert "Description cleared for a." in clr_res.output
    assert store.resolve("a").meta.description == ""


def test_describe_not_found(tmp_path):
    result = runner.invoke(cli.app, ["describe", "ghost", "x"])
    assert result.exit_code == 1


# ============================================================ launch policy: workdir


@pytest.mark.parametrize("literal", ["origin", "store", "invoke"])
def test_params_workdir_literals(tmp_path, literal):
    _shell(tmp_path)
    result = runner.invoke(cli.app, ["params", "sh", "--workdir", literal])
    assert result.exit_code == 0, result.output
    assert f"now runs in: {literal}" in result.output
    assert store.resolve("sh").meta.workdir == literal


def test_params_workdir_absolute_path(tmp_path):
    _shell(tmp_path)
    wd = str(tmp_path / "wd")  # absolute on every platform (a Unix "/opt" is not, on Windows)
    result = runner.invoke(cli.app, ["params", "sh", "--workdir", wd])
    assert result.exit_code == 0, result.output
    assert store.resolve("sh").meta.workdir == wd


def test_params_workdir_relative_is_clean_error(tmp_path):
    _shell(tmp_path)
    result = runner.invoke(cli.app, ["params", "sh", "--workdir", "rel/ative"])
    assert result.exit_code == 1
    assert "origin, store, invoke, or an absolute path" in result.output


def test_params_workdir_origin_on_a_command_fails_cleanly(tmp_path):
    """`skit params <command> --workdir origin` fails cleanly (exit 1, StoreUsageError
    surfaced) — a command has no original file for "origin" to mean."""
    runner.invoke(cli.app, ["add", "--cmd", "echo hi", "--name", "c", "--no-input"])
    result = runner.invoke(cli.app, ["params", "c", "--workdir", "origin"])
    assert result.exit_code == 1
    assert "no original file — origin doesn't apply" in _flat(result.output)
    assert store.resolve("c").meta.workdir != "origin"  # nothing persisted


# ============================================================ launch policy: interpreter


def test_params_interpreter_set_and_clear_shell(tmp_path):
    _shell(tmp_path)
    set_res = runner.invoke(cli.app, ["params", "sh", "--interpreter", "zsh"])
    assert set_res.exit_code == 0, set_res.output
    assert "now runs with: zsh" in set_res.output
    assert store.resolve("sh").meta.interpreter == "zsh"
    clr_res = runner.invoke(cli.app, ["params", "sh", "--interpreter", ""])
    assert clr_res.exit_code == 0, clr_res.output
    assert "back to automatic interpreter detection" in clr_res.output
    assert store.resolve("sh").meta.interpreter == ""


def test_params_interpreter_set_js(tmp_path):
    p = tmp_path / "j.js"
    p.write_text("console.log(1)\n", encoding="utf-8")
    store.add_script(p, kind="js", name="j")
    result = runner.invoke(cli.app, ["params", "j", "--interpreter", "bun"])
    assert result.exit_code == 0, result.output
    assert store.resolve("j").meta.interpreter == "bun"


def test_params_interpreter_refused_on_python_and_prompt(tmp_path):
    _py(tmp_path, name="py")
    _prompt(tmp_path, name="pr")
    for name in ("py", "pr"):
        result = runner.invoke(cli.app, ["params", name, "--interpreter", "zsh"])
        assert result.exit_code == 1, (name, result.output)
        assert "pinnable interpreter" in result.output


def test_params_interpreter_refused_on_exe_and_command(tmp_path):
    prog = tmp_path / "tool"
    prog.write_text("#!/bin/sh\necho\n", encoding="utf-8")
    prog.chmod(0o755)
    store.add_exe(prog, name="ex")
    store.add_command("echo {m}", name="cmd")
    for name in ("ex", "cmd"):
        result = runner.invoke(cli.app, ["params", name, "--interpreter", "zsh"])
        assert result.exit_code == 1, (name, result.output)
        assert "pinnable interpreter" in result.output


# ============================================================ launch policy: template


def test_params_template_rewrite_reextracts_placeholders(tmp_path):
    store.add_command("echo {old}", name="cmd")
    result = runner.invoke(cli.app, ["params", "cmd", "--template", "ffmpeg -i {input} {output}"])
    assert result.exit_code == 0, result.output
    assert "Placeholders: input, output" in result.output
    assert store.resolve("cmd").meta.params == ["input", "output"]


def test_params_template_refused_on_non_command(tmp_path):
    _shell(tmp_path)
    result = runner.invoke(cli.app, ["params", "sh", "--template", "echo {x}"])
    assert result.exit_code == 1
    assert "isn't a command entry" in result.output


def test_params_template_empty_refused(tmp_path):
    store.add_command("echo {x}", name="cmd")
    result = runner.invoke(cli.app, ["params", "cmd", "--template", "   "])
    assert result.exit_code == 1
    assert store.resolve("cmd").meta.template == "echo {x}"  # unchanged


# ============================================================ params: one op per call


def _fingerprint(name):
    """Everything a params edit could touch: the meta AND the stored script text. A
    refused one-op combo must leave BOTH byte-identical (the "nothing was changed"
    contract), so the snapshot has to see the script's own [tool.skit] block too."""
    entry = store.resolve(name)
    script = entry.script_path.read_text(encoding="utf-8") if entry.script_path.exists() else ""
    return (entry.meta, script)


def _assert_one_op_refusal(name, argv, op):
    before = _fingerprint(name)
    result = runner.invoke(cli.app, ["params", name, *argv])
    assert result.exit_code == 2, result.output
    flat = _flat(result.output)
    assert f"{op} is its own operation" in flat
    assert "nothing was changed" in flat
    assert _fingerprint(name) == before  # neither the policy op NOR the rider applied


def test_params_workdir_with_schema_flag_refused_unchanged(tmp_path):
    _shell(tmp_path, name="sh")
    _assert_one_op_refusal(
        "sh", ["--workdir", "invoke", "--manage", "CITY"], "--workdir/--interpreter/--template"
    )


def test_params_interpolate_with_schema_flag_refused_unchanged(tmp_path):
    _prompt(tmp_path, name="pr")
    _assert_one_op_refusal(
        "pr", ["--interpolate", "--secret", "a"], "--interpolate/--no-interpolate"
    )


def test_params_runner_with_schema_flag_refused_unchanged(tmp_path):
    config.ensure_prompt_runners_seeded()
    _prompt(tmp_path, name="pr")
    _assert_one_op_refusal("pr", ["--runner", "claude", "--resync"], "--runner")


def test_params_normalize_with_schema_flag_refused_unchanged(tmp_path):
    _shell(tmp_path, body="#!/usr/bin/env bash\nCITY=Taipei\necho $CITY\n", name="sh")
    _assert_one_op_refusal("sh", ["--normalize", "CITY", "--manage", "CITY"], "--normalize")


def test_params_normalize_with_json_emits_the_read_view(tmp_path):
    """The --normalize own-op honors --json like every other own-op: the source idiom is
    rewritten, then a valid read-view JSON is emitted on stdout — not a silent drop (the
    fourth own-op face, so the rule holds on ALL of interpolate/runner/workdir/normalize)."""
    _shell(tmp_path, body="#!/usr/bin/env bash\nCITY=Taipei\necho $CITY\n", name="sh")
    result = runner.invoke(cli.app, ["params", "sh", "--normalize", "CITY", "--json"])
    assert result.exit_code == 0, result.output
    # The stored copy now carries the ${CITY:-Taipei} idiom (the semantic rewrite).
    assert "${CITY:-Taipei}" in store.resolve("sh").script_path.read_text(encoding="utf-8")
    payload = _json(result)  # stdout is exactly one JSON document (the read view)
    assert "params" in payload  # the entry's read view


def test_params_interpreter_with_json_emits_pure_read_view(tmp_path):
    """The --interpreter own-op (entry policy) honors --json too: the interpreter is written,
    then a valid read-view JSON is the WHOLE of stdout — the human confirmation is silenced
    (the same _edit_entry_policy quiet sink workdir/template share)."""
    _shell(tmp_path, name="sh")
    result = runner.invoke(cli.app, ["params", "sh", "--interpreter", "zsh", "--json"])
    assert result.exit_code == 0, result.output
    assert store.resolve("sh").meta.interpreter == "zsh"  # the policy was written
    payload = _json(result)  # stdout parses whole — no human line leaked
    assert "params" in payload


def test_params_template_with_json_emits_pure_read_view(tmp_path):
    """The --template own-op honors --json the same way (a command entry's editable
    template) — written, then a pure read-view JSON on stdout."""
    store.add_command("echo {msg}", name="cmd")
    result = runner.invoke(cli.app, ["params", "cmd", "--template", "echo {greeting}", "--json"])
    assert result.exit_code == 0, result.output
    assert store.resolve("cmd").meta.template == "echo {greeting}"  # the template was rewritten
    payload = _json(result)  # stdout is exactly one JSON document
    assert "params" in payload


def test_params_two_policy_groups_refused_unchanged(tmp_path):
    """Two policy ops in one call is refused just like a policy op + a schema flag — the
    first-listed op names the refusal, and nothing changes."""
    config.ensure_prompt_runners_seeded()
    _prompt(tmp_path, name="pr")
    _assert_one_op_refusal(
        "pr", ["--interpolate", "--runner", "claude"], "--interpolate/--no-interpolate"
    )


def test_params_workdir_plus_runner_refused_names_runner_first(tmp_path):
    config.ensure_prompt_runners_seeded()
    _prompt(tmp_path, name="pr")
    # --runner is listed before the workdir group, so it's the op named in the refusal.
    _assert_one_op_refusal("pr", ["--workdir", "invoke", "--runner", "claude"], "--runner")


def test_params_launch_policy_group_stays_combinable(tmp_path):
    """--workdir/--interpreter/--template are ONE launch-policy group, not three ops — so
    --workdir + --interpreter in a single call is allowed and both apply (exit 0)."""
    _shell(tmp_path, name="sh")
    result = runner.invoke(cli.app, ["params", "sh", "--workdir", "invoke", "--interpreter", "zsh"])
    assert result.exit_code == 0, result.output
    assert "is its own operation" not in result.output  # NOT one-op-refused
    meta = store.resolve("sh").meta
    assert meta.workdir == "invoke"  # both landed
    assert meta.interpreter == "zsh"


def test_params_command_policy_group_is_atomic_when_interpreter_is_invalid(tmp_path):
    """Every supplied policy is validated before any is written: a command can't accept
    an interpreter, so its otherwise-valid template and workdir must not partially land."""
    entry = store.add_command("echo {old}", name="cmd")
    meta_path = entry.dir / "meta.toml"
    before_bytes = meta_path.read_bytes()
    before_meta = store.resolve("cmd").meta

    # An absolute workdir on every platform, so the interpreter — not a Windows-invalid
    # "/opt" path — is the refusal this atomicity test is about.
    result = runner.invoke(
        cli.app,
        [
            "params",
            "cmd",
            "--template",
            "echo {new}",
            "--workdir",
            str(tmp_path / "new"),
            "--interpreter",
            "zsh",
        ],
    )

    assert result.exit_code == 1, result.output
    assert "pinnable interpreter" in result.output
    assert "Template updated" not in result.output
    assert "now runs in" not in result.output
    assert "now runs with" not in result.output
    assert meta_path.read_bytes() == before_bytes
    assert store.resolve("cmd").meta == before_meta


def test_params_shell_policy_group_is_atomic_when_template_is_invalid(tmp_path):
    """A template on a shell is refused before its valid workdir/interpreter can land."""
    entry = _shell(tmp_path, name="sh")
    meta_path = entry.dir / "meta.toml"
    before_bytes = meta_path.read_bytes()
    before_meta = store.resolve("sh").meta

    result = runner.invoke(
        cli.app,
        [
            "params",
            "sh",
            "--workdir",
            "invoke",
            "--interpreter",
            "zsh",
            "--template",
            "echo {value}",
        ],
    )

    assert result.exit_code == 1, result.output
    assert "isn't a command entry" in result.output
    assert "Template updated" not in result.output
    assert "now runs in" not in result.output
    assert "now runs with" not in result.output
    assert meta_path.read_bytes() == before_bytes
    assert store.resolve("sh").meta == before_meta


# ============================================================ show interpreter key


def test_show_json_and_human_carry_interpreter(tmp_path):
    _shell(tmp_path)
    store.write_interpreter("sh", "zsh")
    j = runner.invoke(cli.app, ["show", "sh", "--json"])
    assert j.exit_code == 0, j.output
    assert json.loads(j.output)["interpreter"] == "zsh"
    human = runner.invoke(cli.app, ["show", "sh"])
    assert human.exit_code == 0, human.output
    assert "Interpreter: zsh" in human.output


def test_show_json_interpreter_none_when_unset(tmp_path):
    _shell(tmp_path)
    j = runner.invoke(cli.app, ["show", "sh", "--json"])
    assert json.loads(j.output)["interpreter"] is None


# ============================================================ stdin --kind


def test_add_stdin_kind_shell_with_shebang_pins_interpreter(tmp_path):
    result = runner.invoke(
        cli.app, ["add", "-", "--kind", "shell", "-n", "x"], input="#!/bin/bash\necho $1\n"
    )
    assert result.exit_code == 0, result.output
    entry = store.resolve("x")
    assert entry.meta.kind == "shell"
    assert entry.script_path.suffix == ".sh"
    assert entry.meta.interpreter == "bash"  # shebang program is a known shell shebang


def test_add_stdin_kind_js_records_scanned_deps(tmp_path):
    result = runner.invoke(
        cli.app,
        ["add", "-", "--kind", "js", "-n", "j"],
        input="import chalk from 'chalk'\nconsole.log(chalk)\n",
    )
    assert result.exit_code == 0, result.output
    assert "chalk" in (store.resolve("j").meta.dependencies or [])


def test_add_stdin_exe_is_refused(tmp_path):
    result = runner.invoke(cli.app, ["add", "-", "--exe", "-n", "x"], input="echo\n")
    assert result.exit_code == 2
    # Two honest refusal voices: the lane matrix's "can't apply here" (which fires first
    # for --exe on stdin) or the older program-on-disk wording — either way exit 2.
    assert (
        "needs an existing program on disk" in result.output
        or "--exe can't apply here" in result.output
    )
    assert not store.list_entries()


def test_add_stdin_kind_exe_is_refused(tmp_path):
    result = runner.invoke(cli.app, ["add", "-", "--kind", "exe", "-n", "x"], input="echo\n")
    assert result.exit_code == 2
    assert "needs an existing program on disk" in result.output


def test_add_edit_with_kind_is_refused(tmp_path):
    result = runner.invoke(cli.app, ["add", "--edit", "--kind", "shell"])
    assert result.exit_code == 2
    # The lane matrix refuses --kind on the --edit lane and the hint points at the shebang
    # (the draft's real kind signal): --edit reads the kind from the draft, so --kind is
    # redundant. The hint also names the sibling lanes it can't do (--ref/--exe, a prompt).
    flat = _flat(result.output)
    assert "its kind comes from the shebang you write" in flat
    assert "#!/usr/bin/env bash" in flat  # the concrete shebang example
    assert "a prompt is drafted with skit add --prompt" in flat  # the prompt sibling lane


def test_add_edit_with_exe_is_refused(tmp_path):
    result = runner.invoke(cli.app, ["add", "--edit", "--exe"])
    assert result.exit_code == 2


def test_add_stdin_kind_js_without_imports_records_no_deps(tmp_path):
    result = runner.invoke(
        cli.app, ["add", "-", "--kind", "js", "-n", "j"], input="console.log(1)\n"
    )
    assert result.exit_code == 0, result.output
    assert store.resolve("j").meta.dependencies in (None, [])  # nothing scanned → no deps


def test_add_stdin_kind_duplicate_name_is_store_error(tmp_path):
    _shell(tmp_path, name="dup")
    result = runner.invoke(
        cli.app, ["add", "-", "--kind", "shell", "-n", "dup"], input="#!/bin/bash\necho x\n"
    )
    assert result.exit_code == 1
    assert "already taken" in result.output


def test_add_stdin_kind_python_routes_to_the_python_lane(tmp_path):
    # --kind python is not the non-python twin's job: it falls through to the PEP 723 lane.
    result = runner.invoke(
        cli.app, ["add", "-", "--kind", "python", "-n", "x"], input="print('hi')\n"
    )
    assert result.exit_code == 0, result.output
    assert store.resolve("x").meta.kind == "python"


def test_add_stdin_kind_missing_name(tmp_path):
    result = runner.invoke(cli.app, ["add", "-", "--kind", "shell"], input="echo hi\n")
    assert result.exit_code == 2
    assert "explicit --name" in result.output


def test_add_stdin_kind_empty_input(tmp_path):
    result = runner.invoke(cli.app, ["add", "-", "--kind", "shell", "-n", "x"], input="   \n")
    assert result.exit_code == 1
    assert "Nothing arrived on stdin" in result.output


# ---- stdin shebang routing (F2): no --kind, the piped text's shebang decides ----


def test_add_stdin_bash_shebang_no_kind_routes_to_shell(tmp_path):
    """No --kind: the piped text's shebang is the explicit signal — a bash snippet lands as
    a SHELL entry (kind_for_shebang_text), never a broken python entry with a bash body."""
    result = runner.invoke(
        cli.app, ["add", "-", "-n", "clip"], input="#!/usr/bin/env bash\necho hi\n"
    )
    assert result.exit_code == 0, result.output
    assert store.resolve("clip").meta.kind == "shell"


def test_add_stdin_zsh_shebang_records_interpreter(tmp_path):
    """Interpreter recording still works through the stdin text: the tmp copy carries the
    shebang the reader reads, so a zsh-shebang snippet lands as shell with interpreter=zsh."""
    result = runner.invoke(
        cli.app, ["add", "-", "-n", "zclip"], input="#!/usr/bin/env zsh\necho hi\n"
    )
    assert result.exit_code == 0, result.output
    entry = store.resolve("zclip")
    assert entry.meta.kind == "shell"
    assert entry.meta.interpreter == "zsh"  # read from the piped shebang, not left blank


def test_add_stdin_no_shebang_defaults_to_python(tmp_path):
    """No shebang → the python lane (unchanged): kind_for_shebang_text returns None, so the
    fallback stays python."""
    result = runner.invoke(cli.app, ["add", "-", "-n", "plainpy"], input="print('hi')\n")
    assert result.exit_code == 0, result.output
    assert store.resolve("plainpy").meta.kind == "python"


def test_add_stdin_explicit_kind_overrides_shebang(tmp_path):
    """An explicit --kind still WINS over the text's shebang: a python-shebang body forced to
    --kind shell lands as shell, never re-inferred back to python."""
    result = runner.invoke(
        cli.app,
        ["add", "-", "--kind", "shell", "-n", "forced"],
        input="#!/usr/bin/env python3\necho hi\n",
    )
    assert result.exit_code == 0, result.output
    assert store.resolve("forced").meta.kind == "shell"


def test_add_from_stdin_reads_stdin_when_text_is_none(tmp_path, monkeypatch):
    """_add_from_stdin keeps its standalone stdin-reading contract: called directly with no
    text (the default), it reads sys.stdin itself — the fallback the `add` lane never uses
    now that it threads the once-read text through."""
    monkeypatch.setattr(sys, "stdin", io.StringIO("print('hi')\n"))
    cli._add_from_stdin("directpy", None)
    assert store.resolve("directpy").meta.kind == "python"


def test_add_script_from_stdin_reads_stdin_when_text_is_none(tmp_path, monkeypatch):
    """The non-python twin keeps the same standalone contract."""
    monkeypatch.setattr(sys, "stdin", io.StringIO("#!/usr/bin/env bash\necho hi\n"))
    cli._add_script_from_stdin("shell", "directsh", None)
    assert store.resolve("directsh").meta.kind == "shell"


# ============================================================ add-lane flag matrix


def _drafts():
    """The kept authoring/stdin drafts under skit's OWN data dir (data_dir/drafts/)."""
    from skit.paths import drafts_dir

    d = drafts_dir()
    return sorted(d.glob("skit-*")) if d.is_dir() else []


def test_add_cmd_refuses_ref_kind_python_loudly(tmp_path):
    """The --cmd lane honors ONLY --name/--description: any of --ref/--kind/--dep/--python
    is refused up front by the matrix (exit 2, nothing added), never silently dropped."""
    for extra in (["--ref"], ["--kind", "shell"], ["--dep", "x"], ["--python", ">=3.11"]):
        result = runner.invoke(cli.app, ["add", "--cmd", "echo {x}", "--name", "c", *extra])
        assert result.exit_code == 2, (extra, result.output)
        assert "can't apply here" in result.output
        assert "a --cmd template takes only --name/--description" in _flat(result.output)
        assert not store.list_entries()


def test_add_stdin_non_prompt_refuses_runner_and_no_interpolate(tmp_path):
    """--runner / --no-interpolate are prompt-only: on a NON-prompt stdin add they are
    refused per final kind (matrix-admitted for the prompt case only), never dropped."""
    for extra in (["--runner", "claude"], ["--no-interpolate"]):
        result = runner.invoke(
            cli.app, ["add", "-", "-n", "s", *extra], input="#!/usr/bin/env bash\necho hi\n"
        )
        assert result.exit_code == 2, (extra, result.output)
        assert "only apply to prompt entries" in result.output
        assert not store.list_entries()


def test_add_prompt_stdin_refuses_dep_and_python(tmp_path):
    """A prompt has no dependencies: --dep/--python on a prompt stdin add exit 2, add nothing."""
    for extra in (["--dep", "requests"], ["--python", ">=3.11"]):
        result = runner.invoke(
            cli.app, ["add", "-", "--prompt", "-n", "p", *extra], input="Summarize {{url}}\n"
        )
        assert result.exit_code == 2, (extra, result.output)
        assert "a prompt has no dependencies" in result.output
        assert not store.list_entries()


def test_add_shell_stdin_refuses_dep_flag(tmp_path):
    """--dep is meaningless for a non-npm stdin kind (shell): refused per the piped text's
    kind, exit 2, nothing added — deps belong to python (uv) and js/ts (npm) only."""
    result = runner.invoke(
        cli.app, ["add", "-", "--kind", "shell", "-n", "s", "--dep", "jq"], input="echo hi\n"
    )
    assert result.exit_code == 2
    assert "don't apply to a shell entry" in result.output
    assert not store.list_entries()


def test_add_shell_stdin_refuses_python_flag(tmp_path):
    result = runner.invoke(
        cli.app,
        ["add", "-", "--kind", "shell", "-n", "s", "--python", ">=3.11"],
        input="echo hi\n",
    )
    assert result.exit_code == 2
    assert "don't apply to a shell entry" in result.output
    assert not store.list_entries()


def test_add_js_stdin_explicit_dep_beats_scanner(tmp_path):
    """An explicit --dep on a js stdin add is HONORED VERBATIM over skit's own import scan
    (the worst drop was substituting the scan for a typed flag): the recorded dep list is
    exactly the flag, not the scanned 'chalk'."""
    result = runner.invoke(
        cli.app,
        ["add", "-", "--kind", "js", "-n", "jx", "--dep", "chalk@5"],
        input="import chalk from 'chalk'\nconsole.log(chalk)\n",
    )
    assert result.exit_code == 0, result.output
    assert store.resolve("jx").meta.dependencies == ["chalk@5"]  # the flag, verbatim


def test_add_js_stdin_without_scanner_still_adds_no_deps(tmp_path, monkeypatch):
    """A2 degradation: a broken js grammar leaves dep_scanner None. The stdin lane must
    still add the entry (no scan, no crash) — it just records no dependencies."""
    import dataclasses

    real = cli.spec_for

    def scannerless(kind):
        s = real(kind)
        if kind == "js" and s is not None:
            return dataclasses.replace(s, dep_scanner=None)
        return s

    monkeypatch.setattr(cli, "spec_for", scannerless)
    result = runner.invoke(
        cli.app, ["add", "-", "--kind", "js", "-n", "jd"], input="import chalk from 'chalk'\n"
    )
    assert result.exit_code == 0, result.output
    assert store.resolve("jd").meta.dependencies in (None, [])  # no scanner -> no deps


# ---- unregistered shebang: stdin refuses with the --kind escape ----


def test_add_stdin_unregistered_shebang_refused(tmp_path):
    """An UNREGISTERED shebang (awk, sed -f, …) piped in with no --kind is a signal skit
    can't honor: refused with the --kind escape, exactly like the path lane — never
    fabricated into a python entry that can only die in uv run."""
    result = runner.invoke(
        cli.app, ["add", "-", "-n", "aw"], input="#!/usr/bin/awk -f\nBEGIN { print 1 }\n"
    )
    assert result.exit_code == 2
    assert "names no interpreter skit knows" in result.output
    assert "--kind" in result.output  # the escape hatch
    assert not store.list_entries()  # nothing fabricated


# ---- stdin keep-on-failure: all three lanes keep the draft under data_dir/drafts/ ----


def test_add_python_stdin_name_conflict_keeps_draft(tmp_path):
    """A NameConflictError on the python stdin lane keeps the temp (the piped text's only
    materialized copy) under data_dir/drafts/ and says where — never destroys the paste."""
    _py(tmp_path, name="dup")
    result = runner.invoke(cli.app, ["add", "-", "-n", "dup"], input="print('hi')\n")
    assert result.exit_code == 1
    assert "Your draft was kept at" in result.output
    from skit.paths import drafts_dir

    kept = _drafts()
    assert kept  # the paste's only materialized copy survived
    assert all(p.parent == drafts_dir() for p in kept)  # kept under data_dir/drafts/


def test_add_script_stdin_name_conflict_keeps_draft(tmp_path):
    """The non-python (script) stdin lane keeps its draft the same way."""
    _shell(tmp_path, name="dup")
    result = runner.invoke(cli.app, ["add", "-", "--kind", "shell", "-n", "dup"], input="echo hi\n")
    assert result.exit_code == 1
    assert "Your draft was kept at" in result.output
    from skit.paths import drafts_dir

    kept = _drafts()
    assert kept  # the paste's only materialized copy survived
    assert all(p.parent == drafts_dir() for p in kept)


def test_add_prompt_stdin_name_conflict_keeps_draft(tmp_path):
    """The prompt stdin lane keeps its draft too — pbpaste of a prompt body isn't lost."""
    _prompt(tmp_path, name="dup")
    result = runner.invoke(
        cli.app, ["add", "-", "--prompt", "-n", "dup"], input="Summarize {{url}}\n"
    )
    assert result.exit_code == 1
    assert "Your draft was kept at" in result.output
    from skit.paths import drafts_dir

    kept = _drafts()
    assert kept  # the paste's only materialized copy survived
    assert all(p.parent == drafts_dir() for p in kept)


# ============================================================ interpreted add review routing


def test_add_shell_interactive_routes_through_review_panel(tmp_path, monkeypatch):
    """A real terminal with form=tui hosts the SAME review panel python gets — the flags
    ride along as prefills and the panel's slug feeds the summary."""
    src = tmp_path / "tool.sh"
    src.write_text("#!/usr/bin/env bash\necho hi\n", encoding="utf-8")
    seen: dict[str, object] = {}

    def fake_panel(path, **kwargs):
        seen["path"] = path
        seen.update(kwargs)
        return store.add_script(path, kind="shell", name="panelled").slug

    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    monkeypatch.setattr("skit.tui_add.run_add_review", fake_panel)
    result = runner.invoke(cli.app, ["add", str(src), "-d", "helper"])
    assert result.exit_code == 0, result.output
    assert seen["kind"] == "shell"  # the kind kwarg threads through
    assert seen["description"] == "helper"
    assert "panelled" in result.output


def test_add_shell_interactive_panel_cancel_exits_130(tmp_path, monkeypatch):
    src = tmp_path / "tool.sh"
    src.write_text("#!/usr/bin/env bash\necho hi\n", encoding="utf-8")
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    monkeypatch.setattr("skit.tui_add.run_add_review", lambda path, **kw: None)
    result = runner.invoke(cli.app, ["add", str(src)])
    assert result.exit_code == 130
    assert "Cancelled" in result.output


# ============================================================ line-mode onboarding


def _fake_tty(monkeypatch, isatty=True):
    monkeypatch.setattr(sys, "stdin", types.SimpleNamespace(isatty=lambda: isatty, read=lambda: ""))
    monkeypatch.setattr("sys.stdout.isatty", lambda: isatty, raising=False)
    monkeypatch.setattr(cli, "_is_interactive", lambda: isatty)


def test_onboard_script_params_writes_picked_shell_params(tmp_path, monkeypatch):
    entry = _shell(tmp_path, body="#!/usr/bin/env bash\nCITY=Taipei\necho $CITY\n", name="d")
    _fake_tty(monkeypatch)
    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(lambda *a, **k: "1"))
    managed = cli._onboard_script_params(store.resolve("d"), _spec("shell"), no_input=False)
    assert managed == ["CITY"]
    # The pick was written into the stored copy — read_params / skit params see it.
    text = entry.script_path.read_text(encoding="utf-8")
    io = _spec("shell").params_io
    assert io is not None
    assert "CITY" in [d.name for d in io.read(text)]


def test_onboard_script_params_none_selection_writes_nothing(tmp_path, monkeypatch):
    entry = _shell(tmp_path, body="#!/usr/bin/env bash\nCITY=Taipei\necho $CITY\n", name="d")
    before = entry.script_path.read_text(encoding="utf-8")
    _fake_tty(monkeypatch)
    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(lambda *a, **k: "none"))
    assert cli._onboard_script_params(store.resolve("d"), _spec("shell"), no_input=False) == []
    assert entry.script_path.read_text(encoding="utf-8") == before  # copy untouched


def test_onboard_script_params_no_input_selects_nothing(tmp_path, monkeypatch):
    _shell(tmp_path, body="#!/usr/bin/env bash\nCITY=Taipei\necho $CITY\n", name="d")
    _fake_tty(monkeypatch)
    assert cli._onboard_script_params(store.resolve("d"), _spec("shell"), no_input=True) == []


def test_onboard_script_params_does_not_prompt_when_stdout_is_piped(tmp_path, monkeypatch):
    entry = _shell(tmp_path, body="#!/usr/bin/env bash\nCITY=Taipei\necho $CITY\n", name="d")
    _fake_tty(monkeypatch)
    monkeypatch.setattr("sys.stdout.isatty", lambda: False, raising=False)
    monkeypatch.setattr(cli, "_is_interactive", lambda: False)
    before = entry.script_path.read_text(encoding="utf-8")

    def _boom(*_a, **_k):
        raise AssertionError("a redirected command must not ask an invisible question")

    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(_boom))
    assert cli._onboard_script_params(entry, _spec("shell"), no_input=False) == []
    assert entry.script_path.read_text(encoding="utf-8") == before


def test_onboard_script_params_skips_reference_entries(tmp_path, monkeypatch):
    src = tmp_path / "d.sh"
    src.write_text("#!/usr/bin/env bash\nCITY=Taipei\necho $CITY\n", encoding="utf-8")
    entry = store.add_script(src, kind="shell", name="d", mode="reference")
    _fake_tty(monkeypatch)
    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(lambda *a, **k: "1"))
    assert cli._onboard_script_params(entry, _spec("shell"), no_input=False) == []


def test_onboard_script_params_skips_when_reader_models_the_form(tmp_path, monkeypatch, capsys):
    # Onboarding skips (manages nothing, never asks) ONLY when the entry's own
    # reader MODELS a form — that form IS the interface, and a managed constant would replace
    # it. The python analyzer models argparse fields, so drive it directly. The ✓ read notice
    # prints even though nothing is managed.
    p = tmp_path / "d.py"
    p.write_text(
        "import argparse\np = argparse.ArgumentParser()\np.add_argument('--n')\n"
        "p.parse_args()\nCITY = 'x'\n",
        encoding="utf-8",
    )
    entry = store.add_python(p, name="d")
    _fake_tty(monkeypatch)
    ask_hit = {"n": 0}
    monkeypatch.setattr(
        cli.Prompt, "ask", staticmethod(lambda *a, **k: ask_hit.__setitem__("n", 1))
    )
    assert cli._onboard_script_params(entry, _spec("python"), no_input=False) == []
    assert ask_hit["n"] == 0  # never reached the ask — the modeled form is the interface
    assert "skit read this script's own arguments" in capsys.readouterr().out


def test_onboard_script_params_offers_candidates_when_reader_unmodeled(tmp_path, monkeypatch):
    # The complement (both branches of the new `if flows.reader_fields(...)` guard): a script
    # that self-parses but skit CANNOT model (a dynamic getopts optstring) runs on the
    # passthrough field, so managed constants are ADDITIVE — the ask IS reached and the picked
    # constant is managed on top of the reader form.
    entry = _shell(
        tmp_path,
        body='#!/usr/bin/env bash\nOUTDIR=/tmp\nOPTS="n:v"\nwhile getopts "$OPTS" o; do :; done\n'
        "echo $OUTDIR\n",
        name="d",
    )
    _fake_tty(monkeypatch)
    calls = {"n": 0}

    def fake_ask(*_a, **_k):
        calls["n"] += 1
        return "1"

    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(fake_ask))
    managed = cli._onboard_script_params(entry, _spec("shell"), no_input=False)
    assert calls["n"] == 1  # the candidate offer was reached — a dynamic optstring is not modeled
    assert managed == ["OUTDIR"]  # candidate #1 managed on top of the passthrough form


# ---- _onboard_script_params: the exact candidate listing, numbering and prompt ------------


def _two_const_shell(tmp_path):
    return _shell(
        tmp_path,
        body='#!/usr/bin/env bash\nWIDTH=800\nHEIGHT=600\necho "$WIDTH $HEIGHT"\n',
        name="d",
    )


def test_onboard_script_params_plural_listing_numbering_and_prompt(tmp_path, monkeypatch, capsys):
    """Two candidates: the plural header, 1-based numbering, and the exact ask (text/default/
    console) are all pinned. Kills the ngettext-plural string mutants, the `console.print(None)`
    arg-drop, the `enumerate(..., start=1)` / `_print_candidate(i, …)` numbering mutants, and every
    nulled/dropped Prompt.ask argument (prompt text, default, console)."""
    i18n.init("en")
    _two_const_shell(tmp_path)
    _fake_tty(monkeypatch)
    ask_args: list[tuple[object, ...]] = []
    ask_kwargs: list[dict[str, object]] = []

    def spy_ask(*args, **kwargs):
        ask_args.append(args)
        ask_kwargs.append(kwargs)
        return "none"  # decline, so nothing is written — this test only reads the prompt surface

    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(spy_ask))
    assert cli._onboard_script_params(store.resolve("d"), _spec("shell"), no_input=False) == []
    out = capsys.readouterr().out
    # plural header (kills the console.print(None) arg-drop and the plural-msgid case-flip); the
    # XX-wrap can't move the visible substring, so pin it separately.
    assert "Found 2 parameter candidates (constants / input() calls):" in out
    assert "XX" not in out
    # 1-based numbering AND the right candidate per row (kills start=0 / start=2 / _print_candidate
    # index-null): row 1 is WIDTH, row 2 is HEIGHT.
    assert "  1. WIDTH" in out
    assert "  2. HEIGHT" in out
    # exactly one ask, with the exact prompt text, default and skit's own console.
    assert ask_args == [("Which ones should skit manage? (e.g. 1,3 / all / none)",)]
    assert ask_kwargs[0].get("default") == "all"  # both candidates clean -> "all"
    assert ask_kwargs[0].get("console") is cli.console


def test_onboard_script_params_singular_listing_copy(tmp_path, monkeypatch, capsys):
    """One candidate selects the SINGULAR ngettext form; pin its exact text (kills the singular
    XX-wrap and case-flip)."""
    i18n.init("en")
    _shell(tmp_path, body='#!/usr/bin/env bash\nWIDTH=800\necho "$WIDTH"\n', name="d")
    _fake_tty(monkeypatch)
    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(lambda *a, **k: "none"))
    assert cli._onboard_script_params(store.resolve("d"), _spec("shell"), no_input=False) == []
    out = capsys.readouterr().out
    assert "Found 1 parameter candidate (constants / input() calls):" in out
    assert "parameter candidates" not in out  # the singular msgid was chosen, not the plural
    assert "XX" not in out


def test_onboard_script_params_or_guard_returns_early_without_analyzer(tmp_path, monkeypatch):
    """The `analyzer is None OR params_io is None` guard must short-circuit on the FIRST truthy
    half: a spec whose analyzer is None (but params_io intact) returns [] and never touches
    `analyzer.analyze`. The `and` mutant would fall through and crash on None.analyze."""
    _shell(tmp_path, name="d")
    _fake_tty(monkeypatch)
    faceless = dataclasses.replace(_spec("shell"), analyzer=None)  # analyzer None, params_io kept
    assert cli._onboard_script_params(store.resolve("d"), faceless, no_input=False) == []


def test_onboard_script_params_forwards_entry_name_to_add_hints(tmp_path, monkeypatch):
    """The add-hints call must carry the ENTRY's name (the filename-literal hint prints
    `skit edit <name>`); a nulled name argument would advertise `skit edit None`."""
    entry = _shell(tmp_path, name="d")
    _fake_tty(monkeypatch)
    seen: dict[str, object] = {}
    monkeypatch.setattr(cli, "_print_add_hints", lambda result, name: seen.update(name=name))
    # no_input=True returns right after the hints print, so the forwarding is still observed.
    assert cli._onboard_script_params(entry, _spec("shell"), no_input=True) == []
    assert seen["name"] == entry.meta.name


def test_onboard_script_params_reference_forwards_frameworks_to_notice(tmp_path, monkeypatch):
    """A reference-mode entry prints the reference add-notice with the REAL detected frameworks
    list (not None): pin the third argument so the nulled-frameworks mutant dies."""
    src = tmp_path / "d.sh"
    src.write_text("#!/usr/bin/env bash\necho hi\n", encoding="utf-8")
    entry = store.add_script(src, kind="shell", name="d", mode="reference")
    text = entry.script_path.read_text(encoding="utf-8")
    expected_fw = _spec("shell").analyzer.analyze(text).frameworks
    _fake_tty(monkeypatch)
    seen: dict[str, object] = {}
    monkeypatch.setattr(
        cli, "_print_reference_add_notice", lambda spec, txt, fw: seen.update(fw=fw)
    )
    assert cli._onboard_script_params(entry, _spec("shell"), no_input=False) == []
    assert seen["fw"] == expected_fw
    assert seen["fw"] is not None  # the nulled-frameworks mutant passes None


def test_onboard_script_params_reads_and_writes_the_copy_as_utf8(tmp_path, monkeypatch):
    """Both stored-copy reads (analyzer input + write-back) and the write use encoding="utf-8",
    while the write-back half uses surrogateescape so arbitrary source bytes round-trip."""
    _two_const_shell(tmp_path)
    entry = store.resolve("d")  # resolve before spying, so only the copy read/write are captured
    _fake_tty(monkeypatch)
    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(lambda *a, **k: "1"))  # pick one -> writes
    reads: list[dict[str, object]] = []
    writes: list[dict[str, object]] = []
    real_read = Path.read_text
    real_write = Path.write_text

    def read_spy(self, encoding=None, errors=None, *a, **k):
        reads.append({"encoding": encoding, "errors": errors})
        return real_read(self, *a, encoding=encoding, errors=errors, **k)

    def write_spy(self, data, encoding=None, errors=None, *a, **k):
        writes.append({"encoding": encoding, "errors": errors})
        return real_write(self, data, *a, encoding=encoding, errors=errors, **k)

    monkeypatch.setattr(Path, "read_text", read_spy)
    monkeypatch.setattr(Path, "write_text", write_spy)
    assert cli._onboard_script_params(entry, _spec("shell"), no_input=False) == ["WIDTH"]
    assert [r["encoding"] for r in reads] == ["utf-8", "utf-8"]  # analyzer read + write-back read
    assert [r["errors"] for r in reads] == ["replace", "surrogateescape"]
    assert [w["encoding"] for w in writes] == ["utf-8"]  # the single copy write
    assert [w["errors"] for w in writes] == ["surrogateescape"]


def test_onboard_script_params_preserves_non_utf8_source_bytes(tmp_path, monkeypatch):
    """Managing a candidate only inserts comments; it must not rewrite an unrelated raw byte."""
    source = tmp_path / "raw.sh"
    original = b"#!/bin/sh\nWIDTH=800\nprintf '\xff\\n'\n"
    source.write_bytes(original)
    entry = store.add_script(source, kind="shell", name="raw")
    _fake_tty(monkeypatch)
    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(lambda *a, **k: "1"))

    assert cli._onboard_script_params(entry, _spec("shell"), no_input=False) == ["WIDTH"]

    rewritten = entry.script_path.read_bytes()
    assert b"\xff" in rewritten
    assert b"\xef\xbf\xbd" not in rewritten  # UTF-8 encoding of U+FFFD
    assert b"[tool.skit]" in rewritten


# ---- _edit_params: the write-back half must be byte-lossless (the onboard idiom) ----------


def test_edit_params_write_back_is_byte_based_not_text_mode(tmp_path, monkeypatch):
    """`params --manage`'s write-back must be BYTE-based (read_bytes + write_bytes), like
    _normalize_params — never text-mode write_text, which re-expands \\n to os.linesep and would
    CRLF-ify the whole stored copy on Windows. The only read_text on the copy is the analysis
    read (errors="replace"); the write-back re-reads raw bytes and writes raw bytes, so a manage
    edit can neither bake U+FFFD over raw bytes nor rewrite line endings to the host default."""
    entry = _two_const_shell(tmp_path)
    text_reads: list[dict[str, object]] = []
    byte_reads: list[Path] = []
    byte_writes: list[bytes] = []
    text_writes: list[Path] = []  # the regression: write_text must never touch the stored copy
    real_read_text = Path.read_text
    real_read_bytes = Path.read_bytes
    real_write_bytes = Path.write_bytes
    real_write_text = Path.write_text

    def read_text_spy(self, encoding=None, errors=None, *a, **k):
        if self == entry.script_path:
            text_reads.append({"encoding": encoding, "errors": errors})
        return real_read_text(self, *a, encoding=encoding, errors=errors, **k)

    def read_bytes_spy(self):
        if self == entry.script_path:
            byte_reads.append(self)
        return real_read_bytes(self)

    def write_bytes_spy(self, data):
        if self == entry.script_path:
            byte_writes.append(data)
        return real_write_bytes(self, data)

    def write_text_spy(self, *a, **k):
        if self == entry.script_path:
            text_writes.append(self)
        return real_write_text(self, *a, **k)

    monkeypatch.setattr(Path, "read_text", read_text_spy)
    monkeypatch.setattr(Path, "read_bytes", read_bytes_spy)
    monkeypatch.setattr(Path, "write_bytes", write_bytes_spy)
    monkeypatch.setattr(Path, "write_text", write_text_spy)
    result = runner.invoke(cli.app, ["params", "d", "--manage", "WIDTH"])
    assert result.exit_code == 0, result.output
    assert [r["errors"] for r in text_reads] == ["replace"]  # analysis read only; no text re-read
    assert byte_reads  # the write-back re-read raw bytes
    assert len(byte_writes) == 1  # a single byte write-back
    assert b"[tool.skit]" in byte_writes[0]  # ...carrying the rewritten block
    assert text_writes == []  # write_text never touched the stored copy (no os.linesep expansion)


def test_edit_params_preserves_non_utf8_source_bytes(tmp_path):
    """A --manage edit only inserts the comment block: an unrelated raw byte elsewhere in the
    script must round-trip, never become U+FFFD (same guarantee as onboarding)."""
    source = tmp_path / "rawedit.sh"
    original = b"#!/bin/sh\nWIDTH=800\nprintf '\xff\\n'\n"
    source.write_bytes(original)
    entry = store.add_script(source, kind="shell", name="rawedit")
    result = runner.invoke(cli.app, ["params", "rawedit", "--manage", "WIDTH"])
    assert result.exit_code == 0, result.output
    rewritten = entry.script_path.read_bytes()
    assert b"\xff" in rewritten
    assert b"\xef\xbf\xbd" not in rewritten  # UTF-8 encoding of U+FFFD
    assert b"[tool.skit]" in rewritten


# ============================================================ run --forget-args


def test_run_forget_args_erases_remembered_tail(tmp_path, spawn_spy):
    entry = _py(tmp_path, name="a")
    argstate.save_last(entry.slug, extra_args=["--old", "value"])
    result = runner.invoke(cli.app, ["run", "a", "--forget-args", "--no-input"])
    assert result.exit_code == 0, result.output
    assert argstate.load_state(entry.slug)["extra_args"] == []  # erased up front
    assert spawn_spy["extra"] == []  # the run reused nothing


def test_run_forget_args_below_usage_gates_survives_a_refusal(tmp_path, spawn_spy):
    """--forget-args moved BELOW every usage gate: an exit-2 invocation must leave no
    fingerprints, so a refused run (here an unknown preset) leaves the remembered tail
    intact — the same "nothing is changed" rule the raw refusal states. The clean-erase
    twin above pins that a passing --forget-args run still wipes it."""
    entry = _py(tmp_path, name="a")
    argstate.save_last(entry.slug, extra_args=["--old", "value"])
    result = runner.invoke(cli.app, ["run", "a", "--forget-args", "--preset", "nope", "--no-input"])
    assert result.exit_code == 2, result.output  # unknown-preset usage error, before the clear
    assert argstate.load_state(entry.slug)["extra_args"] == ["--old", "value"]  # NOT erased
    assert "entry" not in spawn_spy  # nothing launched


# ============================================================ raw refusal


def test_run_raw_refused_on_placeholder_kind(tmp_path, spawn_spy):
    """On a kind whose {placeholders} ARE the artifact (here: a command template) there is
    no "as-is" to run — `--raw` is REFUSED (exit 2) with the placeholder-keyed message, not
    a notice-then-run-anyway. Nothing launches. The gate keys off placeholder_params (the
    interface trait), not an internal analyzer capability."""
    store.add_command("echo hi", name="cmd")
    result = runner.invoke(cli.app, ["run", "cmd", "--raw", "--no-input"])
    assert result.exit_code == 2, result.output
    flat = _flat(result.output)
    assert "--raw doesn't apply to a command entry" in flat
    assert "there is no as-is without them" in flat
    # A command template's grammar is SINGLE braces — the message speaks {placeholders}.
    assert "{placeholders}" in flat
    assert "{{" not in flat
    assert "entry" not in spawn_spy  # refused before any launch


def test_run_raw_allowed_on_injected_kind(tmp_path, spawn_spy):
    """The twin: on an injected kind (python) --raw stays a legitimate escape hatch — it
    runs the stored copy as-is, no refusal."""
    _py(tmp_path, name="job")
    result = runner.invoke(cli.app, ["run", "job", "--raw", "--no-input"])
    assert result.exit_code == 0, result.output
    assert spawn_spy["entry"].meta.name == "job"  # it really launched


def test_run_raw_runs_an_exe_with_the_skip_notice(tmp_path, spawn_spy):
    """A program (exe) has NO {placeholders} — it runs as-is exactly like python does.
    --raw now RUNS it (exit 0) with the "Raw mode: skipping…" notice, not a refusal: the
    gate flipped from the analyzer capability to the placeholder_params interface trait."""
    prog = tmp_path / "tool"
    prog.write_text("bytes\n", encoding="utf-8")
    store.add_exe(prog, name="tool")
    result = runner.invoke(cli.app, ["run", "tool", "--raw", "--no-input"])
    assert result.exit_code == 0, result.output
    assert "Raw mode: skipping the parameter form and injection." in _flat(result.output)
    assert spawn_spy["entry"].meta.name == "tool"  # it really launched


def test_run_raw_on_unpinned_prompt_refuses_before_runner_resolution(tmp_path, spawn_spy):
    """--raw on a prompt (its {placeholders} ARE the artifact) is refused with exit 2 BEFORE
    runner resolution: the refusal never first asks which agent (nor sends the caller through
    the unpinned 126), and last-picked state is NOT written by a refused run."""
    _prompt(tmp_path, name="p")  # no pin, no --runner
    result = runner.invoke(cli.app, ["run", "p", "--raw", "--no-input"])
    assert result.exit_code == 2, result.output  # usage, NOT the unpinned 126
    flat = _flat(result.output)
    assert "--raw doesn't apply to a prompt entry" in flat
    assert "there is no as-is without them" in flat
    # The prompt kind's real grammar is DOUBLE braces — the message must speak {{placeholders}}
    # (single braces here would contradict the syntax the user actually writes).
    assert "{{placeholders}}" in flat
    assert "entry" not in spawn_spy  # nothing launched
    # last-picked state must be untouched (a refused run picks nothing).
    assert not (argstate.state_dir() / "prompt.toml").exists()
    assert argstate.load_last_runner() == ""


def test_run_raw_on_unpinned_prompt_refuses_without_asking_when_interactive(tmp_path, monkeypatch):
    """Even in an interactive terminal with agents configured, the raw refusal fires
    before the runner ask — Prompt.ask is never reached."""
    _prompt(tmp_path, name="p")
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)

    def _boom(*a, **k):
        raise AssertionError("runner ask reached before the raw refusal")

    monkeypatch.setattr(cli.Prompt, "ask", _boom)
    result = runner.invoke(cli.app, ["run", "p", "--raw"])
    assert result.exit_code == 2, result.output
    assert argstate.load_last_runner() == ""  # no pick recorded


# ============================================================ doctor launch_blocked


def test_doctor_reports_launch_blocked(tmp_path, monkeypatch):
    _shell(tmp_path, name="blocked")
    monkeypatch.setattr("skit.langs.launch._which", lambda _name: None)
    j = runner.invoke(cli.app, ["doctor", "--json"])
    assert j.exit_code in (0, 1), j.output
    payload = json.loads(j.output)
    assert "blocked" in payload["launch_blocked"]
    human = runner.invoke(cli.app, ["doctor"])
    assert "a run would refuse to start" in human.output


# ============================================================ runner add --force


def test_runner_add_force_replaces_in_place(tmp_path):
    config.ensure_prompt_runners_seeded()
    before = [r.name for r in config.load_prompt_runners()]
    result = runner.invoke(
        cli.app, ["runner", "add", "codex", "--force", "--", "codex", "--model", "o1", "{{prompt}}"]
    )
    assert result.exit_code == 0, result.output
    assert [r.name for r in config.load_prompt_runners()] == before  # order held
    assert _find_runner("codex").argv == ("codex", "--model", "o1", "{{prompt}}")
    # Replacing says "updated", never "added" — the word must match the act.
    assert "Runner codex updated:" in result.output
    assert "added" not in result.output


def test_runner_add_fresh_says_added(tmp_path):
    config.ensure_prompt_runners_seeded()
    result = runner.invoke(cli.app, ["runner", "add", "sonnet", "--", "claude", "{{prompt}}"])
    assert result.exit_code == 0, result.output
    assert "Runner sonnet added:" in result.output  # a genuinely new row says "added"
    assert "updated" not in result.output


def test_runner_add_without_force_refuses_with_force_hint(tmp_path):
    config.ensure_prompt_runners_seeded()
    result = runner.invoke(cli.app, ["runner", "add", "codex", "--", "codex", "{{prompt}}"])
    assert result.exit_code == 1
    assert "--force" in result.output


# ============================================================ prompt run inline picker


def _prompt_run_interactive(monkeypatch):
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    config.save_form("tui")


def test_run_prompt_inline_picker_resolves_the_runner(tmp_path, spawn_spy, monkeypatch):
    _prompt(tmp_path)
    _prompt_run_interactive(monkeypatch)
    seen: dict[str, object] = {}

    def fake_collect(entry, plan, prefill, runners=None, runner_default=""):
        seen["runners"] = list(runners or [])
        seen["default"] = runner_default
        return {"a": "hi"}, "codex", True

    monkeypatch.setattr("skit.inlineform.collect", fake_collect)
    ask_hit = {"n": 0}
    monkeypatch.setattr(
        cli.Prompt, "ask", staticmethod(lambda *a, **k: ask_hit.__setitem__("n", 1))
    )
    result = runner.invoke(cli.app, ["run", "p"])
    assert result.exit_code == 0, result.output
    assert seen["runners"] == [r.name for r in config.load_prompt_runners()]
    assert ask_hit["n"] == 0  # never line-asked for a runner
    assert spawn_spy["runner"] == config.find_prompt_runner("codex")
    assert spawn_spy["values"] == {"a": "hi"}
    assert argstate.load_last_runner() == "codex"  # the pick was remembered


def test_run_prompt_inline_picker_none_falls_back(tmp_path, spawn_spy, monkeypatch):
    _prompt(tmp_path)
    _prompt_run_interactive(monkeypatch)
    monkeypatch.setattr("skit.inlineform.collect", lambda *a, **k: ({"a": "hi"}, None, False))
    called: dict[str, object] = {}

    def fake_resolve(entry, runner_flag, no_input):
        called["hit"] = True
        return config.find_prompt_runner("amp")

    monkeypatch.setattr(cli, "_resolve_run_runner", fake_resolve)
    result = runner.invoke(cli.app, ["run", "p"])
    assert result.exit_code == 0, result.output
    assert called.get("hit") is True  # degraded → deterministic line-mode resolution
    assert spawn_spy["runner"] == config.find_prompt_runner("amp")


def test_run_prompt_inline_picker_removed_runner_is_126(tmp_path, spawn_spy, monkeypatch):
    _prompt(tmp_path)
    _prompt_run_interactive(monkeypatch)
    monkeypatch.setattr(
        "skit.inlineform.collect", lambda *a, **k: ({"a": "hi"}, "ghostrunner", True)
    )
    result = runner.invoke(cli.app, ["run", "p"])
    assert result.exit_code == 126
    assert "ghostrunner" in result.output
    assert "entry" not in spawn_spy  # never launched


def test_run_prompt_inline_pin_left_untouched_is_not_a_pick(tmp_path, spawn_spy, monkeypatch):
    """A PINNED prompt whose form comes back with the pin unchanged did not "pick" a
    runner — the last-picked state that prefills future pickers stays put (argstate's
    contract: using a pin is not a pick)."""
    _prompt(tmp_path, pin="claude")
    _prompt_run_interactive(monkeypatch)
    monkeypatch.setattr("skit.inlineform.collect", lambda *a, **k: ({"a": "hi"}, "claude", False))
    result = runner.invoke(cli.app, ["run", "p"])
    assert result.exit_code == 0, result.output
    assert spawn_spy["runner"] == config.find_prompt_runner("claude")  # ran with the pin
    assert argstate.load_last_runner() == ""  # last-picked state untouched


def test_run_prompt_inline_pick_differing_from_pin_is_saved(tmp_path, spawn_spy, monkeypatch):
    """The twin: choosing a runner that DIFFERS from the pin is a real pick and IS
    remembered for the next picker."""
    _prompt(tmp_path, pin="claude")
    _prompt_run_interactive(monkeypatch)
    monkeypatch.setattr("skit.inlineform.collect", lambda *a, **k: ({"a": "hi"}, "codex", True))
    result = runner.invoke(cli.app, ["run", "p"])
    assert result.exit_code == 0, result.output
    assert spawn_spy["runner"] == config.find_prompt_runner("codex")
    assert argstate.load_last_runner() == "codex"  # a real change is remembered


def test_run_prompt_inline_move_away_then_back_to_pin_is_saved(tmp_path, spawn_spy, monkeypatch):
    """Final-value comparison loses an actual interaction when the user explores a
    runner and returns to the pin. The form's event bit, not inequality, is authoritative."""
    _prompt(tmp_path, pin="claude")
    argstate.save_last_runner("opencode")
    _prompt_run_interactive(monkeypatch)
    monkeypatch.setattr("skit.inlineform.collect", lambda *a, **k: ({"a": "hi"}, "claude", True))
    result = runner.invoke(cli.app, ["run", "p"])
    assert result.exit_code == 0, result.output
    assert spawn_spy["runner"] == config.find_prompt_runner("claude")
    assert argstate.load_last_runner() == "claude"


def test_run_prompt_dry_run_still_hosts_the_runner_picker(tmp_path, spawn_spy, monkeypatch):
    """--dry-run no longer forces a bare line ask: an interactive tui prompt dry-run hosts
    the runner picker in the form exactly like a real run, then prints the
    resolved command instead of launching."""
    _prompt(tmp_path)
    _prompt_run_interactive(monkeypatch)
    seen: dict[str, object] = {}

    def fake_collect(entry, plan, prefill, runners=None, runner_default=""):
        seen["runners"] = list(runners or [])
        return {"a": "hi"}, "codex", True

    monkeypatch.setattr("skit.inlineform.collect", fake_collect)
    ask_hit = {"n": 0}
    monkeypatch.setattr(
        cli.Prompt, "ask", staticmethod(lambda *a, **k: ask_hit.__setitem__("n", 1))
    )
    result = runner.invoke(cli.app, ["run", "p", "--dry-run"])
    assert result.exit_code == 0, result.output
    assert seen["runners"] == [r.name for r in config.load_prompt_runners()]  # picker hosted
    assert ask_hit["n"] == 0  # NOT line-asked, despite --dry-run
    assert "entry" not in spawn_spy  # dry-run launches nothing
    assert "codex" in result.output  # the resolved runner's real command is shown
