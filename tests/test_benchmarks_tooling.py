"""The benchmark pipeline's contract tests (docs/design/benchmarks.md).

Everything here is hermetic: no hyperfine/pyperf binaries, no network, no real skit
home (conftest isolates SKIT_*). What's tested is the covered trust layer — the
schema, the budget contract's every decay channel, the parsers, the plan/merge/derive
logic, and the dataset generator's load-bearing invariants.
"""

from __future__ import annotations

import json
import os
from importlib.metadata import version
from pathlib import Path
from types import SimpleNamespace
from typing import Any, cast

import pytest
from benchmarks import compare as bcompare
from benchmarks import datasets, envinfo, hyperfine, parsers, pipeline
from benchmarks.budgets import (
    Budget,
    BudgetsError,
    evaluate,
    load_budgets,
    propose,
    render_budgets,
    render_report,
)
from benchmarks.datasets import (
    DatasetError,
    Manifest,
    generate,
    generate_runover,
    skit_dirs,
)
from benchmarks.fixtures import sources
from benchmarks.results import (
    GitInfo,
    HostInfo,
    Meta,
    Metric,
    Results,
    ResultsError,
    Skip,
    SuiteOutput,
    meta_from_dict,
    python_major_minor,
)
from benchmarks.suites import micro as micro_suite
from benchmarks.suites import rss as rss_suite
from benchmarks.suites import tui as tui_suite
from benchmarks.suites._env import RunCtx

REPO_ROOT = Path(__file__).resolve().parent.parent
FIXTURES_DIR = REPO_ROOT / "benchmarks" / "fixtures"


def make_meta(
    *,
    profile: str = "pr",
    python: str = "3.13.5",
    ci_runner: str | None = "ubuntu-24.04",
    platform_key: str = "linux-x86_64",
    ci_image_version: str | None = "20260719.1",
    pyperf: str = "2.10.0",
) -> Meta:
    return Meta(
        generated_at="2026-07-20T12:00:00+00:00",
        profile=profile,
        git=GitInfo(commit="abcdef1234567890", dirty=False),
        skit_version="0.2.1.dev0",
        host=HostInfo(
            os="Linux",
            kernel="6.8.0",
            cpu="Test CPU",
            cpu_count=8,
            mem_total_mib=16384,
            platform_key=platform_key,
            ci_runner=ci_runner,
            ci_image_version=ci_image_version if ci_runner else None,
        ),
        python=python,
        uv="0.11.26",
        textual="8.2.8",
        pyperf=pyperf,
    )


def make_results(
    metrics: dict[str, Metric],
    *,
    meta: Meta | None = None,
    skipped: list[Skip] | None = None,
) -> Results:
    return Results(
        meta=meta or make_meta(),
        metrics=metrics,
        skipped=skipped or [],
        raw={},
    )


def _results_doc() -> dict[str, Any]:
    doc: dict[str, Any] = json.loads(make_results({}).to_json())
    return doc


def _set_path(doc: dict[str, Any], path: tuple[str, ...], value: object) -> str:
    node: Any = doc
    for key in path[:-1]:
        node = node[key]
    node[path[-1]] = value
    return json.dumps(doc)


# ================================================================ results model


class TestResultsModel:
    def test_round_trip(self) -> None:
        results = make_results(
            {"a.b": Metric(value=1.5, unit="ms", n=3, p95=2.0, stddev=0.1)},
            skipped=[Skip(suite="s", case="c", reason="r")],
        )
        assert Results.from_json(results.to_json()) == results

    def test_rejects_wrong_schema_version(self) -> None:
        doc = json.loads(make_results({}).to_json())
        doc["schema_version"] = 99
        with pytest.raises(ResultsError, match="schema_version"):
            Results.from_json(json.dumps(doc))

    def test_rejects_non_json(self) -> None:
        with pytest.raises(ResultsError, match="not valid JSON"):
            Results.from_json("{nope")

    def test_rejects_non_object(self) -> None:
        with pytest.raises(ResultsError, match="expected a JSON object"):
            Results.from_json("[1]")

    @pytest.mark.parametrize(
        ("mutation", "fragment"),
        [
            ({"value": "fast"}, "value"),
            ({"value": True}, "value"),
            ({"unit": ""}, "unit"),
            ({"n": 0}, "n"),
            ({"n": 1.5}, "n"),
            ({"p95": "high"}, "p95"),
            ({"stddev": "low"}, "stddev"),
        ],
    )
    def test_rejects_bad_metric_fields(self, mutation: dict[str, object], fragment: str) -> None:
        doc = json.loads(make_results({"m.x": Metric(value=1, unit="ms", n=1)}).to_json())
        doc["metrics"]["m.x"].update(mutation)
        with pytest.raises(ResultsError, match=f"m.x.*{fragment}|{fragment}"):
            Results.from_json(json.dumps(doc))

    def test_rejects_bad_skip_entry(self) -> None:
        doc = json.loads(make_results({}).to_json())
        doc["skipped"] = [{"suite": "s", "case": "", "reason": "r"}]
        with pytest.raises(ResultsError, match="case"):
            Results.from_json(json.dumps(doc))

    def test_rejects_empty_meta_strings(self) -> None:
        doc = json.loads(make_results({}).to_json())
        doc["meta"]["host"]["platform_key"] = ""
        with pytest.raises(ResultsError, match="platform_key"):
            Results.from_json(json.dumps(doc))

    def test_ci_runner_null_is_valid(self) -> None:
        doc = json.loads(make_results({}).to_json())
        doc["meta"]["host"]["ci_runner"] = None
        assert Results.from_json(json.dumps(doc)).meta.host.ci_runner is None

    def test_meta_from_dict_matches_round_trip(self) -> None:
        meta = make_meta()
        doc = json.loads(make_results({}, meta=meta).to_json())
        assert meta_from_dict(doc["meta"]) == meta

    def test_python_major_minor(self) -> None:
        assert python_major_minor("3.13.7") == "3.13"
        assert python_major_minor("3.14") == "3.14"

    def test_suite_output_round_trip(self) -> None:
        output = SuiteOutput(
            suite="startup",
            metrics={"startup.version.median_ms": Metric(value=200.0, unit="ms", n=15)},
            skipped=[Skip(suite="startup", case="x", reason="y")],
            raw={"times": [1, 2]},
            duration_s=1.25,
        )
        assert SuiteOutput.from_json(output.to_json()) == output

    def test_suite_output_rejects_bad_duration(self) -> None:
        doc = json.loads(SuiteOutput(suite="s").to_json())
        doc["duration_s"] = "long"
        with pytest.raises(ResultsError, match="duration_s"):
            SuiteOutput.from_json(json.dumps(doc))

    def test_suite_output_rejects_non_object_raw(self) -> None:
        doc = json.loads(SuiteOutput(suite="s").to_json())
        doc["raw"] = []
        with pytest.raises(ResultsError, match="raw: expected an object"):
            SuiteOutput.from_json(json.dumps(doc))

    @pytest.mark.parametrize(
        ("path", "value", "fragment"),
        [
            (("metrics",), [1], "metrics: expected an object"),
            (("metrics",), {"m.x": 5}, r"metrics\.m\.x: expected an object"),
            (("skipped",), {}, "skipped: expected an array"),
            (("skipped",), [5], r"skipped\[0\]: expected an object"),
            (("raw",), [], "raw: expected an object"),
            (("meta",), None, "meta: expected an object"),
            (("meta", "git"), 5, "meta.git: expected an object"),
            (("meta", "git", "dirty"), "yes", "meta.git.dirty: expected a boolean"),
            (("meta", "host"), 5, "meta.host: expected an object"),
            (("meta", "host", "cpu_count"), 0, "meta.host.cpu_count: expected a positive integer"),
            (
                ("meta", "host", "cpu_count"),
                True,
                "meta.host.cpu_count: expected a positive integer",
            ),
            (
                ("meta", "host", "cpu_count"),
                "8",
                "meta.host.cpu_count: expected a positive integer",
            ),
            (
                ("meta", "host", "mem_total_mib"),
                -1,
                "meta.host.mem_total_mib: expected a non-negative integer",
            ),
            (
                ("meta", "host", "mem_total_mib"),
                True,
                "meta.host.mem_total_mib: expected a non-negative integer",
            ),
            (
                ("meta", "host", "mem_total_mib"),
                "x",
                "meta.host.mem_total_mib: expected a non-negative integer",
            ),
            (("meta", "host", "ci_runner"), 5, "meta.host.ci_runner: expected a string or null"),
        ],
    )
    def test_rejects_bad_results_structure(
        self, path: tuple[str, ...], value: object, fragment: str
    ) -> None:
        text = _set_path(_results_doc(), path, value)
        with pytest.raises(ResultsError, match=fragment):
            Results.from_json(text)


# ================================================================ budgets


def budgets_from(toml_text: str) -> list[Budget]:
    return load_budgets(toml_text)


ENFORCED_ROW = """
[[budget]]
metric = "imports.version.modules"
max = 320
tier = "enforced"
ratchet = true
context = { python = "3.13", commit = "abc", date = "2026-07-20" }
"""


