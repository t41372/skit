"""Behavior coverage for the design-audit round-9 fixes, headless + CLI half.

Six verified bugs, each kept dead by one section below:

L. A lost or corrupt ``registry.toml`` used to empty the whole library in silence — and
   ``skit doctor``, the command whose job is checking the library is intact, printed
   ``✓ 0 entries registered`` and exited 0 while both scripts sat untouched on disk. The
   index is only a rebuildable projection, and ``_fs_truth`` already cross-checked it
   against disk to protect WRITES; ``store.unindexed_slugs`` is the read side of that same
   cross-check, and it is now what every blank-library surface consults before claiming
   the shelf is bare.
M. ``skit params <entry> --secret NAME`` on a template placeholder with no declared row
   was skipped with a warning, a green "Updated" line and exit 0 — and the value it was
   meant to protect then landed in the state file in plaintext (C3). ``--add`` already
   knew how to materialize that row; now every tweak does, through one constructor.
N. ``store.resolve`` trusted the index row's ``name`` with no freshness check while
   ``_summary_from_row`` deliberately verified the same stamp, so ``skit list`` showed a
   name ``skit run``/``show`` called not-found.
O. ``record_run(values=None)`` replaced the whole ``last_run`` table, so ``skit run --raw``
   deleted the value snapshot its own call site promises to preserve — and left exactly
   the shape ``preset save --from-last`` then refuses with a false message.
P. ``skit doctor`` prints the config and state roots two docs pages have always said it
   prints, and which nothing in skit exposed at all.
Q. ``LangSpec.takes_argv`` is gone: no code read it, while three comments and a design doc
   credited it with a rule ``placeholder_params`` enforces. (Its removal is pinned by the
   spec tests in tests/test_langs.py and tests/test_registry_mut_part01.py, which now
   assert the trait that decides.)
"""

from __future__ import annotations

import json
from pathlib import Path

import pytest
from typer.testing import CliRunner

from skit import argstate, cli, healthcheck, params, store
from skit.langs.registry import spec_for

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


def _py(tmp_path: Path, body: str, name: str = "job.py") -> Path:
    p = tmp_path / name
    p.write_text(body, encoding="utf-8")
    return p


def _lose_the_index() -> None:
    """What a restored backup, a synced data dir or a half-written file leaves behind."""
    store.registry_path().unlink()


# ==========================================================================
# L. A library the index has lost is REPORTED, not silently emptied
# ==========================================================================


def test_unindexed_slugs_finds_the_entries_the_index_forgot() -> None:
    """The read side of the cross-check _fs_truth already ran for writes. Before it, the
    only code that knew the index could fall behind disk used that knowledge to stop `add`
    overwriting a stored script — and never told the user, whose entire library had just
    become invisible."""
    _cmd("one")
    _cmd("two")
    assert store.unindexed_slugs() == []  # a healthy library diverges from nothing

    _lose_the_index()

    assert store.unindexed_slugs() == ["one", "two"]


def test_a_corrupt_index_is_reported_the_same_way_a_missing_one_is() -> None:
    """_load_registry degrades a corrupt registry to an empty one (renaming the bad bytes
    aside so it can't re-trigger). That degrade is only honest if something downstream
    notices — the rename made the failure quiet, not visible."""
    _cmd("one")
    store.registry_path().write_text("[entries.one\nname = ", encoding="utf-8")

    assert store.unindexed_slugs() == ["one"]
    assert store.registry_path().with_name("registry.toml.corrupt").exists()


def test_only_directories_that_hold_an_entry_record_count(tmp_path: Path) -> None:
    """What doctor --rebuild can actually recover, and nothing else: a directory with no
    meta.toml is a crashed-mid-add leftover, not a lost entry, and reporting it would send
    the user to a rebuild that skips it with a problem line."""
    _cmd("one")
    _lose_the_index()
    (store.scripts_dir() / "leftover").mkdir()
    (store.scripts_dir() / "not-a-dir").write_text("", encoding="utf-8")

    assert store.unindexed_slugs() == ["one"]


def test_an_empty_library_reports_nothing_missing(tmp_path: Path) -> None:
    """The complement, and the case a fresh install hits: no scripts directory at all is
    not a divergence, and a first run must not be told to go rebuild anything."""
    assert not store.scripts_dir().exists()
    assert store.unindexed_slugs() == []


