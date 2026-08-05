"""Behavior coverage for the design-audit round-13 fixes (the external PR review).

Each section keeps one confirmed finding dead:

R. ``store.resolve``'s miss-path sweep — added in round 9 to serve hand-edited metas —
   answered a name two metas carried by returning whichever slug sorted first. The
   pre-round-9 code refused that state (as NotFound); the sweep must refuse it too,
   and say why: ``AmbiguousNameError``, listing the claimants, remedied by a slug.
S. State-write failures (a read-only state dir, a full disk) escaped as raw OSError
   tracebacks with Click's generic exit 1. ``argstate`` now types them as
   ``StateWriteError`` (an OSError, mirroring ``config.ConfigWriteError``), the root
   boundary maps them to the operational exit, and the TUI notifies instead of dying.
T. A prompt's managed list and its declared rows were committed as two independent
   metadata transactions; a failure between them left the schema half new. One call,
   one lock, one meta write.
U. ``skit params … --secret`` committed the secret schema first and scrubbed old
   plaintext second, so a failure between the two left "schema says secret, plaintext
   still on disk" — the one state the transition exists to forbid. Purge now runs
   first: every interruption lands on public+value, public+no-value or
   secret+no-value.
V. ``skit run --raw`` re-persisted the preserved ``last_run.values`` snapshot without
   applying the entry's CURRENT secret set, and ``record_run(values=None)`` ignored
   ``secret_names`` outright — the one write entry point that broke argstate's C3
   contract line.
"""

from __future__ import annotations

import types
from pathlib import Path
from typing import cast

import pytest
import typer
from typer.testing import CliRunner

from skit import argstate, cli, flows, store
from skit.exitcodes import EXIT_SKIT, EXIT_USAGE
from skit.params import ParamDecl

runner = CliRunner()


@pytest.fixture(autouse=True)
def tmp_store(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path / "config"))
    monkeypatch.setenv("SKIT_LANG", "en")
    return tmp_path


def _cmd(name: str, template: str = "echo hi") -> store.Entry:
    return store.add_command(template, name=name)


def _rename_meta_only(slug: str, new_name: str) -> None:
    """A hand edit of meta.toml — the state add/rename refuse to create, and the state
    the miss-path sweep exists to serve (round 9's helper, same bytes-level edit)."""
    path = store.scripts_dir() / slug / "meta.toml"
    text = path.read_text(encoding="utf-8")
    old = store.resolve(slug).meta.name
    path.write_text(text.replace(f'name = "{old}"', f'name = "{new_name}"'), encoding="utf-8")


# ==========================================================================
# R. resolve() refuses a name two metas carry
# ==========================================================================


def test_two_metas_hand_edited_to_one_name_refuse_to_resolve() -> None:
    """The finding's exact scenario: two entries whose metas a hand edit gave the same
    name. Resolution must refuse — running whichever slug sorts first is executing an
    entry the user never picked. The message carries every claimant, sorted, so the
    remedy (a slug) is right there in the refusal."""
    a = _cmd("alpha")
    b = _cmd("beta")
    _rename_meta_only(a.slug, "deploy")
    _rename_meta_only(b.slug, "deploy")

    with pytest.raises(store.AmbiguousNameError) as exc_info:
        store.resolve("deploy")
    # The WHOLE sentence: wording, sorted slug order, separator, and the remedy — a
    # substring check would pass on a line that quietly became something else.
    slugs = ", ".join(sorted([a.slug, b.slug]))
    assert str(exc_info.value) == (
        f"The name deploy belongs to more than one entry ({slugs}) — use a slug."
    )
    # The write-side twin NameConflictError is a usage refusal; so is this one.
    assert isinstance(exc_info.value, store.StoreUsageError)


def test_the_repaired_index_refuses_the_same_way() -> None:
    """The first refusal's sweep repairs the rows, so the SECOND resolve sees the
    collision in the registry itself (two rows, one name) and must not treat "not
    exactly one row" as a license to guess via the sweep's first hit."""
    a = _cmd("alpha")
    b = _cmd("beta")
    _rename_meta_only(a.slug, "deploy")
    _rename_meta_only(b.slug, "deploy")
    with pytest.raises(store.AmbiguousNameError):
        store.resolve("deploy")

    with pytest.raises(store.AmbiguousNameError):
        store.resolve("deploy")


