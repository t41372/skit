"""Mutation-kill tests for skit/argstate.py.

argstate persists last-used values, extra args and named presets, and enforces C3 (secret keys
never hit disk, and retroactively scrubbing a value that predates a param becoming secret). These
exercise purge_secret's accumulation of *which* names were cleaned, and save_last's secret-drop on
a values=None call, through the real on-disk read-modify-write.
"""

from __future__ import annotations

import contextlib
from collections.abc import Callable, Iterator
from pathlib import Path

import pytest

from skit import argstate


@pytest.fixture(autouse=True)
def _isolated_state(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))


def test_purge_secret_reports_names_removed_across_values_and_presets() -> None:
    """purge_secret returns the subset of names it actually scrubbed (so the caller can tell the
    user what was cleaned). The name lives in [values]; a preset that does NOT hold it must not
    reset that accumulation — pins that the per-preset union keeps the value-side hit."""
    slug = "purge-demo"
    argstate.save_last(slug, values={"API_TOKEN": "abc", "REGION": "us"})
    argstate.save_preset(slug, "prod", {"REGION": "eu"})

    removed = argstate.purge_secret(slug, ["API_TOKEN"])

    # The token was stored in [values] and gets reported as removed even though the surviving
    # preset never held it. (mutant_34 `removed = …` / mutant_35 `removed &= …` would drop it to set().)
    assert removed == {"API_TOKEN"}

    state = argstate.load_state(slug)
    assert state["values"] == {"REGION": "us"}  # secret plaintext scrubbed from last-used
    assert state["presets"] == {"prod": {"REGION": "eu"}}  # non-secret preset preserved intact


def test_save_last_drops_secret_with_no_stored_values_table() -> None:
    """save_last strips now-secret keys even on a values=None call. When the on-disk doc carries
    only extra_args (no [values] table at all), the strip must default the absent table to {} —
    not None — or it would crash trying to filter a None (mutant_21 `doc.get("values", None)` /
    mutant_23 `doc.get("values")`). The call must complete and leave extra_args untouched."""
    slug = "no-values-table"
    argstate.save_last(slug, extra_args=["--verbose"])
    assert argstate.load_state(slug)["values"] == {}  # precondition: no stored values

    # values=None (no new data) but a param just became secret: reaches the elif banned branch,
    # whose doc.get("values", {}) default is what the mutants attack.
    argstate.save_last(slug, values=None, secret_names=["SECRET"])

    state = argstate.load_state(slug)
    assert state["values"] == {}
    assert state["extra_args"] == ["--verbose"]  # unrelated stored data survived the secret-drop


def test_last_run_snapshot_strips_and_retroactively_purges_secrets() -> None:
    slug = "run-snapshot"
    argstate.record_run(
        slug,
        0,
        at="2026-07-09T00:00:00+00:00",
        values={"TOKEN": "plaintext", "CITY": "Taipei"},
        secret_names=(),
    )
    assert (
        argstate.load_state(slug)["last_run"]["values"]["TOKEN"] == "plaintext"  # noqa: S105
    )

    removed = argstate.purge_secret(slug, ["TOKEN"])

    assert removed == {"TOKEN"}
    assert argstate.load_state(slug)["last_run"]["values"] == {"CITY": "Taipei"}

    # New snapshots enforce C3 before the value reaches disk at all.
    argstate.record_run(
        slug,
        0,
        at="2026-07-10T00:00:00+00:00",
        values={"TOKEN": "new-secret", "CITY": "Osaka"},
        secret_names={"TOKEN"},
    )
    assert argstate.load_state(slug)["last_run"]["values"] == {"CITY": "Osaka"}


# ---------------------------------------------------------------------------
# Cross-process/thread RMW lock around the value-file mutators
# ---------------------------------------------------------------------------


def test_values_lock_path_shape() -> None:
    """The read-modify-write lock lives OUTSIDE values/ — forget() unlinks the values file
    itself, and a lock file must never be a path another process is about to unlink. Its shape is
    state_dir()/.locks/<slug>.values.lock."""
    from skit.paths import state_dir

    path = argstate._values_lock_path("my-slug")
    assert path.name == "my-slug.values.lock"
    assert path.parent.name == ".locks"
    assert path.parent.parent == state_dir()