def test_the_health_sweep_asks_the_store_not_the_entry_list() -> None:
    """An entry the index has lost was never IN the list collect() is handed, so no sweep
    over that list could ever find it. This is the only check in the report that can
    contradict the count printed above it."""
    _cmd("one")
    _lose_the_index()

    report = healthcheck.collect(store.list_entries())

    assert store.list_entries() == []  # the list the faces render is genuinely empty...
    assert report.unindexed == ["one"]  # ...and the report says why


def test_doctor_no_longer_certifies_a_library_it_cannot_see() -> None:
    """THE round-9 HIGH. `✓ 0 entries registered` next to two intact entries, from the
    command that exists to detect exactly this — and `doctor --rebuild`, which fixes it
    instantly, was named nowhere in skit's output."""
    _cmd("one")
    _cmd("two")
    _lose_the_index()

    result = runner.invoke(cli.app, ["doctor"])

    out = " ".join(result.output.split())
    assert "2 stored entries are missing from the index" in out
    assert "one, two" in out
    assert "skit doctor --rebuild" in out
    # ...and the repair it names really is the repair.
    assert runner.invoke(cli.app, ["doctor", "--rebuild"]).exit_code == 0
    assert [e.meta.name for e in store.list_entries()] == ["one", "two"]
    assert "missing from the index" not in runner.invoke(cli.app, ["doctor"]).output


def test_doctor_json_lets_an_agent_tell_empty_from_unreadable() -> None:
    """`entries: 0` is the same payload for "nothing added yet" and "the library is
    unreadable", and an agent acting on the first when it is the second re-adds scripts the
    user already owns. The additive key is what separates them."""
    _cmd("one")
    _lose_the_index()

    payload = json.loads(runner.invoke(cli.app, ["doctor", "--json"]).stdout)

    assert payload["entries"] == 0
    assert payload["unindexed"] == ["one"]
    # ...and a healthy library says so with the same key, not by omitting it.
    runner.invoke(cli.app, ["doctor", "--rebuild"])
    assert json.loads(runner.invoke(cli.app, ["doctor", "--json"]).stdout)["unindexed"] == []


def test_the_empty_listing_stops_asserting_the_library_is_empty() -> None:
    """ "No entries yet. Add one with: skit add <path>" is an ASSERTION, and it was made
    without ever looking at the shelf. To a user whose index just vanished it reads as
    "your scripts are gone", and its call to action is to re-add them."""
    _cmd("one")
    _lose_the_index()

    result = runner.invoke(cli.app, ["list"])

    out = " ".join(result.output.split())
    assert "No entries yet" not in out
    assert "1 stored entry is still on disk" in out
    assert "skit doctor --rebuild" in out


def test_a_genuinely_empty_library_still_gets_the_welcome() -> None:
    """The other half of the same rule: when the shelf really is bare, the invitation is
    the right copy and the recovery line would be nonsense."""
    result = runner.invoke(cli.app, ["list"])

    assert "No entries yet. Add one with: skit add <path>" in " ".join(result.output.split())
    assert "doctor --rebuild" not in result.output


# ==========================================================================
# M. An explicit --secret on a placeholder is honored, not skipped
# ==========================================================================


def test_secret_on_an_undeclared_placeholder_is_real_work() -> None:
    """THE round-9 leak. is_secret_name is RIGHT to miss `cookie` — it is not a credential
    word — which is exactly why the explicit override exists. Dropping it behind exit 0 and
    a green "Updated" line meant the user did the documented thing, was told it worked, and
    got the value written to disk in plaintext on the next run."""
    entry = _cmd("fetch", "curl -H 'Cookie: {cookie}' https://example.com/{path}")

    result = runner.invoke(cli.app, ["params", entry.slug, "--secret", "cookie"])

    assert result.exit_code == 0, result.output
    assert "isn't a declared parameter" not in result.output
    declared = {
        d.name: d for d in params.declared_from_meta(store.resolve(entry.slug).meta.parameters)
    }
    assert declared["cookie"].secret is True
    assert declared["cookie"].delivery == "placeholder"
    assert declared["cookie"].required is True  # a declared placeholder never assembles empty


def test_the_materialized_row_reaches_the_plaintext_scrub(tmp_path: Path) -> None:
    """The purge already existed and the standalone flag never reached it. A value stored
    while the parameter was public must not survive on disk after it becomes secret — and
    the one-call spelling (`--add cookie --secret cookie`) had always done this, which is
    what made the other spelling's silence a leak rather than a missing feature."""
    entry = _cmd("fetch", "echo {cookie}")
    argstate.save_last(entry.slug, values={"cookie": "SESSIONID=abc123"})
    assert argstate.load_state(entry.slug)["values"] == {"cookie": "SESSIONID=abc123"}

    result = runner.invoke(cli.app, ["params", entry.slug, "--secret", "cookie"])

    assert "Removed previously stored plaintext value(s)" in result.output
    assert argstate.load_state(entry.slug)["values"] == {}


