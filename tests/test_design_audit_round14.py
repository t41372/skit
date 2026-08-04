"""Behavior coverage for the design-audit round-14 fix (the external PR review's P1).

The finding: `flows.save_after_run` — the NORMAL lane every accepted CLI/TUI run takes —
trusted a launch-time slug and a launch-time secret set across a run that can last
hours. A param flipped to secret mid-run was persisted in plaintext, and a slug freed
by remove() and reissued to a later add received the dead run's state. Round 13 fixed
exactly this for `--raw`; the review asked for the universal fix, not a second local one.

The shape it took:

W. Entries now have an IDENTITY: ``ScriptMeta.id``, minted at the store's one
   meta-write door, immutable for life, distinct across two owners of one slug.
   A meta from before ids existed reads as ``""`` and heals on its next write.
X. ``flows.persistence_target`` is the single guard every post-acceptance state write
   passes through: re-resolve the held entry's slug, compare identities, and answer
   "write here" or "write NOTHING".
Y. ``save_after_run`` goes through it (entry, not slug), and its strip set is the
   union of launch-time and persistence-time secrecy — the raw lane's rule, now the
   only rule.
Z. The remaining post-acceptance writes go through guarded flows doors too:
   ``save_preset_for`` (`run --save-preset`, the form's Ctrl+S) reports an unwritable
   preset instead of writing it somewhere wrong, and ``clear_remembered_tail``
   (`--forget-args`) treats a vanished entry as already forgotten.
A census pins every remaining direct argstate writer call site in src/skit, so a new
write path must either use the doors or amend the census with a reason.
"""

from __future__ import annotations

import re
from collections import Counter
from pathlib import Path

import pytest
from textual.widgets import Input
from typer.testing import CliRunner

from conftest import real_repo_root
from skit import argstate, cli, flows, store, tui
from skit.models import ScriptMeta
from skit.params import ParamDecl
from skit.paths import values_dir
from skit.tui_form import PresetNameModal, RunFormScreen

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


def _meta_path(slug: str) -> Path:
    return store.scripts_dir() / slug / "meta.toml"


def _strip_id_line(slug: str) -> None:
    """Rewrite a meta.toml the way a pre-id skit wrote it: no ``id`` key at all."""
    path = _meta_path(slug)
    kept = [
        line
        for line in path.read_text(encoding="utf-8").splitlines()
        if not line.startswith("id = ")
    ]
    path.write_text("\n".join(kept) + "\n", encoding="utf-8")


# ==========================================================================
# W. ScriptMeta.id — identity is born at the meta-write door
# ==========================================================================


def test_every_add_mints_a_distinct_identity(tmp_path: Path) -> None:
    """Two adds, two ids — non-empty, uuid4-hex-shaped, different — and the id the add
    returns is the id on disk, because the write door stamped the same object."""
    src = tmp_path / "s.py"
    src.write_text("print(1)\n", encoding="utf-8")
    a = _cmd("alpha")
    b = store.add_python(src, name="beta")
    assert a.meta.id
    assert b.meta.id
    assert len(a.meta.id) == 32
    assert a.meta.id != b.meta.id
    assert store.resolve(a.slug).meta.id == a.meta.id


def test_edits_preserve_identity_for_life(tmp_path: Path) -> None:
    """Every mutator rewrites meta.toml through the same door; none may re-stamp. A
    rename keeps the id for the same reason it keeps the slug: nothing about WHO the
    entry is changed."""
    entry = _cmd("edit")
    born = entry.meta.id
    store.update_description(entry.slug, "touched")
    store.write_parameters(entry.slug, [ParamDecl(name="x", delivery="placeholder")])
    store.rename(entry.slug, "edited")
    assert store.resolve(entry.slug).meta.id == born


def test_a_reused_slug_is_a_different_entry(tmp_path: Path) -> None:
    """The reincarnation the review proved possible: remove frees the slug, a same-name
    add takes it back. Same address, new identity — the fact the guard keys on."""
    old = _cmd("twice")
    store.remove("twice")
    new = _cmd("twice")
    assert new.slug == old.slug  # the reuse is real, not hypothetical
    assert new.meta.id != old.meta.id


