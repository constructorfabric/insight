"""An assignee with no email reaches a person through the account identity binds.

Gold keeps every assigned issue with its assignee's tracker account, so the
runtime resolves it account-first: an account an operator bound to a person
credits that person, and an account bound to the excluded person credits nobody,
whatever email its profile carries.
"""

from __future__ import annotations

import pytest
from insight_datapath.metric_expect import MetricResponse
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "jira_tasks_emailless_assignee"

DAVE = "dave@example.com"
ERIN = "erin@example.com"

APRIL = {"from": "2026-04-01", "to": "2026-04-30"}


def period_of(spec: SpecRun, metric_key: str, person: str) -> MetricResponse:
    return spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [person]},
                "period": APRIL,
                "metrics": [{"metric_key": metric_key, "views": [{"view": "period"}]}],
            },
        }
    )


def test_a_bound_account_with_no_email_credits_its_person(spec: SpecRun) -> None:
    """Both issues on dave's email-less account count as his closures."""
    r = period_of(spec, "tasks.closed", DAVE)
    assert r.status == 200
    r.row("tasks.closed", "period", entity_id=DAVE).equals(value=2)


def test_a_bound_account_with_no_email_credits_its_worklogs(spec: SpecRun) -> None:
    """12 h logged against one day in progress, both on the email-less account: 50%."""
    r = period_of(spec, "tasks.worklog_accuracy", DAVE)
    assert r.status == 200
    r.row("tasks.worklog_accuracy", "period", entity_id=DAVE).equals(value=50)


def test_the_evidence_names_the_issues_of_a_bound_account(spec: SpecRun) -> None:
    """The drilldown behind dave's closures lists both issues of the account."""
    r = spec.call(
        {
            "url": "/v1/metric-drilldown",
            "method": "POST",
            "body": {
                "metric_key": "tasks.closed",
                "entity": {"type": "person", "id": DAVE},
                "period": APRIL,
            },
        }
    )
    assert r.status == 200, r.payload

    refs = sorted(str(row["values"].get("ref")) for row in r.payload["rows"])
    assert refs == ["OLD-1", "OLD-2"], f"evidence refs: {refs}"


def test_an_excluded_account_credits_nobody_even_through_its_email(spec: SpecRun) -> None:
    """The app account carries erin's address, but its exclusion ends resolution there."""
    r = period_of(spec, "tasks.closed", ERIN)
    assert r.status == 200
    r.row("tasks.closed", "period", entity_id=ERIN).equals(value=None)
