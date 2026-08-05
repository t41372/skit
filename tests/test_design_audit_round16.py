"""Behavior coverage for the design-audit round-16 fixes (the doors-outside-the-doors
review).

Round 15 built identity-authorized, entry-locked transactions — and then classified
several CLI writers as "immediate" that genuinely wait on a human or derive their
write from an earlier read. This round closes them:

J. `preset save` waited on an interactive intake and then wrote argstate bare —
   inside a secret transition's own window, re-leaking plaintext the scrub had just
   removed. It now claims identity at hold-start and saves through the guarded
   `flows.save_preset_for` door (union strip, entry lock). `preset delete` waited on
   its confirmation ask the same way; it deletes through `flows.delete_presets_for`.
K. Every params/deps edit lane claims identity BEFORE the read its write depends on
   and authorizes each store call with `expected_id` — "usually fast" is not an
   identity guarantee. `--normalize` moves into `store.rewrite_source`: the whole
   read-transform-write runs under the entry lock, re-derived from the fresh text.
L. `run` claims identity only after its static refusals: a refused invocation leaves
   no fingerprints, stamping included.
M. Unknown identity softening is gone: `expected_id=""` is a real expectation (an
   unstamped handle refuses a stamped stranger), and `claim_identity` answers from the disk after
   a failed stamp so a half-landed id can never strand a handle behind its own entry.
"""

from __future__ import annotations

import os
from collections.abc import Callable
from pathlib import Path

import pytest
from textual.widgets import Input
from typer.testing import CliRunner

from skit import argstate, cli, store, tui
from skit.models import ScriptMeta
from skit.params import ParamDecl
from skit.paths import values_dir
from skit.tui_settings import ScriptSettingsScreen

runner = CliRunner()

STALE = "changed while this edit was underway"


@pytest.fixture(autouse=True)
def tmp_store(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path / "config"))
    monkeypatch.setenv("SKIT_LANG", "en")
    return tmp_path


def _cmd(name: str, template: str = "echo {x}") -> store.Entry:
    return store.add_command(template, name=name)


def _strip_id_line(slug: str) -> None:
    path = store.scripts_dir() / slug / "meta.toml"
    kept = [
        line
        for line in path.read_text(encoding="utf-8").splitlines()
        if not line.startswith("id = ")
    ]
    path.write_text("\n".join(kept) + "\n", encoding="utf-8")


# ==========================================================================
# J. preset save/delete — held commands, guarded doors
# ==========================================================================


