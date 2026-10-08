from pathlib import Path

import pytest
from insight_datapath.metric_coverage import MetricDefinition, build_report, main, owed_by
from insight_datapath.metric_expect import Ledger


def _definition(
    metric_key: str, computation: str = "sum", dimensions: tuple[str, ...] = ()
) -> MetricDefinition:
    return MetricDefinition(
        metric_key=metric_key,
        label=metric_key,
        computation=computation,
        dimensions=dimensions,
        peer_cohort_key="org_unit",
    )


def _ledger(path: Path, metric_key: str, *views: str) -> Path:
    ledger = Ledger()
    ledger.record_request({"metrics": [{"metric_key": metric_key}]})
    for view in views:
        ledger.record_assertion(metric_key, view, "test_case")
    ledger.write(path)
    return path


def test_a_sum_metric_needs_period_peer_and_timeseries(tmp_path: Path) -> None:
    key = "test.sum"
    ledger = _ledger(tmp_path / "ledger.json", key, "period", "peer", "timeseries")
    assert build_report({key: _definition(key)}, [ledger]).passed

    ledger = _ledger(tmp_path / "ledger.json", key, "period", "timeseries")
    assert build_report({key: _definition(key)}, [ledger]).missing == {key: {"peer"}}


def test_dimensions_and_a_median_add_breakdown_and_histogram(tmp_path: Path) -> None:
    key = "test.median"
    definition = _definition(key, computation="median", dimensions=("source",))
    ledger = _ledger(tmp_path / "ledger.json", key, "period", "peer", "timeseries")
    assert build_report({key: definition}, [ledger]).missing == {key: {"breakdown", "histogram"}}

    ledger = _ledger(
        tmp_path / "ledger.json", key, "period", "peer", "timeseries", "breakdown", "histogram"
    )
    assert build_report({key: definition}, [ledger]).passed


def test_unasserted_and_unknown_metrics_fail(tmp_path: Path) -> None:
    ledger = _ledger(tmp_path / "ledger.json", "test.unknown", "period", "peer", "timeseries")
    report = build_report({"test.expected": _definition("test.expected")}, [ledger])
    assert report.missing == {"test.expected": {"period", "peer", "timeseries"}}
    assert report.unknown_asserted == {"test.unknown"}
    assert report.unknown_requested == {"test.unknown"}
    assert not report.passed


def test_coverage_is_the_union_of_every_shards_ledger(tmp_path: Path) -> None:
    """A sharded lane writes one ledger per shard, each covering only what it ran."""
    views = ("period", "peer", "timeseries")
    first = _ledger(tmp_path / "a.json", "test.one", *views)
    second = _ledger(tmp_path / "b.json", "test.two", *views)
    universe = {key: _definition(key) for key in ("test.one", "test.two")}

    assert build_report(universe, [first]).missing == {"test.two": set(views)}
    assert build_report(universe, [first, second]).passed


def test_a_ledger_that_is_not_there_is_refused(tmp_path: Path) -> None:
    """Absent, it would read as a universe of gaps and blame the suite for the run."""
    universe = tmp_path / "universe.json"
    universe.write_text('{"metrics": []}', encoding="utf-8")
    with pytest.raises(SystemExit) as refusal:
        main(["--universe-file", str(universe), "--ledger", str(tmp_path / "absent.json")])
    assert refusal.value.code == 2


@pytest.mark.parametrize(
    ("ran", "owed"),
    [
        ({"git"}, {"git.commits", "orphan.metric"}),
        ({"git", "ai"}, {"git.commits", "ai.seats", "orphan.metric"}),
        ({"collab"}, {"orphan.metric"}),
    ],
)
def test_a_partial_run_owes_its_own_classes_and_every_classless_key(
    ran: set[str], owed: set[str]
) -> None:
    universe = {key: _definition(key) for key in ("git.commits", "ai.seats", "orphan.metric")}
    known = frozenset({"git", "ai", "collab"})
    assert owed_by(universe, frozenset(ran), known) == owed, f"should owe {owed} for a run of {ran}"


def test_a_partial_run_still_fails_on_a_gap_in_a_class_it_ran(tmp_path: Path) -> None:
    views = ("period", "peer", "timeseries")
    ledger = _ledger(tmp_path / "ledger.json", "git.commits", *views)
    universe = {key: _definition(key) for key in ("git.commits", "git.active_days", "ai.seats")}
    owed = owed_by(universe, frozenset({"git"}), frozenset({"git", "ai"}))

    report = build_report(universe, [ledger], owed)

    assert report.missing == {"git.active_days": set(views)}


@pytest.mark.parametrize(
    "arguments",
    [["--classes", "git"], ["--known", "git"], ["--classes", "", "--known", "git"]],
)
def test_a_class_scope_needs_both_halves_and_at_least_one_class(
    tmp_path: Path, arguments: list[str]
) -> None:
    universe = tmp_path / "universe.json"
    universe.write_text('{"metrics": []}', encoding="utf-8")
    ledger = _ledger(tmp_path / "ledger.json", "git.commits", "period")
    with pytest.raises(SystemExit) as refusal:
        main(["--universe-file", str(universe), "--ledger", str(ledger), *arguments])
    assert refusal.value.code == 2, f"should refuse {arguments}"