class TestBudgetLoader:
    def test_loads_the_real_contract_file(self) -> None:
        budgets = budgets_from(
            (REPO_ROOT / "benchmarks" / "budgets.toml").read_text(encoding="utf-8")
        )
        enforced = [b for b in budgets if b.tier == "enforced"]
        assert all(b.context for b in enforced)
        # Both anti-decay rows exist: the skip-prone suites run only nightly, so a
        # pr-only row would never budget them (design: Budgets).
        skip_rows = {b.profiles for b in enforced if b.metric == "pipeline.skipped_count"}
        assert skip_rows == {("pr",), ("full",)}
        # The wheel is built by pr and full but deliberately not by compare (it would
        # weigh the harness ref) — the enforced row must say so.
        (wheel,) = [b for b in enforced if b.metric == "footprint.wheel_bytes"]
        assert wheel.profiles == ("pr", "full")
        ratchets = [b for b in enforced if b.ratchet]
        assert ratchets, "the import ratchets must exist"
        assert all(b.context.get("python") == "3.13" for b in ratchets)

    @pytest.mark.parametrize(
        ("toml_text", "fragment"),
        [
            ("", "no \\[\\[budget\\]\\] rows"),
            ("x = 1", "unknown top-level"),
            ("[[budget]]\nmetric = 1\nmax = 1\ntier = 'target'", "metric"),
            ("[[budget]]\nmetric = 'm'\nmax = 'big'\ntier = 'target'", "max"),
            ("[[budget]]\nmetric = 'm'\nmax = 1\ntier = 'hard'", "tier"),
            ("[[budget]]\nmetric = 'm'\nmax = 1\ntier = 'target'\nratchet = true", "ratchet"),
            ("[[budget]]\nmetric = 'm'\nmax = 1\ntier = 'enforced'", "context"),
            ("[[budget]]\nmetric = 'm'\nmax = 1\ntier = 'target'\nbogus = 1", "unknown keys"),
            ("[[budget]]\nmetric = 'm'\nmax = 1\ntier = 'target'\nprofiles = ['']", "profiles"),
            ("[[budget]]\nmetric = 'm'\nmax = 1\ntier = 'target'\nprofiles = 'pr'", "profiles"),
            ("[[budget]]\nmetric = 'm'\nmax = 1\ntier = 'target'\nplatform = ''", "platform"),
            ("[[budget]]\nmetric = 'm'\nmax = inf\ntier = 'target'", "max must be finite"),
            (
                ENFORCED_ROW + "headroom = 2.0",
                "headroom",
            ),
            ("not toml [", "not valid TOML"),
            ("budget = [1]", "expected a table"),
            (
                "[[budget]]\nmetric = 'm'\nmax = 1\ntier = 'target'\nratchet = 1",
                "ratchet must be a boolean",
            ),
            (
                "[[budget]]\nmetric = 'm'\nmax = 1\ntier = 'target'\nheadroom = 'big'",
                "headroom must be a number",
            ),
            (
                "[[budget]]\nmetric = 'm'\nmax = 1\ntier = 'target'\nci_only = 1",
                "ci_only must be a boolean",
            ),
            (
                "[[budget]]\nmetric = 'm'\nmax = 1\ntier = 'target'\ncontext = { python = 1 }",
                "context must be a table of strings",
            ),
            (
                "[[budget]]\nmetric = 'm'\nmax = 1\ntier = 'target'\nnote = 5",
                "note must be a string",
            ),
        ],
    )
    def test_rejects_malformed_rows(self, toml_text: str, fragment: str) -> None:
        with pytest.raises(BudgetsError, match=fragment):
            budgets_from(toml_text)


class TestBudgetEvaluation:
    def test_pass_and_violation(self) -> None:
        budgets = budgets_from(ENFORCED_ROW)
        ok = evaluate(budgets, make_results({"imports.version.modules": Metric(300, "count", 1)}))
        assert ok.rows[0].outcome == "passed"
        assert not ok.failures
        bad = evaluate(budgets, make_results({"imports.version.modules": Metric(500, "count", 1)}))
        assert bad.rows[0].outcome == "violated"
        assert bad.failures

    def test_missing_metric_fails_enforced(self) -> None:
        report = evaluate(budgets_from(ENFORCED_ROW), make_results({}))
        assert report.rows[0].outcome == "metric-missing"
        assert report.failures

    def test_missing_metric_reported_not_failed_for_target(self) -> None:
        budgets = budgets_from("[[budget]]\nmetric = 'm'\nmax = 1\ntier = 'target'")
        report = evaluate(budgets, make_results({}))
        assert report.rows[0].outcome == "metric-missing"
        assert not report.failures

    def test_profile_predicate_scopes_row(self) -> None:
        budgets = budgets_from(ENFORCED_ROW + 'profiles = ["full"]')
        report = evaluate(
            budgets, make_results({"imports.version.modules": Metric(300, "count", 1)})
        )
        assert report.rows[0].outcome == "not-applicable"
        assert report.enforced_evaluated == 0

    def test_platform_predicate(self) -> None:
        budgets = budgets_from(ENFORCED_ROW + 'platform = "linux-x86_64"')
        metrics = {"imports.version.modules": Metric(300, "count", 1)}
        hit = evaluate(budgets, make_results(metrics))
        assert hit.rows[0].outcome == "passed"
        other = make_results(metrics, meta=make_meta(platform_key="darwin-aarch64"))
        assert evaluate(budgets, other).rows[0].outcome == "not-applicable"

    def test_empty_platform_key_is_unevaluable(self) -> None:
        budgets = budgets_from(ENFORCED_ROW + 'platform = "linux-x86_64"')
        broken = make_results(
            {"imports.version.modules": Metric(300, "count", 1)},
            meta=make_meta(platform_key=""),
        )
        report = evaluate(budgets, broken)
        assert report.rows[0].outcome == "predicate-unevaluable"
        assert report.failures

    def test_empty_ci_runner_is_unevaluable(self) -> None:
        budgets = budgets_from(ENFORCED_ROW)
        report = evaluate(
            budgets,
            make_results(
                {"imports.version.modules": Metric(300, "count", 1)},
                meta=make_meta(ci_runner=""),
            ),
        )
        assert report.rows[0].outcome == "predicate-unevaluable"

    def test_ci_only_row_not_applicable_locally(self) -> None:
        budgets = budgets_from(ENFORCED_ROW + "ci_only = true")
        # The context-python check also matches (3.13 vs 3.13), so ci_only is what
        # decides here: null ci_runner is an EVALUABLE "not CI", never a failure.
        local = make_results(
            {"imports.version.modules": Metric(300, "count", 1)}, meta=make_meta(ci_runner=None)
        )
        report = evaluate(budgets, local)
        assert report.rows[0].outcome == "not-applicable"
        assert not report.failures

    def test_python_mismatch_fails_on_ci_only(self) -> None:
        budgets = budgets_from(ENFORCED_ROW)
        on_ci = make_results(
            {"imports.version.modules": Metric(300, "count", 1)},
            meta=make_meta(python="3.14.2"),
        )
        report = evaluate(budgets, on_ci)
        assert report.rows[0].outcome == "python-mismatch"
        assert report.failures
        local = make_results(
            {"imports.version.modules": Metric(300, "count", 1)},
            meta=make_meta(python="3.14.2", ci_runner=None),
        )
        row = evaluate(budgets, local).rows[0]
        assert row.outcome == "not-applicable"
        assert "3.14" in row.detail

    def test_stale_ceiling_warns_on_ratchet_rows_only(self) -> None:
        ratchet = evaluate(
            budgets_from(ENFORCED_ROW),
            make_results({"imports.version.modules": Metric(100, "count", 1)}),
        )
        assert ratchet.rows[0].stale
        hand_set = budgets_from(
            "[[budget]]\nmetric = 'footprint.wheel_bytes'\nmax = 1048576\n"
            "tier = 'enforced'\ncontext = { commit = 'abc' }"
        )
        report = evaluate(
            hand_set, make_results({"footprint.wheel_bytes": Metric(400000, "bytes", 1)})
        )
        assert not report.rows[0].stale

    def test_render_report_tally_and_stale_nudge(self) -> None:
        budgets = budgets_from(ENFORCED_ROW)
        text = render_report(
            evaluate(budgets, make_results({"imports.version.modules": Metric(100, "count", 1)}))
        )
        assert "enforced: 1 rows, 1 evaluated, 1 passed, 0 failed" in text
        assert "ceiling is stale" in text

    def test_enforced_evaluated_counts_verdicts_not_na(self) -> None:
        budgets = budgets_from(ENFORCED_ROW + 'profiles = ["full"]') + budgets_from(ENFORCED_ROW)
        report = evaluate(
            budgets, make_results({"imports.version.modules": Metric(300, "count", 1)})
        )
        assert report.enforced_evaluated == 1