# Each read-modify-write mutator wraps its body in advisory_file_lock(_values_lock_path(slug)).
# purge_secret only reaches the lock with a NON-EMPTY names (it early-returns set() otherwise), so
# it is invoked with one. delete_preset locks before it checks membership, so it locks even with no
# matching preset. None of these need on-disk preconditions to acquire the lock exactly once.
_RMW_MUTATORS: list[object] = [
    pytest.param(lambda slug: argstate.save_last(slug, values={"A": "1"}), id="save_last"),
    pytest.param(lambda slug: argstate.save_preset(slug, "p", {"A": "1"}), id="save_preset"),
    pytest.param(lambda slug: argstate.delete_preset(slug, "p"), id="delete_preset"),
    pytest.param(lambda slug: argstate.purge_secret(slug, ["A"]), id="purge_secret"),
    pytest.param(
        lambda slug: argstate.record_run(slug, 0, at="2026-01-01T00:00:00+00:00"),
        id="record_run",
    ),
]


@pytest.mark.parametrize("invoke", _RMW_MUTATORS)
def test_rmw_mutator_locks_the_exact_per_slug_values_path(
    invoke: Callable[[str], object], monkeypatch: pytest.MonkeyPatch
) -> None:
    """Every value-file read-modify-write holds advisory_file_lock(_values_lock_path(slug)).

    The lock's ARGUMENT must be this slug's own path. A mutant that passes _values_lock_path(None)
    still hands advisory_file_lock a real, exclusive path (state_dir()/.locks/None.values.lock), so
    a single-process run serializes fine and the wrong slug goes unnoticed — only pinning the exact
    captured path catches it. The spy still yields, so the real RMW runs underneath and the mutator
    keeps functioning.
    """
    slug = "lock-slug"
    captured: list[Path] = []

    @contextlib.contextmanager
    def spy(lock_path: Path, **_kwargs: object) -> Iterator[None]:
        captured.append(lock_path)
        yield

    monkeypatch.setattr(argstate, "advisory_file_lock", spy)

    invoke(slug)

    # Exactly this slug's lock path — not _values_lock_path(None) (…/None.values.lock), which is
    # what the surviving `slug`→`None` mutant would have captured.
    assert captured == [argstate._values_lock_path(slug)]
    assert captured[0].name == "lock-slug.values.lock"


def test_concurrent_save_preset_from_many_threads_loses_no_preset() -> None:
    """Each save_preset wraps its load→modify→save in advisory_file_lock(_values_lock_path(slug)).
    Without it, N threads saving distinct presets from the same stale snapshot would silently drop
    all but the last writer (last-writer-wins on the single values file). The in-process thread
    lock serializes them, so every one of the N presets survives."""
    from concurrent.futures import ThreadPoolExecutor

    slug = "many-threads"
    names = [f"p{i}" for i in range(8)]

    def save(name: str) -> None:
        argstate.save_preset(slug, name, {name: "v"})

    with ThreadPoolExecutor(max_workers=8) as pool:
        list(pool.map(save, names))

    presets = argstate.load_state(slug)["presets"]
    assert set(presets) == set(names)  # not one lost to a stale-snapshot overwrite
    assert all(presets[n] == {n: "v"} for n in names)


# ---------------------------------------------------------------------------
# last_run — the listing's slice of a state file
# ---------------------------------------------------------------------------


def test_last_run_matches_load_state_before_and_after_a_run() -> None:
    """`skit list --json` reports last_run_at/last_exit for every entry, so it reads
    one state file per entry. `last_run` reads the same file and must give the same
    answer as the full `load_state` — it only skips copying out values, extra args and
    every preset, which a listing never renders."""
    assert argstate.last_run("never-run") == argstate.load_state("never-run")["last_run"] == {}

    argstate.save_last("s", values={"a": "1"}, extra_args=["--x"])
    argstate.save_preset("s", "prod", {"a": "2"}, secret_names=())
    assert argstate.last_run("s") == {}

    argstate.record_run("s", 3, at="2026-07-25T00:00:00+00:00")
    assert argstate.last_run("s") == argstate.load_state("s")["last_run"]
    assert argstate.last_run("s")["exit"] == 3


