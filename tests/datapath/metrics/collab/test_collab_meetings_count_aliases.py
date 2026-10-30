"""A meeting counts once for a person however many of their addresses joined it.

Bronze: Zoom meetings with participant sessions and the daily Teams report, each
under one of a person's two addresses. Staging names the logical meetings behind a
Zoom day's count. Gold emits one Zoom row per attended meeting with the meeting as
the subject, and the runtime folds the person's addresses per subject: a meeting
joined from two addresses is one meeting, while a Teams day under each address
adds up as before.
"""

from __future__ import annotations

import pytest
from insight_datapath.metric_expect import one
from insight_datapath.spec_runner import SpecRun

pytestmark = [pytest.mark.fixture, pytest.mark.shared_dataset]

SPEC = "collab_meetings_count_aliases"

ALICE = "alice@example.com"
BOB = "bob@example.com"


def _request(person: str, date_from: str, date_to: str) -> dict[str, object]:
    return {
        "url": "/v1/metric-results",
        "method": "POST",
        "body": {
            "entity": {"type": "person", "ids": [person]},
            "period": {"from": date_from, "to": date_to},
            "metrics": [
                {
                    "metric_key": "collab.meetings_count",
                    "views": [{"view": "period"}, {"view": "breakdown", "dimensions": ["tool"]}],
                }
            ],
        },
    }


def test_a_meeting_joined_from_two_addresses_counts_once(spec: SpecRun) -> None:
    r = spec.call(_request(ALICE, "2026-12-10", "2026-12-10"))
    assert r.status == 200, f"should answer 200 for {ALICE} on the Zoom day"
    r.row("collab.meetings_count", "period", entity_id=ALICE).equals(value=3)
    row = one(
        r.breakdown("collab.meetings_count"),
        entity_id=ALICE,
        dimensions={"key": "tool", "value": "zoom"},
    )
    assert float(row["value"]) == 3, f"should count 3 Zoom meetings, not one per address: {row}"


def test_daily_reports_under_each_address_add_up(spec: SpecRun) -> None:
    r = spec.call(_request(ALICE, "2026-12-11", "2026-12-11"))
    assert r.status == 200, f"should answer 200 for {ALICE} on the Teams day"
    r.row("collab.meetings_count", "period", entity_id=ALICE).equals(value=3)
    row = one(
        r.breakdown("collab.meetings_count"),
        entity_id=ALICE,
        dimensions={"key": "tool", "value": "m365"},
    )
    assert float(row["value"]) == 3, f"should add the Teams counts of both addresses: {row}"


def test_meetings_under_either_address_add_up(spec: SpecRun) -> None:
    r = spec.call(_request(ALICE, "2026-12-01", "2026-12-31"))
    assert r.status == 200, f"should answer 200 for {ALICE} in December"
    r.row("collab.meetings_count", "period", entity_id=ALICE).equals(value=6)


def test_the_other_attendee_counts_the_shared_meeting_once(spec: SpecRun) -> None:
    r = spec.call(_request(BOB, "2026-12-10", "2026-12-10"))
    assert r.status == 200, f"should answer 200 for {BOB} on the Zoom day"
    r.row("collab.meetings_count", "period", entity_id=BOB).equals(value=3)
