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
from insight_datapath import clickhouse
from insight_datapath.instance import InstanceConfig
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


def test_the_corrected_reading_is_what_silver_stores(
    spec: SpecRun, instance_cfg: InstanceConfig
) -> None:
    """Silver holds one row for alice's day, and its credits are zero.

    Read from the relation, not through the serving layer: the two assertions
    above prove gold serves no charge, which a row that never reached silver at
    all would also satisfy. Only the relation can say that the later reading
    replaced the earlier one instead of being dropped.
    """
    rows = clickhouse.query(
        instance_cfg,
        f"""
        SELECT toString(credits)
        FROM silver.class_ai_credit_usage FINAL
        WHERE insight_tenant_id = '{spec.tenant}'
          AND source_id = 'chatgpt-team-test'
          AND email = '{ALICE}'
          AND day = toDate('2026-12-02')
        """,
    )

    assert len(rows) == 1, f"expected one stored reading for alice's day, got {rows}"
    assert float(rows[0][0]) == 0.0, f"the withdrawn charge is still stored: {rows}"
