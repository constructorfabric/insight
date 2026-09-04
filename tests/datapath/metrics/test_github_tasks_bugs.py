"""Task Delivery bug split for GitHub, and the two ways an issue type acquires a kind.

GitHub names an issue's type without lifecycle meaning, so `issue_kind` comes from the
operator's value map, which wins, or the shared name lists, which catch what the map
omits. Carol closes a Bug (mapped to `bug`), a Task (mapped to `other`) and an
Incident mapped by nobody, which stays `unknown`: counted among closures, claimed by
neither side, and visible as a third group in the type breakdown.
"""

from __future__ import annotations

import pytest
from insight_datapath.metric_expect import one, some
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "github_tasks_bugs"

CAROL = "carol@example.com"


def test_bug_split_across_mapped_and_unmapped_types(spec: SpecRun) -> None:
    """One bug plus one non-bug is one short of the three closed; the Incident is the
    gap, and the type breakdown shows it as its own group rather than absorbing it."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [CAROL]},
                "period": {"from": "2026-03-20", "to": "2026-03-31"},
                "metrics": [
                    {
                        "metric_key": "tasks.closed",
                        "views": [
                            {"view": "period"},
                            {"view": "breakdown", "dimensions": ["type"]},
                        ],
                    },
                    {"metric_key": "tasks.bugs_fixed", "views": [{"view": "period"}]},
                    {"metric_key": "tasks.closed_non_bug", "views": [{"view": "period"}]},
                    {"metric_key": "tasks.bugs_ratio", "views": [{"view": "period"}]},
                ],
            },
        }
    )
    assert r.status == 200

    r.row("tasks.closed", "period", entity_id=CAROL).equals(value=3)
    r.row("tasks.bugs_fixed", "period", entity_id=CAROL).equals(value=1)
    r.row("tasks.closed_non_bug", "period", entity_id=CAROL).equals(value=1)

    ratio = one(r.rows("tasks.bugs_ratio", "period"), entity_id=CAROL)
    assert 33.0 < float(ratio["value"]) < 34.0

    by_type = r.breakdown("tasks.closed")
    assert len(some(by_type, entity_id=CAROL)) == 3
    incident = one(by_type, entity_id=CAROL, dimensions={"key": "type", "value": "Incident"})
    assert float(incident["value"]) == 1.0
