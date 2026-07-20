"""The prompt kind's core: analyzer, renderer, registry row, store, plan, launch.

CLI surfaces live in test_prompt_cli.py; TUI surfaces in test_prompt_tui.py. The golden
corpus under tests/corpus/prompt/ is byte-exact (CRLF, missing trailing newline, CJK,
emoji) and excluded from the pre-commit fixers like every other corpus directory.
"""

from __future__ import annotations

import contextlib
import os
import subprocess
import sys
import threading
import time
from pathlib import Path

import pytest

from skit import argstate, config, flows, i18n, launcher, store
from skit.langs import launch as langs_launch
from skit.langs.base import (
    ArgvLaunch,
    LaunchError,
    NotExecutableError,
    TargetMissingError,
)
from skit.langs.launch import PromptLaunch
from skit.langs.prompt import analyzer, render
from skit.langs.registry import infer_kind, spec_for
from skit.params import ParamDecl

CORPUS = Path(__file__).parent / "corpus" / "prompt"


def _write_prompt(tmp_path: Path, text: str, name: str = "p.prompt.md") -> Path:
    path = tmp_path / name
    # write_bytes, not write_text: text mode rewrites "\n" -> "\r\n" on Windows, which
    # the strict byte-fidelity reader then delivers verbatim — the body must be the
    # exact bytes the test names, on every platform.
    path.write_bytes(text.encode("utf-8"))
    return path


def _runner(name: str = "rec", argv: tuple[str, ...] = ("rec-bin", "{{prompt}}")):
    return config.PromptRunner(name, argv)


# --------------------------------------------------------------------------
# analyzer
# --------------------------------------------------------------------------


def test_placeholder_names_dedupes_in_body_order():
    text = "a {{b}} c {{a}} d {{b}} {{_x1}} {{9bad}} {{ spaced }} {{a-b}}"
    assert analyzer.placeholder_names(text) == ["b", "a", "_x1"]


def test_placeholder_names_single_braces_are_never_candidates():
    # The whole point of the double-brace grammar: code-shaped text stays quiet.
    text = 'JSON {"key": 1} f-string {value} shell ${HOME} empty {} plain {word}'
    assert analyzer.placeholder_names(text) == []


def test_placeholder_names_brace_adjacent_is_not_a_candidate():
    # A Handlebars triple-stache (and any brace-hugging shape) is someone else's syntax.
    assert analyzer.placeholder_names("{{{raw}}} and {{{x}} and {{y}}}") == []


def test_placeholder_names_reserved_name_excluded():
    assert analyzer.placeholder_names("{{prompt}} {{real}}") == ["real"]


def test_placeholder_names_accept_unicode_identifiers_and_reject_non_names():
    text = "{{任务}} {{café}} {{é}} {{not-a-name}} {{💥}} {{}}"
    assert analyzer.placeholder_names(text) == ["任务", "café", "é"]


def test_placeholder_names_high_cardinality_stays_ordered_and_complete():
    names = [f"field_{i}" for i in range(10_000)]
    text = " ".join("{{" + name + "}}" for name in names)
    assert analyzer.placeholder_names(text) == names


def test_prompt_grammar_is_independent_of_command_templates():
    # Deliberately NOT the command-template pattern: command {name} stays single-brace
    # (a shipped, shell-quoted surface); the prompt surface is double-brace with no
    # escapes. The two must not be conflated again.
    assert analyzer.TOKEN_RE.pattern != langs_launch._TEMPLATE_TOKEN_RE.pattern
    assert analyzer.placeholder_names("{name}") == []


def test_corpus_basic_detection_and_render_byte_identity():
    text = (CORPUS / "01_basic.prompt.md").read_bytes().decode("utf-8")
    assert analyzer.placeholder_names(text) == ["target", "focus", "x"]
    rendered = render.render_body(text, {"target": "T", "focus": "F"}, ["target", "focus"])
    # Managed holes filled; every literal shape — single braces, JSON, f-string,
    # triple-stache, the unmanaged {{x}} — arrives byte-identical.
    assert "Review T for F. Again: T." in rendered
    assert "Literals: {code} and JSON {\"key\": 1} and f'{value}' and {{{handlebars}}}" in rendered
    assert "Unmanaged hole: {{x}}" in rendered


def test_corpus_crlf_preserved_verbatim():
    raw = (CORPUS / "02_crlf.prompt.md").read_bytes()
    assert b"\r\n" in raw  # the corpus really is CRLF — fixers must not touch it
    text = raw.decode("utf-8")
    rendered = render.render_body(text, {"task": "X", "repo": "Y"}, ["task", "repo"])
    assert "\r\n" in rendered
    assert rendered == text.replace("{{task}}", "X").replace("{{repo}}", "Y")


def test_corpus_cjk_emoji_no_trailing_newline():
    raw = (CORPUS / "03_cjk_emoji.prompt.md").read_bytes()
    assert not raw.endswith(b"\n")  # deliberate: no trailing newline
    text = raw.decode("utf-8")
    assert analyzer.placeholder_names(text) == ["目標檔案", "focus"]
    rendered = render.render_body(
        text,
        {"目標檔案": "src/主程式.py", "focus": "效能"},
        ["目標檔案", "focus"],
    )
    assert "審查 src/主程式.py" in rendered
    assert "專注於 效能" in rendered
    assert not rendered.endswith("\n")


def test_corpus_reserved_prompt_stays_verbatim():
    text = (CORPUS / "05_reserved.prompt.md").read_bytes().decode("utf-8")
    assert analyzer.placeholder_names(text) == ["real"]
    rendered = render.render_body(text, {"real": "R"}, ["real"])
    assert "{{prompt}}\tliterally" in rendered


# --------------------------------------------------------------------------
# render
# --------------------------------------------------------------------------


def test_render_body_missing_managed_value_raises():
    with pytest.raises(LaunchError, match="target"):
        render.render_body("{target}", {}, ["target"])


def test_render_body_substitutes_raw_never_quotes():
    payload = '\'; rm -rf ~; $(touch pwned) `echo hi` "x" {inner} {{deep}}'
    rendered = render.render_body("V={{v}} end", {"v": payload}, ["v"])
    # Byte-identical payload: no quoting, and the replacement is never re-scanned.
    assert rendered == f"V={payload} end"


def test_render_body_empty_value_substitutes_empty():
    assert render.render_body("[{{v}}]", {"v": ""}, ["v"]) == "[]"


def test_fill_runner_argv_replaces_the_one_slot_raw():
    rendered = "line1\nline2 with {braces} and {{more}}"
    argv = render.fill_runner_argv(["agent", "--m={{prompt}}", "{lit}"], rendered)
    assert argv == ["agent", f"--m={rendered}", "{lit}"]


def test_fill_runner_argv_leaves_foreign_holes_verbatim():
    # Validation refuses a stray {{hole}} at save time; the renderer must still be
    # total — and single-brace text is a literal that never even matches.
    assert render.fill_runner_argv(["a", "{{other}}"], "X") == ["a", "{{other}}"]
    assert render.fill_runner_argv(["a", "{single}"], "X") == ["a", "{single}"]


def test_fill_runner_argv_puts_extra_options_before_end_of_options():
    assert render.fill_runner_argv(
        ["claude", "--", "{{prompt}}"], "--help", ["--model", "opus"]
    ) == ["claude", "--model", "opus", "--", "--help"]
    # Flag-delivered/custom runners without a delimiter retain the historical append.
    assert render.fill_runner_argv(["agent", "--prompt={{prompt}}"], "task", ["--verbose"]) == [
        "agent",
        "--prompt=task",
        "--verbose",
    ]
    # A valid custom template may put its delimiter after the prompt slot. Extras are
    # still agent options, so the first literal delimiter anywhere owns insertion.
    assert render.fill_runner_argv(
        ["agent", "--prompt", "{{prompt}}", "--", "literal", "--"],
        "task",
        ["--model", "opus"],
    ) == ["agent", "--prompt", "task", "--model", "opus", "--", "literal", "--"]
    # Delimiter-looking text inside a token is not the argv boundary.
    assert render.fill_runner_argv(["agent", "--marker=--", "{{prompt}}"], "task", ["--fast"]) == [
        "agent",
        "--marker=--",
        "task",
        "--fast",
    ]


