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
