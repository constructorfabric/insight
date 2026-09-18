"""Taking a card off a board ends the status it was last seen in.

`stale_in_progress` is stamped on the date of an issue's last status change, and
the removal is a status event — so the count lands on the day the card left the
board. The window here opens the day after the move into In progress and holds
the removal, which is what makes the two readings separable: with the closing
row the window reads 1, and without it the issue's last status change is the
earlier move and the window is empty.

`dev_time` looks like the natural measure and is not: it is stamped on
`final_close_at` and sums only spans starting before the close, so an issue that
never closes has no value at all regardless of its spans.
"""

from __future__ import annotations

import pytest
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "github_tasks_board_removal"

CAROL = "carol@example.com"


def test_removing_a_card_dates_the_issue_by_the_removal(spec: SpecRun) -> None:
    """The window holds the removal and not the move that preceded it."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [CAROL]},
                "period": {"from": "2026-03-07", "to": "2026-03-31"},
                "metrics": [
                    {"metric_key": "tasks.stale_in_progress", "views": [{"view": "period"}]},
                ],
            },
        }
    )
    assert r.status == 200

    r.row("tasks.stale_in_progress", "period", entity_id=CAROL).equals(value=1)
