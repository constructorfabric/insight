"""Worklog accuracy keeps a worklog the source still holds, however late its issue was
re-listed.

Bronze: five Engineering members, each with one Jira issue held In Progress for a day
(86400 s) and a worklog of rank * 17280 s; dave has an extra 17280 s worklog. Every
issue's row in the issue-key list was extracted two months after the worklogs, and no
worklog carries a tombstone, so none is deleted: dave reaches 100 and the department
spreads {20,40,60,100,100}.
"""

from __future__ import annotations

import pytest
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "tasks_worklog_issue_refetch_kept"

DAVE = "dave@example.com"
ERIN = "erin@example.com"


def test_late_issue_key_row_keeps_worklogs_in_tasks_worklog_accuracy(spec: SpecRun) -> None:
    """Dave logged 69120 s plus 17280 s with no tombstone: both count, so 100 rather than 80."""
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

    r.row("tasks.worklog_accuracy", "period", entity_id=DAVE).equals(value=100)
    r.row("tasks.worklog_accuracy", "period", entity_id=ERIN).equals(value=100)
    r.row("tasks.worklog_accuracy", "peer", entity_id=ERIN).equals(
        target_value=100, p25=40, median=60, p75=100, min=20, max=100, n=5
    )