@pytest.mark.parametrize(
    ("flag", "value", "attr", "expected"),
    [
        ("--prompt", "cookie=Session cookie", "prompt", "Session cookie"),
        ("--type", "cookie=int", "type", "int"),
        ("--help-text", "cookie=paste it here", "help", "paste it here"),
        ("--default", "cookie=none", "default", "none"),
    ],
)
def test_every_tweak_flag_materializes_the_row_not_just_secret(
    flag: str, value: str, attr: str, expected: str
) -> None:
    """The fix is the RULE, not a special case for --secret: a placeholder the entry asks
    for is an editable parameter. Special-casing one flag would have left the other seven
    silently skipping, which is how this seam was built in the first place."""
    entry = _cmd("fetch", "echo {cookie}")

    result = runner.invoke(cli.app, ["params", entry.slug, flag, value])

    assert result.exit_code == 0, result.output
    assert "isn't a declared parameter" not in result.output
    (decl,) = params.declared_from_meta(store.resolve(entry.slug).meta.parameters)
    assert getattr(decl, attr) == expected


def test_optional_on_a_placeholder_survives_its_own_invariants() -> None:
    """A materialized row is born required; --optional must be able to change that, which
    only works if the row exists before the tweak pass runs its normalize/validate step."""
    entry = _cmd("fetch", "echo {cookie}")

    result = runner.invoke(cli.app, ["params", entry.slug, "--optional", "cookie"])

    assert result.exit_code == 0, result.output
    (decl,) = params.declared_from_meta(store.resolve(entry.slug).meta.parameters)
    assert decl.required is False


def test_a_name_that_is_not_a_placeholder_is_still_refused() -> None:
    """The complement that keeps the rule a rule: `--secret typo` names nothing the entry
    asks for, and inventing a row for it would be the refuse-never-drop sin in reverse —
    silently accepting a flag that means nothing."""
    entry = _cmd("fetch", "echo {cookie}")

    result = runner.invoke(cli.app, ["params", entry.slug, "--secret", "typo"])

    assert "typo isn't a declared parameter; skipped." in " ".join(result.output.split())
    assert params.declared_from_meta(store.resolve(entry.slug).meta.parameters) == []


def test_rm_of_an_undeclared_name_still_warns() -> None:
    """--rm is the one op materializing cannot serve: creating a row in order to delete it
    is not work, it is theatre. Its not-declared warning stays."""
    entry = _cmd("fetch", "echo {cookie}")

    result = runner.invoke(cli.app, ["params", entry.slug, "--rm", "cookie"])

    assert "cookie isn't a declared parameter; skipped." in " ".join(result.output.split())


def test_the_shared_constructor_is_what_both_doors_use() -> None:
    """One row shape, whichever door creates it. The two doors used to disagree by
    OMISSION — only --add knew how to build the row — and a duplicated constructor would
    have re-created the same divergence in a form that merely looks symmetric."""
    entry = _cmd("fetch", "echo {cookie}")
    runner.invoke(cli.app, ["params", entry.slug, "--add", "cookie"])
    added = params.declared_from_meta(store.resolve(entry.slug).meta.parameters)

    other = _cmd("fetch2", "echo {cookie}")
    runner.invoke(cli.app, ["params", other.slug, "--help-text", "cookie=x"])
    tweaked = params.declared_from_meta(store.resolve(other.slug).meta.parameters)

    assert added == [params._placeholder_decl("cookie")]
    assert [d.name for d in tweaked] == ["cookie"]
    assert tweaked[0].delivery == added[0].delivery
    assert tweaked[0].required == added[0].required


# ==========================================================================
# N. resolve() verifies the name it was handed
# ==========================================================================


def _rename_meta_only(slug: str, new_name: str) -> None:
    """A hand edit of meta.toml — the file store.py's own docstrings call the truth, and
    the file the whole mtime_ns projection exists to track."""
    path = store.scripts_dir() / slug / "meta.toml"
    text = path.read_text(encoding="utf-8")
    old = store.resolve(slug).meta.name
    path.write_text(text.replace(f'name = "{old}"', f'name = "{new_name}"'), encoding="utf-8")


