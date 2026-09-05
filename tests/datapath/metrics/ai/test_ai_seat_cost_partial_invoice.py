"""AI seat cost with two seat populations, one invoice, and no binding.

A tenant holds two organisations of the vendor, so two seat connector instances
report seats, and only one of them produced an invoice this month; the binding
table is empty. There is exactly one price but nothing says which seat population
it was charged for, so it reaches neither: not the organisation that did not
invoice, and not even the one that did.
"""

from __future__ import annotations

import pytest
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "ai_seat_cost_partial_invoice"

ALICE = "alice@example.com"
CAROL = "carol@example.com"


def test_seat_cost_does_not_lend_one_organisations_price_to_anothers_seats(spec: SpecRun) -> None:
    """The organisation with no invoice is not given the other's price."""
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
    r.row("ai.seat_cost", "period", entity_id=CAROL).equals(value=None)


def test_seat_cost_withholds_an_unbound_price_with_two_seat_populations(spec: SpecRun) -> None:
    """Nor is the invoicing organisation's own seat priced on a guess."""
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
    r.row("ai.seat_cost", "period", entity_id=ALICE).equals(value=None)
