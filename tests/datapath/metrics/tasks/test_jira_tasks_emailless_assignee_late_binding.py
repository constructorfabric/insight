"""An unbound email-less account's work counts once identity binds it, without a rebuild.

Gold keeps the issue under its assignee's account whether or not anything binds
it, and the runtime resolves at read time, so an operator binding made after the
build credits the work on the next request.
"""

from __future__ import annotations

import time

import pytest
from insight_datapath.fixture_loader import IdentityAccount
from insight_datapath.metric_expect import MetricResponse
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "jira_tasks_emailless_assignee_late_binding"

CAROL = "carol@example.com"

FORMER_USER = IdentityAccount(
    source_type="jira", source_id="jira-test", account_id="former-user-acct", person=CAROL
)

PUBLISH_TIMEOUT_SECS = 30
PUBLISH_POLL_SECS = 1


def closed_by_carol(spec: SpecRun) -> MetricResponse:
    return spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [CAROL]},
                "period": {"from": "2026-05-01", "to": "2026-05-31"},
                "metrics": [{"metric_key": "tasks.closed", "views": [{"view": "period"}]}],
            },
        }
    )


def closed_once_published(spec: SpecRun) -> MetricResponse:
    deadline = time.monotonic() + PUBLISH_TIMEOUT_SECS
    while True:
        r = closed_by_carol(spec)
        assert r.status == 200
        value = next((item.get("value") for item in r.items("tasks.closed", "period")), None)
        if value is not None or time.monotonic() > deadline:
            return r
        time.sleep(PUBLISH_POLL_SECS)


def test_an_unbound_account_counts_the_moment_it_is_bound(spec: SpecRun) -> None:
    """None before the binding; the same build serves the closure after it."""
    # The identity store outlives a run, so a previous run's binding is undone first.
    spec.detach(FORMER_USER)
    try:
        before = closed_by_carol(spec)
        assert before.status == 200
        before.row("tasks.closed", "period", entity_id=CAROL).equals(value=None)

        spec.bind(FORMER_USER)

        after = closed_once_published(spec)
        after.row("tasks.closed", "period", entity_id=CAROL).equals(value=1)
    finally:
        spec.detach(FORMER_USER)
