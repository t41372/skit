"""The prompt kind's core: analyzer, renderer, registry row, store, plan, launch.

CLI surfaces live in test_prompt_cli.py; TUI surfaces in test_prompt_tui.py. The golden
corpus under tests/corpus/prompt/ is byte-exact (CRLF, missing trailing newline, CJK,
emoji) and excluded from the pre-commit fixers like every other corpus directory.
"""

from __future__ import annotations

from pathlib import Path

import pytest

from skit import argstate, config, flows, launcher, store
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
    path.write_text(text, encoding="utf-8")
    return path


def _runner(name: str = "rec", argv: tuple[str, ...] = ("rec-bin", "{prompt}")):
    return config.PromptRunner(name, argv)


# --------------------------------------------------------------------------
# analyzer
# --------------------------------------------------------------------------


def test_placeholder_names_dedupes_in_body_order():
    text = "a {b} c {a} d {b} {_x1} {9bad} { spaced} {a-b}"
    assert analyzer.placeholder_names(text) == ["b", "a", "_x1"]


def test_placeholder_names_honors_escapes_and_reserved_name():
    assert analyzer.placeholder_names("{{lit}} {prompt} {real}") == ["real"]


def test_token_pattern_is_shared_with_template_launch():
    # The two surfaces must never drift on what counts as a placeholder.
    assert analyzer.TOKEN_RE is langs_launch._TEMPLATE_TOKEN_RE


def test_corpus_basic_detection_and_render_byte_identity():
    text = (CORPUS / "01_basic.prompt.md").read_bytes().decode("utf-8")
    assert analyzer.placeholder_names(text) == ["target", "focus", "x"]
    rendered = render.render_body(text, {"target": "T", "focus": "F"}, ["target", "focus"])
    # Managed holes filled; the {x} code sample (unmanaged) stays; {{ }} unescape.
    assert "Review T for F. Again: T." in rendered
    assert "{x}" in rendered
    assert "Escaped: {literal}" in rendered


def test_corpus_crlf_preserved_verbatim():
    raw = (CORPUS / "02_crlf.prompt.md").read_bytes()
    assert b"\r\n" in raw  # the corpus really is CRLF — fixers must not touch it
    text = raw.decode("utf-8")
    rendered = render.render_body(text, {"task": "X", "repo": "Y"}, ["task", "repo"])
    assert "\r\n" in rendered
    assert rendered == text.replace("{task}", "X").replace("{repo}", "Y")


def test_corpus_cjk_emoji_no_trailing_newline():
    raw = (CORPUS / "03_cjk_emoji.prompt.md").read_bytes()
    assert not raw.endswith(b"\n")  # deliberate: no trailing newline
    text = raw.decode("utf-8")
    # CJK inside braces is not an identifier, so it is NOT a placeholder — verbatim.
    assert analyzer.placeholder_names(text) == ["focus"]
    rendered = render.render_body(text, {"focus": "效能"}, ["focus"])
    assert "{目標檔案}" in rendered
    assert "專注於 效能" in rendered
    assert not rendered.endswith("\n")


def test_corpus_reserved_prompt_stays_verbatim():
    text = (CORPUS / "05_reserved.prompt.md").read_bytes().decode("utf-8")
    assert analyzer.placeholder_names(text) == ["real"]
    rendered = render.render_body(text, {"real": "R"}, ["real"])
    assert "{prompt}\tliterally" in rendered


# --------------------------------------------------------------------------
# render
# --------------------------------------------------------------------------


def test_render_body_missing_managed_value_raises():
    with pytest.raises(LaunchError, match="target"):
        render.render_body("{target}", {}, ["target"])


def test_render_body_substitutes_raw_never_quotes():
    payload = '\'; rm -rf ~; $(touch pwned) `echo hi` "x" {inner} {{deep}}'
    rendered = render.render_body("V={v} end", {"v": payload}, ["v"])
    # Byte-identical payload: no quoting, and the replacement is never re-scanned.
    assert rendered == f"V={payload} end"


