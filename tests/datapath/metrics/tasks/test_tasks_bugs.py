"""Bug and non-bug subsets of closed JIRA issues, and the bug share, per person.

Bronze: jira_issue, jira_issue_history (the Status -> Closed event dates the close),
jira_user and jira_issuetypes; the issue-type dimension resolves each issue to a bug,
task or unknown kind. Gold serves bugs_fixed, closed_task and bugs_ratio, a share
of closed issues with a ceiling of 100. Five department members each close four
issues on one day, (r-1) bugs and (5-r) tasks, so the shares spread {0,25,50,75,100}.
"""

from __future__ import annotations

import pytest
from insight_datapath.metric_expect import approx, one
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "tasks_bugs"

ALICE = "alice@example.com"
ERIN = "erin@example.com"


def test_unified_task_bug_metrics(spec: SpecRun) -> None:
    """Erin (rank 5) closes 4 bugs of 4 issues: bugs_fixed 4 and bugs_ratio 100 in the
    June window, with the department distribution alongside."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-06-20", "to": "2026-06-30"},
                "metrics": [
                    {
                        "metric_key": "tasks.bugs_fixed",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                        ],
                    },
                    {
                        "metric_key": "tasks.bugs_ratio",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                        ],
                    },
                ],
            },
        }
    )
    assert r.status == 200

    r.row("tasks.bugs_fixed", "period", entity_id=ERIN).equals(value=4)
    r.row("tasks.bugs_fixed", "peer", entity_id=ERIN).equals(
        target_value=4, p25=None, median=None, p75=None, min=None, max=None, n=4
    )
    fixed = one(r.series("tasks.bugs_fixed"), entity_id=ERIN)["points"]
    assert float(one(fixed, bucket_start="2026-06-25")["value"]) == approx(4.0)

    r.row("tasks.bugs_ratio", "period", entity_id=ERIN).equals(value=100)
    r.row("tasks.bugs_ratio", "peer", entity_id=ERIN).equals(
        target_value=100, p25=None, median=None, p75=None, min=None, max=None, n=4
    )
    ratio = one(r.series("tasks.bugs_ratio"), entity_id=ERIN)["points"]
    assert float(one(ratio, bucket_start="2026-06-25")["value"]) == approx(100.0)


def test_task_kind_closed_issues_are_the_complement_subset(spec: SpecRun) -> None:
    """Alice (rank 1) closes 4 tasks and 0 bugs, so closed_task is 4."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ALICE]},
                "period": {"from": "2026-06-20", "to": "2026-06-30"},
                "metrics": [
                    {
                        "metric_key": "tasks.closed_task",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                        ],
                    }
                ],
            },
        }
    )
    assert r.status == 200

    r.row("tasks.closed_task", "period", entity_id=ALICE).equals(value=4)
    r.row("tasks.closed_task", "peer", entity_id=ALICE).equals(
        target_value=4, p25=None, median=None, p75=None, min=None, max=None, n=4
    )
    tasks = one(r.series("tasks.closed_task"), entity_id=ALICE)["points"]
    assert float(one(tasks, bucket_start="2026-06-25")["value"]) == approx(4.0)


def test_unified_task_bug_metrics_empty_window(spec: SpecRun) -> None:
    """A window with no closes serves null for all four metrics."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2025-01-01", "to": "2025-01-31"},
                "metrics": [
                    {"metric_key": "tasks.bugs_fixed", "views": [{"view": "period"}]},
                    {"metric_key": "tasks.closed_task", "views": [{"view": "period"}]},
                    {"metric_key": "tasks.closed_unknown", "views": [{"view": "period"}]},
                    {"metric_key": "tasks.bugs_ratio", "views": [{"view": "period"}]},
                ],
            },
        }
    )
    assert r.status == 200
    r.row("tasks.bugs_fixed", "period", entity_id=ERIN).equals(value=None)
    r.row("tasks.closed_task", "period", entity_id=ERIN).equals(value=None)
    r.row("tasks.closed_unknown", "period", entity_id=ERIN).equals(value=None)
    r.row("tasks.bugs_ratio", "period", entity_id=ERIN).equals(value=None)
