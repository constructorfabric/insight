"""Measures that are not additive across accounts stay whole for one email.

The worklog gate and the day's estimate deviation are taken per person-day, so
two tracker accounts under one email meet before either applies — as they did
when task gold was keyed by email alone.
"""

from __future__ import annotations

import pytest
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "jira_tasks_two_accounts_one_email"

DAVE = "dave@example.com"


@pytest.mark.parametrize(
    ("metric_key", "expected"),
    [
        # dave-b's 12 h meet dave-a's day in progress: kept, not gated out.
        ("tasks.worklog_accuracy", 50),
        # 1 h + 3 h estimated against 2 h + 2 h spent on one day: no deviation.
        ("tasks.estimation_accuracy", 100),
    ],
)
def test_two_accounts_of_one_email_meet_on_the_person_day(
    spec: SpecRun, metric_key: str, expected: int
) -> None:
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [DAVE]},
                "period": {"from": "2026-04-01", "to": "2026-04-30"},
                "metrics": [{"metric_key": metric_key, "views": [{"view": "period"}]}],
            },
        }
    )
    assert r.status == 200
    r.row(metric_key, "period", entity_id=DAVE).equals(value=expected)