def test_render_body_empty_value_substitutes_empty():
    assert render.render_body("[{v}]", {"v": ""}, ["v"]) == "[]"


def test_fill_runner_argv_replaces_the_one_slot_raw():
    rendered = "line1\nline2 with {braces} and {{more}}"
    argv = render.fill_runner_argv(["agent", "--m={prompt}", "{{lit}}"], rendered)
    assert argv == ["agent", f"--m={rendered}", "{lit}"]


def test_fill_runner_argv_leaves_foreign_holes_verbatim():
    # Validation refuses these at save time; the renderer must still be total.
    assert render.fill_runner_argv(["a", "{other}"], "X") == ["a", "{other}"]


def test_check_argv_length_refuses_over_limit():
    render.check_argv_length(["x" * 100])
    with pytest.raises(LaunchError, match=str(render.ARGV_LIMIT)):
        render.check_argv_length(["x" * (render.ARGV_LIMIT + 1)])


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
    src = _write_prompt(tmp_path, "# T\n\nDo {a} then {b}. Sample {a}.\n")
    entry = store.add_prompt(src)
    assert entry.meta.kind == "prompt"
    assert entry.meta.params == ["a", "b"]
    assert entry.meta.workdir == "invoke"
    assert entry.meta.description == "T"
    assert entry.meta.runner == ""
    assert (entry.dir / "prompt.md").read_text() == src.read_text()


def test_add_prompt_managed_subset_keeps_body_order(tmp_path: Path):
    src = _write_prompt(tmp_path, "{a} {b} {c}\n")
    entry = store.add_prompt(src, managed=["c", "a"])
    assert entry.meta.params == ["a", "c"]  # body order, whatever the caller's order


def test_add_prompt_refuses_unknown_managed_name(tmp_path: Path):
    src = _write_prompt(tmp_path, "{a}\n")
    with pytest.raises(store.StoreError, match="ghost"):
        store.add_prompt(src, managed=["ghost"])


def test_add_prompt_reference_mode_still_pins_invoke_workdir(tmp_path: Path):
    src = _write_prompt(tmp_path, "hello {x}\n")
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


def test_write_prompt_managed_and_runner_roundtrip(tmp_path: Path):
    entry = store.add_prompt(_write_prompt(tmp_path, "{a} {b}\n"))
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
        _write_prompt(tmp_path, "{a} {api_key} {skip}\n"), managed=["a", "api_key"]
    )
    plan = flows.plan_for_entry(entry)
    assert plan.source == "command"
    assert [f.key for f in plan.fields] == ["a", "api_key"]
    assert all(f.source == "placeholder" and f.required for f in plan.fields)
    # The secret name heuristic applies to synthesized placeholders (C3, every source).
    assert plan.fields[1].secret
    assert not plan.drift_lines


def test_prompt_plan_reports_drift_for_gone_managed_names(tmp_path: Path):
    entry = store.add_prompt(_write_prompt(tmp_path, "{a} {b}\n"))
    entry.script_path.write_text("only {a} now\n", encoding="utf-8")
    plan = flows.plan_for_entry(entry)
    assert [f.key for f in plan.fields] == ["a", "b"]  # the record stays visible
    assert len(plan.drift_lines) == 1
    assert "b" in plan.drift_lines[0]


def test_prompt_plan_declared_rows_enrich_schema_and_env_riders_ride(tmp_path: Path):
    entry = store.add_prompt(_write_prompt(tmp_path, "{n}\n"))
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
    entry = store.add_prompt(_write_prompt(tmp_path, "{a}\n"))
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


def _entry_with_runner(tmp_path, monkeypatch, text="Do {a}\n", pin="", managed=None):
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


