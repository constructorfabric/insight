"""Which mailboxes and days count toward emails sent.

Bronze: the daily M365 email-activity report per mailbox with the user's assigned
products. Staging drops a mailbox whose product list is explicitly empty (an
unlicensed account) and keeps one whose list is unknown. Gold sums a person's emails
sent over the window with both bounds inclusive, adding up every mailbox bound to
that person.
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


def _period(ids: list[str], date_from: str, date_to: str) -> dict:
    return {
        "url": "/v1/metric-results",
        "method": "POST",
        "body": {
            "entity": {"type": "person", "ids": ids},
            "period": {"from": date_from, "to": date_to},
            "metrics": [{"metric_key": "collab.emails_sent", "views": [{"view": "period"}]}],
        },
    }


def test_window_bounds_are_inclusive(spec: SpecRun) -> None:
    """December takes Dec 1 and Dec 31 (10 + 100) and leaves out Nov 30 and Jan 1."""
    r = spec.call(_period([ALICE], "2026-12-01", "2026-12-31"))
    assert r.status == 200
    r.row("collab.emails_sent", "period", entity_id=ALICE).equals(value=110)


def test_single_day_window(spec: SpecRun) -> None:
    """A one-day window on the upper bound takes that day only."""
    r = spec.call(_period([ALICE], "2026-12-31", "2026-12-31"))
    assert r.status == 200
    r.row("collab.emails_sent", "period", entity_id=ALICE).equals(value=100)


def test_unlicensed_mailbox_is_dropped(spec: SpecRun) -> None:
    """An empty product list, with or without a space, drops the mailbox: an honest null, not 15."""
    r = spec.call(_period([BOB], "2026-12-01", "2026-12-31"))
    assert r.status == 200
    r.row("collab.emails_sent", "period", entity_id=BOB).equals(value=None)


def test_unknown_licence_is_kept(spec: SpecRun) -> None:
    """A report row without a product list is kept."""
    r = spec.call(_period([CAROL], "2026-12-01", "2026-12-31"))
    assert r.status == 200
    r.row("collab.emails_sent", "period", entity_id=CAROL).equals(value=5)


def test_two_mailboxes_of_one_person_add_up(spec: SpecRun) -> None:
    """dave's main mailbox and the one bound to him count as one person: 3 + 4."""
    r = spec.call(_period([DAVE], "2026-12-01", "2026-12-31"))
    assert r.status == 200
    r.row("collab.emails_sent", "period", entity_id=DAVE).equals(value=7)
