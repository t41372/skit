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
    message = str(exc_info.value)
    assert "deploy" in message
    expected = ", ".join(sorted([a.slug, b.slug]))
    assert expected in message
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
