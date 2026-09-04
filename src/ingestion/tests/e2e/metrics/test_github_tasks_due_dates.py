"""Task delivery due-date compliance read from a native GitHub issue field, end to end.

A due date is not a GitHub issue property but a field the organization defined, so
gold reads it only through an operator binding of that field to the `duedate` role.
The REST payload names the field by a numeric id, the timeline and the binding by a
node id, and the catalogue bridges the two. Carol closes two dated issues on
2026-03-25 (one on time, one five days late) and one undated issue that counts as a
closure but stays out of both due-date measures.
"""

from __future__ import annotations

import pytest
from lib.metric_expect import one
from lib.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "github_tasks_due_dates"

CAROL = "carol@example.com"


def test_due_dates_read_through_the_bound_native_field(spec: SpecRun) -> None:
    """All three closures count; compliance is 1 of the 2 dated issues, on-time
    delivery is 1 of all 3, and the slip averages five days over the one late issue."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [CAROL]},
                "period": {"from": "2026-03-20", "to": "2026-03-31"},
                "metrics": [
                    {"metric_key": "tasks.closed", "views": [{"view": "period"}]},
                    {"metric_key": "tasks.due_date_compliance", "views": [{"view": "period"}]},
                    {"metric_key": "tasks.on_time_delivery", "views": [{"view": "period"}]},
                    {"metric_key": "tasks.avg_slip", "views": [{"view": "period"}]},
                ],
            },
        }
    )
    assert r.status == 200

    r.row("tasks.closed", "period", entity_id=CAROL).equals(value=3)
    r.row("tasks.due_date_compliance", "period", entity_id=CAROL).equals(value=50)
    on_time = one(r.rows("tasks.on_time_delivery", "period"), entity_id=CAROL)
    assert 33.0 < float(on_time["value"]) < 34.0
    r.row("tasks.avg_slip", "period", entity_id=CAROL).equals(value=5)
