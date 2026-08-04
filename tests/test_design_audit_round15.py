"""Behavior coverage for the design-audit round-15 fixes (the identity-guard review).

Round 14 introduced entry identity and the guarded persistence doors; this review
found the two holes left in them, and #39's deferral was overruled:

E. The legacy-id WILDCARD authorized writes: an unstamped held handle matched any
   stamped entry, so an upgrade user's long run could still land its state on a
   reincarnated slug. Identity is now EXACT match, and every lane that holds an entry
   across user-paced time claims it at hold-start (``store.claim_identity``) — unknown
   identity may serve reads, but it cannot authorize a write.
F. The guard was check-then-act: ``persistence_target`` verified, released everything,
   then wrote. The doors now hold the ENTRY LOCK — the same lock every meta mutator
   and remove() holds — across verify + strip-set read + every write, and the two
   secret-transition commits (``write_parameters``, ``write_source_params``) run their
   C3 scrub INSIDE that same locked transaction, so a post-run write and a secret flip
   can no longer interleave.
G. #39, fixed rather than deferred: store mutators take ``expected_id`` (checked under
   the lock, ``StaleEntryError`` on mismatch), the settings screen authorizes every
   save against the identity it was OPENED on, and its preset cleanup goes through the
   guarded ``flows.delete_presets_for`` door.
"""

from __future__ import annotations

import os
from collections.abc import Callable, Iterable
from pathlib import Path

import pytest
from textual.pilot import Pilot
from textual.widgets import Checkbox, Input
from typer.testing import CliRunner

from skit import argstate, cli, flows, store, tui
from skit.atomic import try_advisory_file_lock
from skit.params import ParamDecl
from skit.paths import values_dir
from skit.tui_settings import PresetDeleteConfirm, ScriptSettingsScreen

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


def _strip_id_line(slug: str) -> None:
    path = store.scripts_dir() / slug / "meta.toml"
    kept = [
        line
        for line in path.read_text(encoding="utf-8").splitlines()
        if not line.startswith("id = ")
    ]
    path.write_text("\n".join(kept) + "\n", encoding="utf-8")


# ==========================================================================
# E. claim_identity — the hold-start handshake
# ==========================================================================


def test_claim_identity_stamps_a_legacy_meta_at_hold_start(tmp_path: Path) -> None:
    """The wildcard's replacement: the handle a run/form/settings screen holds is
    stamped BEFORE the hold begins, so its later writes authorize by exact match."""
    entry = _cmd("legacy")
    _strip_id_line(entry.slug)
    held = store.claim_identity(store.resolve(entry.slug))
    assert held.meta.id
    assert held.dir == entry.dir
    assert store.resolve(entry.slug).meta.id == held.meta.id  # stamped on disk, not just in hand
    # ...and through the one write door, row and all: a stamp that skipped the registry
    # re-projection would leave a stale row for the next listing's self-heal to absorb.
    assert store._load_registry()[entry.slug] == store._registry_row(held.meta, entry.dir)


def test_claim_identity_never_rewrites_a_stamped_meta(tmp_path: Path) -> None:
    """Idempotent AND write-free on the hot path: every run starts here, so a stamped
    meta must not be rewritten (a gratuitous rewrite would churn the mtime the plan
    cache and the registry row stamp key on)."""
    entry = _cmd("stamped")
    meta_path = store.scripts_dir() / entry.slug / "meta.toml"
    before = os.stat(meta_path).st_mtime_ns
    held = store.claim_identity(entry)
    assert held.meta.id == entry.meta.id
    assert os.stat(meta_path).st_mtime_ns == before


def test_claim_identity_is_honest_about_a_missing_entry(tmp_path: Path) -> None:
    """A vanished entry fails the claim as NotFound — "can't find it" belongs to the
    caller; only the STAMP is best-effort."""
    entry = _cmd("ghost")
    store.remove("ghost")
    with pytest.raises(store.NotFoundError):
        store.claim_identity(entry)


