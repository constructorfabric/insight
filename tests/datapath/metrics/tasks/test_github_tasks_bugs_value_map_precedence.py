"""Value-map precedence over the shared name lists for GitHub's bug split.

An operator's map row beats the name lists only while it is alive. Carol closes a
Bug whose live row says `other` (map wins, non-bug), a Defect with no row (name-list
fallback, bug), a Regression whose row is deleted (fallback, bug) and an Incident
whose row starts in 2030 (ignored, and no list knows the name, so `unknown`). Two
bugs, one non-bug, four closures — and each wrong branch moves the bug count off
two in its own direction.
"""

from __future__ import annotations

import pytest
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "github_tasks_bugs_value_map_precedence"

CAROL = "carol@example.com"


def test_live_map_rows_win_and_dead_or_future_rows_fall_back(spec: SpecRun) -> None:
    """A live `other` row beats the bug-name list; a deleted or future row is ignored
    in favour of the fallback; a name no list knows stays unclaimed by either side."""
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
                    {"metric_key": "tasks.closed_non_bug", "views": [{"view": "period"}]},
                ],
            },
        }
    )
    assert r.status == 200

    r.row("tasks.closed", "period", entity_id=CAROL).equals(value=4)
    r.row("tasks.bugs_fixed", "period", entity_id=CAROL).equals(value=2)
    r.row("tasks.closed_non_bug", "period", entity_id=CAROL).equals(value=1)
