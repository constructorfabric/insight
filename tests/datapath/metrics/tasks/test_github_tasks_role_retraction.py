"""A retracted role binding stops applying, and the field falls back to nothing here.

`config.task_field_roles` is bitemporal, so withdrawing a binding writes a newer row
carrying `is_deleted = 1` beside the one it withdraws. The operator bound GitHub's
`type` field to `issuetype` and then retracted it, and GitHub carries no built-in
default for that field, so Carol's two closures reach `config.field_value_map` with no
type at all and land in `closed_unknown` while `bugs_fixed` is left without a value.
"""

from __future__ import annotations

import pytest
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "github_tasks_role_retraction"

CAROL = "carol@example.com"


def test_a_retracted_role_binding_stops_classifying(spec: SpecRun) -> None:
    """Both closures are unclassified once the `issuetype` binding is withdrawn: the
    live `bug` decision for their type is reachable only through that binding. The
    closures themselves still count, so the untouched `state` binding is unaffected."""
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
                    {"metric_key": "tasks.closed_unknown", "views": [{"view": "period"}]},
                ],
            },
        }
    )
    assert r.status == 200

    r.row("tasks.closed", "period", entity_id=CAROL).equals(value=2)
    r.row("tasks.bugs_fixed", "period", entity_id=CAROL).equals(value=None)
    r.row("tasks.closed_task", "period", entity_id=CAROL).equals(value=None)
    r.row("tasks.closed_unknown", "period", entity_id=CAROL).equals(value=2)
