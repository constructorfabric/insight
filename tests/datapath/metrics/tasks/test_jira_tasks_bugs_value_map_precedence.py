"""Value-map precedence over the shared name lists for Jira's bug split.

An operator's map row beats the name lists only while it is alive, and for Jira it
keys on the normalized type NAME, not the per-project type id. Carol closes a Bug
whose live row says `other` (map wins, non-bug), a Mapped Custom whose live row says
`bug` (map claims a name no list knows), a Regression whose row is deleted (name-list
fallback, bug), an Unmapped Custom with no row (`unknown`) and a Future Custom whose
row starts in 2030 (ignored, `unknown` too). Two bugs, one non-bug, five closures —
and each wrong branch moves the bug count off two in its own direction.
"""

from __future__ import annotations

import pytest
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "jira_tasks_bugs_value_map_precedence"

CAROL = "carol@example.com"


def test_live_map_rows_win_by_name_and_dead_or_future_rows_fall_back(spec: SpecRun) -> None:
    """A live row beats the bug-name list and claims an unlisted name; a deleted or
    future row is ignored in favour of the fallback; an unmapped unlisted name stays
    `unknown` and counts in tasks.closed only."""
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

    r.row("tasks.closed", "period", entity_id=CAROL).equals(value=5)
    r.row("tasks.bugs_fixed", "period", entity_id=CAROL).equals(value=2)
    r.row("tasks.closed_non_bug", "period", entity_id=CAROL).equals(value=1)