class TestBudgetPropose:
    def test_refreshes_ratchet_rows_only(self) -> None:
        toml_text = (
            ENFORCED_ROW
            + """
[[budget]]
metric = "footprint.wheel_bytes"
max = 1048576
tier = "enforced"
context = { commit = "old" }

[[budget]]
metric = "startup.version.over_python_ms"
max = 75
tier = "target"
"""
        )
        results = make_results(
            {
                "imports.version.modules": Metric(291, "count", 1),
                "footprint.wheel_bytes": Metric(462000, "bytes", 1),
            }
        )
        refreshed = load_budgets(propose(load_budgets(toml_text), results))
        by_metric = {b.metric: b for b in refreshed}
        ratchet = by_metric["imports.version.modules"]
        assert ratchet.max_value == 321  # ceil(291 * 1.10)
        assert ratchet.context == {
            "python": "3.13",
            "commit": "abcdef1234567890",
            "date": "2026-07-20",
        }
        assert by_metric["footprint.wheel_bytes"].max_value == 1048576
        assert by_metric["footprint.wheel_bytes"].context == {"commit": "old"}
        assert by_metric["startup.version.over_python_ms"].max_value == 75

    def test_propose_requires_the_metric(self) -> None:
        with pytest.raises(BudgetsError, match="cannot propose"):
            propose(load_budgets(ENFORCED_ROW), make_results({}))

    def test_render_budgets_round_trips(self) -> None:
        budgets = budgets_from(
            (REPO_ROOT / "benchmarks" / "budgets.toml").read_text(encoding="utf-8")
        )
        assert load_budgets(render_budgets(budgets)) == budgets


# ================================================================ parsers


class TestParsers:
    def test_stats(self) -> None:
        assert parsers.median([3.0, 1.0, 2.0]) == 2.0
        assert parsers.median([1.0, 2.0, 3.0, 4.0]) == 2.5
        assert parsers.p95(list(map(float, range(1, 101)))) == 95.0
        assert parsers.p95([5.0]) == 5.0
        assert parsers.stddev([2.0, 4.0]) == pytest.approx(1.4142, rel=1e-3)
        assert parsers.stddev([7.0]) == 0.0
        for fn in (parsers.median, parsers.p95, parsers.stddev):
            with pytest.raises(parsers.ParseError):
                fn([])

    def test_census(self) -> None:
        result = parsers.census(
            ["os", "sys", "typer", "typer.main", "rich.console", "tree_sitter_bash"]
        )
        assert result.modules == 6
        assert result.has_typer
        assert result.has_rich
        assert result.has_tree_sitter
        assert not result.has_textual
        # Prefix matching is per-segment: "richard" is not rich.
        assert not parsers.census(["richard", "typerlike"]).has_rich
        assert not parsers.census(["richard", "typerlike"]).has_typer

    def test_importtime(self) -> None:
        stderr = (
            "import time: self [us] | cumulative | imported package\n"
            "import time:       50 |         50 |   _io\n"
            "import time:      100 |       9000 | skit.cli\n"
            "import time:      200 |       8000 |     typer\n"
            "noise line\n"
        )
        top = parsers.importtime_top(stderr, top=2)
        assert [t.module for t in top] == ["skit.cli", "typer"]
        assert top[0].cumulative_us == 9000
        assert parsers.importtime_top("nothing here") == []

    def test_vmhwm(self) -> None:
        assert parsers.vmhwm_kib("VmPeak: 1 kB\nVmHWM:   4321 kB\n") == 4321
        with pytest.raises(parsers.ParseError):
            parsers.vmhwm_kib("VmPeak: 1 kB\n")
        with pytest.raises(parsers.ParseError):
            parsers.vmhwm_kib("VmHWM: broken\n")

    def test_maxrss(self) -> None:
        assert parsers.maxrss_kib(2048, "Linux") == 2048
        assert parsers.maxrss_kib(2048 * 1024, "Darwin") == 2048
        with pytest.raises(parsers.ParseError):
            parsers.maxrss_kib(-1, "Linux")

    def test_strace(self) -> None:
        table = (
            "% time     seconds  usecs/call     calls    errors syscall\n"
            "------ ----------- ----------- --------- --------- ----------------\n"
            " 50.00    0.001000           2       500           openat\n"
            " 30.00    0.000600           1       600        12 read\n"
            " 10.00    0.000200           1       200           newfstatat\n"
            "  5.00    0.000100           1         2           socket\n"
            "  5.00    0.000100           1         1           connect\n"
            "------ ----------- ----------- --------- --------- ----------------\n"
            "100.00    0.002000                  1303        12 total\n"
        )
        counts = parsers.strace_counts(table)
        assert counts["openat"] == 500
        assert parsers.count_group(counts, parsers.FILE_OP_SYSCALLS) == 1300
        assert parsers.count_group(counts, parsers.NETWORK_SYSCALLS) == 3
        with pytest.raises(parsers.ParseError):
            parsers.strace_counts("no rows at all")

    def test_pyperf(self) -> None:
        doc = {
            "benchmarks": [
                {
                    "metadata": {"name": "store.list_entries.n100"},
                    "runs": [{"warmups": [[1, 0.1]]}, {"values": [0.001, 0.002]}],
                }
            ]
        }
        benches = parsers.pyperf_benchmarks(json.dumps(doc))
        assert benches[0].name == "store.list_entries.n100"
        assert benches[0].values_s == [0.001, 0.002]
        doc_shared_name = {
            "metadata": {"name": "solo"},
            "benchmarks": [{"runs": [{"values": [0.5]}]}],
        }
        assert parsers.pyperf_benchmarks(json.dumps(doc_shared_name))[0].name == "solo"
        with pytest.raises(parsers.ParseError, match="not JSON"):
            parsers.pyperf_benchmarks("{")
        with pytest.raises(parsers.ParseError, match="no benchmarks"):
            parsers.pyperf_benchmarks("{}")
        with pytest.raises(parsers.ParseError, match="no measured values"):
            parsers.pyperf_benchmarks(
                json.dumps({"benchmarks": [{"metadata": {"name": "x"}, "runs": []}]})
            )
        with pytest.raises(parsers.ParseError, match="no name"):
            parsers.pyperf_benchmarks(json.dumps({"benchmarks": [{"runs": [{"values": [1]}]}]}))


# ================================================================ hyperfine


class TestHyperfine:
    def test_build_argv(self) -> None:
        argv = hyperfine.build_argv(
            [hyperfine.Case("a", ("skit", "--version")), hyperfine.Case("b", ("skit", "list"))],
            warmup=3,
            min_runs=15,
            export_json="/tmp/x.json",
        )
        assert argv[0] == "hyperfine"
        assert "--shell=none" in argv
        assert argv[argv.index("--command-name") + 1] == "a"
        assert "skit --version" in argv
        with pytest.raises(ValueError, match="no cases"):
            hyperfine.build_argv([], warmup=1, min_runs=1, export_json="x")

    def test_build_argv_quotes_awkward_paths(self) -> None:
        argv = hyperfine.build_argv(
            [hyperfine.Case("a", ("/bin/echo", "a b"))], warmup=1, min_runs=1, export_json="x"
        )
        assert "/bin/echo 'a b'" in argv

    def test_parse_export(self) -> None:
        doc = {
            "results": [
                {"command": "a", "times": [0.1, 0.2, 0.3], "exit_codes": [0, 0, 0]},
            ]
        }
        assert hyperfine.parse_export(json.dumps(doc)) == {"a": [0.1, 0.2, 0.3]}
        doc["results"][0]["exit_codes"] = [0, 1, 0]
        with pytest.raises(parsers.ParseError, match="non-zero exit"):
            hyperfine.parse_export(json.dumps(doc))
        with pytest.raises(parsers.ParseError, match="no results"):
            hyperfine.parse_export("{}")
        with pytest.raises(parsers.ParseError, match="not JSON"):
            hyperfine.parse_export("{")
        with pytest.raises(parsers.ParseError, match="missing command/times"):
            hyperfine.parse_export(json.dumps({"results": [{"command": "a", "times": []}]}))

    def test_metric_from_times(self) -> None:
        metric = hyperfine.metric_from_times([0.1, 0.2, 0.3])
        assert metric.value == pytest.approx(200.0)
        assert metric.unit == "ms"
        assert metric.n == 3
        assert metric.p95 == pytest.approx(300.0)


# ================================================================ compare