def test_a_legacy_meta_reads_untouched_and_heals_on_its_next_write(tmp_path: Path) -> None:
    """A meta from before ids existed parses with id == "" — and resolving it writes
    NOTHING (reads stay reads). Its next real write mints the missing id, so old
    libraries converge to protection without a migration."""
    entry = _cmd("legacy")
    _strip_id_line(entry.slug)
    assert store.resolve(entry.slug).meta.id == ""
    assert "id = " not in _meta_path(entry.slug).read_text(encoding="utf-8")
    store.update_description(entry.slug, "first write since the upgrade")
    assert store.resolve(entry.slug).meta.id != ""


def test_a_hand_edited_id_of_the_wrong_type_is_meta_corruption(tmp_path: Path) -> None:
    """`id = 3` is a hand edit, and it corrupts the meta the same way a mistyped
    `runner` does — the model boundary raises, the store maps it, doctor names it."""
    entry = _cmd("badid")
    path = _meta_path(entry.slug)
    text = re.sub(r'^id = ".*"$', "id = 3", path.read_text(encoding="utf-8"), flags=re.M)
    path.write_text(text, encoding="utf-8")
    with pytest.raises(store.CorruptEntryError):
        store.resolve(entry.slug)


def test_meta_serialization_only_carries_a_stamped_id() -> None:
    """An unstamped id is OMITTED (a legacy meta round-trips byte-identically); a
    stamped one round-trips; a missing key parses as the wildcard, never a crash."""
    assert "id" not in ScriptMeta(name="x", kind="command").to_toml_dict()
    d = ScriptMeta(name="x", kind="command", id="abc").to_toml_dict()
    assert d["id"] == "abc"
    assert ScriptMeta.from_toml_dict(d).id == "abc"
    assert ScriptMeta.from_toml_dict({"name": "x", "kind": "command"}).id == ""


# ==========================================================================
# X. persistence_target — the one guard every post-acceptance write passes
# ==========================================================================


def test_the_target_is_the_fresh_entry_not_the_held_one(tmp_path: Path) -> None:
    """Same identity → write, and the answer is the CURRENT meta (the strip set is
    computed from it), not an echo of the handle."""
    entry = _cmd("same")
    store.update_description(entry.slug, "changed mid-run")
    fresh = flows.persistence_target(entry)
    assert fresh is not None
    assert fresh.slug == entry.slug
    assert fresh.meta.description == "changed mid-run"


def test_a_removed_entry_is_no_target(tmp_path: Path) -> None:
    entry = _cmd("gone")
    store.remove(entry.slug)
    assert flows.persistence_target(entry) is None


def test_a_reincarnated_slug_is_no_target(tmp_path: Path) -> None:
    old = _cmd("reborn")
    store.remove("reborn")
    _cmd("reborn")  # same slug, new identity
    assert flows.persistence_target(old) is None


def test_a_corrupt_meta_is_no_target(tmp_path: Path) -> None:
    """Unreadable-now is gone-now: a guard that guessed would be resolution guessing."""
    entry = _cmd("rot")
    _meta_path(entry.slug).write_text("not = [ toml", encoding="utf-8")
    assert flows.persistence_target(entry) is None


def test_an_unstamped_handle_cannot_authorize_against_a_stamped_entry(tmp_path: Path) -> None:
    """Identity is EXACT match — unknown identity may serve reads, never authorize a
    write. A handle resolved before the meta carried an id meeting a stamped entry is
    exactly the pairing a remove + same-slug re-add of a legacy entry produces, so it
    fails closed. Legitimate lanes never hit this: they stamp at hold-start
    (store.ensure_identity), so their handles are never unstamped on a writable
    library."""
    entry = _cmd("wild")
    _strip_id_line(entry.slug)
    held = store.resolve(entry.slug)  # id == "" — a pre-id handle, resolve() not claimed
    store.update_description(entry.slug, "stamps the id")
    assert held.meta.id == ""
    assert flows.persistence_target(held) is None


