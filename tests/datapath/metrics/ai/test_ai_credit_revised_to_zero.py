"""Codex: a charge the vendor revises down to nothing stops being charged.

The credit contributor emits zero-credit person-days on purpose. Its relation is
keyed on (tenant, source, email, date) and not on the figure, so a corrected
reading can only replace the one it supersedes by arriving as a row; suppressing
zeros would leave the earlier positive reading stored with nothing able to
withdraw it.

The monetary branch of gold then requires credits > 0, so the stored zero serves
no charge rather than a $0 one. Those are different answers and only the first
is true.

Scope: both readings are seeded before one build, so this covers the collapse to
the newest version within a run and gold's refusal to charge the zero it settles
on. It does not cover the incremental path — a correction arriving in a later run
over a table that already holds the positive row — which the spec runner cannot
express without a second seed-and-rebuild phase. The fixture says so at length.
"""

from __future__ import annotations

import pytest
from insight_datapath.metric_expect import approx
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "ai_credit_revised_to_zero"

ALICE = "alice@example.com"
BOB = "bob@example.com"

WINDOW = {"from": "2026-12-01", "to": "2026-12-31"}


def _daily_extra_usage(spec: SpecRun, person: str) -> object:
    return spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [person]},
                "period": WINDOW,
                "metrics": [
                    {
                        "metric_key": "ai.daily_approximate_extra_usage_cost",
                        "views": [{"view": "period"}],
                    }
                ],
            },
        }
    )


def test_a_withdrawn_charge_serves_nothing(spec: SpecRun) -> None:
    """alice's 500 credits were revised to zero, so she is charged nothing at all.

    Not zero — nothing. A $0 row would assert the day was measured and cost
    nothing, where the truth is that the charge was taken back.
    """
    r = _daily_extra_usage(spec, ALICE)
    assert r.status == 200
    r.row("ai.daily_approximate_extra_usage_cost", "period", entity_id=ALICE).equals(value=None)


def test_an_unrevised_charge_survives(spec: SpecRun) -> None:
    """bob's reading never changed, so his charge is served in full.

    The control. Without it a filter that dropped every credit row would pass
    the case above for the wrong reason.

    200 credits x 4 minor units = 800 billed, presented at 1.08 => 864 cents.
    """
    r = _daily_extra_usage(spec, BOB)
    assert r.status == 200
    r.row("ai.daily_approximate_extra_usage_cost", "period", entity_id=BOB).equals(
        value=approx(8.64)
    )


def test_the_corrected_reading_is_what_silver_stores(spec: SpecRun) -> None:
    """Silver holds alice's zero, not the 500 it superseded.

    Read through the serving layer rather than the relation: the point is that
    the later reading won, and a stored 500 would surface here as a charge.
    """
    r = _daily_extra_usage(spec, ALICE)
    assert r.status == 200
    row = r.row("ai.daily_approximate_extra_usage_cost", "period", entity_id=ALICE)
    row.equals(value=None)
