"""AI seat cost on an installation where no seat-tier binding has been written.

Bronze: two billable Claude Team seats on different tiers, and a December invoice
carrying exactly one priced subscription line; `config.ai_seat_tier_map` is empty.
A month that prices exactly one tier needs no binding: that amount is the only
per-seat price the vendor stated, so every billable seat takes it whatever its tier.
"""

from __future__ import annotations

import pytest
from lib.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "ai_seat_cost_unbound"

ALICE = "alice@example.com"
BOB = "bob@example.com"


def test_ai_seat_cost_with_no_binding_takes_the_months_only_price(spec: SpecRun) -> None:
    """The seat whose tier the single price does not name still takes that price."""
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
    r.row("ai.seat_cost", "period", entity_id=ALICE).equals(value=18.0)


def test_ai_seat_cost_with_no_binding_does_not_depend_on_the_seats_tier(spec: SpecRun) -> None:
    """A seat on another tier takes the same price, because there is only one."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [BOB]},
                "period": {"from": "2026-11-01", "to": "2026-12-12"},
                "metrics": [{"metric_key": "ai.seat_cost", "views": [{"view": "period"}]}],
            },
        }
    )
    assert r.status == 200
    r.row("ai.seat_cost", "period", entity_id=BOB).equals(value=18.0)