def test_a_stamped_handle_refuses_a_hand_stripped_meta(tmp_path: Path) -> None:
    """The other asymmetry: the disk lost the identity the handle was authorized
    against (a hand edit). Cannot confirm — do not write."""
    entry = _cmd("wildback")
    assert entry.meta.id
    _strip_id_line(entry.slug)
    assert flows.persistence_target(entry) is None


def test_two_unstamped_readings_still_match(tmp_path: Path) -> None:
    """ "" meets "" only where nothing can write the meta at all (a read-only library —
    the one place ensure_identity leaves a handle unstamped), and there the ids cannot
    diverge either: exact match holds, persistence still lands."""
    entry = _cmd("readonly")
    _strip_id_line(entry.slug)
    held = store.resolve(entry.slug)
    assert flows.persistence_target(held) is not None


# ==========================================================================
# Y. save_after_run — the normal lane honors the guard and the union
# ==========================================================================


def test_a_normal_run_writes_nothing_for_a_removed_entry(tmp_path: Path) -> None:
    """The review's resurrection scenario on the NORMAL lane: remove() deleted the
    values file mid-run; the post-run save must not write it back."""
    entry = _cmd("vanish", "echo {msg}")
    plan = flows.plan_for_entry(entry)
    store.remove(entry.slug)
    flows.save_after_run(
        entry, plan, {"msg": "hi"}, [], 0, at="2026-02-01T00:00:00+00:00", extra_raw=False
    )
    assert not (values_dir() / f"{entry.slug}.toml").exists()


def test_a_normal_run_never_writes_onto_a_reused_slug(tmp_path: Path) -> None:
    """The reincarnation half: the slug resolves fine — to a DIFFERENT entry. Its
    state must stay exactly as the new owner left it: no dead run's values, no
    phantom run stamp."""
    old = _cmd("host", "echo {msg}")
    plan = flows.plan_for_entry(old)
    store.remove("host")
    new = _cmd("host")
    argstate.save_last(new.slug, values={"theirs": "kept"})
    flows.save_after_run(
        old, plan, {"msg": "mine"}, ["--x"], 0, at="2026-02-01T00:00:00+00:00", extra_raw=False
    )
    state = argstate.load_state(new.slug)
    assert state["values"] == {"theirs": "kept"}
    assert state["extra_args"] == []
    assert state["last_run"] == {}


def test_a_mid_run_secret_flip_scrubs_the_normal_lane(tmp_path: Path) -> None:
    """The P1's exact scenario, previously raw-only: public at launch, declared secret
    mid-run. The post-run save must strip the value from every surface AND purge the
    plaintext earlier runs left — with the fresh reading, not the launch one."""
    entry = _cmd("flip", "echo {mode}")
    argstate.save_preset(entry.slug, "old", {"mode": "hunter2"})
    argstate.save_last(entry.slug, values={"mode": "hunter2"})
    plan = flows.plan_for_entry(entry)
    assert "mode" not in plan.secret_names  # public at launch — the stale set
    store.write_parameters(
        entry.slug, [ParamDecl(name="mode", delivery="placeholder", secret=True)]
    )
    flows.save_after_run(
        entry, plan, {"mode": "hunter3"}, [], 0, at="2026-02-01T00:00:00+00:00", extra_raw=False
    )
    state = argstate.load_state(entry.slug)
    assert "mode" not in state["values"]
    assert all("mode" not in preset for preset in state["presets"].values())
    assert "mode" not in state["last_run"]["values"]


