"""One person with three source accounts active on the same day resolves as one.

alice holds a second Cursor account under alice.dev@example.com and a third recorded
with different case and surrounding space; `identity_aliases` binds all of them to
her person. ai.active_days is a day flag, so it serves 1, not 3: the accounts collapse
before the days are summed. ai.accepted_lines is additive and keeps summing across the
accounts (10 + 20 + 5). A person with several accounts is one peer, so the pool is 3.
"""

from __future__ import annotations

import pytest
from insight_datapath.metric_expect import approx, one
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "identity_alias_collapse"

ALICE = "alice@example.com"


def test_two_accounts_of_one_person_count_as_one_active_day(spec: SpecRun) -> None:
    """One day however many accounts were active in it; the additive lines still sum
    across all three accounts, and alice is one peer of three."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ALICE]},
                "period": {"from": "2026-01-01", "to": "2026-01-31"},
                "metrics": [
                    {
                        "metric_key": "ai.active_days",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                        ],
                    },
                    {"metric_key": "ai.accepted_lines", "views": [{"view": "period"}]},
                ],
            },
        }
    )
    assert r.status == 200

    r.row("ai.active_days", "period", entity_id=ALICE).equals(value=1)
    r.row("ai.active_days", "peer", entity_id=ALICE).equals(
        target_value=1, p25=None, median=None, p75=None, min=None, max=None, n=3
    )
    active = one(r.series("ai.active_days"), entity_id=ALICE)["points"]
    assert float(one(active, bucket_start="2026-01-05")["value"]) == approx(1.0)

    r.row("ai.accepted_lines", "period", entity_id=ALICE).equals(value=35)
