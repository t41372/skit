"""Mutation-kill tests for ``MenuApp._detail_state_lines`` (src/skit/tui.py, chunk 4).

``_detail_state_lines`` renders the Library detail pane's per-script status block:
the Parameters summary, Presets, Depends-on, the Last-run outcome, and the
missing-target / drift markers. It is a near-pure function of the entry plus its
on-disk state, so these tests build real entries and real state, call the method
directly, and pin every rendered line by exact value — mirroring how the detail
pane consumes the returned list. The English catalog is active (conftest pins
SKIT_LANG=en and resets the i18n singleton), so ``gettext`` returns the msgid
verbatim and every copy/format mutation shows up in the output.
"""

from __future__ import annotations

import datetime
from pathlib import Path

from skit import argstate, launcher, store, tui
from skit.atomic import atomic_write_toml
from skit.langs.python import metawriter
from skit.params import ParamDecl
from skit.paths import values_dir


def _py(tmp_path: Path, body: str, name: str = "job.py") -> Path:
    p = tmp_path / name
    p.write_text(body, encoding="utf-8")
    return p


def _now_iso() -> str:
    return datetime.datetime.now(datetime.UTC).isoformat()


# ---------------------------------------------------------------- Parameters


def test_params_line_shows_six_then_ellipsis_with_seven_fields(tmp_path: Path) -> None:
    """A seven-placeholder template renders exactly the first six fields plus a
    trailing ' …', and a stored value overrides the field default in the summary.

    Pins: the [:6] slice (a 7th field must NOT appear), the '<=6 else " …"' branch
    (the seventh field forces the ellipsis), the ' …' literal, the "Parameters
    %(summary)s" msgid, the '  ' field separator, and state["values"].get(f.key, …).
    """
    entry = store.add_command("echo {aa} {bb} {cc} {dd} {ee} {ff} {gg}", name="seven")
    argstate.save_last(entry.slug, values={"aa": "hello"})
    lines = tui.MenuApp()._detail_state_lines(entry)

    assert "Parameters  aa=hello  bb  cc  dd  ee  ff …" in lines
    # The 7th placeholder is sliced off; a [:7] mutant would surface it.
    assert not any("gg" in line for line in lines)


def test_params_line_has_no_ellipsis_at_exactly_six_fields(tmp_path: Path) -> None:
    """Exactly six placeholders render all six with no trailing marker.

    Pins the '<= 6' boundary (six fields is still "all shown") and the empty-string
    "no more" literal — a '< 6' or a non-empty replacement would add a marker.
    """
    entry = store.add_command("echo {aa} {bb} {cc} {dd} {ee} {ff}", name="six")
    lines = tui.MenuApp()._detail_state_lines(entry)

    assert "Parameters  aa  bb  cc  dd  ee  ff" in lines


# ---------------------------------------------------------------- Presets


def test_presets_line_lists_sorted_names(tmp_path: Path) -> None:
    """Saved presets render sorted and ' · '-joined under the "Presets" label."""
    entry = store.add_command("echo hi", name="preset-host")
    argstate.save_preset(entry.slug, "zzz", {})
    argstate.save_preset(entry.slug, "aaa", {})
    lines = tui.MenuApp()._detail_state_lines(entry)

    assert "Presets  aaa · zzz" in lines


# ---------------------------------------------------------------- Depends on


def test_depends_on_line_joins_dependencies(tmp_path: Path) -> None:
    """Declared dependencies render ', '-joined under the "Depends on" label."""
    entry = store.add_command("echo hi", name="dep-host")
    # meta.dependencies is the field the branch reads directly; a real entry carries it.
    entry.meta.dependencies = ["requests", "rich"]
    lines = tui.MenuApp()._detail_state_lines(entry)

    assert "Depends on  requests, rich" in lines


# ---------------------------------------------------------------- Last run


def test_last_run_finished_line(tmp_path: Path) -> None:
    """A successful last run renders the timestamp + green "finished" outcome.

    Pins the "Last run %(when)s · %(outcome)s" msgid, the 'finished' msgid, and the
    ``last.get("at", "")`` lookup (a recent timestamp must read as "just now", so a
    mutated key/default that loses it would blank or garble the relative time).
    """
    entry = store.add_command("echo hi", name="ok")
    argstate.record_run(entry.slug, 0, at=_now_iso())
    lines = tui.MenuApp()._detail_state_lines(entry)

    assert "Last run  just now · [green]✓ finished[/green]" in lines


def test_last_run_failed_line_carries_the_exit_code(tmp_path: Path) -> None:
    """A failed last run renders the yellow "failed (code N)" outcome with the real code.

    Pins the 'failed (code %(code)s)' msgid and ``last.get("exit")`` — a mutated key
    would substitute None for the exit code ("code None").
    """
    entry = store.add_command("echo hi", name="bad")
    argstate.record_run(entry.slug, 3, at=_now_iso())
    lines = tui.MenuApp()._detail_state_lines(entry)

    assert "Last run  just now · [yellow]✗ failed (code 3)[/yellow]" in lines


def test_last_run_without_at_timestamp_renders_empty_when(tmp_path: Path) -> None:
    """A last_run table missing its "at" key falls back to an empty relative time.

    ``_detail_state_lines`` guards the timestamp with ``last.get("at", "")``; a state
    file that recorded an outcome without a timestamp must render a blank "when",
    never "None" or a stray sentinel. Written straight to the state file because
    record_run always supplies "at".
    """
    entry = store.add_command("echo hi", name="noat")
    atomic_write_toml(values_dir() / f"{entry.slug}.toml", {"last_run": {"exit": 0}})
    lines = tui.MenuApp()._detail_state_lines(entry)

    assert "Last run   · [green]✓ finished[/green]" in lines


def test_not_run_yet_line(tmp_path: Path) -> None:
    """An entry that has never run renders the dim "Not run yet" placeholder."""
    entry = store.add_command("echo hi", name="fresh")
    lines = tui.MenuApp()._detail_state_lines(entry)

    assert "[dim]Not run yet[/dim]" in lines


# ---------------------------------------------------------------- markers


def test_missing_target_marker_line(tmp_path: Path) -> None:
    """When the launch target is gone, the detail pane shows the missing-target marker.

    Pins ``marker = launcher.missing_marker(entry)``: dropping it to None would erase
    the warning line entirely.
    """
    entry = store.add_python(_py(tmp_path, "print(1)\n", "gone.py"), name="gone")
    entry.script_path.unlink()  # target now missing
    marker = launcher.missing_marker(entry)
    assert marker is not None  # the fixture really produced a marker

    lines = tui.MenuApp()._detail_state_lines(entry)
    assert any(line.startswith("[yellow]⚠ missing:") for line in lines)


def test_drift_marker_line(tmp_path: Path) -> None:
    """A script that drifted from its declared params shows the drift warning.

    GONE is declared as a managed const but never assigned in the body, so
    ``plan_for_entry`` reports drift and ``_has_drift`` is True. Pins the exact drift
    copy passed to gettext (msgid, not None, and its casing).
    """
    drifted = metawriter.write_params(
        "CITY = 'x'\nprint(CITY)\n",
        [
            ParamDecl(name="CITY", binding="const", type="str"),
            ParamDecl(name="GONE", binding="const", type="str"),
        ],
    )
    entry = store.add_python(_py(tmp_path, drifted, "drifty.py"), name="drifty")
    app = tui.MenuApp()
    assert app._has_drift(entry)  # the fixture really drifted

    lines = app._detail_state_lines(entry)
    assert (
        "[yellow]⚠ The script changed — skit checks the form against it before "
        "every run.[/yellow]" in lines
    )
