"""The configured default kind: where an unmapped Jira type id lands.

Gold resolves the kind as mapping row -> tenant default -> 'unknown'. This tenant
defaults issue_type to `task`, so Carol's closed Incident (no map row) counts in
closed_task instead of staying `unknown`, while her closed Bug keeps its mapped
`bug` — the default never overrides a decision. The absent-default case is pinned by
the *_value_map_precedence specs.
"""

from __future__ import annotations

import pytest
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "jira_tasks_bugs_default_kind"

CAROL = "carol@example.com"


def test_unmapped_type_falls_to_the_configured_default(spec: SpecRun) -> None:
    """One mapped bug, one unmapped issue claimed by the `task` default: both
    closures are classified, nothing lands in the unknown bucket."""
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
                    {"metric_key": "tasks.closed_task", "views": [{"view": "period"}]},
                ],
            },
        }
    )
    assert r.status == 200

    r.row("tasks.closed", "period", entity_id=CAROL).equals(value=2)
    r.row("tasks.bugs_fixed", "period", entity_id=CAROL).equals(value=1)
    r.row("tasks.closed_task", "period", entity_id=CAROL).equals(value=1)
