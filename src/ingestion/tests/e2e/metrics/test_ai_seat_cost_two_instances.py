"""AI seat cost when one tenant holds two organisations of the same vendor.

Each organisation has its own invoice connector instance and its own seat connector
instance, so nothing in the data says which seats an invoice billed; the operator's
binding names the seat instance its prices reach. Both organisations price a tier of
the same name at different amounts, so a price grain that forgets its instance would
hand one seat population the other's price or drop every seat to absent.
"""

from __future__ import annotations

import pytest
from lib.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "ai_seat_cost_two_instances"

ALICE = "alice@example.com"
CAROL = "carol@example.com"


def test_seat_cost_takes_the_price_of_the_organisation_that_billed_it(spec: SpecRun) -> None:
    """A seat of organisation A takes A's price, never B's."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ALICE]},
                "period": {"from": "2026-11-01", "to": "2026-12-12"},
                "metrics": [{"metric_key": "ai.seat_cost", "views": [{"view": "period"}]}],
            },
        }
    )
    assert r.status == 200
    r.row("ai.seat_cost", "period", entity_id=ALICE).equals(value=25.0)


def test_seat_cost_keeps_the_two_organisations_prices_apart(spec: SpecRun) -> None:
    """The other organisation's seat takes the other price."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [CAROL]},
                "period": {"from": "2026-11-01", "to": "2026-12-12"},
                "metrics": [{"metric_key": "ai.seat_cost", "views": [{"view": "period"}]}],
            },
        }
    )
    assert r.status == 200
    r.row("ai.seat_cost", "period", entity_id=CAROL).equals(value=30.0)