def test_launch_time_heuristic_secrecy_still_strips(tmp_path: Path) -> None:
    """The union's launch term carries what only the analyzer knew: is_secret_name
    secrecy never reaches the stored schema, so dropping plan.secret_names from the
    union would persist it in plaintext."""
    src = tmp_path / "tool.py"
    src.write_text(
        "import argparse\nap = argparse.ArgumentParser()\n"
        "ap.add_argument('--api-key')\nap.parse_args()\n",
        encoding="utf-8",
    )
    entry = store.add_python(src, name="heur")
    plan = flows.plan_for_entry(entry)
    assert "api_key" in plan.secret_names  # heuristic-only: no declared row, no block
    flows.save_after_run(
        entry, plan, {"api_key": "sk-1"}, [], 0, at="2026-02-01T00:00:00+00:00", extra_raw=False
    )
    state = argstate.load_state(entry.slug)
    assert "api_key" not in state["values"]
    assert "api_key" not in state["last_run"]["values"]


# ==========================================================================
# Z. the other acceptance doors — presets and --forget-args
# ==========================================================================


def test_a_preset_for_a_living_entry_saves_with_the_fresh_secret_set(tmp_path: Path) -> None:
    """save_preset_for reports True AND strips with persistence-time secrecy: a flip
    that landed while the form sat open must not ride into the preset it saves."""
    entry = _cmd("pf", "echo {mode}")
    plan = flows.plan_for_entry(entry)
    store.write_parameters(
        entry.slug, [ParamDecl(name="mode", delivery="placeholder", secret=True)]
    )
    assert flows.save_preset_for(entry, "p", {"mode": "hunter2"}, secret_names=plan.secret_names)
    assert argstate.load_state(entry.slug)["presets"]["p"] == {}


def test_a_preset_for_a_vanished_entry_reports_instead_of_writing(tmp_path: Path) -> None:
    entry = _cmd("pgone", "echo {msg}")
    store.remove(entry.slug)
    assert not flows.save_preset_for(entry, "p", {"msg": "x"}, secret_names=frozenset())
    assert not (values_dir() / f"{entry.slug}.toml").exists()


def test_a_preset_never_lands_on_a_reused_slug(tmp_path: Path) -> None:
    old = _cmd("preborn", "echo {msg}")
    store.remove("preborn")
    new = _cmd("preborn")
    assert not flows.save_preset_for(old, "p", {"msg": "x"}, secret_names=frozenset())
    assert argstate.load_state(new.slug)["presets"] == {}


def test_forget_args_clears_only_the_tail_for_a_living_entry(tmp_path: Path) -> None:
    entry = _cmd("alive")
    argstate.save_last(entry.slug, values={"v": "1"}, extra_args=["--x"])
    flows.clear_remembered_tail(entry)
    state = argstate.load_state(entry.slug)
    assert state["extra_args"] == []
    assert state["values"] == {"v": "1"}


def test_forget_args_on_a_vanished_entry_is_vacuously_done(tmp_path: Path) -> None:
    """remove() already forgot the whole state file. Writing the clear would resurrect
    the file just to hold an empty tail — the exact bug class the guard exists for."""
    entry = _cmd("vf")
    argstate.save_last(entry.slug, extra_args=["--x"])
    store.remove(entry.slug)
    assert not (values_dir() / f"{entry.slug}.toml").exists()
    flows.clear_remembered_tail(entry)
    assert not (values_dir() / f"{entry.slug}.toml").exists()


# --------------------------------------------------------------------------
# the CLI lane, end to end: the run outlives its entry
# --------------------------------------------------------------------------


def _remove_during_run(monkeypatch: pytest.MonkeyPatch, slug: str, exit_code: int) -> None:
    """Stand in for a script whose whole run raced a removal: the launch succeeded,
    the entry died before the exit-code stamp."""

    def run_and_remove(*_args: object, **_kwargs: object) -> flows.RunOutcome:
        store.remove(slug)
        return flows.RunOutcome(exit_code)

    monkeypatch.setattr(flows, "execute", run_and_remove)


