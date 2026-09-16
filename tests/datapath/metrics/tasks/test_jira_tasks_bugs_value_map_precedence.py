"""Jira's bug split comes from the operator's issue-type decision, keyed by type id.

There is no name-list fallback: an id with no live row is `unknown`. Carol closes a
type named Bug whose live row says `task` (non-bug), a custom type whose live row
says `bug`, a type renamed to `Task` since its `bug` decision was recorded (same id,
still a bug), a Regression whose row is deleted, a Future Custom whose row starts in
2099 and an Unmapped Custom with no row (the last three `unknown`). Two bugs, one
non-bug, six closures — and each wrong branch moves the bug count off two in its own
direction.
"""

from __future__ import annotations

import pytest
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "jira_tasks_bugs_value_map_precedence"

CAROL = "carol@example.com"


def test_live_rows_classify_by_id_and_anything_else_is_unknown(spec: SpecRun) -> None:
    """A live row keyed on the type id decides the kind whatever the type is named,
    including after a rename; a deleted, future or absent row leaves the type
    `unknown`, counting in tasks.closed only."""
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

    r.row("tasks.closed", "period", entity_id=CAROL).equals(value=6)
    r.row("tasks.bugs_fixed", "period", entity_id=CAROL).equals(value=2)
    r.row("tasks.closed_non_bug", "period", entity_id=CAROL).equals(value=1)
