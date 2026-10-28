"""A Zoom participant who joined without signing in is attributed through the
account's user list.

Zoom reports no email for a joiner who did not sign in, only the display name typed
into the client. Silver takes the email of the one Zoom user whose display name equals
it, ignoring case and surrounding spaces. A name two Zoom users carry, or none, stays
unattributed. alice (signed in) is the control proving the pipeline is healthy.
"""

from __future__ import annotations

import pytest
from insight_datapath.spec_runner import SpecRun

pytestmark = [pytest.mark.fixture, pytest.mark.shared_dataset]

SPEC = "collab_meetings_unsigned_zoom"

ALICE = "alice@example.com"
BOB = "bob@example.com"
CAROL = "carol@example.com"
DAVE = "dave@example.com"
ERIN = "erin@example.com"

PERIOD = {"from": "2026-12-01", "to": "2026-12-31"}


def test_unsigned_joiner_with_a_unique_display_name_is_counted(spec: SpecRun) -> None:
    """bob's unsigned session resolves through the one Zoom user named "Bob Beta",
    whatever the case and spacing typed into the client; alice is the signed-in control."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ALICE, BOB]},
                "period": PERIOD,
                "metrics": [
                    {"metric_key": "collab.meetings_count", "views": [{"view": "period"}]},
                    {"metric_key": "collab.meeting_hours", "views": [{"view": "period"}]},
                ],
            },
        }
    )
    assert r.status == 200

    r.row("collab.meetings_count", "period", entity_id=ALICE).equals(value=1)
    r.row("collab.meetings_count", "period", entity_id=BOB).equals(value=1)
    r.row("collab.meeting_hours", "period", entity_id=ALICE).equals(value=1)
    r.row("collab.meeting_hours", "period", entity_id=BOB).equals(value=1)


def test_shared_or_unknown_display_name_attributes_nobody(spec: SpecRun) -> None:
    """ "Team Lead" is carried by two Zoom users, so carol's unsigned session lands on
    neither her nor dave; "Erin E" is carried by none, so erin's lands nowhere."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [CAROL, DAVE, ERIN]},
                "period": PERIOD,
                "metrics": [{"metric_key": "collab.meetings_count", "views": [{"view": "period"}]}],
            },
        }
    )
    assert r.status == 200

    r.row("collab.meetings_count", "period", entity_id=CAROL).equals(value=None)
    r.row("collab.meetings_count", "period", entity_id=DAVE).equals(value=None)
    r.row("collab.meetings_count", "period", entity_id=ERIN).equals(value=None)
