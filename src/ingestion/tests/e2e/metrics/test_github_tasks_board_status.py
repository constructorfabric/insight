"""Task delivery for GitHub driven by a Projects V2 board column: dev time, pickup
time, resolution time and flow efficiency.

GitHub says only open or closed, so a board column is the only place an issue is
in progress; the bound board's column names resolve through an operator's binding
into lifecycle categories. Created on the 1st, Todo on the 2nd, In progress on the
6th, Done on the 11th: pickup five days, dev five days, lifetime ten, flow efficiency
half. A second board the issue also sits on is bound to nothing and contributes nothing.
"""

from __future__ import annotations

import pytest
from lib.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "github_tasks_board_status"

CAROL = "carol@example.com"


def test_board_column_supplies_in_progress_lifecycle_github_never_states(spec: SpecRun) -> None:
    """Closed by the board's Done column; dev time is the 6th to the 11th (120 hours, not the
    control board's 408), pickup counts from creation (5 days, not 4), and five of ten is 50."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [CAROL]},
                "period": {"from": "2026-03-01", "to": "2026-03-31"},
                "metrics": [
                    {"metric_key": "tasks.closed", "views": [{"view": "period"}]},
                    {"metric_key": "tasks.dev_time", "views": [{"view": "period"}]},
                    {"metric_key": "tasks.pickup_time", "views": [{"view": "period"}]},
                    {"metric_key": "tasks.resolution_time", "views": [{"view": "period"}]},
                    {"metric_key": "tasks.flow_efficiency", "views": [{"view": "period"}]},
                ],
            },
        }
    )
    assert r.status == 200

    r.row("tasks.closed", "period", entity_id=CAROL).equals(value=1)
    r.row("tasks.dev_time", "period", entity_id=CAROL).equals(value=120)
    r.row("tasks.pickup_time", "period", entity_id=CAROL).equals(value=5)
    r.row("tasks.resolution_time", "period", entity_id=CAROL).equals(value=10)
    r.row("tasks.flow_efficiency", "period", entity_id=CAROL).equals(value=50)