class TestCompare:
    def test_thresholds(self) -> None:
        base = make_results(
            {
                "startup.version.median_ms": Metric(200.0, "ms", 15),
                "small.wiggle_ms": Metric(10.0, "ms", 15),
                "imports.version.modules": Metric(100, "count", 1),
                "gone.metric": Metric(1, "count", 1),
            }
        )
        head = make_results(
            {
                "startup.version.median_ms": Metric(230.0, "ms", 15),  # +15%, > 2ms → notable
                "small.wiggle_ms": Metric(11.0, "ms", 15),  # +10% but ≤ 2ms floor → noise
                "imports.version.modules": Metric(104, "count", 1),  # +4% < 5% → noise
                "new.metric": Metric(1, "count", 1),
            }
        )
        comparison = bcompare.compare(base, head)
        assert [d.metric for d in comparison.notable] == ["startup.version.median_ms"]
        assert comparison.only_base == ["gone.metric"]
        assert comparison.only_head == ["new.metric"]

    def test_zero_base(self) -> None:
        base = make_results({"c.count": Metric(0, "count", 1)})
        grown = make_results({"c.count": Metric(3, "count", 1)})
        delta = bcompare.compare(base, grown).deltas[0]
        assert delta.pct is None
        assert delta.notable
        same = bcompare.compare(base, base).deltas[0]
        assert not same.notable

    def test_render(self) -> None:
        base = make_results({"a.ms": Metric(100.0, "ms", 5), "b.ms": Metric(10.0, "ms", 5)})
        head = make_results({"a.ms": Metric(200.0, "ms", 5), "b.ms": Metric(10.5, "ms", 5)})
        text = bcompare.render_markdown(base, head, bcompare.compare(base, head))
        assert "### Notable (1)" in text
        assert "Within noise" in text
        assert "`a.ms`" in text
        empty = bcompare.render_markdown(base, base, bcompare.compare(base, base))
        assert "### Notable (none)" in empty

    def test_render_only_in_sections(self) -> None:
        base = make_results(
            {"shared.ms": Metric(100.0, "ms", 5), "gone.metric": Metric(1, "count", 1)}
        )
        head = make_results(
            {"shared.ms": Metric(100.0, "ms", 5), "new.metric": Metric(2, "count", 1)}
        )
        text = bcompare.render_markdown(base, head, bcompare.compare(base, head))
        assert "### Only in base" in text
        assert "- `gone.metric`" in text
        assert "### Only in head" in text
        assert "- `new.metric`" in text

    def test_unit_mismatch_is_loud_and_never_mints_a_false_delta(self) -> None:
        base = make_results({"elapsed": Metric(1.0, "s", 5)})
        head = make_results({"elapsed": Metric(1000.0, "ms", 5)})
        comparison = bcompare.compare(base, head)
        assert comparison.deltas == []
        assert comparison.incomparable == ["unit elapsed: s vs ms"]
        text = bcompare.render_markdown(base, head, comparison)
        assert "not directly comparable" in text
        assert "+99900" not in text


# ================================================================ envinfo


class TestEnvinfo:
    def test_platform_key(self) -> None:
        assert envinfo.platform_key("Linux", "x86_64") == "linux-x86_64"
        assert envinfo.platform_key("Darwin", "arm64") == "darwin-aarch64"
        assert envinfo.platform_key("Linux", "AMD64") == "linux-x86_64"

    def test_ci_runner(self) -> None:
        assert envinfo.ci_runner({"BENCH_CI_RUNNER": "ubuntu-24.04"}) == "ubuntu-24.04"
        assert envinfo.ci_runner({"BENCH_CI_RUNNER": ""}) is None
        assert envinfo.ci_runner({}) is None
        assert envinfo.ci_image_version({"ImageVersion": "20260719.1"}) == "20260719.1"
        assert envinfo.ci_image_version({"ImageVersion": ""}) is None
        assert envinfo.ci_image_version({}) is None

    def test_cpu_model(self) -> None:
        text = "processor: 0\nmodel name\t: AMD Ryzen 9\nflags: x\n"
        assert envinfo.cpu_model(text, "x86_64") == "AMD Ryzen 9"
        assert envinfo.cpu_model("", "arm-fallback") == "arm-fallback"
        assert envinfo.cpu_model("model name :\n", "fb") == "fb"

    def test_mem_and_git(self) -> None:
        assert envinfo.mem_total_mib(4096, 1024 * 1024) == 4096
        assert envinfo.git_dirty(" M file\n")
        assert not envinfo.git_dirty("\n")

    def test_uv_version_from_output(self) -> None:
        assert envinfo.uv_version_from_output("uv 0.11.26 (abc 2026-01-01)") == "0.11.26"
        assert envinfo.uv_version_from_output("garbage") == "unknown"

    def test_dist_version_fallback(self) -> None:
        assert envinfo.dist_version("this-distribution-does-not-exist") == "unknown"
        assert envinfo.dist_version("pytest") != "unknown"

    def test_build_host_and_meta(self) -> None:
        host = envinfo.build_host(
            system="Linux",
            machine="x86_64",
            kernel="6.8.0",
            cpu="Test CPU",
            cpu_count=8,
            mem_mib=16384,
            env={
                "BENCH_CI_RUNNER": "ubuntu-24.04",
                "ImageVersion": "20260719.1",
            },
        )
        assert host.platform_key == "linux-x86_64"
        meta = envinfo.build_meta(
            profile="pr",
            generated_at="2026-07-20T00:00:00+00:00",
            commit="abc",
            dirty=False,
            host=host,
            python_version="3.13.5",
            uv_version="0.11.26",
            skit_version="0.2.1.dev0",
            textual_version="8.2.8",
            pyperf_version="2.10.0",
        )
        assert meta.host is host
        assert meta.profile == "pr"
        assert meta.host.ci_image_version == "20260719.1"
        assert meta.pyperf == "2.10.0"


# ================================================================ pipeline


class TestPipeline:
    def test_profiles(self) -> None:
        pr = pipeline.build_plan("pr")
        full = pipeline.build_plan("full")
        cmp_plan = pipeline.build_plan("compare")
        assert {p.suite for p in pr} == {
            "imports",
            "footprint",
            "rss",
            "startup",
            "scale",
            "run_overhead",
            "micro",
            "tui",
        }
        assert "syscalls" in {p.suite for p in full}
        # compare must NOT build the wheel: footprint would measure the harness ref.
        assert "footprint" not in {p.suite for p in cmp_plan}
        assert not next(p for p in pr if p.suite == "run_overhead").js_lane
        assert next(p for p in full if p.suite == "run_overhead").js_lane
        assert next(p for p in full if p.suite == "scale").ns == (0, 10, 100, 1000)
        with pytest.raises(pipeline.PipelineError, match="unknown profile"):
            pipeline.build_plan("nightly")

    def test_dataset_ns(self) -> None:
        assert pipeline.dataset_ns(pipeline.build_plan("pr")) == (0, 100, 1000)
        assert pipeline.dataset_ns(pipeline.build_plan("full")) == (0, 10, 100, 1000)

    def test_merge_and_derive(self) -> None:
        outputs = [
            SuiteOutput(
                suite="startup",
                metrics={
                    "startup.python.median_ms": Metric(35.0, "ms", 15),
                    "startup.version.median_ms": Metric(218.0, "ms", 15),
                },
                duration_s=10.0,
            ),
            SuiteOutput(
                suite="scale",
                metrics={
                    "scale.list_json.n0.median_ms": Metric(220.0, "ms", 15),
                    "scale.list_json.n1000.median_ms": Metric(720.0, "ms", 15),
                },
                skipped=[Skip(suite="scale", case="x", reason="y")],
            ),
        ]
        results = pipeline.merge(make_meta(), outputs, total_duration_s=100.0)
        assert results.metrics["startup.version.over_python_ms"].value == pytest.approx(183.0)
        assert results.metrics["scale.list_json.per_entry_us"].value == pytest.approx(500.0)
        assert results.metrics["pipeline.skipped_count"].value == 1
        assert results.metrics["pipeline.duration_s"].value == 100.0
        assert results.metrics["pipeline.suite.startup.duration_s"].value == 10.0
        # Partial inputs: no derived metric, so a budgeted derivation goes
        # metric-missing instead of being faked from half the data.
        partial = pipeline.merge(make_meta(), [outputs[0]], total_duration_s=1.0)
        assert "scale.list_json.per_entry_us" not in partial.metrics

    def test_merge_rejects_duplicate_ids(self) -> None:
        dup = SuiteOutput(suite="a", metrics={"m.x": Metric(1, "ms", 1)})
        dup2 = SuiteOutput(suite="b", metrics={"m.x": Metric(2, "ms", 1)})
        with pytest.raises(pipeline.PipelineError, match="duplicate metric id"):
            pipeline.merge(make_meta(), [dup, dup2], total_duration_s=1.0)

    @pytest.mark.parametrize(
        "metric_id",
        [
            "pipeline.duration_s",
            "pipeline.skipped_count",
            "pipeline.suite.demo.duration_s",
        ],
    )
    def test_merge_rejects_reserved_pipeline_ids(self, metric_id: str) -> None:
        output = SuiteOutput(suite="demo", metrics={metric_id: Metric(999, "s", 1)})
        with pytest.raises(pipeline.PipelineError, match="reserved pipeline metric id"):
            pipeline.merge(make_meta(), [output], total_duration_s=1.0)

    def test_merge_rejects_derived_collision(self) -> None:
        clash = SuiteOutput(
            suite="a",
            metrics={
                "startup.python.median_ms": Metric(35.0, "ms", 1),
                "startup.version.median_ms": Metric(218.0, "ms", 1),
                "startup.version.over_python_ms": Metric(1.0, "ms", 1),
            },
        )
        with pytest.raises(pipeline.PipelineError, match="already present"):
            pipeline.merge(make_meta(), [clash], total_duration_s=1.0)

    def test_render_markdown(self) -> None:
        results = make_results(
            {
                "startup.version.median_ms": Metric(218.0, "ms", 15, p95=230.0),
                "imports.version.modules": Metric(291, "count", 1),
            },
            skipped=[Skip(suite="run_overhead", case="js", reason="node not found")],
        )
        report = evaluate(load_budgets(ENFORCED_ROW), results)
        text = pipeline.render_markdown(results, report)
        assert "`startup.version.median_ms` | 218 ms | 230 | 15" in text
        assert "run_overhead/js" in text
        assert "### Budgets" in text
        clean = pipeline.render_markdown(make_results({}))
        assert "No skipped cases." in clean

    def test_export_gha(self) -> None:
        results = make_results(
            {
                "startup.version.median_ms": Metric(218.0, "ms", 15),
                "not.a.headline": Metric(1.0, "ms", 1),
            }
        )
        rows = pipeline.export_gha(results)
        assert rows == [{"name": "startup.version.median_ms", "unit": "ms", "value": 218.0}]
        with pytest.raises(pipeline.PipelineError, match="no headline metrics"):
            pipeline.export_gha(make_results({}))

    def test_summarize_dir(self, tmp_path: Path) -> None:
        bench = tmp_path / "bench"
        (bench / "suites").mkdir(parents=True)
        meta = make_meta()
        import dataclasses

        (bench / "run.json").write_text(
            json.dumps({"meta": dataclasses.asdict(meta), "total_duration_s": 12.5})
        )
        output = SuiteOutput(
            suite="startup",
            metrics={
                "startup.python.median_ms": Metric(35.0, "ms", 15),
                "startup.version.median_ms": Metric(218.0, "ms", 15),
            },
        )
        (bench / "suites" / "startup.json").write_text(output.to_json())
        results = pipeline.summarize_dir(bench)
        assert (bench / "results.json").exists()
        assert (bench / "results.md").exists()
        assert results.metrics["pipeline.duration_s"].value == 12.5
        again = Results.from_json((bench / "results.json").read_text())
        assert again == results

    def test_summarize_dir_failures(self, tmp_path: Path) -> None:
        with pytest.raises(pipeline.PipelineError, match=r"no run\.json"):
            pipeline.summarize_dir(tmp_path)
        import dataclasses

        (tmp_path / "run.json").write_text(
            json.dumps({"meta": dataclasses.asdict(make_meta()), "total_duration_s": "slow"})
        )
        with pytest.raises(pipeline.PipelineError, match="total_duration_s"):
            pipeline.summarize_dir(tmp_path)
        (tmp_path / "run.json").write_text(
            json.dumps({"meta": dataclasses.asdict(make_meta()), "total_duration_s": 1.0})
        )
        (tmp_path / "suites").mkdir()
        with pytest.raises(pipeline.PipelineError, match="no suite outputs"):
            pipeline.summarize_dir(tmp_path)