def test_resolve_finds_the_name_the_meta_carries_with_no_listing_first() -> None:
    """THE round-9 MEDIUM. list_summaries' docstring gives the reason its freshness proof
    exists: without it "the CLI would list entries the TUI, doctor and `run` all refuse".
    `run` reaches the store through resolve, and resolve was the one door that never
    checked — so whether an entry existed depended on whether some earlier, unrelated
    command happened to be a listing."""
    entry = _cmd("my tool")
    _rename_meta_only(entry.slug, "hola")

    assert store.resolve("hola").slug == entry.slug
    assert store.resolve("hola").meta.name == "hola"


def test_the_name_the_index_still_advertises_is_not_accepted() -> None:
    """The other direction of the same check. The row says "my tool"; the meta does not.
    Serving it would mean two names resolving to one entry, one of which nothing on disk
    agrees with."""
    entry = _cmd("my tool")
    _rename_meta_only(entry.slug, "hola")

    with pytest.raises(store.NotFoundError):
        store.resolve("my tool")


def test_a_slug_is_never_a_stale_projection() -> None:
    """A slug IS the directory name, so there is no row to be stale about it and the fast
    path stays fast: a slug hit is served without the sweep."""
    entry = _cmd("my tool")
    _rename_meta_only(entry.slug, "hola")

    assert store.resolve(entry.slug).meta.name == "hola"


def test_a_typo_still_raises_not_found() -> None:
    """The miss path pays for a full sweep before it raises — it must still raise."""
    _cmd("one")

    with pytest.raises(store.NotFoundError):
        store.resolve("nosuchthing")


def test_run_and_list_now_agree_about_the_same_entry() -> None:
    """The user-visible shape of the bug, on the two commands that disagreed: `skit list`
    showed `hola` and `skit run hola` exited 127 Script not found."""
    entry = _cmd("my tool", "echo hi")
    _rename_meta_only(entry.slug, "hola")

    listed = json.loads(runner.invoke(cli.app, ["list", "--json"]).stdout)
    assert [(e["name"], e["slug"]) for e in listed] == [("hola", entry.slug)]
    assert runner.invoke(cli.app, ["run", "hola", "--no-input"]).exit_code == 0


def test_a_corrupt_meta_keeps_its_own_message_through_either_path() -> None:
    """The corrupt-meta refusal names the entry the way the USER asked for it and points at
    doctor --rebuild; routing through a second lookup must not turn it into a bare
    not-found, which says nothing about what is wrong."""
    entry = _cmd("one")
    (store.scripts_dir() / entry.slug / "meta.toml").write_text("oops = [", encoding="utf-8")

    with pytest.raises(store.NotFoundError) as excinfo:
        store.resolve(entry.slug)

    assert "metadata is corrupt" in str(excinfo.value)
    assert "skit doctor --rebuild" in str(excinfo.value)


# ==========================================================================
# O. record_run keeps a snapshot it was given no replacement for
# ==========================================================================


def test_record_run_without_values_leaves_the_snapshot_alone() -> None:
    """The convention save_last states one screen up, applied to the function beside it:
    None means "no new data", not "erase what is there". record_run replaced the whole
    table, so a caller that only had a timestamp deleted the values."""
    argstate.save_last("s", values={"WIDTH": "1200"})
    argstate.record_run("s", 0, at="2026-07-27T00:00:00+00:00", values={"WIDTH": "1200"})

    argstate.record_run("s", 0, at="2026-07-27T01:00:00+00:00")

    state = argstate.load_state("s")
    assert state["last_run"]["values"] == {"WIDTH": "1200"}
    assert state["last_run"]["at"] == "2026-07-27T01:00:00+00:00"  # the stamp still moves
    assert state["last_run"]["exit"] == 0


def test_record_run_with_values_still_replaces_the_snapshot() -> None:
    """The preserving branch must not become a merging one: the snapshot is the EXACT
    accepted invocation, so a parameter dropped from the run is dropped from the record."""
    argstate.record_run("s", 0, at="t1", values={"A": "1", "B": "2"})
    argstate.record_run("s", 0, at="t2", values={"A": "9"})

    assert argstate.load_state("s")["last_run"]["values"] == {"A": "9"}


def test_a_run_that_never_recorded_values_still_records_none() -> None:
    """Nothing is conjured: with no stored snapshot, values=None leaves the table without
    one, which is the legacy shape --from-last is written to recognize."""
    argstate.record_run("s", 0, at="t1")

    assert "values" not in argstate.load_state("s")["last_run"]