def test_build_resolves_the_pin_when_no_override_is_given(tmp_path, monkeypatch):
    entry = _entry_with_runner(tmp_path, monkeypatch, pin="claude")
    monkeypatch.setattr(
        config,
        "load_prompt_runners",
        lambda: [config.PromptRunner("claude", ("claude", "{prompt}"))],
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
    entry = store.add_prompt(_write_prompt(tmp_path, "Do {a}\n"))
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
    with pytest.raises(LaunchError, match="characters"):
        PromptLaunch().build(
            entry, [], {"a": "x" * (render.ARGV_LIMIT + 10)}, None, runner=_runner()
        )


def test_build_script_override_reads_the_override(tmp_path, monkeypatch):
    entry = _entry_with_runner(tmp_path, monkeypatch)
    override = tmp_path / "other.md"
    override.write_text("Other {a}!", encoding="utf-8")
    payload = PromptLaunch().build(entry, [], {"a": "Z"}, override, runner=_runner())
    assert isinstance(payload, ArgvLaunch)
    assert payload.argv[1] == "Other Z!"


def test_describe_with_runner_shows_the_real_argv(tmp_path, monkeypatch):
    entry = _entry_with_runner(tmp_path, monkeypatch)
    shown = PromptLaunch().describe(entry, [], {"a": "•••"}, None, runner=_runner())
    assert "rec-bin" in shown
    assert "•••" in shown


def test_describe_without_runner_never_reads_config(tmp_path, monkeypatch):
    entry = _entry_with_runner(tmp_path, monkeypatch, pin="claude")
    monkeypatch.setattr(
        config,
        "load_prompt_runners",
        lambda: pytest.fail("describe must not read config (a read can seed = a write)"),
    )
    shown = PromptLaunch().describe(entry, [], {"a": "1"}, None)
    assert "claude" in shown  # the pin's NAME stands in for its argv


def test_describe_degrades_on_missing_body_and_missing_values(tmp_path, monkeypatch):
    entry = _entry_with_runner(tmp_path, monkeypatch)
    assert "{prompt}" in PromptLaunch().describe(entry, [], {}, None, runner=_runner())
    entry.script_path.unlink()
    shown = PromptLaunch().describe(entry, [], {"a": "1"}, None, runner=_runner())
    assert "rec-bin" in shown
    assert "{prompt}" in shown


def test_preflight_checks_the_pin_only(tmp_path, monkeypatch):
    strategy = PromptLaunch()
    unpinned = _entry_with_runner(tmp_path, monkeypatch)
    strategy.preflight(unpinned)  # no pin: nothing to validate — the form asks
    pinned = store.write_prompt_runner(unpinned.slug, "claude")
    monkeypatch.setattr(
        config,
        "load_prompt_runners",
        lambda: [config.PromptRunner("claude", ("claude", "{prompt}"))],
    )
    strategy.preflight(pinned)
    monkeypatch.setattr(langs_launch, "_which", lambda name: None)
    with pytest.raises(NotExecutableError):
        strategy.preflight(pinned)


def test_preflight_missing_body(tmp_path, monkeypatch):
    entry = _entry_with_runner(tmp_path, monkeypatch)
    entry.script_path.unlink()
    with pytest.raises(TargetMissingError):
        PromptLaunch().preflight(entry)


def test_target_is_the_prompt_body(tmp_path, monkeypatch):
    entry = _entry_with_runner(tmp_path, monkeypatch)
    assert PromptLaunch().target(entry) == entry.script_path


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
    runner = config.PromptRunner("rec", (sys.executable, str(recorder), str(out), "{prompt}"))
    code = launcher.run_entry(entry, [], values={"path": "src/x.py"}, runner=runner)
    assert code == 0
    captured = _json.loads(out.read_text(encoding="utf-8"))
    assert captured == [text.replace("{path}", "src/x.py")]
    assert not (tmp_path / "pwned").exists()  # $(touch pwned) never ran


# --------------------------------------------------------------------------
# config: the runner registry
# --------------------------------------------------------------------------


def test_validate_prompt_runner_argv_rules():
    ok = config.validate_prompt_runner_argv
    assert ok(["claude", "{prompt}"]) is None
    assert ok(["a", "--m={prompt}"]) is None
    assert ok(["a", "{{lit}}", "{prompt}"]) is None  # escapes are not holes
    assert ok([]) == "empty"
    assert ok([""]) == "empty"
    assert ok(["claude"]) == "prompt-slot-count"
    assert ok(["a", "{prompt}", "{prompt}"]) == "prompt-slot-count"
    assert ok(["{prompt}"]) == "prompt-in-binary"
    assert ok(["a", "{other}"]) == "stray-hole"


def test_load_prompt_runners_is_read_only_before_seeding(tmp_path):
    assert not config.prompt_runners_seeded()
    runners = config.load_prompt_runners()
    assert [r.name for r in runners] == ["claude", "codex", "opencode", "amp", "antigravity"]
    assert not config.prompt_runners_seeded()  # reading never wrote


def test_ensure_seeded_materializes_once_and_empty_stays_empty():
    config.ensure_prompt_runners_seeded()
    assert config.prompt_runners_seeded()
    assert "runners" in config.load_config()["prompt"]
    config.save_prompt_runners([])
    config.ensure_prompt_runners_seeded()  # must NOT resurrect the seeds
    assert config.load_prompt_runners() == []


def test_hand_authored_rows_without_marker_count_as_seeded():
    config.save_config({"prompt": {"runners": [{"name": "mine", "argv": ["m", "{prompt}"]}]}})
    assert config.prompt_runners_seeded()
    assert [r.name for r in config.load_prompt_runners()] == ["mine"]


def test_malformed_runner_rows_are_skipped_and_reported():
    config.save_config(
        {
            "prompt": {
                "runners_seeded": True,
                "runners": [
                    {"name": "good", "argv": ["g", "{prompt}"]},
                    {"name": "bad-no-slot", "argv": ["g"]},
                    {"name": "", "argv": ["g", "{prompt}"]},
                    {"name": "bad-argv", "argv": "not-a-list"},
                    {"name": "bad-token-type", "argv": ["g", 3]},
                    "not-a-table",
                ],
            }
        }
    )
    assert [r.name for r in config.load_prompt_runners()] == ["good"]
    reported = config.invalid_prompt_runners()
    assert "bad-no-slot" in reported
    assert len(reported) == 5


def test_runners_section_of_wrong_type_degrades():
    config.save_config({"prompt": {"runners_seeded": True, "runners": "garbage"}})
    assert config.load_prompt_runners() == []
    assert config.invalid_prompt_runners() == ["prompt.runners"]
    config.save_config({"prompt": "not-a-table"})
    assert [r.name for r in config.load_prompt_runners()] == [
        "claude",
        "codex",
        "opencode",
        "amp",
        "antigravity",
    ]
    assert config.invalid_prompt_runners() == []


def test_save_prompt_runners_preserves_other_keys():
    config.save_config({"editor": "vi", "prompt": {"other": 1}})
    config.save_prompt_runners([config.PromptRunner("x", ("x", "{prompt}"))])
    doc = config.load_config()
    assert doc["editor"] == "vi"
    assert doc["prompt"]["other"] == 1
    assert doc["prompt"]["runners_seeded"] is True
    assert config.find_prompt_runner("x") == config.PromptRunner("x", ("x", "{prompt}"))
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
    entry = store.add_prompt(_write_prompt(tmp_path, "Do {a}\n"))
    monkeypatch.setattr(langs_launch, "_which", lambda name: f"/bin/{name}")
    entry.script_path.unlink()
    entry.script_path.mkdir()  # exists, but read_text raises IsADirectoryError
    with pytest.raises(LaunchError, match="Can't read"):
        PromptLaunch().build(entry, [], {"a": "1"}, None, runner=_runner())