def test_slugs_still_resolve_either_claimant() -> None:
    """Ambiguity is a NAME problem. The slug is the directory — it cannot collide, and
    it must keep working while the name is contested, because it IS the remedy the
    refusal message hands out."""
    a = _cmd("alpha")
    b = _cmd("beta")
    _rename_meta_only(a.slug, "deploy")
    _rename_meta_only(b.slug, "deploy")

    assert store.resolve(a.slug).slug == a.slug
    assert store.resolve(b.slug).slug == b.slug


def test_the_cli_face_is_a_usage_exit_not_a_traceback() -> None:
    """Docker-convention taxonomy: the refusal is a usage error (exit 2) like every
    StoreUsageError, and the message that reaches the user is the remedy-bearing one,
    not a stack trace."""
    a = _cmd("alpha")
    b = _cmd("beta")
    _rename_meta_only(a.slug, "deploy")
    _rename_meta_only(b.slug, "deploy")

    result = runner.invoke(cli.app, ["show", "deploy"])

    assert result.exit_code == EXIT_USAGE
    assert "more than one entry" in result.output
    assert "Traceback" not in result.output


def test_a_lone_sweep_hit_is_reverified_against_the_meta_it_names(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """TOCTOU parity with the registry fast path: a summary may claim a name the meta
    no longer carries by the time the entry is read. The sweep must re-check the truth
    it is about to hand back, exactly as the fast path does, and answer NotFound for a
    match that evaporated — never serve the lying entry."""
    entry = _cmd("my tool")
    real_summaries = store.list_summaries()
    stale = [
        type(s)(
            slug=s.slug,
            name="ghost",
            kind=s.kind,
            mode=s.mode,
            description=s.description,
            dir=s.dir,
            target=s.target,
        )
        for s in real_summaries
    ]
    monkeypatch.setattr(store, "list_summaries", lambda: stale)

    with pytest.raises(store.NotFoundError):
        store.resolve("ghost")
    assert store.resolve(entry.slug).meta.name == "my tool"


def test_a_single_stale_row_still_heals_and_resolves() -> None:
    """The sweep's original job survives the refusal logic: ONE meta renamed by hand is
    unambiguous, and must keep resolving (round 9's promise, re-pinned here against the
    new candidate-counting branch)."""
    entry = _cmd("my tool")
    _rename_meta_only(entry.slug, "hola")

    assert store.resolve("hola").slug == entry.slug


def test_completion_swallows_the_ambiguous_refusal() -> None:
    """Shell completion must never crash the shell: its except-Exception guard has to
    hold for the new refusal type too."""
    a = _cmd("alpha")
    b = _cmd("beta")
    _rename_meta_only(a.slug, "deploy")
    _rename_meta_only(b.slug, "deploy")

    # The honest Context stand-in from test_cli_gaps_cov: only .params is read.
    ctx = cast(typer.Context, types.SimpleNamespace(params={"name": "deploy"}))
    assert cli._complete_preset(ctx, "") == []


# ==========================================================================
# S. state-write failures join the exit taxonomy
# ==========================================================================


def _state_boom(*_args: object, **_kwargs: object) -> None:
    raise OSError(30, "Read-only file system", "/state/values/x.toml")


@pytest.mark.parametrize(
    "argv",
    [
        pytest.param(
            ["preset", "save", "{slug}", "fresh", "--from-last", "--no-input"],
            id="preset-save",
        ),
        pytest.param(["preset", "delete", "{slug}", "prod", "--yes"], id="preset-delete"),
    ],
)
def test_a_state_write_failure_is_an_operational_exit_not_a_traceback(
    argv: list[str], monkeypatch: pytest.MonkeyPatch
) -> None:
    """The commands whose ONLY job is a state write used to answer a read-only state
    dir with a raw traceback and Click's generic exit 1 — outside the taxonomy every
    other skit failure honors. StateWriteError now reaches the root boundary and maps
    to the operational exit, same sentence as a config write failure."""
    entry = store.add_command("echo {A}", name="job")
    argstate.save_preset(entry.slug, "prod", {"A": "1"})
    argstate.record_run(entry.slug, 0, at="2026-01-01T00:00:00+00:00", values={"A": "1"})
    monkeypatch.setattr(argstate, "atomic_write_toml", _state_boom)

    result = runner.invoke(cli.app, [a.format(slug=entry.slug) for a in argv])

    assert result.exit_code == EXIT_SKIT
    assert "filesystem operation" in result.output
    assert "Traceback" not in result.output


def test_post_run_persistence_still_degrades_the_typed_state_failure() -> None:
    """The run lane's contract must survive the typing: a StateWriteError from the
    accepted-run persistence stays a warning that the entry ran but state wasn't saved
    — never a raised error that would steal the script's own exit code."""

    def _persist() -> None:
        raise argstate.StateWriteError(28, "No space left on device", "x.toml")

    message = flows.post_run_persistence_error(_persist)

    assert message is not None
    assert "couldn't save its state" in message
    assert "No space left on device" in message


# ==========================================================================
# T. a prompt's schema is one meta write
# ==========================================================================


def _prompt(tmp_path: Path, body: str = "Do {{topic}} then {{tone}}\n") -> store.Entry:
    src = tmp_path / "p.prompt.md"
    src.write_text(body, encoding="utf-8")
    return store.add_prompt(src, name="p")


def test_managed_list_and_declared_rows_land_in_one_meta_write(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The schema is one logical unit; the disk must see it as one transaction. The spy
    counts the actual meta commits — one, carrying BOTH halves."""
    entry = _prompt(tmp_path)
    writes: list[object] = []
    real = store._write_meta_and_row
    monkeypatch.setattr(
        store,
        "_write_meta_and_row",
        lambda entry_dir, slug, meta: (writes.append(slug), real(entry_dir, slug, meta))[1],
    )

    decls = [ParamDecl(name="topic", delivery="placeholder", help="the subject")]
    updated, _ = store.write_parameters(entry.slug, decls, managed=["topic"])

    assert writes == [entry.slug]  # exactly one commit
    assert updated.meta.params == ["topic"]
    assert [d["name"] for d in updated.meta.parameters or []] == ["topic"]
    fresh = store.resolve(entry.slug)
    assert fresh.meta.params == ["topic"]
    assert [d["name"] for d in fresh.meta.parameters or []] == ["topic"]


def test_managed_none_leaves_the_list_alone_and_empty_clears_it(tmp_path: Path) -> None:
    """The tri-state must not blur: None is "don't touch", [] is "clear" — exactly
    write_prompt_managed's own storage rule (an empty list stores as absence)."""
    entry = _prompt(tmp_path)
    store.write_prompt_managed(entry.slug, ["topic", "tone"])

    store.write_parameters(entry.slug, [ParamDecl(name="topic", delivery="placeholder")])
    assert store.resolve(entry.slug).meta.params == ["topic", "tone"]  # None: untouched

    store.write_parameters(entry.slug, [], managed=[])
    assert store.resolve(entry.slug).meta.params is None  # []: cleared, stored as absence


def test_managed_on_a_non_prompt_is_refused_before_any_write(tmp_path: Path) -> None:
    """write_prompt_managed's prompt-only rule travels with the fold: a non-prompt
    entry refuses the managed half, and the refusal happens before the meta moves."""
    entry = store.add_command("echo {A}", name="cmdjob")
    before = store.resolve(entry.slug).meta

    with pytest.raises(store.StoreUsageError, match=r"^cmdjob isn't a prompt entry\.$"):
        store.write_parameters(entry.slug, [ParamDecl(name="A")], managed=["A"])

    after = store.resolve(entry.slug).meta
    assert after.params == before.params
    assert after.parameters == before.parameters


def test_a_failed_schema_write_leaves_both_halves_old(tmp_path: Path) -> None:
    """The finding's failure mode, injected: the meta commit dies — and the on-disk
    schema stays WHOLLY old, managed list and declared rows alike. No half state."""
    entry = _prompt(tmp_path)
    store.write_parameters(
        entry.slug, [ParamDecl(name="topic", delivery="placeholder")], managed=["topic"]
    )

    def _boom(*_a: object, **_k: object) -> None:
        raise OSError(28, "No space left on device", "meta.toml")

    # A private MonkeyPatch context: the function-scoped fixture also holds this
    # test's SKIT_*_DIR env vars, and undoing those with the patch would point the
    # post-failure read at the real library.
    with pytest.MonkeyPatch.context() as mp:
        mp.setattr(store, "_write_meta_and_row", _boom)
        with pytest.raises(OSError, match="No space left on device"):
            store.write_parameters(
                entry.slug,
                [ParamDecl(name="tone", delivery="placeholder")],
                managed=["tone"],
            )

    fresh = store.resolve(entry.slug)
    assert fresh.meta.params == ["topic"]
    assert [d["name"] for d in fresh.meta.parameters or []] == ["topic"]


# ==========================================================================
# U. the secret transition purges before it commits
# ==========================================================================


def _python_with_public_token(tmp_path: Path) -> store.Entry:
    from skit.langs.python import metawriter

    text = metawriter.write_params(
        'TOKEN = "t"\nprint(TOKEN)\n',
        [ParamDecl(name="TOKEN", binding="const", type="str", default="t")],
    )
    script = tmp_path / "job.py"
    script.write_text(text, encoding="utf-8")
    entry = store.add_python(script, name="job")
    # Plaintext remembered while TOKEN was public — the transition's whole subject.
    argstate.save_last(entry.slug, values={"TOKEN": "plaintext"})
    return entry


def _stored_secret_flag(entry: store.Entry) -> bool:
    from skit.langs.python import metawriter

    text = entry.script_path.read_text(encoding="utf-8")
    (spec,) = metawriter.read_params(text)
    return spec.secret


def test_spec_lane_interruption_never_leaves_secret_schema_with_plaintext(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    """The finding's exact window, injected at the schema write: the purge has already
    run, so the failed transition lands on public+no-value — allowed — and never on
    secret+plaintext. (This also PINS the order: were the write first, the plaintext
    would still be on disk here.)"""
    entry = _python_with_public_token(tmp_path)

    def _boom(*_a: object, **_k: object) -> None:
        raise OSError(28, "No space left on device", "script.py")

    monkeypatch.setattr(store, "write_block_edit", _boom)  # the write half of write_source_params
    result = runner.invoke(cli.app, ["params", entry.slug, "--secret", "TOKEN"])

    assert result.exit_code != 0
    assert _stored_secret_flag(entry) is False  # schema still public…
    assert "TOKEN" not in argstate.load_state(entry.slug)["values"]  # …and the value is gone


def test_declared_lane_interruption_never_leaves_secret_schema_with_plaintext(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    """Same invariant on the declared lane (prompt/exe/command rows), injected at the
    one merged meta write."""
    entry = _prompt(tmp_path)
    argstate.save_last(entry.slug, values={"topic": "plaintext"})

    def _boom(*_a: object, **_k: object) -> None:
        raise OSError(28, "No space left on device", "meta.toml")

    monkeypatch.setattr(store, "_write_meta_and_row", _boom)  # write_parameters' commit half
    result = runner.invoke(cli.app, ["params", entry.slug, "--secret", "topic"])

    assert result.exit_code != 0
    rows = store.resolve(entry.slug).meta.parameters or []
    assert not any(r.get("secret") for r in rows)  # schema still public…
    assert "topic" not in argstate.load_state(entry.slug)["values"]  # …value gone


def test_a_failed_purge_aborts_the_transition_with_the_schema_still_public(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    """The other interruption point: the purge itself dies. The transition must stop
    BEFORE the schema commits — operational exit, schema public, plaintext still there
    (public+value is an allowed state; secret+value is not)."""
    entry = _python_with_public_token(tmp_path)
    # Injected under the purge's own write, so the failure arrives TYPED — the same
    # StateWriteError lane every argstate writer feeds the boundary.
    monkeypatch.setattr(argstate, "atomic_write_toml", _state_boom)

    result = runner.invoke(cli.app, ["params", entry.slug, "--secret", "TOKEN"])

    assert result.exit_code == EXIT_SKIT
    assert "Traceback" not in result.output
    assert _stored_secret_flag(entry) is False  # the write never ran
    assert argstate.load_state(entry.slug)["values"] == {"TOKEN": "plaintext"}


def test_the_happy_transition_still_scrubs_and_commits(tmp_path: Path) -> None:
    """No interruption: the reorder must not change the destination — schema secret,
    plaintext scrubbed, and the cleanup reported."""
    entry = _python_with_public_token(tmp_path)

    result = runner.invoke(cli.app, ["params", entry.slug, "--secret", "TOKEN"])

    assert result.exit_code == 0
    assert "Removed previously stored plaintext" in result.output
    assert _stored_secret_flag(entry) is True
    assert "TOKEN" not in argstate.load_state(entry.slug)["values"]


# ==========================================================================
# V. raw runs honor the C3 scrub
# ==========================================================================


def test_stored_secret_names_placeholder_kind_follows_the_declared_override(
    tmp_path: Path,
) -> None:
    """Placeholder kinds read the meta exactly as the plan does: a declared row's
    verdict beats the name heuristic (secret=False on a scary name stays public), a
    managed hole with no row falls to the heuristic, and a plain name stays out."""
    src = tmp_path / "p.prompt.md"
    src.write_text("Use {{api_key}} for {{topic}} via {{api_token}}\n", encoding="utf-8")
    entry = store.add_prompt(src, name="holes")
    store.write_parameters(
        entry.slug,
        [ParamDecl(name="api_token", delivery="placeholder", secret=False)],  # explicit override
        managed=["api_key", "topic", "api_token"],
    )

    assert flows.stored_secret_names(store.resolve(entry.slug)) == {"api_key"}


def test_stored_secret_names_reads_the_block_without_an_analyzer(tmp_path: Path) -> None:
    """A params_io kind unions the block's own secret flags with the declared rider
    rows — grammar-free reads only, no tree-sitter, no analyzer."""
    entry = _python_with_public_token(tmp_path)
    assert flows.stored_secret_names(store.resolve(entry.slug)) == set()  # still public

    runner.invoke(cli.app, ["params", entry.slug, "--secret", "TOKEN"])

    assert flows.stored_secret_names(store.resolve(entry.slug)) == {"TOKEN"}


def test_stored_secret_names_survives_an_unreadable_copy(tmp_path: Path) -> None:
    """The helper is total: a copy that cannot be read degrades to the declared rows
    instead of failing the run lane that asked."""
    entry = _python_with_public_token(tmp_path)
    entry.script_path.unlink()
    entry.script_path.mkdir()  # exists, unreadable as a file → OSError on read

    assert flows.stored_secret_names(store.resolve(entry.slug)) == set()


def test_stored_secret_names_declared_only_kind(tmp_path: Path) -> None:
    """No params_io, no placeholders (exe): the declared rows are the whole answer."""
    tool = tmp_path / "mytool"
    tool.write_text("#!/bin/sh\necho hi\n", encoding="utf-8")
    tool.chmod(0o755)
    entry = store.add_exe(tool, name="prog")
    store.write_parameters(
        entry.slug,
        [
            ParamDecl(name="token", delivery="env", secret=True),
            ParamDecl(name="region", delivery="env"),
        ],
    )

    assert flows.stored_secret_names(store.resolve(entry.slug)) == {"token"}


def test_save_after_raw_run_scrubs_and_stamps_but_rewrites_no_form_memory(
    tmp_path: Path,
) -> None:
    """The raw twin: [values]/presets/last_run lose the now-secret plaintext (the same
    purge every accepted run performs), the stamp lands — and extra_args, the form
    memory --raw promises not to rewrite, survives byte-for-byte."""
    entry = _python_with_public_token(tmp_path)
    runner.invoke(cli.app, ["params", entry.slug, "--secret", "TOKEN"])
    # Stale plaintext written while TOKEN was public (the purge-bypassing route: a
    # $EDITOR edit of the stored copy, or a purge that died mid-transition).
    argstate.save_last(entry.slug, values={"TOKEN": "plain"}, extra_args=["--fast"])
    argstate.save_preset(entry.slug, "prod", {"TOKEN": "plain", "REGION": "eu"})
    argstate.record_run(entry.slug, 1, at="2026-01-01T00:00:00+00:00", values={"TOKEN": "plain"})

    flows.save_after_raw_run(store.resolve(entry.slug), 0, at="2026-02-01T00:00:00+00:00")

    state = argstate.load_state(entry.slug)
    assert "TOKEN" not in state["values"]
    assert state["presets"] == {"prod": {"REGION": "eu"}}
    assert "TOKEN" not in state["last_run"].get("values", {})
    assert state["last_run"]["exit"] == 0  # the stamp landed
    assert state["extra_args"] == ["--fast"]  # form memory untouched


def test_a_raw_run_applies_the_current_secret_set_to_the_preserved_snapshot(
    tmp_path: Path,
) -> None:
    """The finding end-to-end, on the CLI face: plaintext recorded while the parameter
    was public, the schema flipped to secret out of band, then `run --raw` — the run
    passes its code through and the re-persisted state carries no plaintext on any
    surface."""
    script = tmp_path / "job.sh"
    script.write_text('#!/bin/sh\nTOKEN="x"\necho ok\n', encoding="utf-8")
    result = runner.invoke(cli.app, ["add", str(script), "--name", "job"])
    assert result.exit_code == 0
    entry = store.resolve("job")
    result = runner.invoke(cli.app, ["params", entry.slug, "--manage", "TOKEN"])
    assert result.exit_code == 0
    result = runner.invoke(cli.app, ["params", entry.slug, "--secret", "TOKEN"])
    assert result.exit_code == 0
    assert flows.stored_secret_names(store.resolve(entry.slug)) == {"TOKEN"}
    # Plaintext that predates the transition, on every value-bearing surface — seeded
    # AFTER the purge, as a hand edit of the block (no purge) would have left it.
    argstate.save_last(entry.slug, values={"TOKEN": "plain"})
    argstate.save_preset(entry.slug, "prod", {"TOKEN": "plain"})
    argstate.record_run(entry.slug, 1, at="2026-01-01T00:00:00+00:00", values={"TOKEN": "plain"})

    result = runner.invoke(cli.app, ["run", entry.slug, "--raw", "--no-input"])

    assert result.exit_code == 0  # the script's own code, passed through
    state = argstate.load_state(entry.slug)
    assert "TOKEN" not in state["values"]
    assert state["presets"] == {}  # the preset held only the secret → dropped whole
    assert "TOKEN" not in state["last_run"].get("values", {})


def test_stored_secret_names_unions_the_block_with_declared_riders(tmp_path: Path) -> None:
    """The two sources ACCUMULATE: a block-declared secret and a meta-declared rider
    secret both strip. An overwrite of one set by the other would drop whichever source
    happened to be read first."""
    entry = _python_with_public_token(tmp_path)
    runner.invoke(cli.app, ["params", entry.slug, "--secret", "TOKEN"])
    store.write_parameters(entry.slug, [ParamDecl(name="EXTRA_KEY", delivery="env", secret=True)])

    assert flows.stored_secret_names(store.resolve(entry.slug)) == {"TOKEN", "EXTRA_KEY"}


def test_stored_secret_names_survives_a_non_utf8_copy(tmp_path: Path) -> None:
    """errors="replace", behaviourally: a stored copy carrying non-UTF-8 bytes outside
    the block still yields the block's own secret flags instead of raising — the same
    tolerance every analyzer read applies."""
    entry = _python_with_public_token(tmp_path)
    runner.invoke(cli.app, ["params", entry.slug, "--secret", "TOKEN"])
    raw = entry.script_path.read_bytes()
    entry.script_path.write_bytes(raw + b"# caf\xe9 comment, latin-1 bytes\n")

    assert flows.stored_secret_names(store.resolve(entry.slug)) == {"TOKEN"}


def test_record_runs_own_strip_holds_even_if_the_purge_is_gone(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """save_after_raw_run's record_run carries the secret set in its OWN right — the
    C3 'every write entry point strips' line, not a rider on the purge. Proven by
    removing the purge: the re-persisted snapshot still loses the now-secret key."""
    entry = _python_with_public_token(tmp_path)
    runner.invoke(cli.app, ["params", entry.slug, "--secret", "TOKEN"])
    argstate.record_run(entry.slug, 1, at="2026-01-01T00:00:00+00:00", values={"TOKEN": "plain"})
    monkeypatch.setattr(argstate, "purge_secret", lambda *_a, **_k: set())

    flows.save_after_raw_run(store.resolve(entry.slug), 0, at="2026-02-01T00:00:00+00:00")

    state = argstate.load_state(entry.slug)
    assert "TOKEN" not in state["last_run"].get("values", {})
    assert state["last_run"]["exit"] == 0


def test_remove_reports_partial_success_when_state_cleanup_fails(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """forget joins the typed-writer contract, and remove answers its failure the way
    the rmtree branch always has: the entry IS removed (registry and directory), and
    the refusal says exactly what survived and how to finish the job — never a raw
    traceback for cleanup that happened after the removal already succeeded."""
    entry = _cmd("doomed")
    argstate.save_last(entry.slug, values={"A": "1"})

    def _boom_forget(_slug: str) -> None:
        raise argstate.StateWriteError(13, "Permission denied", f"{entry.slug}.toml")

    monkeypatch.setattr(argstate, "forget", _boom_forget)
    result = runner.invoke(cli.app, ["remove", entry.slug, "--yes"])

    assert result.exit_code == EXIT_SKIT
    assert "couldn't be deleted" in result.output
    assert "Traceback" not in result.output
    with pytest.raises(store.NotFoundError):
        store.resolve(entry.slug)  # the removal itself held


def test_a_failed_pick_memory_never_vetoes_the_cli_run(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The CLI twin of the TUI rule: remembering --runner is incidental prefill state,
    so a state dir that cannot take the write warns and the run proceeds — before this,
    the typed failure reached the boundary and vetoed the accepted run with 125."""
    from skit import config

    src = tmp_path / "p.prompt.md"
    src.write_text("Say {{a}}\n", encoding="utf-8")
    entry = store.add_prompt(src, name="p")
    runner_name = config.load_prompt_runners()[0].name
    monkeypatch.setattr(argstate, "atomic_write_toml", _state_boom)

    result = runner.invoke(
        cli.app,
        ["run", entry.slug, "--runner", runner_name, "--set", "a=x", "--no-input", "--dry-run"],
    )

    assert result.exit_code == 0
    assert "couldn't be remembered" in result.output


def test_raw_persistence_reads_the_meta_fresh_at_stamp_time(tmp_path: Path) -> None:
    """The entry object in hand predates a run that may have lasted hours. A parameter
    flipped SECRET on disk mid-run must still be scrubbed at stamp time — the stale
    launch-time verdict alone would re-persist the plaintext."""
    entry = _python_with_public_token(tmp_path)
    stale = store.resolve(entry.slug)  # public at launch
    runner.invoke(cli.app, ["params", entry.slug, "--secret", "TOKEN"])
    # Plaintext seeded AFTER the transition's purge — the stale snapshot the finding
    # says a long raw run would re-persist.
    argstate.record_run(entry.slug, 1, at="2026-01-01T00:00:00+00:00", values={"TOKEN": "plain"})

    flows.save_after_raw_run(stale, 0, at="2026-02-01T00:00:00+00:00")

    state = argstate.load_state(entry.slug)
    assert "TOKEN" not in state["last_run"].get("values", {})
    assert state["last_run"]["exit"] == 0


def test_a_mid_run_unsecreting_cannot_cancel_the_scrub(tmp_path: Path) -> None:
    """The union's other direction: a race can only WIDEN the scrub. A declared row
    that was secret at launch and public by stamp time still strips — replacing the
    set with the fresh reading alone would let the race talk a name out of secrecy.
    (A prompt, because declared-row secrecy is what the held entry object actually
    freezes: block-based kinds re-read their file either way.)"""
    entry = _prompt(tmp_path)
    store.write_parameters(
        entry.slug,
        [ParamDecl(name="topic", delivery="placeholder", secret=True)],
        managed=["topic"],
    )
    stale = store.resolve(entry.slug)  # secret at launch
    store.write_parameters(
        entry.slug,
        [ParamDecl(name="topic", delivery="placeholder", secret=False)],
        managed=["topic"],
    )
    argstate.record_run(entry.slug, 1, at="2026-01-01T00:00:00+00:00", values={"topic": "plain"})

    flows.save_after_raw_run(stale, 0, at="2026-02-01T00:00:00+00:00")

    assert "topic" not in argstate.load_state(entry.slug)["last_run"].get("values", {})


def test_a_removed_entry_gets_no_posthumous_state(tmp_path: Path) -> None:
    """An entry removed mid-run gets no stamp: remove() just deleted the values file,
    and writing state for a slug the library no longer claims would resurrect it as
    an orphan."""
    from skit.paths import values_dir

    entry = _python_with_public_token(tmp_path)
    stale = store.resolve(entry.slug)
    store.remove(entry.slug)

    flows.save_after_raw_run(stale, 0, at="2026-02-01T00:00:00+00:00")  # no raise

    assert not (values_dir() / f"{entry.slug}.toml").exists()