def test_claim_identity_degrades_to_a_verified_unstamped_handle_on_a_readonly_library(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """A library that cannot be written still gets a VERIFIED handle — unstamped,
    which post-run persistence then declines to trust: unwritable-by-us is not
    provably unwritable by other users or OLDER versions, whose adds write no id."""
    entry = _cmd("frozen")
    _strip_id_line(entry.slug)

    def _denied(*_a: object, **_k: object) -> None:
        raise OSError(30, "Read-only file system", "meta.toml")

    monkeypatch.setattr(store, "_write_meta_and_row", _denied)
    held = store.claim_identity(store.resolve(entry.slug))
    assert held.meta.id == ""
    assert flows.persistence_target(held) is None  # unstamped cannot authorize


def test_a_cli_run_stamps_a_legacy_entry_at_hold_start(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The integration proof: `skit run` on a pre-id entry mints the id before the
    run, so even a same-invocation remove + re-add cannot be matched by wildcard —
    and the run's own persistence lands under the stamped identity."""
    entry = _cmd("upgraded")
    _strip_id_line(entry.slug)
    monkeypatch.setattr(flows, "execute", lambda *_a, **_k: flows.RunOutcome(0))
    result = runner.invoke(cli.app, ["run", "upgraded", "--no-input"])
    assert result.exit_code == 0, result.output
    assert store.resolve(entry.slug).meta.id  # stamped by the run's hold-start
    assert argstate.load_state(entry.slug)["last_run"]["exit"] == 0


async def test_the_tui_claim_degrades_like_fresh_when_the_store_refuses(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """_claimed mirrors _fresh's degrade: a handshake that cannot even resolve falls
    back to the held snapshot, and the lane's own missing/error paths then speak."""
    _cmd("held")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()

        def _refuse(_entry: store.Entry) -> store.Entry:
            raise store.CorruptEntryError("meta rotted mid-click")

        monkeypatch.setattr(store, "claim_identity", _refuse)
        held = app._claimed(store.resolve("held"))
        assert held is not None
        assert held.meta.name == "held"


# ==========================================================================
# F. the doors hold the entry lock across verify + write
# ==========================================================================


def test_save_after_run_writes_under_the_entry_lock(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The check-then-act hole, pinned shut: at the moment the door's argstate writes
    run, the entry lock must already be held — remove() and every schema commit need
    that same lock, so nothing can slip between the identity check and the last
    write."""
    entry = _cmd("locked", "echo {msg}")
    plan = flows.plan_for_entry(entry)
    observed: list[bool] = []
    original = argstate.save_last

    def _probe(
        slug: str,
        *,
        values: dict[str, str] | None = None,
        extra_args: list[str] | None = None,
        extra_args_raw: bool = False,
        secret_names: Iterable[str] = (),
    ) -> None:
        with try_advisory_file_lock(store.entry_lock_path(entry.slug)) as acquired:
            observed.append(acquired)
        original(
            slug,
            values=values,
            extra_args=extra_args,
            extra_args_raw=extra_args_raw,
            secret_names=secret_names,
        )

    monkeypatch.setattr(argstate, "save_last", _probe)
    flows.save_after_run(
        entry, plan, {"msg": "hi"}, [], 0, at="2026-02-01T00:00:00+00:00", extra_raw=False
    )
    assert observed == [False]  # the lock was taken — and not by us
    assert argstate.load_state(entry.slug)["last_run"]["exit"] == 0


def test_schema_commits_scrub_under_the_same_entry_lock(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The secret-transition half of the race: write_parameters runs its C3 scrub
    inside the same locked transaction as the meta commit, so a post-run door (which
    holds that lock for its whole verify-then-write) can never interleave between
    "plaintext scrubbed" and "schema says secret"."""
    entry = _cmd("fliplock", "echo {mode}")
    argstate.save_last(entry.slug, values={"mode": "hunter2"})
    observed: list[bool] = []
    original = argstate.purge_secret

    def _probe(slug: str, names: Iterable[str]) -> set[str]:
        with try_advisory_file_lock(store.entry_lock_path(entry.slug)) as acquired:
            observed.append(acquired)
        return original(slug, names)

    monkeypatch.setattr(argstate, "purge_secret", _probe)
    _, purged = store.write_parameters(
        entry.slug, [ParamDecl(name="mode", delivery="placeholder", secret=True)]
    )
    assert observed == [False]
    assert purged == {"mode"}
    assert "mode" not in argstate.load_state(entry.slug)["values"]


def test_write_source_params_is_one_locked_scrub_then_commit(tmp_path: Path) -> None:
    """The spec lane's transaction, driven directly: scrub first (the value dies even
    when the commit then fails — round 13's interruption tests pin that half), commit
    second, byte discipline intact."""
    src = tmp_path / "tool.sh"
    src.write_text('#!/bin/sh\n# TOKEN="x"\necho "$TOKEN"\n', encoding="utf-8")
    entry = store.add_script(src, kind="shell", name="spec")
    argstate.save_last(entry.slug, values={"TOKEN": "plaintext"})
    purged = store.write_source_params(
        entry.slug,
        [ParamDecl(name="TOKEN", delivery="env", type="str", secret=True)],
        expected_id=entry.meta.id,
    )
    assert purged == {"TOKEN"}
    assert "TOKEN" not in argstate.load_state(entry.slug)["values"]
    text = entry.script_path.read_text(encoding="utf-8")
    assert "[tool.skit]" in text  # the block landed in the stored copy
    assert store.resolve(entry.slug).meta.id == entry.meta.id  # meta untouched


def test_write_source_params_refuses_a_kind_without_a_block(tmp_path: Path) -> None:
    """The chokepoint guard: a data-driven kind (ruby) launches fine but carries no
    parameter block engine — the transaction refuses instead of inventing one. A
    copy-mode kind on purpose: it isolates the params_io check from the mode check."""
    src = tmp_path / "tool.rb"
    src.write_text('puts "hi"\n', encoding="utf-8")
    entry = store.add_script(src, kind="ruby", name="noblock")
    with pytest.raises(store.StoreUsageError) as refusal:
        store.write_source_params(entry.slug, [ParamDecl(name="x")])
    assert str(refusal.value) == "noblock doesn't carry an editable [tool.skit] block."


def test_write_source_params_refuses_reference_mode(tmp_path: Path) -> None:
    """A5 at the chokepoint: skit edits its stored copy, never the user's original."""
    src = tmp_path / "ref.sh"
    src.write_text("#!/bin/sh\necho hi\n", encoding="utf-8")
    entry = store.add_script(src, kind="shell", name="ref", mode="reference")
    with pytest.raises(store.StoreUsageError) as refusal:
        store.write_source_params(entry.slug, [ParamDecl(name="x")])
    assert str(refusal.value) == (
        "ref is in reference mode, and skit never writes the original file. "
        "Edit the [tool.skit] block in the source directly."
    )


# ==========================================================================
# G. expected_id — write authorization for held screens (#39)
# ==========================================================================


def test_a_stale_expected_id_fails_the_mutation_closed(tmp_path: Path) -> None:
    """The #39 scenario at the store boundary: the screen's identity no longer owns
    the slug — the mutation must refuse, and the new owner's meta must be untouched."""
    old = _cmd("owner")
    store.remove("owner")
    new = _cmd("owner")
    with pytest.raises(store.StaleEntryError) as refusal:
        store.update_description(new.slug, "the old screen's text", expected_id=old.meta.id)
    assert (
        str(refusal.value)
        == "owner changed while this edit was underway — reopen it and try again."
    )
    assert store.resolve(new.slug).meta.description == ""


def test_a_matching_expected_id_authorizes_the_mutation(tmp_path: Path) -> None:
    entry = _cmd("mine")
    store.update_description(entry.slug, "authorized", expected_id=entry.meta.id)
    assert store.resolve(entry.slug).meta.description == "authorized"


def test_rename_honors_the_authorization_too(tmp_path: Path) -> None:
    """rename inlines its own lock ceremony around the uniqueness check — the
    authorization must hold there exactly like in every _locked_entry mutator."""
    old = _cmd("label")
    store.remove("label")
    new = _cmd("label")
    with pytest.raises(store.StaleEntryError):
        store.rename(new.slug, "relabeled", expected_id=old.meta.id)
    assert store.resolve(new.slug).meta.name == "label"


def test_delete_presets_door_refuses_a_reincarnated_slug(tmp_path: Path) -> None:
    """Unticking a checkbox on a dead screen must not delete the new owner's presets
    — and must not conjure a state file for it either."""
    old = _cmd("cleanup")
    store.remove("cleanup")
    new = _cmd("cleanup")
    argstate.save_preset(new.slug, "theirs", {"x": "1"})
    assert flows.delete_presets_for(old, ["theirs"]) is None
    assert argstate.load_state(new.slug)["presets"] == {"theirs": {"x": "1"}}


def test_delete_presets_door_deletes_for_the_living_entry(tmp_path: Path) -> None:
    entry = _cmd("tidy")
    argstate.save_preset(entry.slug, "a", {"x": "1"})
    argstate.save_preset(entry.slug, "b", {"x": "2"})
    assert flows.delete_presets_for(entry, ["a", "ghost"]) == ["a"]
    assert set(argstate.load_state(entry.slug)["presets"]) == {"b"}


async def test_a_settings_save_on_a_reincarnated_slug_refuses_and_touches_nothing(
    tmp_path: Path,
) -> None:
    """#39 end to end: settings opened on one entry, saved after a remove + same-name
    re-add reissued its slug. Every write is authorized against the OPEN-time identity,
    so the save refuses with the reopen remedy and the new owner keeps its meta."""
    old = _cmd("screen")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        screen = ScriptSettingsScreen(store.claim_identity(old))
        app.push_screen(screen)
        await pilot.pause()
        store.remove("screen")
        new = _cmd("screen")
        screen.query_one("#st-desc", Input).value = "the dead screen's edit"
        screen.action_save()
        await pilot.pause()
        notes = [(n.message, n.severity) for n in app._notifications]
        assert (
            "screen changed while this edit was underway — reopen it and try again.",
            "error",
        ) in notes
    assert store.resolve(new.slug).meta.description == ""


async def test_a_stale_preset_untick_reaches_the_guarded_door_and_stops(tmp_path: Path) -> None:
    """When the ONLY pending write is the preset cleanup, the stale save must stop AT
    THE GUARDED DOOR — same refusal, same remedy, and no state file conjured for the
    new owner. A prompt with interpolation off is the one settings shape whose save
    carries no other write (a command entry's declared-rows commit refuses first)."""
    src = tmp_path / "quiet.prompt.md"
    src.write_text("Just words\n", encoding="utf-8")
    old = store.add_prompt(src, name="presets", interpolate=False)
    argstate.save_preset(old.slug, "doomed", {"x": "1"})
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        screen = ScriptSettingsScreen(store.claim_identity(old))
        app.push_screen(screen)
        await pilot.pause()
        store.remove("presets")
        new = store.add_prompt(src, name="presets", interpolate=False)
        assert new.slug == old.slug
        screen.query_one("#st-preset-0", Checkbox).value = False
        screen.action_save()
        await pilot.pause()
        confirm = app.screen
        assert isinstance(confirm, PresetDeleteConfirm)
        confirm.action_confirm()
        await pilot.pause()
        notes = [(n.message, n.severity) for n in app._notifications]
        assert (
            "presets changed while this edit was underway — reopen it and try again.",
            "error",
        ) in notes
    assert not (values_dir() / f"{new.slug}.toml").exists()


async def test_a_reconcile_write_refusal_lands_on_the_status_line(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The library's post-edit reconcile picker holds its entry while the picker sits
    open; a write the store refuses (stale, gone, corrupt) must land on the status
    line — never crash the app, never toast success."""
    src = tmp_path / "p.prompt.md"
    src.write_text("Ask {{x}}\n", encoding="utf-8")
    entry = store.add_prompt(src, name="p", managed=[])
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()

        def _stale(*_a: object, **_k: object) -> store.Entry:
            raise store.StaleEntryError("p changed while this edit was underway")

        monkeypatch.setattr(store, "write_prompt_managed", _stale)
        assert app._offer_prompt_reconcile(store.resolve(entry.slug))
        await pilot.pause()
        app.screen.dismiss({"x"})
        await pilot.pause()
        from textual.widgets import Static

        status = str(app.query_one("#status", Static).render())
        assert "changed while this edit was underway" in status
    assert store.resolve(entry.slug).meta.params is None  # nothing was managed


def test_claim_identity_refuses_a_handle_the_disk_outgrew(tmp_path: Path) -> None:
    """A handle resolved while the meta was unstamped meeting a stamped disk REFUSES —
    the asymmetry cannot distinguish a concurrent heal from an old entry's slug
    reissued by a stamping add, and a claim must never guess. The retry is cheap and
    honest: re-resolve, claim the stamped handle, proceed."""
    entry = _cmd("raced")
    _strip_id_line(entry.slug)
    held = store.resolve(entry.slug)  # unstamped snapshot
    store.update_description(entry.slug, "concurrently healed")  # stamps the disk
    with pytest.raises(store.StaleEntryError):
        store.claim_identity(held)
    retried = store.claim_identity(store.resolve(entry.slug))
    assert retried.meta.id  # the retry adopts the healed identity


# ==========================================================================
# H. mutation hardening — every door under the ONE lock, every axis authorized
# ==========================================================================


@pytest.mark.parametrize(
    ("writer", "fire"),
    [
        ("record_run", lambda e: flows.save_after_raw_run(e, 0, at="2026-02-01T00:00:00+00:00")),
        (
            "save_preset",
            lambda e: flows.save_preset_for(e, "q", {"msg": "x"}, secret_names=frozenset()),
        ),
        ("save_last", flows.clear_remembered_tail),
        ("delete_presets", lambda e: flows.delete_presets_for(e, ["p"])),
    ],
)
def test_every_door_writes_under_the_canonical_entry_lock(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    writer: str,
    fire: Callable[[store.Entry], object],
) -> None:
    """save_after_run's lock probe, for the other four doors: at the moment each
    door's argstate write runs, the CANONICAL entry lock (the exact path remove() and
    every schema commit contend on) must already be held — a door locking any other
    path would serialize against nothing."""
    entry = _cmd("door", "echo {msg}")
    argstate.save_preset(entry.slug, "p", {"msg": "x"})
    observed: list[bool] = []

    def probe(*_a: object, **_k: object) -> None:
        with try_advisory_file_lock(store.entry_lock_path(entry.slug)) as acquired:
            observed.append(acquired)

    target = "delete_preset" if writer == "delete_presets" else writer
    monkeypatch.setattr(argstate, target, probe)
    fire(entry)
    assert observed == [False]


def test_the_entry_lock_path_is_a_cross_version_contract(tmp_path: Path) -> None:
    """The lock file's location IS the serialization contract: an old and a new skit
    running side by side must contend on the same path, or neither excludes the
    other. Pinned byte-for-byte."""
    assert store.entry_lock_path("s") == store.scripts_dir().parent / ".locks" / "s.meta.lock"


def test_write_source_params_refuses_a_stale_authorization(tmp_path: Path) -> None:
    """The spec-lane commit under a reissued slug: refuse exactly, touch nothing —
    the new owner's stored copy keeps its bytes."""
    src = tmp_path / "tool.sh"
    src.write_text('#!/bin/sh\necho "$TOKEN"\n', encoding="utf-8")
    old = store.add_script(src, kind="shell", name="spec2")
    store.remove("spec2")
    new = store.add_script(src, kind="shell", name="spec2")
    before = new.script_path.read_bytes()
    with pytest.raises(store.StaleEntryError):
        store.write_source_params(
            new.slug,
            [ParamDecl(name="TOKEN", delivery="env", secret=True)],
            expected_id=old.meta.id,
        )
    assert new.script_path.read_bytes() == before


def test_write_parameters_refuses_a_stale_authorization(tmp_path: Path) -> None:
    old = _cmd("rows", "echo {x}")
    store.remove("rows")
    new = _cmd("rows", "echo {x}")
    with pytest.raises(store.StaleEntryError):
        store.write_parameters(new.slug, [ParamDecl(name="x")], expected_id=old.meta.id)
    assert store.resolve(new.slug).meta.parameters is None


async def test_a_tui_run_stamps_a_legacy_entry_at_hold_start(tmp_path: Path) -> None:
    """The TUI face of the hold-start handshake: opening the run form on a pre-id
    entry mints the id (the form and the run hold this handle)."""
    entry = _cmd("tuirun")
    _strip_id_line(entry.slug)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        app.action_run()
        await pilot.pause()
    assert store.resolve(entry.slug).meta.id


async def test_a_stale_reconcile_pick_refuses_and_touches_nothing(
    tmp_path: Path,
) -> None:
    """The reconcile picker's write, against a REAL reincarnation (no patched store):
    the held identity no longer owns the slug, so the managed-list write fails closed,
    the exact refusal lands on the status line, and the new owner keeps its own
    managed list."""
    src = tmp_path / "p.prompt.md"
    src.write_text("Ask {{x}}\n", encoding="utf-8")
    old = store.add_prompt(src, name="p2", managed=[])
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        assert app._offer_prompt_reconcile(store.resolve(old.slug))
        await pilot.pause()
        store.remove("p2")
        new = store.add_prompt(src, name="p2", managed=[])
        assert new.slug == old.slug
        app.screen.dismiss({"x"})
        await pilot.pause()
        from textual.widgets import Static

        status = str(app.query_one("#status", Static).render())
        assert status == "Error: p2 changed while this edit was underway — reopen it and try again."
    assert store.resolve(new.slug).meta.params is None


async def test_reconcile_status_lines_are_exact(tmp_path: Path) -> None:
    """The picker's three closing status lines, byte-for-byte: nothing chosen says
    Edited, a choice names every managed placeholder (comma-joined), and both name
    the entry the user acted on."""
    from textual.widgets import Static

    src = tmp_path / "p.prompt.md"
    src.write_text("Ask {{a}} then {{b}}\n", encoding="utf-8")
    entry = store.add_prompt(src, name="p3", managed=[])
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        assert app._offer_prompt_reconcile(store.resolve(entry.slug))
        await pilot.pause()
        app.screen.dismiss(set())  # nothing chosen
        await pilot.pause()
        assert str(app.query_one("#status", Static).render()) == "Edited p3."
        assert app._offer_prompt_reconcile(store.resolve(entry.slug))
        await pilot.pause()
        app.screen.dismiss({"a", "b"})
        await pilot.pause()
        assert str(app.query_one("#status", Static).render()) == "Now managed: a, b"
    assert store.resolve(entry.slug).meta.params == ["a", "b"]


async def test_the_reconcile_flood_guard_holds_its_exact_boundary(tmp_path: Path) -> None:
    """AUTO_MANAGE_LIMIT detections preselect everything; one more preselects NOTHING
    (a prompt tripping the flood was never written for insertion, and preselecting
    hundreds of required fields would make accepting the modal a trap)."""
    from skit.langs.prompt.analyzer import AUTO_MANAGE_LIMIT
    from skit.tui_prompt import PromptCandidatePickerModal

    at_limit = [f"u{i}" for i in range(AUTO_MANAGE_LIMIT)]
    src = tmp_path / "p.prompt.md"
    src.write_text(" ".join(f"{{{{{n}}}}}" for n in at_limit) + "\n", encoding="utf-8")
    entry = store.add_prompt(src, name="p4", managed=[])
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        assert app._offer_prompt_reconcile(store.resolve(entry.slug))
        await pilot.pause()
        modal = app.screen
        assert isinstance(modal, PromptCandidatePickerModal)
        assert modal._selected == set(at_limit)  # at the limit: everything preselected
        modal.dismiss(None)
        await pilot.pause()
    src.write_text(src.read_text(encoding="utf-8").rstrip() + " {{overflow}}\n", encoding="utf-8")
    store.remove("p4")
    entry = store.add_prompt(src, name="p4", managed=[])
    app = tui.MenuApp()  # a fresh app: run_test is single-shot per instance
    async with app.run_test() as pilot:
        await pilot.pause()
        assert app._offer_prompt_reconcile(store.resolve(entry.slug))
        await pilot.pause()
        modal = app.screen
        assert isinstance(modal, PromptCandidatePickerModal)
        assert modal._selected == set()  # one past the limit: flooded, nothing preselected
        modal.dismiss(None)
        await pilot.pause()


# ==========================================================================
# I. every settings axis is individually authorized (#39, axis by axis)
# ==========================================================================
# Each test isolates ONE write as the first (and only) mutation its save carries, on
# a screen whose slug was reissued mid-hold: the exact stale refusal must appear and
# the new owner must keep its record. Dropping expected_id from any single call site
# makes that write land silently — these pin every axis, not just the first one a
# combined save happens to hit.

_STALE = "%s changed while this edit was underway — reopen it and try again."


async def _stale_settings(
    app: tui.MenuApp, pilot: Pilot[int | tui.PendingRun], factory: Callable[[], store.Entry]
) -> tuple[ScriptSettingsScreen, store.Entry]:
    """Open settings on a claimed entry, then reissue its slug behind the screen."""
    old = factory()
    screen = ScriptSettingsScreen(store.claim_identity(old))
    app.push_screen(screen)
    await pilot.pause()
    store.remove(old.slug)
    new = factory()
    assert new.slug == old.slug
    return screen, new


def _assert_stale_refusal(app: tui.MenuApp, name: str) -> None:
    assert (_STALE % name, "error") in [(n.message, n.severity) for n in app._notifications]


async def test_a_stale_rename_never_lands(tmp_path: Path) -> None:
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        screen, new = await _stale_settings(app, pilot, lambda: _cmd("label2"))
        screen.query_one("#st-name", Input).value = "relabeled"
        screen.action_save()
        await pilot.pause()
        _assert_stale_refusal(app, "label2")
    assert store.resolve(new.slug).meta.name == "label2"


async def test_a_stale_template_edit_never_lands(tmp_path: Path) -> None:
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        screen, new = await _stale_settings(app, pilot, lambda: _cmd("tmpl"))
        screen.query_one("#st-template", Input).value = "echo changed"
        screen.action_save()
        await pilot.pause()
        _assert_stale_refusal(app, "tmpl")
    assert store.resolve(new.slug).meta.template == "echo hi"


async def test_a_stale_interpolate_toggle_never_lands(tmp_path: Path) -> None:
    src = tmp_path / "i.prompt.md"
    src.write_text("Hi {{x}}\n", encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        screen, new = await _stale_settings(
            app, pilot, lambda: store.add_prompt(src, name="interp")
        )
        screen.query_one("#st-interpolate", Checkbox).value = False
        screen.action_save()
        await pilot.pause()
        _assert_stale_refusal(app, "interp")
    assert store.resolve(new.slug).meta.interpolate is True


async def test_a_stale_declared_commit_never_lands(tmp_path: Path) -> None:
    """A command entry's save always carries its declared-rows commit — even a
    NOTHING-changed save must authorize it."""
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        screen, new = await _stale_settings(app, pilot, lambda: _cmd("rows2"))
        screen.action_save()
        await pilot.pause()
        _assert_stale_refusal(app, "rows2")
    assert store.resolve(new.slug).meta.parameters is None


async def test_a_stale_source_commit_never_lands(tmp_path: Path) -> None:
    """Same for an analyzable copy's save: the [tool.skit] block commit is always
    carried, and must always be authorized."""
    src = tmp_path / "s.sh"
    src.write_text('#!/bin/sh\nGREETING="hi"\necho "$GREETING"\n', encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        screen, new = await _stale_settings(
            app, pilot, lambda: store.add_script(src, kind="shell", name="src2")
        )
        before = new.script_path.read_bytes()
        screen.action_save()
        await pilot.pause()
        _assert_stale_refusal(app, "src2")
    assert new.script_path.read_bytes() == before


async def test_a_stale_deps_edit_never_lands(tmp_path: Path) -> None:
    """Reference-mode python: deps are meta-only and the block lane is off, so the
    dependency write is the save's first authorized mutation."""
    src = tmp_path / "d.py"
    src.write_text('"""Doc."""\nprint(1)\n', encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        screen, new = await _stale_settings(
            app, pilot, lambda: store.add_python(src, name="deps2", mode="reference")
        )
        screen.query_one("#st-deps", Input).value = "httpx"
        screen.action_save()
        await pilot.pause()
        _assert_stale_refusal(app, "deps2")
    assert store.resolve(new.slug).meta.dependencies is None


async def test_a_stale_npm_clear_never_lands(tmp_path: Path) -> None:
    """The early npm-clear write is the save's very FIRST mutation — its
    authorization cannot ride on any later call's."""
    src = tmp_path / "j.js"
    src.write_text("console.log(1)\n", encoding="utf-8")

    def factory() -> store.Entry:
        entry = store.add_script(src, kind="js", name="npm2")
        entry.meta.dependencies = ["chalk"]  # recorded deps, seeded without an installer
        store._write_meta_and_row(entry.dir, entry.slug, entry.meta)
        return store.resolve(entry.slug)

    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        screen, new = await _stale_settings(app, pilot, factory)
        screen.query_one("#st-deps", Input).value = ""
        screen.action_save()
        await pilot.pause()
        _assert_stale_refusal(app, "npm2")
    assert store.resolve(new.slug).meta.dependencies == ["chalk"]


async def test_a_stale_needs_edit_never_lands(tmp_path: Path) -> None:
    src = tmp_path / "n.py"
    src.write_text('"""Doc."""\nprint(1)\n', encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        screen, new = await _stale_settings(
            app, pilot, lambda: store.add_python(src, name="needs2", mode="reference")
        )
        screen.query_one("#st-needs", Input).value = "jq"
        screen.action_save()
        await pilot.pause()
        _assert_stale_refusal(app, "needs2")
    assert store.resolve(new.slug).meta.needs is None


async def test_stale_launch_policy_writes_raise_through_their_helper(tmp_path: Path) -> None:
    """_write_launch's two axes, driven at the helper (the workdir/interpreter radios
    are UI sugar around it): each write authorizes against the held identity."""
    src = tmp_path / "w.sh"
    src.write_text("#!/bin/sh\necho hi\n", encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        screen, new = await _stale_settings(
            app, pilot, lambda: store.add_script(src, kind="shell", name="launch2")
        )
        with pytest.raises(store.StaleEntryError):
            screen._write_launch(("store", ""))  # workdir axis
        with pytest.raises(store.StaleEntryError):
            screen._write_launch(("invoke", "zsh"))  # interpreter axis
    fresh = store.resolve(new.slug)
    assert fresh.meta.workdir == "invoke"
    assert fresh.meta.interpreter == ""


async def test_a_stale_runner_pin_raises_through_its_helper(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    from textual.widgets import Select

    from skit import config

    monkeypatch.setattr(
        config, "load_prompt_runners", lambda: [config.PromptRunner(name="other", argv=("other",))]
    )
    src = tmp_path / "r.prompt.md"
    src.write_text("Hello\n", encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        screen, new = await _stale_settings(app, pilot, lambda: store.add_prompt(src, name="pin2"))
        screen.query_one("#st-runner-select", Select).value = "other"
        with pytest.raises(store.StaleEntryError):
            screen._save_runner_pin()
    assert store.resolve(new.slug).meta.runner == ""


async def test_a_cleared_name_box_means_no_rename_not_an_error(tmp_path: Path) -> None:
    """Emptying the name field is "leave the name alone" — the save must succeed and
    rename nothing, never trip the store's empty-name refusal."""
    entry = _cmd("keepname")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        screen = ScriptSettingsScreen(store.claim_identity(entry))
        results: list[bool | None] = []
        app.push_screen(screen, results.append)
        await pilot.pause()
        screen.query_one("#st-name", Input).value = ""
        screen.action_save()
        await pilot.pause()
        assert results == [True]
        assert not any(n.severity == "error" for n in app._notifications)
    assert store.resolve(entry.slug).meta.name == "keepname"


async def test_a_no_change_save_dismisses_true_and_leaves_the_meta_alone(tmp_path: Path) -> None:
    """A save with nothing to say must still report success (dismiss True — the
    caller reloads on it) and must not churn meta.toml: its mtime keys the plan cache
    and the registry row stamp."""
    src = tmp_path / "q.sh"
    src.write_text('#!/bin/sh\nGREETING="hi"\necho "$GREETING"\n', encoding="utf-8")
    entry = store.add_script(src, kind="shell", name="noop")
    meta_path = store.scripts_dir() / entry.slug / "meta.toml"
    before = os.stat(meta_path).st_mtime_ns
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        screen = ScriptSettingsScreen(store.claim_identity(entry))
        results: list[bool | None] = []
        app.push_screen(screen, results.append)
        await pilot.pause()
        screen.action_save()
        await pilot.pause()
        assert results == [True]
    assert os.stat(meta_path).st_mtime_ns == before


async def test_the_purge_notice_names_every_scrubbed_value_spec_lane(tmp_path: Path) -> None:
    """Two flips, one notice, exact copy: the message is the user's only record of
    what was deleted, so it must name each value, comma-joined."""
    from skit.langs.python import metawriter
    from skit.tui_settings import ParamRow

    text = metawriter.write_params(
        'A = "1"\nB = "2"\nprint(A, B)\n',
        [
            ParamDecl(name="A", binding="const", type="str", default="1"),
            ParamDecl(name="B", binding="const", type="str", default="2"),
        ],
    )
    src = tmp_path / "two.py"
    src.write_text(text, encoding="utf-8")
    entry = store.add_python(src, name="two")
    argstate.save_last(entry.slug, values={"A": "x", "B": "y"})
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        screen = ScriptSettingsScreen(store.claim_identity(entry))
        app.push_screen(screen)
        await pilot.pause()
        for row in screen.query(ParamRow):
            row.query_one(".p-secret", Checkbox).value = True
            row.query_one(".p-env", Input).value = f"{row.spec.name}_ENV"
        await pilot.pause()
        screen.action_save()
        await pilot.pause()
        assert any(
            n.message == "Deleted previously remembered value(s): A, B" for n in app._notifications
        )
    state = argstate.load_state(entry.slug)
    assert "A" not in state["values"]
    assert "B" not in state["values"]


async def test_the_purge_notice_names_every_scrubbed_value_declared_lane(tmp_path: Path) -> None:
    exe = tmp_path / "tool"
    exe.touch()
    entry = store.add_exe(exe, name="twodecl")
    store.write_parameters(
        entry.slug,
        [
            ParamDecl(name="a", delivery="flag", type="str", flag="--a"),
            ParamDecl(name="b", delivery="flag", type="str", flag="--b"),
        ],
    )
    argstate.save_last(entry.slug, values={"a": "x", "b": "y"})
    from skit.tui_settings import DeclParamRow

    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        screen = ScriptSettingsScreen(store.claim_identity(entry))
        app.push_screen(screen)
        await pilot.pause()
        for row in screen.query(DeclParamRow):
            row.query_one(".p-secret", Checkbox).value = True
            row.query_one(".p-env", Input).value = f"{row.decl.name.upper()}_ENV"
        await pilot.pause()
        screen.action_save()
        await pilot.pause()
        assert any(
            n.message == "Deleted previously remembered value(s): a, b" for n in app._notifications
        )
    state = argstate.load_state(entry.slug)
    assert "a" not in state["values"]
    assert "b" not in state["values"]