# ================================================================ fixture sources


class TestSources:
    @pytest.mark.parametrize("lang", sources.LANGS)
    @pytest.mark.parametrize("lines", [20, 200])
    def test_exact_line_counts(self, lang: str, lines: int) -> None:
        text = sources.generate(lang, lines)
        assert len(text.splitlines()) == lines
        assert text.endswith("\n")

    def test_deterministic(self) -> None:
        assert sources.generate("shell", 40) == sources.generate("shell", 40)
        assert sources.generate("shell", 40, seed=1) != sources.generate("shell", 40, seed=2)

    def test_rejects_bad_inputs(self) -> None:
        with pytest.raises(ValueError, match="unknown language"):
            sources.generate("cobol", 20)
        with pytest.raises(ValueError, match="at least 8"):
            sources.generate("shell", 4)

    def test_analyzer_constructs_present(self) -> None:
        assert "argparse" in sources.generate("python", 20)
        assert ":-" in sources.generate("shell", 20)  # env-default idiom
        assert "process.argv" in sources.generate("js", 20)
        assert ": number" in sources.generate("ts", 20)

    def test_generate_asserts_generator_line_count(self, monkeypatch: pytest.MonkeyPatch) -> None:
        # A generator that lies about its line count must be caught, not silently
        # emitted — the analyzer cost curves are plotted against the requested count.
        monkeypatch.setattr(sources, "_shell", lambda lines, rng: ["one", "two"])
        with pytest.raises(AssertionError, match="generator produced 2 lines, wanted 20"):
            sources.generate("shell", 20)


# ================================================================ datasets


class TestDatasets:
    def test_generate_small_library(self, tmp_path: Path) -> None:
        manifest = generate(tmp_path / "ds", 30)
        assert manifest.n == 30
        assert len(manifest.slugs) == 30
        assert len(set(manifest.slugs)) == 30
        # The store agrees (post-generate self-check ran), through the same env vars
        # any suite would use.
        with datasets.scoped_skit_dirs(manifest.root):
            from skit import store

            entries = {e.slug: e for e in store.list_entries()}
            assert set(entries) == set(manifest.slugs)
            # The search-probe invariant: entry 0 never matches the probe char.
            from benchmarks.datasets import SEARCH_PROBE_CHAR

            first = entries[manifest.slugs[0]]
            assert SEARCH_PROBE_CHAR not in f"{first.meta.name} {first.meta.description}"

    def test_generate_is_deterministic(self, tmp_path: Path) -> None:
        a = generate(tmp_path / "a", 15)
        b = generate(tmp_path / "b", 15)
        assert a.slugs == b.slugs
        assert a.kinds == b.kinds
        c = generate(tmp_path / "c", 15, seed=999)
        assert c.slugs != a.slugs

    def test_generate_env_is_restored(self, tmp_path: Path) -> None:
        before = {
            k: os.environ.get(k) for k in ("SKIT_DATA_DIR", "SKIT_STATE_DIR", "SKIT_CONFIG_DIR")
        }
        generate(tmp_path / "ds", 3)
        after = {
            k: os.environ.get(k) for k in ("SKIT_DATA_DIR", "SKIT_STATE_DIR", "SKIT_CONFIG_DIR")
        }
        assert before == after

    def test_kind_mix_and_missing_targets_at_100(self, tmp_path: Path) -> None:
        manifest = generate(tmp_path / "ds", 100)
        kinds = list(manifest.kinds.values())
        assert kinds.count("python") == 30
        assert kinds.count("shell") == 20
        assert kinds.count("prompt") == 10
        assert kinds.count("exe") == 6
        # Long tail present: one each of ruby/perl/lua/r.
        assert {"ruby", "perl", "lua", "r"} <= set(kinds)
        with datasets.scoped_skit_dirs(manifest.root):
            from skit import launcher, store

            entries = store.list_entries()
            missing = [e for e in entries if launcher.target_missing(e)]
            assert missing, "every 10th reference entry's target is deliberately deleted"

    @pytest.mark.parametrize(("fraction", "expect_state"), [(0.0, False), (1.0, True)])
    def test_state_fraction(self, tmp_path: Path, fraction: float, expect_state: bool) -> None:
        manifest = generate(tmp_path / "ds", 8, state_fraction=fraction)
        with datasets.scoped_skit_dirs(manifest.root):
            from skit import argstate

            with_state = [
                slug for slug in manifest.slugs if argstate.load_state(slug).get("last_run")
            ]
            assert bool(with_state) is expect_state
            if expect_state:
                assert len(with_state) == manifest.n

    def test_refuses_non_empty_root(self, tmp_path: Path) -> None:
        root = tmp_path / "ds"
        root.mkdir()
        (root / "junk").write_text("x")
        with pytest.raises(DatasetError, match="refusing"):
            generate(root, 3)

    def test_rejects_bad_inputs(self, tmp_path: Path) -> None:
        with pytest.raises(DatasetError, match="n must be"):
            generate(tmp_path / "a", -1)
        with pytest.raises(DatasetError, match="state_fraction"):
            generate(tmp_path / "b", 1, state_fraction=1.5)

    def test_manifest_round_trip_and_mid_slug(self, tmp_path: Path) -> None:
        manifest = generate(tmp_path / "ds", 9)
        loaded = Manifest.load(manifest.root)
        assert loaded.slugs == manifest.slugs
        assert loaded.mid_slug == manifest.slugs[4]
        empty = generate(tmp_path / "empty", 0)
        with pytest.raises(DatasetError, match="no mid entry"):
            _ = empty.mid_slug

    def test_runover_library(self, tmp_path: Path) -> None:
        manifest = generate_runover(tmp_path / "ro", FIXTURES_DIR)
        assert manifest.n == 3
        assert set(manifest.kinds.values()) == {"python", "shell", "js"}
        with datasets.scoped_skit_dirs(manifest.root):
            from skit import store

            names = {e.meta.name for e in store.list_entries()}
            assert names == {"noop-py", "noop-sh", "noop-js"}

    def test_scoped_skit_dirs_restores_previously_unset_var(
        self, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
    ) -> None:
        # When a SKIT_* var was unset before the scope, leaving it must remove it again
        # (pop), not resurrect it with an empty/stale value.
        monkeypatch.delenv("SKIT_DATA_DIR", raising=False)
        want = skit_dirs(tmp_path)
        with datasets.scoped_skit_dirs(tmp_path):
            assert os.environ["SKIT_DATA_DIR"] == want["SKIT_DATA_DIR"]
        assert "SKIT_DATA_DIR" not in os.environ

    def test_source_text_rejects_unknown_kind(self) -> None:
        with pytest.raises(DatasetError, match="no source template for kind 'cobol'"):
            datasets._source_text("cobol", 0)

    def test_generate_refuses_silent_store_undercount(
        self, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
    ) -> None:
        # The generator self-checks the store's own count instead of trusting it: if the
        # store reports the wrong number of entries, generation fails loudly.
        from skit import store

        monkeypatch.setattr(store, "list_entries", lambda: [])
        with pytest.raises(DatasetError, match="generated 0 entries, expected 3"):
            generate(tmp_path / "ds", 3)

    def test_runover_refuses_silent_store_undercount(
        self, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
    ) -> None:
        from skit import store

        monkeypatch.setattr(store, "list_entries", lambda: [])
        with pytest.raises(DatasetError, match="runover library has 0 entries, expected 3"):
            generate_runover(tmp_path / "ro", FIXTURES_DIR)


