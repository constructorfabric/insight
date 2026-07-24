from pathlib import Path

from lib.metric_coverage import MetricDefinition, build_report, coverage_from_tests


def _definition(
    metric_key: str,
    computation: str = "sum",
    dimensions: tuple[str, ...] = (),
) -> MetricDefinition:
    return MetricDefinition(
        metric_key=metric_key,
        label=metric_key,
        computation=computation,
        dimensions=dimensions,
        peer_cohort_key="org_unit",
    )


def _write_fixture(path: Path, metric_key: str, extra_views: str = "") -> None:
    path.write_text(
        f"""spec_version: 1
cases:
  - name: coverage
    request:
      body:
        metrics:
          - metric_key: {metric_key}
    expect:
      - metric: {metric_key}
        view: period
        find: {{ entity_id: person@example.com }}
        equal: {{ value: 1 }}
      - metric: {metric_key}
        view: peer
        find: {{ entity_id: person@example.com }}
        equal: {{ target_value: 1, p25: 1, median: 1, p75: 1, min: 1, max: 1, n: 5 }}
      - metric: {metric_key}
        view: timeseries
        assert: "items.exists(s, size(s.points) > 0)"
{extra_views}""",
        encoding="utf-8",
    )


def test_sum_metric_requires_period_peer_and_timeseries(tmp_path: Path) -> None:
    key = "test.sum"
    _write_fixture(tmp_path / "sum.test.yaml", key)
    report = build_report({key: _definition(key)}, tmp_path)
    assert report.passed


def test_dimensions_and_median_require_breakdown_and_histogram(tmp_path: Path) -> None:
    key = "test.median"
    _write_fixture(
        tmp_path / "median.test.yaml",
        key,
        f"""      - metric: {key}
        view: breakdown
        assert: "items.exists(v, size(v.dimensions) > 0 && v.value != null)"
      - metric: {key}
        view: histogram
        assert: "items.exists(h, size(h.bins) > 0)"
""",
    )
    report = build_report(
        {key: _definition(key, computation="median", dimensions=("source",))},
        tmp_path,
    )
    assert report.passed


def test_missing_and_unknown_metrics_fail(tmp_path: Path) -> None:
    _write_fixture(tmp_path / "unknown.test.yaml", "test.unknown")
    report = build_report({"test.expected": _definition("test.expected")}, tmp_path)
    assert report.missing == {
        "test.expected": {"period", "peer", "timeseries"}
    }
    assert report.unknown_asserted == {"test.unknown"}
    assert report.unknown_requested == {"test.unknown"}
    assert not report.passed


def test_incomplete_peer_fields_do_not_cover_peer(tmp_path: Path) -> None:
    key = "test.peer"
    _write_fixture(tmp_path / "peer.test.yaml", key)
    text = (tmp_path / "peer.test.yaml").read_text(encoding="utf-8")
    (tmp_path / "peer.test.yaml").write_text(
        text.replace(", p25: 1, median: 1, p75: 1, min: 1, max: 1, n: 5", ""),
        encoding="utf-8",
    )
    asserted, _ = coverage_from_tests(tmp_path)
    assert "peer" not in asserted[key]


def test_typed_collection_assertions_cover_views(tmp_path: Path) -> None:
    key = "test.median"
    _write_fixture(
        tmp_path / "median.test.yaml",
        key,
        f"""      - metric: {key}
        view: breakdown
        assert: "size(items) == 0"
      - metric: {key}
        view: histogram
        find: {{ entity_id: person@example.com }}
        nonempty: [bins]
""",
    )
    text = (tmp_path / "median.test.yaml").read_text(encoding="utf-8")
    (tmp_path / "median.test.yaml").write_text(
        text.replace(
            'view: timeseries\n        assert: "items.exists(s, size(s.points) > 0)"',
            "view: timeseries\n        find: { entity_id: person@example.com }\n        contains: { points: { value: 1 } }",
        ),
        encoding="utf-8",
    )
    report = build_report(
        {key: _definition(key, computation="median", dimensions=("source",))},
        tmp_path,
    )
    assert report.passed