def test_preset_save_scrubs_a_secret_that_flipped_during_the_intake(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The review's re-leak scenario: the schema turns a name secret while the intake
    waits on the user. The door's strip set is the union of launch-time and CURRENT
    secrecy, so the value the transition just scrubbed can never ride back in through
    this save."""
    entry = _cmd("intake", "echo {mode}")

    def _flip_then_answer(_entry: store.Entry, _plan: object, *, from_last: bool) -> dict[str, str]:
        store.write_parameters(
            entry.slug, [ParamDecl(name="mode", delivery="placeholder", secret=True)]
        )
        return {"mode": "hunter2"}

    monkeypatch.setattr(cli, "_preset_values", _flip_then_answer)
    result = runner.invoke(cli.app, ["preset", "save", "intake", "p"])
    assert result.exit_code == 0, result.output
    assert argstate.load_state(entry.slug)["presets"]["p"] == {}  # stripped, not stored


def test_preset_save_refuses_a_slug_reissued_during_the_intake(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    entry = _cmd("reissued", "echo {x}")

    def _race_then_answer(_entry: store.Entry, _plan: object, *, from_last: bool) -> dict[str, str]:
        store.remove("reissued")
        _cmd("reissued", "echo {x}")
        return {"x": "old value"}

    monkeypatch.setattr(cli, "_preset_values", _race_then_answer)
    result = runner.invoke(cli.app, ["preset", "save", "reissued", "p"])
    assert result.exit_code == 127
    assert "wasn't saved" in result.output
    assert not (values_dir() / f"{entry.slug}.toml").exists()  # the new owner got nothing


def test_preset_save_claims_identity_at_hold_start(tmp_path: Path) -> None:
    """A pre-id entry gets stamped by the save's handshake — the door's exact match
    needs an identity to authorize against."""
    entry = _cmd("stampme", "echo {x}")
    argstate.record_run(entry.slug, 0, at="2026-01-01T00:00:00+00:00", values={"x": "1"})
    _strip_id_line(entry.slug)
    result = runner.invoke(cli.app, ["preset", "save", "stampme", "p", "--from-last"])
    assert result.exit_code == 0, result.output
    assert store.resolve(entry.slug).meta.id
    assert argstate.load_state(entry.slug)["presets"]["p"] == {"x": "1"}


def test_preset_delete_refuses_a_slug_reissued_during_the_ask(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The confirmed deletion has no owner anymore: the answer was never about the new
    entry's same-named preset, so that preset survives."""
    old = _cmd("askrace", "echo {x}")
    argstate.save_preset(old.slug, "p", {"x": "1"})

    def _race(_message: str) -> None:
        store.remove("askrace")
        fresh = _cmd("askrace", "echo {x}")
        argstate.save_preset(fresh.slug, "p", {"x": "theirs"})

    monkeypatch.setattr(cli, "_require_yes", lambda *_a, **_k: None)
    monkeypatch.setattr(cli, "_confirm_destructive", _race)
    result = runner.invoke(cli.app, ["preset", "delete", "askrace", "p"])
    assert result.exit_code == 127
    assert STALE in result.output
    assert argstate.load_state(old.slug)["presets"] == {"p": {"x": "theirs"}}


# ==========================================================================
# K. params/deps edit lanes — claimed identities, authorized writes
# ==========================================================================


def _reissue_and_hold(
    monkeypatch: pytest.MonkeyPatch, factory: Callable[[], store.Entry]
) -> tuple[store.Entry, store.Entry]:
    """Simulate the race at its seam: the claim hands back a handle whose slug has
    already been reissued — every authorized write after it must refuse."""
    old = factory()
    store.remove(old.slug)
    new = factory()
    assert new.slug == old.slug
    monkeypatch.setattr(store, "claim_identity", lambda _entry: old)
    return old, new


def test_a_stale_declared_edit_refuses(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    _, new = _reissue_and_hold(monkeypatch, lambda: _cmd("rows16"))
    result = runner.invoke(cli.app, ["params", "rows16", "--add", "y"])
    assert result.exit_code == 125
    assert STALE in result.output
    assert store.resolve(new.slug).meta.parameters is None


def test_a_stale_template_edit_refuses(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    _, new = _reissue_and_hold(monkeypatch, lambda: _cmd("tmpl16"))
    result = runner.invoke(cli.app, ["params", "tmpl16", "--template", "echo changed"])
    assert result.exit_code == 125
    assert STALE in result.output
    assert store.resolve(new.slug).meta.template == "echo {x}"


def test_a_stale_spec_edit_refuses(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    src = tmp_path / "s.sh"
    src.write_text('#!/bin/sh\nTOKEN="x"\necho "$TOKEN"\n', encoding="utf-8")
    _, new = _reissue_and_hold(
        monkeypatch, lambda: store.add_script(src, kind="shell", name="spec16")
    )
    before = new.script_path.read_bytes()
    result = runner.invoke(cli.app, ["params", "spec16", "--manage", "TOKEN"])
    assert result.exit_code == 125
    assert STALE in result.output
    assert new.script_path.read_bytes() == before


def test_a_stale_normalize_refuses_before_transforming(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """rewrite_source checks identity under the lock BEFORE reading or transforming:
    the dead handle's analysis never even runs against the new owner's text."""
    src = tmp_path / "n.sh"
    src.write_text('#!/bin/sh\nGREETING="hi"\necho "$GREETING"\n', encoding="utf-8")
    _, new = _reissue_and_hold(
        monkeypatch, lambda: store.add_script(src, kind="shell", name="norm16")
    )
    before = new.script_path.read_bytes()
    result = runner.invoke(cli.app, ["params", "norm16", "--normalize", "GREETING"])
    assert result.exit_code == 125
    assert STALE in result.output
    assert new.script_path.read_bytes() == before


def test_a_stale_interpolate_flip_refuses(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    src = tmp_path / "p.prompt.md"
    src.write_text("Hi {{x}}\n", encoding="utf-8")
    _, new = _reissue_and_hold(monkeypatch, lambda: store.add_prompt(src, name="interp16"))
    result = runner.invoke(cli.app, ["params", "interp16", "--no-interpolate"])
    assert result.exit_code == 125
    assert STALE in result.output
    assert store.resolve(new.slug).meta.interpolate is True


def test_a_stale_runner_pin_refuses(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    src = tmp_path / "p.prompt.md"
    src.write_text("Hi\n", encoding="utf-8")
    _, new = _reissue_and_hold(monkeypatch, lambda: store.add_prompt(src, name="pin16"))
    result = runner.invoke(cli.app, ["params", "pin16", "--runner", ""])
    assert result.exit_code == 125
    assert STALE in result.output
    assert store.resolve(new.slug).meta.runner == ""


def test_a_stale_deps_edit_refuses(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    src = tmp_path / "d.py"
    src.write_text('"""Doc."""\nprint(1)\n', encoding="utf-8")
    _, new = _reissue_and_hold(
        monkeypatch, lambda: store.add_python(src, name="deps16", mode="reference")
    )
    result = runner.invoke(cli.app, ["deps", "deps16", "--dep", "httpx"])
    assert result.exit_code == 125
    assert STALE in result.output
    assert store.resolve(new.slug).meta.dependencies is None


def test_a_stale_needs_edit_refuses(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    _, new = _reissue_and_hold(monkeypatch, lambda: _cmd("needs16"))
    result = runner.invoke(cli.app, ["deps", "needs16", "--need", "jq"])
    assert result.exit_code == 125
    assert STALE in result.output
    assert store.resolve(new.slug).meta.needs is None


def test_the_params_read_view_never_claims(tmp_path: Path) -> None:
    """Reads stay reads: a bare `skit params X` (and `skit deps X`) on a legacy meta
    stamps nothing."""
    entry = _cmd("view16")
    _strip_id_line(entry.slug)
    assert runner.invoke(cli.app, ["params", "view16"]).exit_code == 0
    assert runner.invoke(cli.app, ["deps", "view16"]).exit_code == 0
    assert store.resolve(entry.slug).meta.id == ""


# ==========================================================================
# L. run — no fingerprints on a refused invocation
# ==========================================================================


def test_a_refused_run_leaves_a_legacy_meta_unstamped(tmp_path: Path) -> None:
    """The command's own doctrine, extended to the stamp: a run that exits on a static
    refusal has written NOTHING — not even the identity heal."""
    entry = _cmd("norun", "echo hi")
    _strip_id_line(entry.slug)
    conflict = runner.invoke(cli.app, ["run", "norun", "--raw", "--set", "a=1"])
    assert conflict.exit_code == 2
    assert store.resolve(entry.slug).meta.id == ""
    ghost = runner.invoke(cli.app, ["run", "norun", "--preset", "ghost", "--no-input"])
    assert ghost.exit_code == 2
    assert store.resolve(entry.slug).meta.id == ""


# ==========================================================================
# M. the store honors "" as a real expectation; the stamp cannot strand a handle
# ==========================================================================


def test_an_empty_expectation_refuses_a_stamped_stranger(tmp_path: Path) -> None:
    """expected_id="" says "I hold an unstamped handle": a stamped entry under that
    slug proves the disk changed owners, so the write fails closed — "" is never a
    way to switch the guard off."""
    entry = _cmd("strict16")
    with pytest.raises(store.StaleEntryError):
        store.update_description(entry.slug, "from the unstamped handle", expected_id="")
    assert store.resolve(entry.slug).meta.description == ""


def test_an_empty_expectation_matches_an_unstamped_meta(tmp_path: Path) -> None:
    entry = _cmd("lenient16")
    _strip_id_line(entry.slug)
    store.update_description(entry.slug, "authorized", expected_id="")
    assert store.resolve(entry.slug).meta.description == "authorized"


async def test_an_unstamped_settings_screen_cannot_write_over_a_stamped_entry(
    tmp_path: Path,
) -> None:
    """The `id or None` hole, closed: a screen holding an unstamped handle meets a
    stamped disk — the save refuses instead of silently disabling its own guard."""
    entry = _cmd("orhole")
    _strip_id_line(entry.slug)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        screen = ScriptSettingsScreen(store.resolve(entry.slug))  # unstamped, unclaimed
        app.push_screen(screen)
        await pilot.pause()
        store.update_description(entry.slug, "concurrently healed")  # stamps the disk
        screen.query_one("#st-desc", Input).value = "the unstamped screen's edit"
        screen.action_save()
        await pilot.pause()
        assert any(STALE in n.message for n in app._notifications)
    assert store.resolve(entry.slug).meta.description == "concurrently healed"


def test_a_half_landed_stamp_still_returns_the_stamped_handle(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """meta written, registry row failed: the handle must carry the id that IS on
    disk, or every later exact-match would refuse the handle's own entry."""
    entry = _cmd("halfstamp")
    _strip_id_line(entry.slug)

    def _half(entry_dir: Path, _slug: str, meta: ScriptMeta) -> None:
        store._write_meta(entry_dir, meta)
        raise OSError(28, "No space left on device", "registry.toml")

    monkeypatch.setattr(store, "_write_meta_and_row", _half)
    held = store.claim_identity(store.resolve(entry.slug))
    assert held.meta.id
    assert held.meta.id == store.resolve(entry.slug).meta.id


def test_rewrite_source_is_a_locked_identity_checked_transaction(tmp_path: Path) -> None:
    """The A5 lane's contract: fresh-read, transform, byte-disciplined write — and a
    stale expectation refuses before the transform ever runs."""
    src = tmp_path / "r.sh"
    src.write_bytes(b'#!/bin/sh\r\nGREETING="hi"\r\necho "$GREETING"\r\n')
    entry = store.add_script(src, kind="shell", name="rw16")
    ran: list[str] = []

    def _upper(text: str) -> str:
        ran.append(text)
        return text.replace('GREETING="hi"', 'GREETING="HI"')

    store.rewrite_source(entry.slug, _upper, expected_id=entry.meta.id)
    assert ran  # the transform saw the fresh text
    assert b'GREETING="HI"\r\n' in entry.script_path.read_bytes()  # CRLF survived

    with pytest.raises(store.StaleEntryError):
        store.rewrite_source(entry.slug, _upper, expected_id="someone-else")
    assert len(ran) == 1  # refused BEFORE transforming

    before = os.stat(entry.script_path).st_mtime_ns
    store.rewrite_source(entry.slug, lambda _text: None, expected_id=entry.meta.id)
    assert os.stat(entry.script_path).st_mtime_ns == before  # None = write nothing


def test_rewrite_source_names_a_missing_copy(tmp_path: Path) -> None:
    src = tmp_path / "gone.sh"
    src.write_text("#!/bin/sh\necho hi\n", encoding="utf-8")
    entry = store.add_script(src, kind="shell", name="gone16")
    entry.script_path.unlink()
    with pytest.raises(store.NotFoundError, match="no stored copy"):
        store.rewrite_source(entry.slug, lambda text: text)


def test_a_stamp_failure_on_an_unreadable_library_fails_the_claim_honestly(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """When the stamp fails AND the library cannot even be read back, the claim
    RAISES — its answer is load-bearing (writes authorize against it), so a handle it
    cannot verify is a refusal, never a guess."""
    entry = _cmd("degrade16")
    _strip_id_line(entry.slug)
    state = {"broken": False}

    def _boom(*_args: object, **_kwargs: object) -> None:
        state["broken"] = True
        raise OSError(5, "I/O error", "meta.toml")

    real_resolve = store.resolve

    def _flaky(name_or_slug: str) -> store.Entry:
        if state["broken"]:
            raise store.CorruptEntryError("unreadable after the failure")
        return real_resolve(name_or_slug)

    held = store.resolve(entry.slug)
    monkeypatch.setattr(store, "_write_meta_and_row", _boom)
    monkeypatch.setattr(store, "resolve", _flaky)
    with pytest.raises(store.CorruptEntryError):
        store.claim_identity(held)


# ==========================================================================
# exact copy pins — the commit helpers' messages are UI contract, not decoration
# ==========================================================================


def _output_lines(output: str) -> list[str]:
    return [line.strip() for line in output.splitlines()]


def test_preset_delete_success_says_exactly_what_it_deleted(tmp_path: Path) -> None:
    entry = _cmd("delok")
    argstate.save_preset(entry.slug, "p", {"x": "1"})
    result = runner.invoke(cli.app, ["preset", "delete", "delok", "p", "--yes"])
    assert result.exit_code == 0, result.output
    assert 'Preset "p" deleted from delok.' in _output_lines(result.output)
    assert argstate.load_state(entry.slug)["presets"] == {}


def test_a_preset_vanishing_during_the_ask_lands_in_the_unknown_refusal(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The pre-ask check's own promise: a preset deleted out from under the ask gives
    the same unknown-preset error the check would have given — through the door."""
    entry = _cmd("delrace")
    argstate.save_preset(entry.slug, "p", {"x": "1"})
    monkeypatch.setattr(cli, "_require_yes", lambda *_a, **_k: None)
    monkeypatch.setattr(
        cli, "_confirm_destructive", lambda _m: argstate.delete_preset(entry.slug, "p")
    )
    result = runner.invoke(cli.app, ["preset", "delete", "delrace", "p"])
    assert result.exit_code == 2
    assert 'Unknown preset "p"' in result.output


def test_the_stale_delete_refusal_is_exact(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    old = _cmd("delstale")
    argstate.save_preset(old.slug, "p", {"x": "1"})

    def _race(_message: str) -> None:
        store.remove("delstale")
        _cmd("delstale")

    monkeypatch.setattr(cli, "_require_yes", lambda *_a, **_k: None)
    monkeypatch.setattr(cli, "_confirm_destructive", _race)
    result = runner.invoke(cli.app, ["preset", "delete", "delstale", "p"])
    assert result.exit_code == 127
    assert (
        "delstale changed while this edit was underway — reopen it and try again."
        in _output_lines(result.output)
    )


def test_preset_save_success_and_refusal_copy_are_exact(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    entry = _cmd("saveok", "echo {x}")
    argstate.record_run(entry.slug, 0, at="2026-01-01T00:00:00+00:00", values={"x": "1"})
    ok = runner.invoke(cli.app, ["preset", "save", "saveok", "p", "--from-last"])
    assert ok.exit_code == 0, ok.output
    assert 'Preset "p" saved for saveok.' in _output_lines(ok.output)

    def _race_then_answer(_entry: store.Entry, _plan: object, *, from_last: bool) -> dict[str, str]:
        store.remove("saveok")
        _cmd("saveok", "echo {x}")
        return {"x": "old"}

    monkeypatch.setattr(cli, "_preset_values", _race_then_answer)
    refused = runner.invoke(cli.app, ["preset", "save", "saveok", "q"])
    assert refused.exit_code == 127
    assert 'Preset "q" wasn\'t saved — saveok is no longer in the library.' in _output_lines(
        refused.output
    )


def test_rewrite_source_refuses_a_copy_that_does_not_decode(tmp_path: Path) -> None:
    """The strict-UTF-8 policy is the lane's own: a copy that doesn't decode is
    refused whole, before the transform ever sees a replacement character."""
    src = tmp_path / "b.sh"
    src.write_text("#!/bin/sh\necho hi\n", encoding="utf-8")
    entry = store.add_script(src, kind="shell", name="bytes16")
    entry.script_path.write_bytes(b"#!/bin/sh\necho '\xff\xfe'\n")
    with pytest.raises(UnicodeDecodeError):
        store.rewrite_source(entry.slug, lambda text: text)


def test_the_missing_copy_refusal_is_exact(tmp_path: Path) -> None:
    src = tmp_path / "gone2.sh"
    src.write_text("#!/bin/sh\necho hi\n", encoding="utf-8")
    entry = store.add_script(src, kind="shell", name="gone162")
    entry.script_path.unlink()
    with pytest.raises(store.NotFoundError) as refusal:
        store.rewrite_source(entry.slug, lambda text: text)
    assert str(refusal.value) == "gone162 has no stored copy to edit."
