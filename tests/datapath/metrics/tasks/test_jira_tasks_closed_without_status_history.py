"""A Jira issue closed with no status entry is closed on its resolution date.

Its history seeds the done status at creation, because nothing in the changelog
rolls it back; the closure must still land when Jira resolved the issue, not on
the day it was created, and the resolution time must run between the two.
"""

from __future__ import annotations

import pytest
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "jira_tasks_closed_without_status_history"

CAROL = "carol@example.com"


@pytest.mark.parametrize(
    ("window", "closed"),
    [
        (("2026-11-01", "2026-11-30"), None),
        (("2026-12-01", "2026-12-31"), 1),
    ],
    ids=["creation month", "resolution month"],
)
def test_the_closure_is_counted_in_the_month_it_was_resolved(
    spec: SpecRun, window: tuple[str, str], closed: int | None
) -> None:
    """Counted in December, when it was resolved; November, when it was
    created, holds no closure."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [CAROL]},
                "period": {"from": window[0], "to": window[1]},
                "metrics": [{"metric_key": "tasks.closed", "views": [{"view": "period"}]}],
            },
        }
    )
    assert r.status == 200
    r.row("tasks.closed", "period", entity_id=CAROL).equals(value=closed)


def test_the_resolution_time_runs_from_creation_to_resolution(spec: SpecRun) -> None:
    """Created 2 November at 14:00, resolved 25 December at 14:00: 53 days,
    where a closure dated at creation would leave no resolution time at all."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [CAROL]},
                "period": {"from": "2026-12-01", "to": "2026-12-31"},
                "metrics": [{"metric_key": "tasks.resolution_time", "views": [{"view": "period"}]}],
            },
        }
    )
    assert r.status == 200
    r.row("tasks.resolution_time", "period", entity_id=CAROL).equals(value=53)
