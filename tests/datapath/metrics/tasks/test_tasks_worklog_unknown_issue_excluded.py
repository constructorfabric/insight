"""Worklog accuracy counts only worklogs of issues the issue streams collected.

Bronze: five Engineering members, each with one Jira issue held In Progress for a
day (86400 s) and a worklog of rank * 17280 s, so the department spreads
{20,40,60,80,100}. Dave has an extra 17280 s worklog on an issue absent from the issue
streams and no tombstone; it never reaches the class contract, so dave stays at 80.
"""

from __future__ import annotations

import pytest
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "tasks_worklog_unknown_issue_excluded"

DAVE = "dave@example.com"
ERIN = "erin@example.com"


def test_unknown_issue_worklog_leaves_tasks_worklog_accuracy(spec: SpecRun) -> None:
    """Dave logged 69120 s plus 17280 s on an uncollected issue: only the first counts, so 80
    rather than 100; erin's 86400 s counts in full and the department spread is unchanged."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [DAVE, ERIN]},
                "period": {"from": "2026-03-20", "to": "2026-03-31"},
                "metrics": [
                    {
                        "metric_key": "tasks.worklog_accuracy",
                        "views": [{"view": "period"}, {"view": "peer"}],
                    }
                ],
            },
        }
    )
    assert r.status == 200

    r.row("tasks.worklog_accuracy", "period", entity_id=DAVE).equals(value=80)
    r.row("tasks.worklog_accuracy", "period", entity_id=ERIN).equals(value=100)
    r.row("tasks.worklog_accuracy", "peer", entity_id=ERIN).equals(
        target_value=100, p25=40, median=60, p75=80, min=20, max=100, n=5
    )
