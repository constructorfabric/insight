"""A Zoom joiner no Zoom user is named after attributes through the guest account
an operator binds.

The session is kept under a guest account keyed on the display name, with no email,
and the gold row carries that account instead of an email. The analytics runtime
resolves it through the account binding, so the operator's decision is what makes
the meetings count; an unbound guest account counts for nobody. alice (signed in)
is the control proving the pipeline is healthy.
"""

from __future__ import annotations

import pytest
from insight_datapath.spec_runner import SpecRun

pytestmark = [pytest.mark.fixture, pytest.mark.shared_dataset]

SPEC = "collab_meetings_guest_binding"

ALICE = "alice@example.com"
BOB = "bob@example.com"
CAROL = "carol@example.com"

PERIOD = {"from": "2026-12-01", "to": "2026-12-31"}


def test_bound_guest_account_attributes_its_sessions(spec: SpecRun) -> None:
    """bob's two unsigned sessions reach him through the binding of the guest
    account "bobby b"; alice is the signed-in control."""
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

    r.row("collab.meetings_count", "period", entity_id=ALICE).equals(value=2)
    r.row("collab.meetings_count", "period", entity_id=BOB).equals(value=2)
    r.row("collab.meeting_hours", "period", entity_id=ALICE).equals(value=2)
    r.row("collab.meeting_hours", "period", entity_id=BOB).equals(value=2)


def test_unbound_guest_account_attributes_nobody(spec: SpecRun) -> None:
    """carol's sessions sit under a guest account nobody bound, so she reads null."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [CAROL]},
                "period": PERIOD,
                "metrics": [{"metric_key": "collab.meetings_count", "views": [{"view": "period"}]}],
            },
        }
    )
    assert r.status == 200

    r.row("collab.meetings_count", "period", entity_id=CAROL).equals(value=None)