def test_a_cli_run_that_outlives_its_entry_leaves_no_state(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    entry = _cmd("raced")
    _remove_during_run(monkeypatch, entry.slug, 0)
    result = runner.invoke(cli.app, ["run", "raced", "--no-input"])
    assert result.exit_code == 0, result.output
    assert not (values_dir() / f"{entry.slug}.toml").exists()


def test_save_preset_on_a_vanished_entry_says_so_and_keeps_the_scripts_exit_code(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """--save-preset is an explicit request: eating it silently would be worse than
    the stale write. The message is a warning on stderr — the script's own exit code
    (7 here) still passes through untouched."""
    entry = _cmd("vp", "echo {msg}")
    _remove_during_run(monkeypatch, entry.slug, 7)
    result = runner.invoke(
        cli.app, ["run", "vp", "--no-input", "--set", "msg=hi", "--save-preset", "p"]
    )
    assert result.exit_code == 7
    assert "wasn't saved" in result.output
    assert "no longer in the library" in result.output
    assert not (values_dir() / f"{entry.slug}.toml").exists()


def test_forget_args_on_the_cli_stays_vacuous_when_the_entry_dies_mid_run(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    entry = _cmd("vfcli")
    argstate.save_last(entry.slug, extra_args=["--x"])
    _remove_during_run(monkeypatch, entry.slug, 0)
    result = runner.invoke(cli.app, ["run", "vfcli", "--no-input", "--forget-args"])
    assert result.exit_code == 0, result.output
    assert not (values_dir() / f"{entry.slug}.toml").exists()


# --------------------------------------------------------------------------
# the TUI form lane: Ctrl+S after the form outlived its entry
# --------------------------------------------------------------------------


async def test_a_form_that_outlived_its_entry_refuses_the_preset(tmp_path: Path) -> None:
    """The form's Ctrl+S is the same explicit request as --save-preset, with the same
    window (a form can sit open for hours). Refusal is an error toast that names the
    preset and the entry — never a success toast for a preset with no entry to
    belong to, and never a write onto whoever owns the slug now."""
    entry = _cmd("formgone", "echo {msg}")
    plan = flows.plan_for_entry(entry)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = RunFormScreen(entry, plan, {})
        app.push_screen(screen)
        await pilot.pause()
        store.remove(entry.slug)
        screen.action_save_preset()
        await pilot.pause()
        modal = app.screen
        assert isinstance(modal, PresetNameModal)
        modal.query_one(Input).value = "p"
        modal.action_save_name()
        await pilot.pause()
        notes = [(n.message, n.severity) for n in app._notifications]
        assert (
            'Preset "p" wasn\'t saved — formgone is no longer in the library.',
            "error",
        ) in notes
        assert not any("saved." in message for message, _severity in notes)
    assert not (values_dir() / f"{entry.slug}.toml").exists()


# ==========================================================================
# the census: no slug-keyed argstate write outside the guarded doors
# ==========================================================================


def test_slug_keyed_argstate_writes_stay_behind_the_guarded_doors() -> None:
    """Pin every direct argstate writer call site in src/skit. The flows rows are the
    four guarded doors (each opens with persistence_target). Every other row is an
    immediate resolve→write command lane — no handle is held across user-paced time,
    so there is nothing to go stale. A new call site fails this census on purpose:
    either route it through a door, or add it here with that justification.

    - cli.py: `preset save`/`preset delete` — resolve and write in one motion.
    - store.py: remove()'s own forget, and the C3 scrubs inside the two locked
      schema-commit transactions (write_parameters, write_source_params) — purge and
      commit under one entry lock, where nothing can interleave between them.
    """
    root = real_repo_root() / "src" / "skit"
    writer = re.compile(
        r"argstate\.(save_last|save_preset|record_run|purge_secret|delete_preset|forget)\("
    )
    census: dict[str, dict[str, int]] = {}
    for path in sorted(root.rglob("*.py")):
        counts = Counter(m.group(1) for m in writer.finditer(path.read_text(encoding="utf-8")))
        if counts:
            census[path.relative_to(root).as_posix()] = dict(counts)
    assert census == {
        "cli.py": {"save_preset": 1, "delete_preset": 1},
        "flows.py": {
            "save_last": 2,
            "save_preset": 1,
            "record_run": 2,
            "purge_secret": 2,
            "delete_preset": 1,
        },
        "store.py": {"forget": 1, "purge_secret": 2},
    }