# ================================================================ front door


class TestFrontDoor:
    def test_check_exit_codes(self, tmp_path: Path) -> None:
        from benchmarks.__main__ import main

        budgets_path = tmp_path / "budgets.toml"
        budgets_path.write_text(ENFORCED_ROW)
        good = tmp_path / "good.json"
        good.write_text(
            make_results({"imports.version.modules": Metric(300, "count", 1)}).to_json()
        )
        assert main(["check", str(good), "--budgets", str(budgets_path)]) == 0
        bad = tmp_path / "bad.json"
        bad.write_text(make_results({"imports.version.modules": Metric(999, "count", 1)}).to_json())
        assert main(["check", str(bad), "--budgets", str(budgets_path)]) == 1

    def test_check_require_enforced(self, tmp_path: Path) -> None:
        from benchmarks.__main__ import main

        budgets_path = tmp_path / "budgets.toml"
        budgets_path.write_text(ENFORCED_ROW + 'profiles = ["full"]')
        results_path = tmp_path / "r.json"
        results_path.write_text(
            make_results({"imports.version.modules": Metric(300, "count", 1)}).to_json()
        )
        assert main(["check", str(results_path), "--budgets", str(budgets_path)]) == 0
        assert (
            main(["check", str(results_path), "--budgets", str(budgets_path), "--require-enforced"])
            == 1
        )

    def test_check_propose_prints_toml(
        self, tmp_path: Path, capsys: pytest.CaptureFixture[str]
    ) -> None:
        from benchmarks.__main__ import main

        budgets_path = tmp_path / "budgets.toml"
        budgets_path.write_text(ENFORCED_ROW)
        results_path = tmp_path / "r.json"
        results_path.write_text(
            make_results({"imports.version.modules": Metric(291, "count", 1)}).to_json()
        )
        assert main(["check", str(results_path), "--budgets", str(budgets_path), "--propose"]) == 0
        out = capsys.readouterr().out
        assert load_budgets(out)[0].max_value == 321

    def test_export_gha_writes_file(self, tmp_path: Path) -> None:
        from benchmarks.__main__ import main

        results_path = tmp_path / "r.json"
        results_path.write_text(
            make_results({"startup.version.median_ms": Metric(218.0, "ms", 15)}).to_json()
        )
        out = tmp_path / "gha.json"
        assert main(["export-gha", str(results_path), "--out", str(out)]) == 0
        rows = json.loads(out.read_text())
        assert rows[0]["name"] == "startup.version.median_ms"

    def test_compare_command(self, tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> None:
        from benchmarks.__main__ import main

        a = tmp_path / "a.json"
        b = tmp_path / "b.json"
        a.write_text(make_results({"x.ms": Metric(100.0, "ms", 5)}).to_json())
        b.write_text(make_results({"x.ms": Metric(300.0, "ms", 5)}).to_json())
        assert main(["compare", str(a), str(b)]) == 0
        assert "Notable (1)" in capsys.readouterr().out


# ================================================================ contract sync
# The repo's precedent (tests/test_agent_skill.py) is that duplicated artifacts get
# byte-level sync enforcement — free-floating copies drift and then lie.


class TestContractSync:
    def test_budgets_file_is_canonical(self) -> None:
        """The committed contract must be exactly what its own regenerator emits, or
        the first `check --propose` diff drowns the value change in format churn."""
        text = (REPO_ROOT / "benchmarks" / "budgets.toml").read_text(encoding="utf-8")
        assert render_budgets(load_budgets(text)) == text

    def test_hyperfine_pin_synced_to_install_action(self) -> None:
        """hyperfine.py's pinned version + sha256 + URL are the single source of
        truth; the composite install action must carry the same pin."""
        action = (REPO_ROOT / ".github" / "actions" / "install-hyperfine" / "action.yml").read_text(
            encoding="utf-8"
        )
        assert hyperfine.HYPERFINE_URL in action
        assert hyperfine.HYPERFINE_SHA256 in action
        assert f"hyperfine-v{hyperfine.HYPERFINE_VERSION}-x86_64" in action

    @pytest.mark.parametrize(
        "workflow",
        ["benchmark.yml", "benchmark-nightly.yml", "benchmark-compare.yml"],
    )
    def test_workflows_install_hyperfine_via_the_action(self, workflow: str) -> None:
        """One install block, not three drifting copies: every bench workflow must go
        through the composite action."""
        text = (REPO_ROOT / ".github" / "workflows" / workflow).read_text(encoding="utf-8")
        assert "uses: ./.github/actions/install-hyperfine" in text

    def test_compare_workflow_pins_pyperf_to_the_harness_lock(self) -> None:
        workflow = (REPO_ROOT / ".github" / "workflows" / "benchmark-compare.yml").read_text(
            encoding="utf-8"
        )
        # This environment was created from uv.lock. Comparing the workflow to the
        # resolved distribution keeps the test runnable in mutmut's isolated tree,
        # which deliberately contains source + workflows but not the repository lock.
        assert workflow.count(f'pyperf=={version("pyperf")}"') == 2

    @pytest.mark.parametrize(
        "workflow",
        ["benchmark.yml", "benchmark-nightly.yml", "benchmark-compare.yml"],
    )
    def test_ci_runner_label_matches_runs_on(self, workflow: str) -> None:
        """BENCH_CI_RUNNER (what envinfo stamps into results, what budget predicates
        match) must equal the runner the job actually runs on — a drifted label makes
        meta.host.ci_runner lie to the enforced tier."""
        import re

        text = (REPO_ROOT / ".github" / "workflows" / workflow).read_text(encoding="utf-8")
        runs_on = set(re.findall(r"runs-on:\s*(\S+)", text))
        declared = set(re.findall(r"BENCH_CI_RUNNER:\s*(\S+)", text))
        assert declared, f"{workflow} must export BENCH_CI_RUNNER"
        assert runs_on == declared


# ================================================================ envspec


class TestEnvspec:
    def test_build_env_composes_path_and_pins_locale(self, tmp_path: Path) -> None:
        from benchmarks import envspec

        manifest = generate(tmp_path / "ds", 3)
        env = envspec.build_env(
            skit="/venv/bin/skit",
            uv="/tools/uv/uv",
            node="/usr/bin/node",
            workdir=tmp_path / "work",
            dataset_root=manifest.root,
        )
        assert env["PATH"] == "/venv/bin:/tools/uv:/usr/bin:/bin"
        assert env["SKIT_DATA_DIR"] == str((tmp_path / "ds" / "data").resolve())
        assert env["SKIT_LANG"] == "en"
        assert env["TERM"] == "dumb"
        assert env["UV_CACHE_DIR"] == str(tmp_path / "work" / "uv-cache")
        assert (tmp_path / "work" / "home").is_dir()
        # Constructed, never inherited: no ambient variable can leak in.
        assert "PYTHONPATH" not in env
        assert "FORCE_COLOR" not in env

    def test_build_env_dedupes_and_tolerates_missing_tools(self, tmp_path: Path) -> None:
        from benchmarks import envspec

        # The empty library is the generated, manifest-verified n=0 dataset — there
        # is no unverified special case.
        manifest = generate(tmp_path / "n0", 0)
        env = envspec.build_env(
            skit="/usr/bin/skit",
            uv=None,
            node=None,
            workdir=tmp_path / "work",
            dataset_root=manifest.root,
        )
        assert env["PATH"] == "/usr/bin:/bin"
        assert env["SKIT_DATA_DIR"] == str((tmp_path / "n0" / "data").resolve())

    def test_build_env_refuses_non_dataset_roots(self, tmp_path: Path) -> None:
        from benchmarks import envspec

        bogus = tmp_path / "not-a-dataset"
        bogus.mkdir()
        with pytest.raises(RuntimeError, match="not a generated dataset"):
            envspec.build_env(
                skit="/usr/bin/skit", uv=None, node=None, workdir=tmp_path, dataset_root=bogus
            )

    def test_pyperf_inherit_covers_the_fixture_vars(self) -> None:
        from benchmarks import envspec

        assert {"SKIT_DATA_DIR", "SKIT_STATE_DIR", "SKIT_CONFIG_DIR"} <= set(envspec.PYPERF_INHERIT)
        assert {"BENCH_N", "BENCH_SOURCES_DIR"} <= set(envspec.PYPERF_INHERIT)


# ================================================================ dataset reuse


class TestDatasetReuse:
    def test_check_reusable_accepts_a_fresh_dataset(self, tmp_path: Path) -> None:
        from benchmarks.datasets import check_reusable

        manifest = generate(tmp_path / "ds", 3)
        check_reusable(Manifest.load(manifest.root), 3)

    def test_check_reusable_rejects_any_drift(self, tmp_path: Path) -> None:
        import dataclasses

        from benchmarks.datasets import check_reusable

        manifest = generate(tmp_path / "ds", 3)
        with pytest.raises(DatasetError, match="different inputs"):
            check_reusable(manifest, 4)  # wrong n
        stale = dataclasses.replace(manifest, skit_version="0.0.0+other")
        with pytest.raises(DatasetError, match="different inputs"):
            check_reusable(stale, 3)  # written by a different skit (branch switch)

    def test_manifest_stamps_the_writing_skit(self, tmp_path: Path) -> None:
        import skit

        manifest = generate(tmp_path / "ds", 3)
        assert manifest.skit_version == skit.__version__
        assert Manifest.load(manifest.root).skit_version == skit.__version__


# ================================================================ source validity


class TestSourceValidity:
    """The analyzer corpus must PARSE — a truncated brace hands tree-sitter an
    error-recovery tree and silently under-reports analyze times ~5x (this
    happened; these tests pin it)."""

    @pytest.mark.parametrize("lang", ["js", "ts"])
    @pytest.mark.parametrize("lines", [20, 200, 2000])
    def test_js_ts_braces_balance(self, lang: str, lines: int) -> None:
        text = sources.generate(lang, lines)
        assert text.count("{") == text.count("}")

    @pytest.mark.parametrize("lines", [20, 200, 2000])
    def test_python_compiles(self, lines: int) -> None:
        compile(sources.generate("python", lines), "<bench-source>", "exec")

    @pytest.mark.parametrize(
        ("lang", "grammar"),
        [("js", "tree_sitter_javascript"), ("ts", "tree_sitter_typescript")],
    )
    def test_tree_sitter_parses_without_errors(self, lang: str, grammar: str) -> None:
        tree_sitter = pytest.importorskip("tree_sitter")
        grammar_mod = pytest.importorskip(grammar)
        language = tree_sitter.Language(
            grammar_mod.language() if lang == "js" else grammar_mod.language_typescript()
        )
        parser = tree_sitter.Parser(language)
        for lines in (20, 200, 2000):
            tree = parser.parse(sources.generate(lang, lines).encode())
            assert not tree.root_node.has_error, f"{lang} @ {lines} lines parses with errors"


# ================================================================ probe-char invariant


class TestSearchProbeInvariant:
    # n=3 is the documented boundary (the matching-entry guarantee starts at n >= 3
    # and the probe asserts survivors from there) — pin the boundary itself, not just
    # a comfortable larger n.
    @pytest.mark.parametrize("n", [3, 9])
    def test_dataset_guarantees_both_sides_of_the_filter_assertion(
        self, tmp_path: Path, n: int
    ) -> None:
        from benchmarks.datasets import SEARCH_PROBE_CHAR

        manifest = generate(tmp_path / "ds", n)
        with datasets.scoped_skit_dirs(manifest.root):
            from skit import store

            texts = [f"{e.meta.name} {e.meta.description}" for e in store.list_entries()]
            assert any(SEARCH_PROBE_CHAR not in t for t in texts), "a non-match must exist"
            assert any(SEARCH_PROBE_CHAR in t for t in texts), "a match must survive"


# ================================================================ round 1 review fixes


class TestReviewFixes:
    def test_compare_excludes_pipeline_self_timings(self) -> None:
        base = make_results(
            {
                "pipeline.duration_s": Metric(100.0, "s", 1),
                "pipeline.suite.tui.duration_s": Metric(10.0, "s", 1),
                "startup.version.median_ms": Metric(100.0, "ms", 5),
            }
        )
        head = make_results(
            {
                "pipeline.duration_s": Metric(300.0, "s", 1),
                "startup.version.median_ms": Metric(100.0, "ms", 5),
            }
        )
        comparison = bcompare.compare(base, head)
        assert [d.metric for d in comparison.deltas] == ["startup.version.median_ms"]
        assert comparison.only_base == []

    def test_budget_bounds_render_as_plain_numbers(self) -> None:
        budgets = budgets_from(
            "[[budget]]\nmetric = 'footprint.wheel_bytes'\nmax = 1048576\n"
            "tier = 'enforced'\ncontext = { commit = 'abc' }"
        )
        text = render_report(
            evaluate(budgets, make_results({"footprint.wheel_bytes": Metric(461803, "bytes", 1)}))
        )
        assert "1048576" in text
        assert "e+06" not in text

    def test_hyperfine_metrics_from_export_mints_ids(self) -> None:
        metrics = hyperfine.metrics_from_export({"scale.list.n100": [0.1, 0.2, 0.3]})
        assert set(metrics) == {"scale.list.n100.median_ms"}
        assert metrics["scale.list.n100.median_ms"].value == pytest.approx(200.0)

    def test_fractional_bounds_render_compactly(self) -> None:
        budgets = budgets_from("[[budget]]\nmetric = 'ratio'\nmax = 0.5\ntier = 'target'")
        text = render_report(evaluate(budgets, make_results({"ratio": Metric(0.25, "x", 1)})))
        assert "0.25 x ≤ 0.5" in text


class TestHarnessImportSurface:
    def test_benchmarks_imports_stay_on_runtime_deps(self) -> None:
        """benchmark-compare runs the harness on a side venv holding only skit's
        RUNTIME dependencies + pyperf — a dev-only import anywhere in benchmarks/
        would kill every A/B run at startup, discovered only on dispatch. Enforce the
        surface: stdlib, skit, benchmarks-internal, tomli_w (a skit runtime dep);
        pyperf only in micro/ scripts; textual only in the tui probe."""
        import ast
        import sys

        allowed_anywhere = set(sys.stdlib_module_names) | {"skit", "benchmarks", "tomli_w"}
        # micro scripts and the tui probe are re-exec'd/spawned by PATH from a scratch
        # cwd, where `import benchmarks` is unresolvable — their documented contract
        # is self-containment (skit + tool + stdlib), so hold them to it.
        self_contained = set(sys.stdlib_module_names) | {"skit"}
        per_file = {
            "micro": self_contained | {"pyperf"},
            "tui_probe.py": self_contained | {"textual"},
        }
        for path in sorted((REPO_ROOT / "benchmarks").rglob("*.py")):
            allowed = allowed_anywhere
            for key, names in per_file.items():
                if key in path.parts or path.name == key:
                    allowed = names
            tree = ast.parse(path.read_text(encoding="utf-8"))
            for node in ast.walk(tree):
                tops: list[str] = []
                if isinstance(node, ast.Import):
                    tops = [alias.name.split(".")[0] for alias in node.names]
                elif isinstance(node, ast.ImportFrom) and node.level == 0 and node.module:
                    tops = [node.module.split(".")[0]]
                for top in tops:
                    assert top in allowed, (
                        f"{path.relative_to(REPO_ROOT)} imports {top!r} — outside the "
                        "A/B harness surface (runtime deps + pyperf + stdlib)"
                    )


class TestCodeReviewFixes:
    """Behaviors added by the /code-review round (see the PR's review appendix)."""

    def test_rss_keeps_samples_and_full_statistics(self, tmp_path: Path, monkeypatch) -> None:
        ctx = cast(
            RunCtx,
            SimpleNamespace(
                skit="/bin/skit",
                datasets={0: SimpleNamespace(root=tmp_path)},
            ),
        )
        samples = iter([100, 120, 140, 200, 220, 240])
        monkeypatch.setattr(rss_suite, "bench_env", lambda _ctx, _root: {})
        monkeypatch.setattr(rss_suite, "_sample", lambda _ctx, _argv, _env: next(samples))
        output = rss_suite.run(ctx, pipeline.SuitePlan("rss", ns=(0,), samples=3))
        version = output.metrics["rss.version.peak_kib"]
        assert version == Metric(
            value=120.0,
            unit="KiB",
            n=3,
            p95=140.0,
            stddev=20.0,
        )
        assert output.raw["rss.version"]["samples_kib"] == [100, 120, 140]

    def test_tui_keeps_import_and_rss_samples(self, tmp_path: Path, monkeypatch) -> None:
        manifest = SimpleNamespace(root=tmp_path, probe_char="o")
        ctx = cast(
            RunCtx,
            SimpleNamespace(
                python="/bin/python",
                workdir=tmp_path,
                datasets={0: manifest},
            ),
        )
        calls = 0

        def fake_run(argv, **_kwargs):
            nonlocal calls
            calls += 1
            out_file = Path(argv[argv.index("--out") + 1])
            out_file.write_text(
                json.dumps(
                    {
                        "first_idle_ms": float(calls),
                        "search_ms": float(calls + 10),
                        "import_ms": float(calls + 20),
                        "status_text": f"VmHWM: {calls * 100} kB",
                    }
                )
            )
            return SimpleNamespace(returncode=0)

        monkeypatch.setattr(tui_suite, "bench_env", lambda _ctx, _root: {})
        monkeypatch.setattr(tui_suite.subprocess, "run", fake_run)
        original_exists = Path.exists
        monkeypatch.setattr(
            Path,
            "exists",
            lambda self: True if self == Path("/proc/self/status") else original_exists(self),
        )
        output = tui_suite.run(ctx, pipeline.SuitePlan("tui", ns=(0,), samples=3))
        assert output.raw["n0"] == {
            "first_idle_ms": [1.0, 2.0, 3.0],
            "search_ms": [11.0, 12.0, 13.0],
            "rss_kib": [100.0, 200.0, 300.0],
            "import_ms": [21.0, 22.0, 23.0],
        }
        assert output.metrics["tui.rss.n0.peak_kib"].p95 == 300.0
        assert output.metrics["tui.rss.n0.peak_kib"].stddev == 100.0

    def test_cold_parse_keeps_raw_samples(self, tmp_path: Path, monkeypatch) -> None:
        ctx = cast(
            RunCtx,
            SimpleNamespace(
                python="/bin/python",
                workdir=tmp_path,
                datasets={0: SimpleNamespace(root=tmp_path)},
            ),
        )
        values = iter(["1.0", "2.0", "3.0", "4.0", "5.0"])
        monkeypatch.setattr(micro_suite, "bench_env", lambda _ctx, _root: {})
        monkeypatch.setattr(
            micro_suite.subprocess,
            "run",
            lambda *_args, **_kwargs: SimpleNamespace(returncode=0, stdout=next(values), stderr=""),
        )
        output = SuiteOutput(suite="micro")
        micro_suite._cold_parse(
            ctx,
            pipeline.SuitePlan("micro"),
            "python",
            tmp_path,
            output,
        )
        assert output.raw["analyze_cold"]["python"]["samples_ms"] == [
            1.0,
            2.0,
            3.0,
            4.0,
            5.0,
        ]
        assert output.metrics["micro.analyze_cold.python.median_ms"].value == 3.0

    def test_micro_deltas_clear_the_us_floor(self) -> None:
        base = make_results({"micro.store.resolve.mid.n1000.median_us": Metric(100.0, "us", 40)})
        head = make_results({"micro.store.resolve.mid.n1000.median_us": Metric(300.0, "us", 40)})
        comparison = bcompare.compare(base, head)
        assert [d.metric for d in comparison.notable] == ["micro.store.resolve.mid.n1000.median_us"]
        # Sub-µs wiggle stays noise.
        near = make_results({"micro.store.resolve.mid.n1000.median_us": Metric(100.5, "us", 40)})
        assert bcompare.compare(base, near).notable == []

    def test_compare_flags_incomparable_sides(self) -> None:
        base = make_results({"x.ms": Metric(100.0, "ms", 5)}, meta=make_meta(profile="full"))
        head = make_results(
            {"x.ms": Metric(100.0, "ms", 5)},
            meta=make_meta(profile="pr", platform_key="darwin-aarch64", python="3.14.2"),
        )
        comparison = bcompare.compare(base, head)
        assert comparison.incomparable == [
            "profile: full vs pr",
            "platform: linux-x86_64 vs darwin-aarch64",
            "python: 3.13 vs 3.14",
        ]
        text = bcompare.render_markdown(base, head, comparison)
        assert "not directly comparable" in text
        matched = bcompare.compare(base, base)
        assert matched.incomparable == []
        assert "not directly comparable" not in bcompare.render_markdown(base, base, matched)

    def test_compare_flags_harness_provenance_changes(self) -> None:
        base = make_results(
            {"x.ms": Metric(100.0, "ms", 5)},
            meta=make_meta(ci_image_version="20260719.1", pyperf="2.10.0"),
        )
        head = make_results(
            {"x.ms": Metric(100.0, "ms", 5)},
            meta=make_meta(ci_image_version="20260726.1", pyperf="2.11.0"),
        )
        assert bcompare.compare(base, head).incomparable == [
            "runner image: 20260719.1 vs 20260726.1",
            "pyperf: 2.10.0 vs 2.11.0",
        ]

    def test_old_schema_defaults_missing_harness_provenance(self) -> None:
        doc = json.loads(make_results({}).to_json())
        del doc["meta"]["host"]["ci_image_version"]
        del doc["meta"]["pyperf"]
        restored = Results.from_json(json.dumps(doc))
        assert restored.meta.host.ci_image_version is None
        assert restored.meta.pyperf == "unknown"

    def test_results_rejects_invalid_harness_provenance(self) -> None:
        doc = json.loads(make_results({}).to_json())
        doc["meta"]["host"]["ci_image_version"] = 123
        with pytest.raises(ResultsError, match="ci_image_version"):
            Results.from_json(json.dumps(doc))
        doc["meta"]["host"]["ci_image_version"] = None
        doc["meta"]["pyperf"] = ""
        with pytest.raises(ResultsError, match=r"meta\.pyperf"):
            Results.from_json(json.dumps(doc))

    def test_budgets_reject_non_finite_bounds(self) -> None:
        with pytest.raises(BudgetsError, match="max must be finite"):
            budgets_from("[[budget]]\nmetric = 'm'\nmax = nan\ntier = 'target'")

    @pytest.mark.parametrize("bad", [float("inf"), float("nan")])
    def test_results_reject_non_finite_values(self, bad: float) -> None:
        doc = json.loads(make_results({"m.x": Metric(1.0, "ms", 1)}).to_json())
        doc["metrics"]["m.x"]["value"] = bad
        with pytest.raises(ResultsError, match="finite"):
            Results.from_json(json.dumps(doc))

    def test_pyperf_parser_rejects_malformed_elements(self) -> None:
        with pytest.raises(parsers.ParseError, match="not an object"):
            parsers.pyperf_benchmarks('{"benchmarks": [1]}')
        with pytest.raises(parsers.ParseError, match="runs is not an array"):
            parsers.pyperf_benchmarks(
                json.dumps({"benchmarks": [{"metadata": {"name": "x"}, "runs": 5}]})
            )
        with pytest.raises(parsers.ParseError, match="non-numeric value"):
            parsers.pyperf_benchmarks(
                json.dumps(
                    {"benchmarks": [{"metadata": {"name": "x"}, "runs": [{"values": ["y"]}]}]}
                )
            )

    def test_hyperfine_parser_rejects_malformed_entries(self) -> None:
        with pytest.raises(parsers.ParseError, match="not an object"):
            hyperfine.parse_export('{"results": [1]}')
        with pytest.raises(parsers.ParseError, match="non-numeric time"):
            hyperfine.parse_export(
                json.dumps({"results": [{"command": "a", "times": ["x"], "exit_codes": [0]}]})
            )

    def test_derive_strict_pair_half_present_fails_loud(self) -> None:
        # A renamed producer metric must never silently drop a headline derivation.
        with pytest.raises(pipeline.PipelineError, match="half-present"):
            pipeline.derive({"startup.version.median_ms": Metric(218.0, "ms", 15)})

    def test_derive_scale_grid_half_is_legitimate(self) -> None:
        # The scale pair rides on the plan's N grid; one endpoint alone is a config
        # choice, not a rename — silently skipped, budget reports metric-missing.
        derived = pipeline.derive({"scale.list_json.n0.median_ms": Metric(220.0, "ms", 15)})
        assert "scale.list_json.per_entry_us" not in derived

    def test_merge_rejects_duplicate_suite_outputs(self) -> None:
        a = SuiteOutput(suite="micro", metrics={"m.a": Metric(1, "us", 1)})
        b = SuiteOutput(suite="micro", metrics={"m.b": Metric(2, "us", 1)})
        with pytest.raises(pipeline.PipelineError, match="duplicate suite output"):
            pipeline.merge(make_meta(), [a, b], total_duration_s=1.0)

    def test_summarize_dir_rejects_corrupt_run_json(self, tmp_path: Path) -> None:
        (tmp_path / "run.json").write_text("{truncated")
        with pytest.raises(pipeline.PipelineError, match="not valid JSON"):
            pipeline.summarize_dir(tmp_path)

    def test_summarize_dir_rejects_non_finite_total(self, tmp_path: Path) -> None:
        import dataclasses

        (tmp_path / "run.json").write_text(
            json.dumps({"meta": dataclasses.asdict(make_meta()), "total_duration_s": float("inf")})
        )
        with pytest.raises(pipeline.PipelineError, match="finite"):
            pipeline.summarize_dir(tmp_path)

    def test_skip_all_fills_both_suite_fields(self) -> None:
        output = SuiteOutput.skip_all("rss", "no resource module on Windows")
        assert output.suite == "rss"
        assert output.skipped == [
            Skip(suite="rss", case="all", reason="no resource module on Windows")
        ]
        assert SuiteOutput.from_json(output.to_json()) == output

    def test_compare_profile_carries_compare_mode(self) -> None:
        assert all(plan.compare_mode for plan in pipeline.build_plan("compare"))
        assert not any(plan.compare_mode for plan in pipeline.build_plan("pr"))
        assert not any(plan.compare_mode for plan in pipeline.build_plan("full"))

    def test_startup_and_imports_declare_their_empty_library(self) -> None:
        # Their n=0 need must be plan-visible (dataset_ns feeds prepare_datasets),
        # never satisfied incidentally by a sibling suite.
        for profile in pipeline.PROFILES:
            for plan in pipeline.build_plan(profile):
                if plan.suite in ("startup", "imports"):
                    assert 0 in plan.ns

    def test_manifest_records_the_probe_char(self, tmp_path: Path) -> None:
        import dataclasses

        from benchmarks.datasets import SEARCH_PROBE_CHAR, check_reusable

        manifest = generate(tmp_path / "ds", 3)
        assert manifest.probe_char == SEARCH_PROBE_CHAR
        assert Manifest.load(manifest.root).probe_char == SEARCH_PROBE_CHAR
        stale = dataclasses.replace(manifest, probe_char="e")
        with pytest.raises(DatasetError, match="different inputs"):
            check_reusable(stale, 3)