def test_last_run_is_a_copy_not_the_stored_mapping() -> None:
    """A caller mutating what it got back must not corrupt the next read."""
    argstate.record_run("s", 0, at="2026-07-25T00:00:00+00:00")
    first = argstate.last_run("s")
    first["exit"] = 99
    assert argstate.last_run("s")["exit"] == 0


def test_a_missing_state_file_is_empty_not_an_error() -> None:
    """The common case for an entry that has never run — and the reason the exists()
    check ahead of the open was pure cost."""
    assert argstate.load_state("absent") == {
        "values": {},
        "extra_args": [],
        "extra_args_raw": False,
        "presets": {},
        "last_run": {},
    }


# ---------------------------------------------------------------------------
# The _load_doc shape guard — a values file is TOML a person can edit
# ---------------------------------------------------------------------------


def _write_values_file(slug: str, body: str) -> None:
    """Hand-write state_dir()/values/<slug>.toml — the file a user (or a stray editor)
    can put anything valid-TOML into. Deliberately not through argstate's own writers:
    the shapes under test are ones no writer would ever produce."""
    from skit.paths import values_dir

    values_dir().mkdir(parents=True, exist_ok=True)
    (values_dir() / f"{slug}.toml").write_text(body, encoding="utf-8")


_EMPTY_STATE = {
    "values": {},
    "extra_args": [],
    "extra_args_raw": False,
    "presets": {},
    "last_run": {},
}


@pytest.mark.parametrize(
    ("body", "expected"),
    [
        (
            'values = 5\nextra_args = ["--verbose"]\n',
            {**_EMPTY_STATE, "extra_args": ["--verbose"]},
        ),
        (
            'extra_args = "--verbose"\n[values]\nCITY = "Taipei"\n',
            {**_EMPTY_STATE, "values": {"CITY": "Taipei"}},
        ),
        (
            'presets = 5\n[values]\nCITY = "Taipei"\n',
            {**_EMPTY_STATE, "values": {"CITY": "Taipei"}},
        ),
        (
            '[presets]\nbroken = "not a table"\n\n[presets.prod]\nCITY = "Osaka"\n',
            {**_EMPTY_STATE, "presets": {"prod": {"CITY": "Osaka"}}},
        ),
        (
            'last_run = "garbage"\n[values]\nCITY = "Taipei"\n',
            {**_EMPTY_STATE, "values": {"CITY": "Taipei"}},
        ),
        (
            '[last_run]\nat = "2026-07-25T00:00:00+00:00"\nexit = 0\nvalues = "garbage"\n',
            {**_EMPTY_STATE, "last_run": {"at": "2026-07-25T00:00:00+00:00", "exit": 0}},
        ),
    ],
    ids=[
        "scalar-values",
        "scalar-extra-args",
        "scalar-presets",
        "scalar-preset-row",
        "scalar-last-run",
        "scalar-last-run-values",
    ],
)
def test_a_hand_edited_state_file_drops_only_the_malformed_section(
    body: str, expected: dict[str, object]
) -> None:
    """Every reader subscripts these four sections, so a scalar where a table (or array)
    belongs used to crash whichever reader touched it first. The guard sits at the single
    load chokepoint: the malformed section degrades to its documented empty shape, one
    bad preset row is dropped without taking its healthy siblings with it, and everything
    else in the file round-trips untouched."""
    slug = "hand-edited"
    _write_values_file(slug, body)

    assert argstate.load_state(slug) == expected
    # last_run reads the same file through the same guard, so it can never disagree.
    assert argstate.last_run(slug) == expected["last_run"]


