"""Bug metrics across the issue lifecycle: two sources on one person, a reopen and
re-close, and a type change after the close.

Bronze: a Jira Bug and a GitHub Bug both closed by bob on one day; a Jira Bug carol
closes, reopens and re-closes; and a Jira issue dave closes as a Bug that a later
changelog row retypes to Task, the type the current snapshot carries. The person
aggregate sums across sources by design, a reopened bug counts once on its final
close day, and the kind of a closed issue follows its CURRENT type.
"""

from __future__ import annotations

import pytest
from insight_datapath.metric_expect import approx, one
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "tasks_bugs_lifecycle"

BOB = "bob@example.com"
CAROL = "carol@example.com"
DAVE = "dave@example.com"


def test_bug_closes_sum_across_sources_for_one_person(spec: SpecRun) -> None:
    """Bob closes one Jira Bug and one GitHub Bug on 2026-06-25: the person aggregate
    carries no source dimension, so bugs_fixed and tasks.closed are each 2 and
    bugs_ratio is 100."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [BOB]},
                "period": {"from": "2026-06-20", "to": "2026-06-30"},
                "metrics": [
                    {"metric_key": "tasks.bugs_fixed", "views": [{"view": "period"}]},
                    {"metric_key": "tasks.closed", "views": [{"view": "period"}]},
                    {"metric_key": "tasks.bugs_ratio", "views": [{"view": "period"}]},
                ],
            },
        }
    )
    assert r.status == 200

    r.row("tasks.bugs_fixed", "period", entity_id=BOB).equals(value=2)
    r.row("tasks.closed", "period", entity_id=BOB).equals(value=2)
    r.row("tasks.bugs_ratio", "period", entity_id=BOB).equals(value=100)


def test_reopened_bug_counts_once_on_its_final_close_day(spec: SpecRun) -> None:
    """Carol's bug closes 2026-06-20, reopens and re-closes 2026-06-27: bugs_fixed is 1
    over the window, scored on the re-close day only — the first close day serves
    nothing."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [CAROL]},
                "period": {"from": "2026-06-20", "to": "2026-06-30"},
                "metrics": [
                    {
                        "metric_key": "tasks.bugs_fixed",
                        "views": [{"view": "period"}, {"view": "timeseries", "bucket": "day"}],
                    }
                ],
            },
        }
    )
    assert r.status == 200

    r.row("tasks.bugs_fixed", "period", entity_id=CAROL).equals(value=1)

    points = one(r.series("tasks.bugs_fixed"), entity_id=CAROL)["points"]
    scored = [point for point in points if point["value"] is not None]
    assert len(scored) == 1, f"expected one scored day, got {scored!r}"
    assert scored[0]["bucket_start"] == "2026-06-27"
    assert float(scored[0]["value"]) == approx(1.0)


def test_type_change_after_close_reclassifies_the_close(spec: SpecRun) -> None:
    """Dave's issue was a Bug when it closed, but the current snapshot says Task after
    the 2026-06-26 retype, and the kind joins on the CURRENT type: the close counts in
    closed_non_bug, not bugs_fixed. Pins current behavior (snapshot type wins), not
    intent."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [DAVE]},
                "period": {"from": "2026-06-20", "to": "2026-06-30"},
                "metrics": [
                    {"metric_key": "tasks.bugs_fixed", "views": [{"view": "period"}]},
                    {"metric_key": "tasks.closed_non_bug", "views": [{"view": "period"}]},
                    {"metric_key": "tasks.closed", "views": [{"view": "period"}]},
                ],
            },
        }
    )
    assert r.status == 200

    r.row("tasks.bugs_fixed", "period", entity_id=DAVE).equals(value=None)
    r.row("tasks.closed_non_bug", "period", entity_id=DAVE).equals(value=1)
    r.row("tasks.closed", "period", entity_id=DAVE).equals(value=1)
