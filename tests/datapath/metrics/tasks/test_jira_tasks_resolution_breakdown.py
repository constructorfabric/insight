"""The `resolution` breakdown dimension on tasks.closed and tasks.bugs_fixed.

Gold resolves `resolution_kind` per issue as mapping row -> tenant default ->
'unknown', except an issue with NO resolution value at all is 'unknown'
outright — the default speaks only for unmapped values. Carol's five closures
carry a mapped fixed, a mapped duplicate, a mapped wontfix, an unmapped id
claimed by the wontfix default, and one unresolved issue that stays unknown
despite that default.
"""

from __future__ import annotations

import pytest
from insight_datapath.metric_expect import one, some
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "jira_tasks_resolution_breakdown"

CAROL = "carol@example.com"


def test_tasks_closed_split_by_resolution(spec: SpecRun) -> None:
    """Every closure lands in exactly one resolution class; the unmapped id
    falls to the default while the unresolved issue stays unknown."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [CAROL]},
                "period": {"from": "2026-03-20", "to": "2026-03-31"},
                "metrics": [
                    {
                        "metric_key": "tasks.closed",
                        "views": [
                            {"view": "period"},
                            {"view": "breakdown", "dimensions": ["resolution"]},
                        ],
                    }
                ],
            },
        }
    )
    assert r.status == 200

    r.row("tasks.closed", "period", entity_id=CAROL).equals(value=5)
    by_resolution = r.breakdown("tasks.closed")
    for kind, count in [("fixed", 1.0), ("duplicate", 1.0), ("wontfix", 2.0), ("unknown", 1.0)]:
        row = one(by_resolution, entity_id=CAROL, dimensions={"key": "resolution", "value": kind})
        assert float(row["value"]) == count, f"resolution {kind!r}"
    assert len(some(by_resolution, entity_id=CAROL)) == 4


def test_bugs_fixed_split_by_resolution(spec: SpecRun) -> None:
    """Only the bug-kind closures reach bugs_fixed; each keeps its own
    resolution class, including the unresolved bug's unknown."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [CAROL]},
                "period": {"from": "2026-03-20", "to": "2026-03-31"},
                "metrics": [
                    {
                        "metric_key": "tasks.bugs_fixed",
                        "views": [
                            {"view": "period"},
                            {"view": "breakdown", "dimensions": ["resolution"]},
                        ],
                    }
                ],
            },
        }
    )
    assert r.status == 200

    r.row("tasks.bugs_fixed", "period", entity_id=CAROL).equals(value=3)
    by_resolution = r.breakdown("tasks.bugs_fixed")
    for kind in ["fixed", "wontfix", "unknown"]:
        row = one(by_resolution, entity_id=CAROL, dimensions={"key": "resolution", "value": kind})
        assert float(row["value"]) == 1.0, f"resolution {kind!r}"
    assert len(some(by_resolution, entity_id=CAROL)) == 3


def test_closed_by_resolution_class_metrics(spec: SpecRun) -> None:
    """The per-class subsets of tasks.closed: each classified closure counts in
    exactly one of closed_fixed / closed_duplicate / closed_wontfix, and the
    unknown class has no metric of its own — the alert owns it."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [CAROL]},
                "period": {"from": "2026-03-20", "to": "2026-03-31"},
                "metrics": [
                    {
                        "metric_key": metric_key,
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                        ],
                    }
                    for metric_key in (
                        "tasks.closed_fixed",
                        "tasks.closed_duplicate",
                        "tasks.closed_wontfix",
                    )
                ],
            },
        }
    )
    assert r.status == 200

    for metric_key, expected in [
        ("tasks.closed_fixed", 1),
        ("tasks.closed_duplicate", 1),
        ("tasks.closed_wontfix", 2),
    ]:
        r.row(metric_key, "period", entity_id=CAROL).equals(value=expected)
        # A one-person pool sits below the disclosure minimum: every
        # percentile is withheld, only the person's own figure serves.
        r.row(metric_key, "peer", entity_id=CAROL).equals(
            target_value=expected, p25=None, median=None, p75=None, min=None, max=None, n=1
        )
        points = one(r.series(metric_key), entity_id=CAROL)["points"]
        day = one(points, bucket_start="2026-03-25")
        assert float(day["value"]) == float(expected), f"{metric_key} on the close day"
