"""A Microsoft 365 report day the source never computed is not counted.

Bronze: the daily M365 email-activity and Teams-activity reports. When Microsoft has
no report for a day, Graph answers with a copy of the last computed day stamped with
the requested date, so a copy holds rows with activity whose lastActivityDate is
earlier than the report day. The staging feeders drop such a day whole, for every
M365 report alike, before silver. A quiet day and a day without lastActivityDate are
real and kept, and a receive-only mailbox keeps its row on a real day, because
receiving mail does not move lastActivityDate. Gold sums the kept days only.
"""

from __future__ import annotations

import pytest
from insight_datapath.metric_expect import one
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "collab_m365_reported_days"

ALICE = "alice@example.com"
BOB = "bob@example.com"

WINDOW = {"from": "2026-12-23", "to": "2026-12-29"}


def _period_request(ids: list[str], metric_keys: list[str]) -> dict:
    return {
        "url": "/v1/metric-results",
        "method": "POST",
        "body": {
            "entity": {"type": "person", "ids": ids},
            "period": WINDOW,
            "metrics": [{"metric_key": key, "views": [{"view": "period"}]} for key in metric_keys],
        },
    }


def test_email_counts_exclude_the_carried_forward_day(spec: SpecRun) -> None:
    """Dec 24, Dec 26, the unknown Dec 27 and the quiet Dec 28 count; the Dec 25 copy of Dec 24 does not: 10+3+1 sent, 30+9+1+2 received, 5+2+1 read."""
    r = spec.call(
        _period_request(
            [ALICE], ["collab.emails_sent", "collab.emails_received", "collab.emails_read"]
        )
    )
    assert r.status == 200
    r.row("collab.emails_sent", "period", entity_id=ALICE).equals(value=14)
    r.row("collab.emails_received", "period", entity_id=ALICE).equals(value=42)
    r.row("collab.emails_read", "period", entity_id=ALICE).equals(value=8)


def test_receive_only_mailbox_keeps_its_real_day(spec: SpecRun) -> None:
    """bob's last activity predates Dec 24, yet the day is real (alice proves it), so his 7 received stay; the Dec 25 copy adds nothing."""
    r = spec.call(_period_request([BOB], ["collab.emails_received"]))
    assert r.status == 200
    r.row("collab.emails_received", "period", entity_id=BOB).equals(value=7)


def test_teams_chat_excludes_the_carried_forward_day(spec: SpecRun) -> None:
    """The same copy pattern in the Teams report: 4 messages once, not twice."""
    r = spec.call(_period_request([ALICE], ["collab.messages_sent"]))
    assert r.status == 200
    r.row("collab.messages_sent", "period", entity_id=ALICE).equals(value=4)


def test_daily_series_has_no_value_on_the_dropped_day(spec: SpecRun) -> None:
    """The series carries 10 on Dec 24, 3 on Dec 26, 1 on Dec 27 and 0 on the quiet Dec 28, and nothing on Dec 25."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ALICE]},
                "period": WINDOW,
                "metrics": [
                    {
                        "metric_key": "collab.emails_sent",
                        "views": [{"view": "timeseries", "bucket": "day"}],
                    }
                ],
            },
        }
    )
    assert r.status == 200
    points = one(r.series("collab.emails_sent"), entity_id=ALICE)["points"]
    assert float(one(points, bucket_start="2026-12-24")["value"]) == 10.0
    assert float(one(points, bucket_start="2026-12-26")["value"]) == 3.0
    assert float(one(points, bucket_start="2026-12-27")["value"]) == 1.0
    assert float(one(points, bucket_start="2026-12-28")["value"]) == 0.0
    assert one(points, bucket_start="2026-12-25")["value"] is None
