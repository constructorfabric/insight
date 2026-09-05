"""Authored commits divided by the days that actually carried one.

Bronze: GitHub commits across two repositories, plus a merge commit. Silver counts an
authored commit and the calendar day it lands on; gold divides the commits by the
DISTINCT days with at least one. Erin's six commits fall on two days inside a
seven-day window, so the period reads 3. Every wrong denominator disagrees: the
window's seven days read 0.86, the three per-repository day rows read 2, and counting
the merge-only day as active reads 2 as well.
"""

from __future__ import annotations

import pytest
from insight_datapath.metric_expect import one
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "git_commits_per_active_day"

ERIN = "erin@example.com"

API = {"key": "repository", "value": "git-test:acme/api"}
WEB = {"key": "repository", "value": "git-test:acme/web"}


def test_commits_divide_by_distinct_active_days(spec: SpecRun) -> None:
    """Six commits over the two days that carried one, not over the window's seven
    days nor the three per-repository day rows; the merge-only day carries no ratio."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-10-01", "to": "2026-10-07"},
                "metrics": [
                    {
                        "metric_key": "git.commits_per_active_day",
                        "views": [
                            {"view": "period"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["repository"]},
                        ],
                    }
                ],
            },
        }
    )
    assert r.status == 200

    r.row("git.commits_per_active_day", "period", entity_id=ERIN).equals(value=3)

    daily = r.row("git.commits_per_active_day", "timeseries", entity_id=ERIN)
    daily.contains(points={"bucket_start": "2026-10-01", "value": 5})
    daily.contains(points={"bucket_start": "2026-10-02", "value": 1})
    assert one(daily["points"], bucket_start="2026-10-03")["value"] is None

    r.row("git.commits_per_active_day", "breakdown", entity_id=ERIN, dimensions=API).equals(
        value=1.5
    )
    r.row("git.commits_per_active_day", "breakdown", entity_id=ERIN, dimensions=WEB).equals(value=3)


def test_an_empty_window_serves_no_ratio(spec: SpecRun) -> None:
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2025-01-01", "to": "2025-01-31"},
                "metrics": [
                    {"metric_key": "git.commits_per_active_day", "views": [{"view": "period"}]}
                ],
            },
        }
    )
    assert r.status == 200
    r.row("git.commits_per_active_day", "period", entity_id=ERIN).equals(value=None)