def test_a_raw_run_keeps_the_preset_source_it_promises_to_keep(tmp_path: Path) -> None:
    """The escape hatch's own comment: it "consulted no form memory, so it must not rewrite
    it either (values/extra args survive for the next real run)". It half did — and what it
    left behind made `preset save --from-last` refuse with "no remembered values yet — run
    it once first" about an entry whose values were in the same file and which had just run
    twice."""
    src = _py(tmp_path, 'WIDTH = "800"\nprint(WIDTH)\n')
    entry = store.add_python(src, name="hello")
    runner.invoke(cli.app, ["params", entry.slug, "--manage", "WIDTH"])
    runner.invoke(cli.app, ["run", entry.slug, "--set", "WIDTH=1200", "--no-input"])

    raw = runner.invoke(cli.app, ["run", entry.slug, "--raw", "--no-input"])
    assert raw.exit_code == 0, raw.output

    saved = runner.invoke(cli.app, ["preset", "save", entry.slug, "nightly", "--from-last"])
    assert saved.exit_code == 0, saved.output
    assert argstate.load_state(entry.slug)["presets"]["nightly"] == {"WIDTH": "1200"}


# ==========================================================================
# P. doctor prints the three roots there are
# ==========================================================================


def test_doctor_prints_the_config_and_state_roots(tmp_path: Path) -> None:
    """Two docs pages have always said doctor prints them and nothing in skit did, so
    "where is my config.toml / my presets, and what do I back up?" had no answer anywhere —
    a zero-memorization gap in the command that exists to answer it."""
    result = runner.invoke(cli.app, ["doctor"])

    out = " ".join(result.output.split())
    assert f"Config: {tmp_path / 'config'}" in out
    assert f"State: {tmp_path / 'state'}" in out
    assert "Library:" in out  # ...beside the one it always printed


def test_doctor_json_carries_all_three_roots(tmp_path: Path) -> None:
    """The machine contract gains them additively, next to the `location` key it already
    had — an agent asked to find a user's presets should not have to reimplement
    platformdirs."""
    payload = json.loads(runner.invoke(cli.app, ["doctor", "--json"]).stdout)

    assert payload["location"] == str(store.scripts_dir())
    assert payload["config_dir"] == str(tmp_path / "config")
    assert payload["state_dir"] == str(tmp_path / "state")


# ==========================================================================
# Q. the trait that decides is the one the code reads
# ==========================================================================


def test_the_placeholder_kinds_are_marked_by_the_trait_that_is_consulted() -> None:
    """takes_argv was assigned for exactly these two kinds, asserted in five tests, cited
    by three comments and a design doc — and read by no code at all. placeholder_params is
    what every template/non-template decision actually keys off, so it is the only marker
    left. A trait no code consults is a story, not a contract."""
    for kind in ("command", "prompt"):
        spec = spec_for(kind)
        assert spec is not None
        assert spec.placeholder_params is True
        assert not hasattr(spec, "takes_argv")


def test_the_json_listing_carries_the_same_signal_on_stderr() -> None:
    """`skit list --json` is the path an agent calls most, and its contract is exactly one
    JSON array on stdout — so the array stays `[]` and the fact rides stderr, like every
    other skit-side warning. Without it an agent reads `[]`, concludes the user has no
    scripts, and offers to add the ones they already own."""
    _cmd("one")
    _lose_the_index()

    result = runner.invoke(cli.app, ["list", "--json"])

    assert json.loads(result.stdout) == []  # stdout is still one clean array
    assert "1 stored entry is still on disk" in " ".join(result.stderr.split())
    assert "skit doctor --rebuild" in result.stderr


def test_a_healthy_json_listing_says_nothing_on_stderr() -> None:
    """The complement: no warning when there is nothing to warn about, so a stderr line
    stays a signal rather than noise an agent learns to ignore."""
    _cmd("one")

    result = runner.invoke(cli.app, ["list", "--json"])

    assert [e["name"] for e in json.loads(result.stdout)] == ["one"]
    assert "doctor --rebuild" not in result.stderr


def test_both_list_faces_print_the_same_sentence() -> None:
    """ONE msgid, two registers. Two wordings of one fact is how the empty-state copy and
    doctor came to disagree in the first place."""
    _cmd("one")
    _lose_the_index()

    human = " ".join(runner.invoke(cli.app, ["list"]).output.split())
    machine = " ".join(runner.invoke(cli.app, ["list", "--json"]).stderr.split())

    assert human == machine