def test_check_argv_length_refuses_over_limit():
    render.check_argv_length(["x" * 100])
    with pytest.raises(LaunchError, match=str(render.ARGV_LIMIT)):
        render.check_argv_length(["x" * (render.ARGV_LIMIT + 1)])


def test_check_argv_length_measures_bytes_not_characters():
    # A CJK char is multiple bytes on every platform's measure (3 in UTF-8 on POSIX, 2
    # in UTF-16 on Windows), so the OS byte bound is what matters. //2 + 10 chars stays
    # under the character count on both while its byte measure overflows both.
    cjk = "中" * (render.ARGV_LIMIT // 2 + 10)
    assert len(cjk) < render.ARGV_LIMIT  # passes a character count…
    with pytest.raises(LaunchError):  # …but not the byte count
        render.check_argv_length([cjk])


@pytest.mark.skipif(os.name == "nt", reason="POSIX argv uses filesystem encoding")
def test_check_argv_length_accepts_surrogateescaped_os_bytes(monkeypatch):
    token = os.fsdecode(b"\xff")
    monkeypatch.setattr(render, "ARGV_LIMIT", 2)  # one byte plus argv's NUL terminator

    render.check_argv_length([token])


@pytest.mark.skipif(os.name == "nt", reason="POSIX argv uses filesystem encoding")
def test_check_argv_length_refuses_unencodable_surrogate_cleanly():
    with pytest.raises(LaunchError, match="can't encode") as caught:
        render.check_argv_length(["\ud800"])

    assert isinstance(caught.value.__cause__, UnicodeEncodeError)


@pytest.mark.skipif(os.name == "nt", reason="POSIX permits arbitrary non-NUL argv bytes")
def test_surrogateescaped_value_reaches_a_real_child_as_the_original_byte(tmp_path):
    entry = store.add_prompt(_write_prompt(tmp_path, "{{value}}"), managed=["value"])
    recorder = config.PromptRunner(
        "bytes",
        (
            sys.executable,
            "-c",
            "import os,sys; os.write(1, os.fsencode(sys.argv[1]))",
            "{{prompt}}",
        ),
    )

    argv = launcher.build_command(entry, values={"value": os.fsdecode(b"\xff")}, runner=recorder)
    completed = subprocess.run(argv, capture_output=True, check=False)

    assert completed.returncode == 0
    assert completed.stdout == b"\xff"


def test_check_argv_length_measures_windows_quoted_utf16(monkeypatch):
    monkeypatch.setattr(render.sys, "platform", "win32")
    monkeypatch.setattr(render, "ARGV_LIMIT", 60_000)
    # Raw UTF-8 is only ~20 KiB, but list2cmdline must double every backslash before
    # each quote; the actual CreateProcessW command line is ~80 KiB in UTF-16LE.
    argv = ["agent", '\\"' * 10_000]
    assert sum(len(token.encode("utf-8")) for token in argv) < render.ARGV_LIMIT
    with pytest.raises(LaunchError, match=str(render.ARGV_LIMIT)):
        render.check_argv_length(argv)


def test_check_argv_length_refuses_nul_before_subprocess():
    with pytest.raises(LaunchError, match="NUL byte"):
        render.check_argv_length(["agent", "before\x00after"])


# --------------------------------------------------------------------------
# registry: the spec row + compound-suffix inference
# --------------------------------------------------------------------------


def test_prompt_spec_shape():
    spec = spec_for("prompt")
    assert spec is not None
    assert spec.family == "interpreted"  # has_original_file must stay True
    assert spec.has_original_file
    assert spec.stored_name == "prompt.md"
    assert spec.editable
    assert spec.supports_modes
    assert not spec.takes_argv
    assert spec.placeholder_params
    assert spec.analyzer is None  # command-kind parity (raw mode, params surfaces)
    assert spec.params_io is None


def test_command_spec_carries_the_placeholder_trait():
    spec = spec_for("command")
    assert spec is not None
    assert spec.placeholder_params


def test_infer_kind_compound_suffix():
    assert infer_kind(Path("notes/review.prompt.md")) == "prompt"
    assert infer_kind(Path("REVIEW.PROMPT.MD")) == "prompt"
    assert infer_kind(Path("x.prompt")) == "prompt"
    assert infer_kind(Path("notes.md")) == "unknown"
    # Single-suffix kinds are untouched, and ".mts" never bleeds into ".ts" handling.
    assert infer_kind(Path("a.mts")) == "ts"
    assert infer_kind(Path("b.sh")) == "shell"


# --------------------------------------------------------------------------
# store: add_prompt / managed / runner / workdir pin
# --------------------------------------------------------------------------


def test_add_prompt_manages_all_detected_by_default(tmp_path: Path):
    src = _write_prompt(tmp_path, "# T\n\nDo {{a}} then {{b}}. Sample {{a}}.\n")
    entry = store.add_prompt(src)
    assert entry.meta.kind == "prompt"
    assert entry.meta.params == ["a", "b"]
    assert entry.meta.workdir == "invoke"
    assert entry.meta.description == "T"
    assert entry.meta.runner == ""
    assert (entry.dir / "prompt.md").read_text() == src.read_text()


def test_add_prompt_managed_subset_keeps_body_order(tmp_path: Path):
    src = _write_prompt(tmp_path, "{{a}} {{b}} {{c}}\n")
    entry = store.add_prompt(src, managed=["c", "a"])
    assert entry.meta.params == ["a", "c"]  # body order, whatever the caller's order


def test_add_prompt_refuses_unknown_managed_name(tmp_path: Path):
    src = _write_prompt(tmp_path, "{{a}}\n")
    with pytest.raises(store.StoreError, match="ghost"):
        store.add_prompt(src, managed=["ghost"])


def test_add_prompt_reference_mode_still_pins_invoke_workdir(tmp_path: Path):
    src = _write_prompt(tmp_path, "hello {{x}}\n")
    entry = store.add_prompt(src, mode="reference")
    assert entry.meta.mode == "reference"
    assert entry.meta.workdir == "invoke"  # never the prompt file's directory
    assert entry.script_path == src


def test_add_prompt_name_strips_double_extension(tmp_path: Path):
    entry = store.add_prompt(_write_prompt(tmp_path, "x\n", name="review.prompt.md"))
    assert entry.meta.name == "review"


def test_add_prompt_missing_file(tmp_path: Path):
    with pytest.raises(store.StoreError, match="File not found"):
        store.add_prompt(tmp_path / "ghost.prompt.md")


def test_prompt_description_takes_first_line_minus_heading():
    assert store.prompt_description("\n\n## A title ##\nbody") == "A title ##"
    assert store.prompt_description("plain line\n") == "plain line"
    assert store.prompt_description("\n\n") == ""


def test_prompt_description_caps_derived_metadata_without_breaking_unicode():
    limit = store._PROMPT_DESCRIPTION_LIMIT
    assert limit == 120
    exact = "界" * (limit - 1) + "🙂"
    assert len(exact) == limit
    assert store.prompt_description(f"# {exact}\nbody") == exact

    over = exact + "尾"
    assert store.prompt_description(over) == exact[:-1] + "…"
    assert len(store.prompt_description(over)) == limit

    huge = "提示🙂" * 40_000
    derived = store.prompt_description(huge)
    assert len(derived) == limit
    assert derived.endswith("…")
    assert derived == huge[: limit - 1] + "…"


def test_write_prompt_managed_and_runner_roundtrip(tmp_path: Path):
    entry = store.add_prompt(_write_prompt(tmp_path, "{{a}} {{b}}\n"))
    store.write_prompt_managed(entry.slug, ["b"])
    store.write_prompt_runner(entry.slug, "claude")
    reloaded = store.resolve(entry.slug)
    assert reloaded.meta.params == ["b"]
    assert reloaded.meta.runner == "claude"
    store.write_prompt_managed(entry.slug, [])
    store.write_prompt_runner(entry.slug, "")
    reloaded = store.resolve(entry.slug)
    assert reloaded.meta.params is None
    assert reloaded.meta.runner == ""


def test_prompt_entries_pinned_to_filters_by_kind_and_runner(tmp_path: Path):
    first = store.add_prompt(_write_prompt(tmp_path, "a\n", name="first.prompt.md"))
    second = store.add_prompt(_write_prompt(tmp_path, "b\n", name="second.prompt.md"))
    unpinned = store.add_prompt(_write_prompt(tmp_path, "c\n", name="third.prompt.md"))
    store.write_prompt_runner(first.slug, "claude")
    store.write_prompt_runner(second.slug, "codex")
    non_prompt = store.add_command("echo ok", name="not-a-prompt")
    # A hand-edited irrelevant key must not make a command count as a prompt pin.
    non_prompt.meta.runner = "claude"
    store._write_meta(non_prompt.dir, non_prompt.meta)

    assert [entry.slug for entry in store.prompt_entries_pinned_to("claude")] == [first.slug]
    assert unpinned not in store.prompt_entries_pinned_to("claude")


def test_write_prompt_helpers_refuse_non_prompt(tmp_path: Path):
    entry = store.add_command("echo {x}", name="cmd")
    with pytest.raises(store.StoreUsageError):
        store.write_prompt_managed(entry.slug, ["x"])
    with pytest.raises(store.StoreUsageError):
        store.write_prompt_runner(entry.slug, "claude")


def test_add_script_explicit_workdir_wins_in_reference_mode(tmp_path: Path):
    # The docs/design/prompt.md amendment: an explicit workdir beats the reference
    # default; callers that pass none keep the historical origin default byte-for-byte.
    script = tmp_path / "s.sh"
    script.write_text("#!/bin/bash\necho hi\n")
    with_default = store.add_script(script, kind="shell", mode="reference", name="d1")
    assert with_default.meta.workdir == "origin"
    explicit = store.add_script(script, kind="shell", mode="reference", name="d2", workdir="invoke")
    assert explicit.meta.workdir == "invoke"


# --------------------------------------------------------------------------
# flows: the placeholder body plan
# --------------------------------------------------------------------------


def test_prompt_plan_fields_follow_managed_list(tmp_path: Path):
    entry = store.add_prompt(
        _write_prompt(tmp_path, "{{a}} {{api_key}} {{skip}}\n"), managed=["a", "api_key"]
    )
    plan = flows.plan_for_entry(entry)
    assert plan.source == "command"
    assert [f.key for f in plan.fields] == ["a", "api_key"]
    assert all(f.source == "placeholder" and f.required for f in plan.fields)
    # The secret name heuristic applies to synthesized placeholders (C3, every source).
    assert plan.fields[1].secret
    assert not plan.drift_lines


def test_prompt_plan_reports_drift_for_gone_managed_names(tmp_path: Path):
    entry = store.add_prompt(_write_prompt(tmp_path, "{{a}} {{b}}\n"))
    entry.script_path.write_text("only {{a}} now\n", encoding="utf-8")
    plan = flows.plan_for_entry(entry)
    assert [f.key for f in plan.fields] == ["a", "b"]  # the record stays visible
    assert len(plan.drift_lines) == 1
    assert "b" in plan.drift_lines[0]


def test_prompt_plan_declared_rows_enrich_schema_and_env_riders_ride(tmp_path: Path):
    entry = store.add_prompt(_write_prompt(tmp_path, "{{n}}\n"))
    store.write_parameters(
        entry.slug,
        [
            ParamDecl(name="n", delivery="placeholder", type="int", default=3, required=False),
            ParamDecl(name="EXTRA", delivery="env"),
        ],
    )
    plan = flows.plan_for_entry(store.resolve(entry.slug))
    assert [(f.key, f.source, f.kind) for f in plan.fields] == [
        ("n", "placeholder", "int"),
        ("EXTRA", "env", "str"),
    ]
    assert plan.fields[0].default == "3"
    assert not plan.fields[0].required


def test_prompt_plan_unreadable_body_degrades_to_none_plan(tmp_path: Path):
    entry = store.add_prompt(_write_prompt(tmp_path, "{{a}}\n"))
    entry.script_path.unlink()
    plan = flows.plan_for_entry(entry)
    assert plan.source == "none"
    assert not plan.fields


def test_command_plan_is_unaffected_by_the_trait_refactor(tmp_path: Path):
    # The regression pin: the command kind's plan comes out byte-for-byte the same
    # (same source tag, same synthesized fields) after the placeholder_params gate.
    entry = store.add_command("convert {size} {out}", name="conv")
    plan = flows.plan_for_entry(entry)
    assert plan.source == "command"
    assert [f.key for f in plan.fields] == ["size", "out"]
    assert all(f.source == "placeholder" and f.required for f in plan.fields)
    assert plan.text == ""  # commands carry no body text on the plan


# --------------------------------------------------------------------------
# PromptLaunch
# --------------------------------------------------------------------------


def _entry_with_runner(tmp_path, monkeypatch, text="Do {{a}}\n", pin="", managed=None):
    entry = store.add_prompt(_write_prompt(tmp_path, text), managed=managed)
    if pin:
        entry = store.write_prompt_runner(entry.slug, pin)
    monkeypatch.setattr(langs_launch, "_which", lambda name: f"/bin/{name}")
    return entry


def test_build_renders_two_stages_and_appends_extra(tmp_path, monkeypatch):
    entry = _entry_with_runner(tmp_path, monkeypatch)
    payload = PromptLaunch().build(entry, ["--model", "opus"], {"a": "X"}, None, runner=_runner())
    assert isinstance(payload, ArgvLaunch)
    assert payload.argv == ["/bin/rec-bin", "Do X\n", "--model", "opus"]


def test_seeded_positional_runner_protects_dash_prefixed_prompt_and_keeps_extra(
    tmp_path, monkeypatch
):
    entry = _entry_with_runner(tmp_path, monkeypatch, text="--help", pin="claude", managed=[])
    payload = PromptLaunch().build(entry, ["--model", "opus"], {}, None)
    assert isinstance(payload, ArgvLaunch)
    assert payload.argv == ["/bin/claude", "--model", "opus", "--", "--help"]


def test_seeded_opencode_binds_dash_prefixed_prompt_and_keeps_extra(tmp_path, monkeypatch):
    entry = _entry_with_runner(tmp_path, monkeypatch, text="--version", pin="opencode", managed=[])
    payload = PromptLaunch().build(entry, ["--model", "provider/model"], {}, None)
    assert isinstance(payload, ArgvLaunch)
    assert payload.argv == [
        "/bin/opencode",
        "--prompt=--version",
        "--model",
        "provider/model",
    ]


def test_build_refuses_nul_in_prompt_as_launch_error(tmp_path, monkeypatch):
    entry = _entry_with_runner(tmp_path, monkeypatch, text="bad\x00prompt", managed=[])
    with pytest.raises(LaunchError, match="NUL byte"):
        PromptLaunch().build(entry, [], {}, None, runner=_runner())


def test_build_resolves_the_pin_when_no_override_is_given(tmp_path, monkeypatch):
    entry = _entry_with_runner(tmp_path, monkeypatch, pin="claude")
    monkeypatch.setattr(
        config,
        "load_prompt_runners",
        lambda: [config.PromptRunner("claude", ("claude", "{{prompt}}"))],
    )
    payload = PromptLaunch().build(entry, [], {"a": "1"}, None)
    assert isinstance(payload, ArgvLaunch)
    assert payload.argv == ["/bin/claude", "Do 1\n"]


def test_build_without_pin_or_override_is_exit_126(tmp_path, monkeypatch):
    entry = _entry_with_runner(tmp_path, monkeypatch)
    with pytest.raises(NotExecutableError, match="No runner selected"):
        PromptLaunch().build(entry, [], {"a": "1"}, None)


def test_build_with_unconfigured_pin_is_exit_126(tmp_path, monkeypatch):
    entry = _entry_with_runner(tmp_path, monkeypatch, pin="ghost")
    monkeypatch.setattr(config, "load_prompt_runners", lambda: [])
    with pytest.raises(NotExecutableError, match="ghost"):
        PromptLaunch().build(entry, [], {"a": "1"}, None)


def test_build_missing_binary_is_exit_126(tmp_path, monkeypatch):
    entry = store.add_prompt(_write_prompt(tmp_path, "Do {{a}}\n"))
    monkeypatch.setattr(langs_launch, "_which", lambda name: None)
    with pytest.raises(NotExecutableError, match="rec-bin"):
        PromptLaunch().build(entry, [], {"a": "1"}, None, runner=_runner())


def test_build_missing_body_is_exit_127(tmp_path, monkeypatch):
    entry = _entry_with_runner(tmp_path, monkeypatch)
    entry.script_path.unlink()
    with pytest.raises(TargetMissingError):
        PromptLaunch().build(entry, [], {"a": "1"}, None, runner=_runner())


def test_build_over_long_render_is_a_clean_launch_error(tmp_path, monkeypatch):
    entry = _entry_with_runner(tmp_path, monkeypatch)
    # The size is measured (and worded) in BYTES, not characters — the limit is an OS
    # argv byte cap, so the message must say "bytes"/"-byte limit", never "characters".
    with pytest.raises(LaunchError, match=r"bytes.*-byte limit") as excinfo:
        PromptLaunch().build(
            entry, [], {"a": "x" * (render.ARGV_LIMIT + 10)}, None, runner=_runner()
        )
    assert "characters" not in str(excinfo.value)


def test_build_script_override_reads_the_override(tmp_path, monkeypatch):
    entry = _entry_with_runner(tmp_path, monkeypatch)
    override = tmp_path / "other.md"
    override.write_text("Other {{a}}!", encoding="utf-8")
    payload = PromptLaunch().build(entry, [], {"a": "Z"}, override, runner=_runner())
    assert isinstance(payload, ArgvLaunch)
    assert payload.argv[1] == "Other Z!"


def test_describe_with_runner_shows_the_real_argv(tmp_path, monkeypatch):
    entry = _entry_with_runner(tmp_path, monkeypatch)
    shown = PromptLaunch().describe(entry, [], {"a": "•••"}, None, runner=_runner())
    assert "rec-bin" in shown
    assert "•••" in shown


def test_validate_argv_without_a_display_twin_returns_the_real_prompt(tmp_path, monkeypatch):
    """Callers that do not request masking get the validated configured argv itself."""
    entry = _entry_with_runner(tmp_path, monkeypatch)
    shown = PromptLaunch().validate_argv(
        entry,
        ["--model", "fast"],
        {"a": "actual-value"},
        None,
        runner=_runner(),
    )
    assert "rec-bin" in shown
    assert "actual-value" in shown
    assert "--model fast" in shown


def test_describe_resolves_a_pinned_multi_token_runner(tmp_path, monkeypatch):
    # The transparency contract on the TUI rerun path: a pinned run arrives with
    # runner=None, and the line must still show the runner's REAL flags (the opencode
    # seed is three tokens), not a two-token stub. Reading config is safe — loading
    # never seeds (materializing is the management surface's act).
    entry = _entry_with_runner(tmp_path, monkeypatch, pin="opencode")
    shown = PromptLaunch().describe(entry, [], {"a": "1"}, None)
    assert "--prompt" in shown  # the seed's real flag, not just the name
    assert "Do 1" in shown


def test_describe_unresolvable_pin_degrades_to_the_name_stub(tmp_path, monkeypatch):
    entry = _entry_with_runner(tmp_path, monkeypatch, pin="ghost")
    monkeypatch.setattr(config, "load_prompt_runners", lambda: [])
    shown = PromptLaunch().describe(entry, [], {"a": "1"}, None)
    assert "ghost" in shown  # the pin's NAME stands in when its row is gone
    assert "Do 1" in shown


def test_describe_with_no_pin_and_no_runner_never_reads_config(tmp_path, monkeypatch):
    entry = _entry_with_runner(tmp_path, monkeypatch)
    monkeypatch.setattr(
        config,
        "load_prompt_runners",
        lambda: pytest.fail("an unpinned describe has no reason to touch config"),
    )
    shown = PromptLaunch().describe(entry, [], {"a": "1"}, None)
    assert "?" in shown


def test_describe_degrades_on_missing_body_and_missing_values(tmp_path, monkeypatch):
    entry = _entry_with_runner(tmp_path, monkeypatch)
    assert "{{prompt}}" in PromptLaunch().describe(entry, [], {}, None, runner=_runner())
    entry.script_path.unlink()
    shown = PromptLaunch().describe(entry, [], {"a": "1"}, None, runner=_runner())
    assert "rec-bin" in shown
    assert "{{prompt}}" in shown


def test_preflight_checks_the_pin_only(tmp_path, monkeypatch):
    strategy = PromptLaunch()
    unpinned = _entry_with_runner(tmp_path, monkeypatch)
    strategy.preflight(unpinned)  # no pin: nothing to validate — the form asks
    pinned = store.write_prompt_runner(unpinned.slug, "claude")
    monkeypatch.setattr(
        config,
        "load_prompt_runners",
        lambda: [config.PromptRunner("claude", ("claude", "{{prompt}}"))],
    )
    strategy.preflight(pinned)
    monkeypatch.setattr(langs_launch, "_which", lambda name: None)
    with pytest.raises(NotExecutableError):
        strategy.preflight(pinned)


def test_preflight_explicit_runner_overrides_a_stale_pin(tmp_path, monkeypatch):
    entry = _entry_with_runner(tmp_path, monkeypatch, pin="removed")
    config.save_prompt_runners([_runner("working", ("working-bin", "{{prompt}}"))])

    # The run form resolved a configured replacement.  Preflight must validate that
    # actual choice, not consult the stale entry pin and veto the override.
    PromptLaunch().preflight(entry, runner=config.find_prompt_runner("working"))


def test_preflight_missing_body(tmp_path, monkeypatch):
    entry = _entry_with_runner(tmp_path, monkeypatch)
    entry.script_path.unlink()
    with pytest.raises(TargetMissingError):
        PromptLaunch().preflight(entry)


def test_target_is_the_prompt_body(tmp_path, monkeypatch):
    entry = _entry_with_runner(tmp_path, monkeypatch)
    assert PromptLaunch().target(entry) == entry.script_path


def test_run_entry_preserves_crlf_bodies_byte_for_byte(tmp_path):
    # Through the REAL read+render+spawn path: read_text's universal-newline mode would
    # quietly rewrite CRLF to LF, so build reads bytes (design risk #5).
    import json as _json
    import sys

    recorder = tmp_path / "recorder.py"
    recorder.write_text(
        "import json, sys\n"
        "from pathlib import Path\n"
        "Path(sys.argv[1]).write_text(json.dumps(sys.argv[2:]), encoding='utf-8')\n",
        encoding="utf-8",
    )
    out = tmp_path / "captured.json"
    raw = (CORPUS / "02_crlf.prompt.md").read_bytes()
    src = tmp_path / "crlf.prompt.md"
    src.write_bytes(raw)
    entry = store.add_prompt(src)
    runner = config.PromptRunner("rec", (sys.executable, str(recorder), str(out), "{{prompt}}"))
    assert launcher.run_entry(entry, [], values={"task": "T", "repo": "R"}, runner=runner) == 0
    (captured,) = _json.loads(out.read_text(encoding="utf-8"))
    expected = raw.decode("utf-8").replace("{{task}}", "T").replace("{{repo}}", "R")
    assert captured == expected
    assert "\r\n" in captured


def test_run_entry_executes_the_recorder_end_to_end(tmp_path):
    # The full no-shell contract, through the REAL spawn path: a corpus injection
    # payload arrives byte-identical as ONE argv element and nothing executes.
    import json as _json
    import sys

    recorder = tmp_path / "recorder.py"
    recorder.write_text(
        "import json, sys\n"
        "from pathlib import Path\n"
        "Path(sys.argv[1]).write_text(json.dumps(sys.argv[2:]), encoding='utf-8')\n",
        encoding="utf-8",
    )
    out = tmp_path / "captured.json"
    text = (CORPUS / "04_injection.prompt.md").read_bytes().decode("utf-8")
    entry = store.add_prompt(_write_prompt(tmp_path, text))
    runner = config.PromptRunner("rec", (sys.executable, str(recorder), str(out), "{{prompt}}"))
    code = launcher.run_entry(entry, [], values={"path": "src/x.py"}, runner=runner)
    assert code == 0
    captured = _json.loads(out.read_text(encoding="utf-8"))
    assert captured == [text.replace("{{path}}", "src/x.py")]
    assert not (tmp_path / "pwned").exists()  # $(touch pwned) never ran


# --------------------------------------------------------------------------
# config: the runner registry
# --------------------------------------------------------------------------


def test_validate_prompt_runner_argv_rules():
    ok = config.validate_prompt_runner_argv
    assert ok(["claude", "{{prompt}}"]) is None
    assert ok(["a", "--m={{prompt}}"]) is None
    assert ok(["a", "{lit}", "{{prompt}}"]) is None  # single braces are literals
    assert ok(["a", "{lit} {{prompt}}"]) is None  # literal AND slot in the SAME token
    assert ok([]) == "empty"
    assert ok([""]) == "empty"
    assert ok(["claude"]) == "prompt-slot-count"
    assert ok(["a", "{{prompt}}", "{{prompt}}"]) == "prompt-slot-count"
    assert ok(["{{prompt}}"]) == "prompt-in-binary"
    assert ok(["a", "{{other}}"]) == "stray-hole"
    assert ok(["a", "{{占位符}}", "{{prompt}}"]) == "stray-hole"
    assert ok(["a", "{{not-a-name}}", "{{prompt}}"]) == "stray-hole"
    assert ok(["a", "{{💥}}", "{{prompt}}"]) == "stray-hole"


def test_load_prompt_runners_is_read_only_before_seeding(tmp_path):
    assert not config.prompt_runners_seeded()
    raw_rows = config.prompt_runner_rows()
    assert [row.name for row in raw_rows] == [
        "claude",
        "codex",
        "opencode",
        "amp",
        "antigravity",
    ]
    assert all(config.prompt_runner_row_reason(row) == "valid" for row in raw_rows)
    runners = config.load_prompt_runners()
    assert [r.name for r in runners] == ["claude", "codex", "opencode", "amp", "antigravity"]
    assert config.find_prompt_runner("antigravity") == config.PromptRunner(
        "antigravity", ("agy", "--prompt-interactive", "{{prompt}}")
    )
    assert config.find_prompt_runner("opencode") == config.PromptRunner(
        "opencode", ("opencode", "--prompt={{prompt}}")
    )
    assert not config.prompt_runners_seeded()  # reading never wrote


def test_ensure_seeded_materializes_once_and_empty_stays_empty():
    config.ensure_prompt_runners_seeded()
    assert config.prompt_runners_seeded()
    assert "runners" in config.load_config()["prompt"]
    config.save_prompt_runners([])
    config.ensure_prompt_runners_seeded()  # must NOT resurrect the seeds
    assert config.load_prompt_runners() == []


def _save_barrier(monkeypatch: pytest.MonkeyPatch) -> None:
    """Force two old-snapshot writers to meet; a real transaction lock makes the
    first wait time out before the second can enter, then both preserve each other."""
    real_save = config.save_config
    barrier = threading.Barrier(2, timeout=0.2)

    def synchronized_save(doc) -> None:
        with contextlib.suppress(threading.BrokenBarrierError):
            barrier.wait()
        real_save(doc)

    monkeypatch.setattr(config, "save_config", synchronized_save)


def _run_threads(*targets) -> None:
    errors: list[BaseException] = []

    def guarded(target) -> None:
        try:
            target()
        except BaseException as exc:
            errors.append(exc)

    threads = [threading.Thread(target=guarded, args=(target,)) for target in targets]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join(timeout=3)
    assert not any(thread.is_alive() for thread in threads)
    assert errors == []


def test_runner_targeted_transactions_do_not_lose_concurrent_distinct_adds(monkeypatch):
    """Both setters used to load [] before either save; atomic replace then preserved
    only whichever one wrote last. The lock must cover reload through final replace."""
    config.save_prompt_runners([])
    _save_barrier(monkeypatch)

    _run_threads(
        lambda: config.set_prompt_runner(_runner("one", ("one", "{{prompt}}"))),
        lambda: config.set_prompt_runner(_runner("two", ("two", "{{prompt}}"))),
    )

    assert {runner.name for runner in config.load_prompt_runners()} == {"one", "two"}


def test_runner_transaction_and_non_runner_config_update_preserve_each_other(monkeypatch):
    """The lock is config-wide, not runner-private: editor/mirror/form/etc. writers
    must not erase a runner mutation that committed while they held an old snapshot."""
    config.save_prompt_runners([])
    _save_barrier(monkeypatch)

    _run_threads(
        lambda: config.set_prompt_runner(_runner("agent", ("agent", "{{prompt}}"))),
        lambda: config.save_editor("code --wait"),
    )

    assert config.find_prompt_runner("agent") is not None
    assert config.load_editor() == "code --wait"


def test_runner_transaction_and_i18n_update_share_the_neutral_config_lock(monkeypatch):
    """i18n cannot import config without a cycle, but its RMW must honor the same
    config.lock or a language change can still erase a concurrent runner add."""
    from skit import atomic

    config.save_prompt_runners([])
    real_write = atomic.atomic_write_toml
    barrier = threading.Barrier(2, timeout=0.2)

    def synchronized_write(path: Path, doc) -> None:
        with contextlib.suppress(threading.BrokenBarrierError):
            barrier.wait()
        real_write(path, doc)

    monkeypatch.setattr(config, "atomic_write_toml", synchronized_write)
    monkeypatch.setattr(i18n, "atomic_write_toml", synchronized_write)

    _run_threads(
        lambda: config.set_prompt_runner(_runner("agent", ("agent", "{{prompt}}"))),
        lambda: i18n.set_language("zh-TW"),
    )

    assert config.find_prompt_runner("agent") is not None
    assert config.load_config()["language"] == "zh-TW"


def test_config_lock_serializes_a_real_subprocess(tmp_path: Path):
    """The lockfile is process-wide, not merely a Python threading lock."""
    attempted = tmp_path / "attempted"
    acquired = tmp_path / "acquired"
    code = (
        "from pathlib import Path; from skit import config; "
        f"Path({str(attempted)!r}).write_text('yes'); "
        "ctx = config._config_lock(); ctx.__enter__(); "
        f"Path({str(acquired)!r}).write_text('yes'); ctx.__exit__(None, None, None)"
    )
    process: subprocess.Popen[bytes] | None = None
    try:
        with config._config_lock():
            process = subprocess.Popen([sys.executable, "-c", code])
            deadline = time.monotonic() + 3
            while not attempted.exists() and time.monotonic() < deadline:
                time.sleep(0.01)
            assert attempted.exists()
            assert not acquired.exists()
            assert process.poll() is None
        assert process.wait(timeout=3) == 0
    finally:
        if process is not None and process.poll() is None:
            process.kill()
            process.wait()
    assert acquired.read_text() == "yes"


def test_marker_alone_counts_as_seeded_and_stays_empty():
    # A hand-written marker with NO rows means "deliberately empty" — the seeds must
    # not resurrect just because the runners key is absent.
    config.save_config({"prompt": {"runners_seeded": True}})
    assert config.prompt_runners_seeded()
    assert config.load_prompt_runners() == []


def test_hand_authored_rows_without_marker_count_as_seeded():
    config.save_config({"prompt": {"runners": [{"name": "mine", "argv": ["m", "{{prompt}}"]}]}})
    assert config.prompt_runners_seeded()
    assert [r.name for r in config.load_prompt_runners()] == ["mine"]


def test_malformed_runner_rows_are_skipped_and_reported():
    config.save_config(
        {
            "prompt": {
                "runners_seeded": True,
                "runners": [
                    {"name": "good", "argv": ["g", "{{prompt}}"]},
                    {"name": "bad-no-slot", "argv": ["g"]},
                    {"name": "", "argv": ["g", "{{prompt}}"]},
                    {"name": "bad-argv", "argv": "not-a-list"},
                    {"name": "bad-token-type", "argv": ["g", 3]},
                    "not-a-table",
                ],
            }
        }
    )
    assert [r.name for r in config.load_prompt_runners()] == ["good"]
    rows = config.prompt_runner_rows()
    assert rows[2].name == ""
    assert rows[2].argv == ("g", "{{prompt}}")  # invalid name must not hide usable argv
    assert rows[2].descriptor.startswith("{")  # a whitespace name is not an invisible label
    reported = config.invalid_prompt_runners()
    assert "bad-no-slot" in reported
    assert len(reported) == 5


def test_duplicate_normalized_runner_names_keep_first_and_are_reported():
    config.save_config(
        {
            "prompt": {
                "runners_seeded": True,
                "runners": [
                    {"name": "same", "argv": ["first", "{{prompt}}"]},
                    {"name": " same ", "argv": ["second", "{{prompt}}"]},
                ],
            }
        }
    )
    assert config.load_prompt_runners() == [config.PromptRunner("same", ("first", "{{prompt}}"))]
    assert config.invalid_prompt_runners() == ["same"]


def test_runners_section_of_wrong_type_degrades():
    config.save_config({"prompt": {"runners_seeded": True, "runners": "garbage"}})
    assert config.load_prompt_runners() == []
    assert config.invalid_prompt_runners() == ["prompt.runners"]
    config.save_config({"prompt": "not-a-table"})
    assert config.load_prompt_runners() == []
    assert config.invalid_prompt_runners() == ["prompt"]
    config.ensure_prompt_runners_seeded()
    assert config.load_config()["prompt"] == "not-a-table"  # opening management is read-only


@pytest.mark.parametrize(
    ("doc", "reason", "needle"),
    [
        ({"prompt": "bad"}, "prompt-section-not-table", "isn't a table"),
        (
            {"prompt": {"runners": "bad"}},
            "runners-not-list",
            "isn't a list",
        ),
    ],
)
def test_runner_container_rows_have_localized_human_recovery_reason(doc, reason, needle):
    config.save_config(doc)
    row = config.prompt_runner_rows()[0]
    assert row.invalid_reason == reason
    assert needle in config.prompt_runner_row_reason(row)


def test_targeted_runner_mutations_preserve_unrelated_malformed_rows():
    malformed = {"name": "typo", "argv": ["mycli", "{{promt}}"], "future": 7}
    anonymous = "not-a-table"
    config.save_config(
        {
            "prompt": {
                "runners_seeded": True,
                "runners": [malformed, anonymous],
            }
        }
    )

    config.set_prompt_runner(config.PromptRunner("good", ("good", "{{prompt}}")))
    rows = config.load_config()["prompt"]["runners"]
    assert rows[:2] == [malformed, anonymous]
    assert rows[2] == {"name": "good", "argv": ["good", "{{prompt}}"]}

    assert config.remove_prompt_runner("good") is True
    assert config.load_config()["prompt"]["runners"] == [malformed, anonymous]


def test_targeted_runner_savers_refuse_malformed_containers_and_handle_absent_section():
    synthetic = config.PromptRunnerRow(0, "", ("x", "{{prompt}}"), "name", "synthetic", {})
    with pytest.raises(config.PromptRunnerChangedError):
        config.replace_prompt_runner_row(
            0, config.PromptRunner("x", ("x", "{{prompt}}")), expected=synthetic
        )

    config.save_config({"prompt": "bad"})
    with pytest.raises(config.PromptRunnerConfigError, match="isn't a table"):
        config.set_prompt_runner(config.PromptRunner("x", ("x", "{{prompt}}")))

    config.save_config({"prompt": {"runners": "bad"}})
    with pytest.raises(config.PromptRunnerConfigError, match="isn't a list"):
        config.set_prompt_runner(config.PromptRunner("x", ("x", "{{prompt}}")))


def test_explicit_runner_replace_repairs_same_name_malformed_rows_only():
    untouched = {"name": "other", "argv": ["other"]}
    config.save_config(
        {
            "prompt": {
                "runners_seeded": True,
                "runners": [
                    {"name": " typo ", "argv": ["old"]},
                    untouched,
                    {"name": "typo", "argv": "also-bad"},
                ],
            }
        }
    )
    replacement = config.PromptRunner("typo", ("fixed", "{{prompt}}"))

    with pytest.raises(config.PromptRunnerExistsError):
        config.set_prompt_runner(replacement)
    assert config.set_prompt_runner(replacement, replace_existing=True) is True

    assert config.load_config()["prompt"]["runners"] == [
        {"name": "typo", "argv": ["fixed", "{{prompt}}"]},
        untouched,
    ]
    assert config.find_prompt_runner("typo") == replacement


def test_tui_targeted_row_removal_can_recover_bad_containers():
    config.save_config({"language": "zh-TW", "prompt": {"runners": "garbage", "other": 1}})
    assert config.remove_prompt_runner_row(None) is True
    assert config.load_config() == {
        "language": "zh-TW",
        "prompt": {"runners": [], "other": 1, "runners_seeded": True},
    }

    config.save_config({"language": "zh-TW", "prompt": "not-a-table"})
    assert config.remove_prompt_runner_row(None) is True
    assert config.load_config() == {
        "language": "zh-TW",
        "prompt": {"runners_seeded": True, "runners": []},
    }


def test_raw_row_remove_snapshot_includes_unknown_fields_and_container_value():
    config.save_config(
        {
            "prompt": {
                "runners_seeded": True,
                "runners": [{"name": "bad", "argv": ["bad"], "future": 1}],
            }
        }
    )
    expected = config.prompt_runner_rows()[0]
    doc = config.load_config()
    doc["prompt"]["runners"][0]["future"] = 2
    config.save_config(doc)
    assert config.remove_prompt_runner_row(0, expected=expected) is False
    assert config.load_config()["prompt"]["runners"][0]["future"] == 2

    config.save_config({"prompt": "before"})
    expected_container = config.prompt_runner_rows()[0]
    config.save_config({"prompt": "after"})
    assert config.remove_prompt_runner_row(None, expected=expected_container) is False
    assert config.load_config()["prompt"] == "after"


def test_runner_raw_snapshots_are_recursively_type_sensitive():
    config.save_config(
        {
            "prompt": {
                "runners_seeded": True,
                "runners": [
                    {
                        "name": "bad",
                        "argv": ["bad"],
                        "future": {"nested": [1, {"flag": 0}]},
                    }
                ],
            }
        }
    )
    expected = config.prompt_runner_rows()[0]
    doc = config.load_config()
    doc["prompt"]["runners"][0]["future"] = {"nested": [True, {"flag": False}]}
    config.save_config(doc)
    assert config.remove_prompt_runner_row(0, expected=expected) is False
    assert config.load_config()["prompt"]["runners"][0]["future"] == {
        "nested": [True, {"flag": False}]
    }

    config.save_config({"prompt": 1})
    expected_container = config.prompt_runner_rows()[0]
    config.save_config({"prompt": True})
    assert config.remove_prompt_runner_row(None, expected=expected_container) is False
    assert config.load_config()["prompt"] is True


def test_runner_stable_key_remove_refuses_blank_without_seeding_or_deleting_rows():
    assert config.remove_prompt_runner("   ") is False
    assert not config.prompt_runners_seeded()

    rows = [
        {"name": " ", "argv": ["one", "{{prompt}}"]},
        {"argv": ["two", "{{prompt}}"]},
    ]
    config.save_config({"prompt": {"runners_seeded": True, "runners": rows}})
    assert config.remove_prompt_runner("") is False
    assert config.load_config()["prompt"]["runners"] == rows


def test_runner_edit_snapshot_checks_only_the_target_key():
    original = config.PromptRunner("victim", ("old", "{{prompt}}"))
    replacement = config.PromptRunner("victim", ("mine", "{{prompt}}"))
    config.save_prompt_runners([original, config.PromptRunner("other", ("other", "{{prompt}}"))])
    expected = [row for row in config.prompt_runner_rows() if row.name == "victim"]
    doc = config.load_config()
    doc["prompt"]["runners"][1]["argv"] = ["unrelated", "{{prompt}}"]
    config.save_config(doc)
    assert config.set_prompt_runner(replacement, replace_existing=True, expected=expected) is True
    assert config.find_prompt_runner("victim") == replacement
    assert config.find_prompt_runner("other") == config.PromptRunner(
        "other", ("unrelated", "{{prompt}}")
    )

    expected = [row for row in config.prompt_runner_rows() if row.name == "victim"]
    concurrent = config.PromptRunner("victim", ("external", "{{prompt}}"))
    config.set_prompt_runner(concurrent, replace_existing=True)
    with pytest.raises(config.PromptRunnerChangedError):
        config.set_prompt_runner(original, replace_existing=True, expected=expected)
    assert config.find_prompt_runner("victim") == concurrent


def test_exact_row_repair_can_name_a_recognizable_anonymous_command():
    anonymous = {"argv": ["valuable-agent", "--model", "x", "{{prompt}}"]}
    config.save_config({"prompt": {"runners_seeded": True, "runners": [anonymous, "untouched"]}})
    expected = config.prompt_runner_rows()[0]
    replacement = config.PromptRunner("valuable", ("valuable-agent", "--model", "x", "{{prompt}}"))
    assert config.replace_prompt_runner_row(0, replacement, expected=expected) is True
    assert config.load_config()["prompt"]["runners"] == [
        {
            "name": "valuable",
            "argv": ["valuable-agent", "--model", "x", "{{prompt}}"],
        },
        "untouched",
    ]


def test_exact_row_repair_refuses_a_stale_snapshot_or_colliding_new_name():
    anonymous = {"argv": ["valuable", "{{prompt}}"]}
    taken = {"name": "taken", "argv": ["taken", "{{prompt}}"]}
    config.save_config({"prompt": {"runners_seeded": True, "runners": [anonymous, taken]}})
    expected = config.prompt_runner_rows()[0]
    doc = config.load_config()
    doc["prompt"]["runners"][0]["future"] = True
    config.save_config(doc)
    with pytest.raises(config.PromptRunnerChangedError):
        config.replace_prompt_runner_row(
            0, config.PromptRunner("fresh", ("valuable", "{{prompt}}")), expected=expected
        )

    expected = config.prompt_runner_rows()[0]
    with pytest.raises(config.PromptRunnerExistsError):
        config.replace_prompt_runner_row(
            0, config.PromptRunner("taken", ("valuable", "{{prompt}}")), expected=expected
        )


def test_runner_remove_helpers_report_absent_targets_and_bad_shapes_without_writing():
    config.save_prompt_runners([config.PromptRunner("kept", ("kept", "{{prompt}}"))])
    before = config.load_config()
    assert config.remove_prompt_runner("ghost") is False
    assert config.remove_prompt_runner_row(None) is False  # healthy list, not a container repair
    assert config.remove_prompt_runner_row(-1) is False
    assert config.remove_prompt_runner_row(99) is False
    assert config.load_config() == before

    config.save_config({"prompt": "scalar"})
    assert config.remove_prompt_runner_row(0) is False

    config.save_config({"prompt": {"runners": "before"}})
    expected = config.prompt_runner_rows()[0]
    config.save_config({"prompt": {"runners": "after"}})
    assert config.remove_prompt_runner_row(None, expected=expected) is False
    assert config.load_config()["prompt"]["runners"] == "after"


def test_name_remove_snapshot_checks_only_target_key():
    config.save_prompt_runners(
        [
            config.PromptRunner("victim", ("old", "{{prompt}}")),
            config.PromptRunner("other", ("other", "{{prompt}}")),
        ]
    )
    expected = [row for row in config.prompt_runner_rows() if row.name == "victim"]
    doc = config.load_config()
    doc["prompt"]["runners"].insert(0, {"name": "unrelated", "argv": ["unrelated", "{{prompt}}"]})
    config.save_config(doc)

    assert config.remove_prompt_runner("victim", expected=expected) is True
    assert [row["name"] for row in config.load_config()["prompt"]["runners"]] == [
        "unrelated",
        "other",
    ]


def test_save_prompt_runners_preserves_other_keys():
    config.save_config({"editor": "vi", "prompt": {"other": 1}})
    config.save_prompt_runners([config.PromptRunner("x", ("x", "{{prompt}}"))])
    doc = config.load_config()
    assert doc["editor"] == "vi"
    assert doc["prompt"]["other"] == 1
    assert doc["prompt"]["runners_seeded"] is True
    assert config.find_prompt_runner("x") == config.PromptRunner("x", ("x", "{{prompt}}"))
    assert config.find_prompt_runner("ghost") is None


# --------------------------------------------------------------------------
# argstate: the last-picked runner
# --------------------------------------------------------------------------


def test_last_runner_roundtrip_and_corruption_degrades(tmp_path, monkeypatch):
    assert argstate.load_last_runner() == ""
    argstate.save_last_runner("codex")
    assert argstate.load_last_runner() == "codex"
    from skit.paths import state_dir

    (state_dir() / "prompt.toml").write_text("not = [toml", encoding="utf-8")
    assert argstate.load_last_runner() == ""
    (state_dir() / "prompt.toml").write_text("last_runner = 3", encoding="utf-8")
    assert argstate.load_last_runner() == ""


def test_build_unreadable_body_is_a_clean_launch_error(tmp_path, monkeypatch):
    entry = store.add_prompt(_write_prompt(tmp_path, "Do {{a}}\n"))
    monkeypatch.setattr(langs_launch, "_which", lambda name: f"/bin/{name}")
    entry.script_path.unlink()
    entry.script_path.mkdir()  # exists, but read_text raises IsADirectoryError
    with pytest.raises(LaunchError, match="Can't read"):
        PromptLaunch().build(entry, [], {"a": "1"}, None, runner=_runner())


# --------------------------------------------------------------------------
# the interpolate master switch + flood caps
# --------------------------------------------------------------------------


def test_meta_interpolate_round_trip_and_garbage_tolerance():
    from skit.models import ScriptMeta

    meta = ScriptMeta(name="p", kind="prompt", interpolate=False)
    d = meta.to_toml_dict()
    assert d["interpolate"] is False
    assert ScriptMeta.from_toml_dict(d).interpolate is False
    on = ScriptMeta(name="p", kind="prompt").to_toml_dict()
    assert "interpolate" not in on  # default omitted — old metas stay untouched
    # A hand-edited non-bool must not silently kill the feature (genuine-False rule).
    assert ScriptMeta.from_toml_dict(
        {"name": "p", "kind": "prompt", "interpolate": "no"}
    ).interpolate


def test_meta_rejects_wrong_typed_runner_at_the_corruption_boundary():
    from skit.models import ScriptMeta, ScriptMetaError

    with pytest.raises(ScriptMetaError, match="runner"):
        ScriptMeta.from_toml_dict({"name": "p", "kind": "prompt", "runner": 123})


def test_add_prompt_interpolate_off_scans_and_manages_nothing(tmp_path: Path):
    entry = store.add_prompt(_write_prompt(tmp_path, "{{a}} {{b}}\n"), interpolate=False)
    assert entry.meta.interpolate is False
    assert entry.meta.params is None


def test_add_prompt_auto_manage_flood_cap(tmp_path: Path):
    from skit.langs.prompt.analyzer import AUTO_MANAGE_LIMIT

    many = " ".join("{{h" + str(i) + "}}" for i in range(AUTO_MANAGE_LIMIT + 1))
    entry = store.add_prompt(_write_prompt(tmp_path, many + "\n"))
    assert entry.meta.params is None  # over the cap: nothing auto-managed
    assert entry.meta.interpolate is True
    # An EXPLICIT selection is always honored — the user asked.
    explicit = store.add_prompt(
        _write_prompt(tmp_path, many + "\n", name="explicit.prompt.md"),
        name="explicit",
        managed=["h0", "h3"],
    )
    assert explicit.meta.params == ["h0", "h3"]


def test_write_prompt_interpolate_keeps_the_managed_list(tmp_path: Path):
    entry = store.add_prompt(_write_prompt(tmp_path, "{{a}}\n"))
    store.write_prompt_interpolate(entry.slug, False)
    off = store.resolve(entry.slug)
    assert off.meta.interpolate is False
    assert off.meta.params == ["a"]  # survives for a later switch-on
    store.write_prompt_interpolate(entry.slug, True)
    assert store.resolve(entry.slug).meta.interpolate is True
    entry2 = store.add_command("echo {x}", name="cmd")
    with pytest.raises(store.StoreUsageError):
        store.write_prompt_interpolate(entry2.slug, False)


def _pause_first_meta_write(
    monkeypatch: pytest.MonkeyPatch,
) -> tuple[threading.Event, threading.Event, threading.Event]:
    """Hold one old-snapshot writer while a second attempts its replace.

    Without an entry transaction the second writer reaches disk and the released first
    writer then erases its field.  With the lock, the second writer cannot enter
    ``_write_meta`` until it can reload the first writer's committed metadata.
    """
    real_write = store._write_meta
    first_entered = threading.Event()
    release_first = threading.Event()
    later_write_finished = threading.Event()
    call_lock = threading.Lock()
    calls = 0

    def controlled_write(entry_dir: Path, meta) -> None:
        nonlocal calls
        with call_lock:
            calls += 1
            first = calls == 1
        if first:
            first_entered.set()
            assert release_first.wait(timeout=2)
        real_write(entry_dir, meta)
        if not first:
            later_write_finished.set()

    monkeypatch.setattr(store, "_write_meta", controlled_write)
    return first_entered, release_first, later_write_finished


def test_prompt_meta_setters_preserve_concurrent_distinct_fields(tmp_path, monkeypatch):
    entry = store.add_prompt(_write_prompt(tmp_path, "{{a}}\n"))
    first_entered, release_first, later_finished = _pause_first_meta_write(monkeypatch)
    pin_thread = threading.Thread(target=store.write_prompt_runner, args=(entry.slug, "claude"))
    interpolate_thread = threading.Thread(
        target=store.write_prompt_interpolate, args=(entry.slug, False)
    )

    pin_thread.start()
    assert first_entered.wait(timeout=2)
    interpolate_thread.start()
    # An unlocked writer completes from its stale snapshot. A locked writer waits;
    # either way, releasing here leaves the final assertions deterministic.
    later_finished.wait(timeout=0.2)
    release_first.set()
    pin_thread.join(timeout=2)
    interpolate_thread.join(timeout=2)

    assert not pin_thread.is_alive()
    assert not interpolate_thread.is_alive()
    saved = store.resolve(entry.slug)
    assert saved.meta.runner == "claude"
    assert saved.meta.interpolate is False


def test_prompt_and_generic_meta_setters_share_one_entry_lock(tmp_path, monkeypatch):
    entry = store.add_prompt(_write_prompt(tmp_path, "{{a}}\n"))
    first_entered, release_first, later_finished = _pause_first_meta_write(monkeypatch)
    pin_thread = threading.Thread(target=store.write_prompt_runner, args=(entry.slug, "claude"))
    needs_thread = threading.Thread(target=store.update_needs, args=(entry.slug, ["jq"]))

    pin_thread.start()
    assert first_entered.wait(timeout=2)
    needs_thread.start()
    later_finished.wait(timeout=0.2)
    release_first.set()
    pin_thread.join(timeout=2)
    needs_thread.join(timeout=2)

    assert not pin_thread.is_alive()
    assert not needs_thread.is_alive()
    saved = store.resolve(entry.slug)
    assert saved.meta.runner == "claude"
    assert saved.meta.needs == ["jq"]


def test_remove_waits_for_meta_writer_and_leaves_no_resurrectable_orphan(tmp_path, monkeypatch):
    entry = store.add_prompt(_write_prompt(tmp_path, "{{a}}\n"))
    first_entered, release_first, _later_finished = _pause_first_meta_write(monkeypatch)
    writer = threading.Thread(target=store.write_prompt_runner, args=(entry.slug, "claude"))
    removed: list[str] = []
    remover = threading.Thread(target=lambda: removed.append(store.remove(entry.slug)))

    writer.start()
    assert first_entered.wait(timeout=2)
    remover.start()
    # remove must be waiting on the stable sibling lock, not deleting the directory
    # under the paused writer and allowing its atomic write to recreate an orphan.
    remover.join(timeout=0.2)
    assert remover.is_alive()
    release_first.set()
    writer.join(timeout=2)
    remover.join(timeout=2)

    assert not writer.is_alive()
    assert not remover.is_alive()
    assert removed == [entry.meta.name]
    assert not entry.dir.exists()
    assert store._entry_lock_path(entry.slug).is_file()  # persistent inode; kernel lock released
    with pytest.raises(store.NotFoundError):
        store.resolve(entry.slug)
    count, problems = store.doctor_rebuild()
    assert (count, problems) == (0, [])


def test_plan_for_an_insertion_off_prompt_is_fieldless_and_driftless(tmp_path: Path):
    entry = store.add_prompt(_write_prompt(tmp_path, "{{a}}\n"))
    store.write_prompt_interpolate(entry.slug, False)
    entry.script_path.write_text("no holes anymore\n", encoding="utf-8")
    plan = flows.plan_for_entry(store.resolve(entry.slug))
    assert plan.source == "command"
    assert not plan.fields
    assert not plan.drift_lines  # an off prompt can't drift


def test_build_for_an_insertion_off_prompt_sends_the_body_verbatim(tmp_path, monkeypatch):
    entry = store.add_prompt(_write_prompt(tmp_path, "Keep {{a}} as-is\n"))
    store.write_prompt_interpolate(entry.slug, False)
    monkeypatch.setattr(langs_launch, "_which", lambda name: f"/bin/{name}")
    payload = PromptLaunch().build(store.resolve(entry.slug), [], {}, None, runner=_runner())
    assert isinstance(payload, ArgvLaunch)
    assert payload.argv[1] == "Keep {{a}} as-is\n"  # managed name NOT substituted
    shown = PromptLaunch().describe(store.resolve(entry.slug), [], {}, None, runner=_runner())
    assert "Keep {{a}} as-is" in shown


def test_preview_names_caps_the_list():
    from skit.langs.prompt.analyzer import LIST_PREVIEW_LIMIT, preview_names

    short = [f"n{i}" for i in range(3)]
    assert preview_names(short) == ("n0, n1, n2", 0)
    long = [f"n{i}" for i in range(LIST_PREVIEW_LIMIT + 5)]
    shown, remaining = preview_names(long)
    assert remaining == 5
    assert f"n{LIST_PREVIEW_LIMIT - 1}" in shown
    assert f"n{LIST_PREVIEW_LIMIT}" not in shown


def test_unmanaged_prompt_placeholders_is_body_minus_managed_in_order(tmp_path):
    entry = store.add_prompt(_write_prompt(tmp_path, "{{a}} {{b}} {{c}}\n"), managed=["b"])
    # First appearance order, managed removed — the one rule params/settings/edit share.
    assert store.unmanaged_prompt_placeholders(store.resolve(entry.slug)) == ["a", "c"]


def test_unmanaged_prompt_placeholders_empty_when_insertion_off(tmp_path):
    entry = store.add_prompt(_write_prompt(tmp_path, "{{a}}\n"))
    store.write_prompt_interpolate(entry.slug, False)
    # Insertion off: the body travels verbatim, so nothing is a candidate.
    assert store.unmanaged_prompt_placeholders(store.resolve(entry.slug)) == []


def test_unmanaged_prompt_placeholders_empty_for_non_prompt(tmp_path):
    script = tmp_path / "s.py"
    script.write_text("print(1)\n", encoding="utf-8")
    entry = store.add_python(script, name="notaprompt")
    assert store.unmanaged_prompt_placeholders(entry) == []


def test_unmanaged_prompt_placeholders_empty_when_body_missing_or_undecodable(tmp_path):
    entry = store.add_prompt(_write_prompt(tmp_path, "{{a}}\n"))
    fresh = store.resolve(entry.slug)
    fresh.script_path.write_bytes(b"\xff\xfe not utf-8 {{a}}")
    # Undecodable body → no schema invented from replacement bytes (preflight owns it).
    assert store.unmanaged_prompt_placeholders(fresh) == []
    fresh.script_path.unlink()
    assert store.unmanaged_prompt_placeholders(fresh) == []
