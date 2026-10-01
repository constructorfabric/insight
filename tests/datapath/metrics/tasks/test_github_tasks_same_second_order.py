"""Two GitHub type changes in one second, ordered by their from→to chain.

The event ids sort the second change first; the chain allows only Defect → Task → Bug.
The closure is classified by the newest type, so the order decides whether it counts as
a fixed bug or a closed task.
"""

from __future__ import annotations

import pytest
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "github_tasks_same_second_order"

CAROL = "carol@example.com"


def test_the_chain_orders_type_changes_that_share_a_second(spec: SpecRun) -> None:
    """Ordered by id the issue ends on Task and the closure is no fixed bug; ordered by
    the chain it ends on Bug."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [CAROL]},
                "period": {"from": "2026-03-20", "to": "2026-03-31"},
                "metrics": [
                    {"metric_key": "tasks.closed", "views": [{"view": "period"}]},
                    {"metric_key": "tasks.bugs_fixed", "views": [{"view": "period"}]},
                ],
            },
        }
    )
    assert r.status == 200

    r.row("tasks.closed", "period", entity_id=CAROL).equals(value=1)
    r.row("tasks.bugs_fixed", "period", entity_id=CAROL).equals(value=1)