def test_a_scalar_last_run_still_lists_through_the_cli(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The user-visible surface: `skit list --json` reads one last-run stamp per entry, so
    a single hand-broken values file used to take the WHOLE listing down with a ValueError
    — an agent asking what the library contains got a traceback instead of the JSON
    contract. The bad section degrades to {}; the entry still lists, with null stamps."""
    import json

    from typer.testing import CliRunner

    from skit import cli, store

    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path / "config"))
    entry = store.add_command("echo hi", name="chores")
    _write_values_file(entry.slug, 'last_run = "garbage"\n')

    assert argstate.last_run(entry.slug) == {}
    assert argstate.load_state(entry.slug) == _EMPTY_STATE

    result = CliRunner().invoke(cli.app, ["list", "--json"])

    assert result.exit_code == 0
    (row,) = json.loads(result.output)
    assert row["name"] == "chores"
    assert (row["last_run_at"], row["last_exit"]) == (None, None)


def test_purge_secret_survives_a_last_run_values_that_is_not_a_table() -> None:
    """C3's retroactive scrub subscripts the nested last-run snapshot, so a hand-edited
    `values = "garbage"` under [last_run] used to crash the very call that exists to get
    plaintext off disk. The nested section is dropped while the stamp around it survives —
    there is nothing left to scrub, and the purge reports exactly that."""
    slug = "broken-snapshot"
    _write_values_file(
        slug,
        '[values]\nAPI_KEY = "plaintext"\n'
        '[last_run]\nat = "2026-07-25T00:00:00+00:00"\nexit = 0\nvalues = "garbage"\n',
    )

    assert argstate.last_run(slug) == {"at": "2026-07-25T00:00:00+00:00", "exit": 0}

    removed = argstate.purge_secret(slug, ["API_KEY"])

    assert removed == {"API_KEY"}  # the readable plaintext still got scrubbed
    state = argstate.load_state(slug)
    assert state["values"] == {}
    assert state["last_run"] == {"at": "2026-07-25T00:00:00+00:00", "exit": 0}


# ---------------------------------------------------------------------------
# StateWriteError: every writer fails typed (round 13, finding S)
# ---------------------------------------------------------------------------


def _boom_write(*_args: object, **_kwargs: object) -> None:
    raise OSError(30, "Read-only file system", "/state/values/x.toml")


@pytest.mark.parametrize(
    "write",
    [
        pytest.param(lambda: argstate.save_last("s", values={"A": "1"}), id="save_last"),
        pytest.param(lambda: argstate.save_preset("s", "p", {"A": "1"}), id="save_preset"),
        pytest.param(lambda: argstate.delete_preset("s", "p"), id="delete_preset"),
        pytest.param(lambda: argstate.purge_secret("s", ["A"]), id="purge_secret"),
        pytest.param(
            lambda: argstate.record_run("s", 0, at="2026-01-01T00:00:00+00:00"),
            id="record_run",
        ),
        pytest.param(lambda: argstate.save_last_runner("amp"), id="save_last_runner"),
    ],
)
def test_a_failed_state_write_raises_the_typed_error(
    write: Callable[[], object], monkeypatch: pytest.MonkeyPatch
) -> None:
    """Every argstate writer re-raises an OSError as StateWriteError — an OSError
    subclass, config.ConfigWriteError's mirror — with errno/strerror/filename intact,
    so the CLI boundary renders the same operational sentence it renders for a config
    write and post_run_persistence_error's `except OSError` keeps catching it."""
    argstate.save_preset("s", "p", {"KEEP": "1"})  # delete_preset needs a hit to write
    monkeypatch.setattr(argstate, "atomic_write_toml", _boom_write)

    with pytest.raises(argstate.StateWriteError) as exc_info:
        write()

    err = exc_info.value
    assert isinstance(err, OSError)
    assert err.errno == 30
    assert err.strerror == "Read-only file system"
    assert err.filename == "/state/values/x.toml"


def test_the_rewrap_falls_back_to_the_exception_text_without_a_strerror(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """A bare OSError("boom") carries no strerror; the rewrap must surface the message
    anyway — a refusal whose sentence is empty helps nobody."""

    def _bare_boom(*_args: object, **_kwargs: object) -> None:
        raise OSError("boom")

    monkeypatch.setattr(argstate, "atomic_write_toml", _bare_boom)

    with pytest.raises(argstate.StateWriteError) as exc_info:
        argstate.save_last_runner("amp")

    assert exc_info.value.strerror == "boom"
