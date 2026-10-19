"""Which mailboxes and days count toward emails sent.

Bronze: the daily M365 email-activity report per mailbox with the user's assigned
products. Staging drops a report row whose product list is empty (an unlicensed
account) and keeps one with no list at all. Gold sums a person's emails sent over
the window with both bounds inclusive, adding up every mailbox bound to that person.
"""

from __future__ import annotations

import pytest
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "collab_emails_sent_scope"

ALICE = "alice@example.com"
BOB = "bob@example.com"
CAROL = "carol@example.com"
DAVE = "dave@example.com"


@pytest.mark.parametrize(
    ("person", "date_from", "date_to", "expected"),
    [
        pytest.param(ALICE, "2026-12-01", "2026-12-31", 110, id="window-bounds-are-inclusive"),
        pytest.param(
            ALICE, "2026-12-31", "2026-12-31", 100, id="single-day-window-takes-that-day-only"
        ),
        pytest.param(BOB, "2026-12-01", "2026-12-31", 2, id="unlicensed-days-are-dropped"),
        pytest.param(CAROL, "2026-12-01", "2026-12-31", 5, id="row-without-product-list-is-kept"),
        pytest.param(DAVE, "2026-12-01", "2026-12-31", 7, id="two-mailboxes-of-one-person-add-up"),
    ],
)
def test_emails_sent_counts_the_right_mailboxes_and_days(
    spec: SpecRun, person: str, date_from: str, date_to: str, expected: int
) -> None:
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [person]},
                "period": {"from": date_from, "to": date_to},
                "metrics": [{"metric_key": "collab.emails_sent", "views": [{"view": "period"}]}],
            },
        }
    )
    assert r.status == 200, f"should answer 200 for {person} {date_from}..{date_to}"
    r.row("collab.emails_sent", "period", entity_id=person).equals(value=expected)
