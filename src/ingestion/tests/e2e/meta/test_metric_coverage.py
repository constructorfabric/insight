from pathlib import Path

from lib.metric_coverage import MetricDefinition, build_report
from lib.metric_expect import Ledger


def _definition(metric_key: str, computation: str = "sum", dimensions: tuple[str, ...] = ()) -> MetricDefinition:
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
    assert build_report({key: _definition(key)}, ledger).passed

    ledger = _ledger(tmp_path / "ledger.json", key, "period", "timeseries")
    assert build_report({key: _definition(key)}, ledger).missing == {key: {"peer"}}


def test_dimensions_and_a_median_add_breakdown_and_histogram(tmp_path: Path) -> None:
    key = "test.median"
    definition = _definition(key, computation="median", dimensions=("source",))
    ledger = _ledger(tmp_path / "ledger.json", key, "period", "peer", "timeseries")
    assert build_report({key: definition}, ledger).missing == {key: {"breakdown", "histogram"}}

    ledger = _ledger(tmp_path / "ledger.json", key, "period", "peer", "timeseries", "breakdown", "histogram")
    assert build_report({key: definition}, ledger).passed


def test_unasserted_and_unknown_metrics_fail(tmp_path: Path) -> None:
    ledger = _ledger(tmp_path / "ledger.json", "test.unknown", "period", "peer", "timeseries")
    report = build_report({"test.expected": _definition("test.expected")}, ledger)
    assert report.missing == {"test.expected": {"period", "peer", "timeseries"}}
    assert report.unknown_asserted == {"test.unknown"}
    assert report.unknown_requested == {"test.unknown"}
    assert not report.passed


def test_a_run_that_wrote_no_ledger_asserted_nothing(tmp_path: Path) -> None:
    report = build_report({"test.sum": _definition("test.sum")}, tmp_path / "absent.json")
    assert report.missing == {"test.sum": {"period", "peer", "timeseries"}}
