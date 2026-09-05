"""CI gate first-try pass rate: gate runs that went green without a retry, at tenant grain.

Bronze: workflow run attempts. Silver: a run counts once at its last attempt, and it is
first-try when that attempt is attempt 1 and it passed. Gold: first-try passes over gate
runs. Seeded four gate runs -- two first-try successes, one success that needed a retry,
one failure -- so the rate is 2/4 = 50%; the retried success lifts the plain pass rate
but not this one, and the gap is the retry tax.
"""

from __future__ import annotations

import pytest
from insight_datapath.metric_expect import approx, one, some
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "ci_gate_first_try_pass_rate"


def test_a_retried_success_is_green_but_not_first_try_green(spec: SpecRun) -> None:
    """Period reads 50, the Mar 01 daily point reads 50, and the acme/app repository
    breakdown reads 50."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "tenant"},
                "period": {"from": "2026-03-01", "to": "2026-03-07"},
                "metrics": [
                    {
                        "metric_key": "ci.gate_first_try_pass_rate",
                        "views": [
                            {"view": "period"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["repository"]},
                        ],
                    }
                ],
            },
        }
    )
    assert r.status == 200

    r.row("ci.gate_first_try_pass_rate", "period", entity_id=spec.tenant).equals(value=50.0)
    assert any(
        some(entry["points"], bucket_start="2026-03-01", value=50.0)
        for entry in r.series("ci.gate_first_try_pass_rate")
    )
    by_repository = one(
        r.breakdown("ci.gate_first_try_pass_rate"),
        dimensions={"key": "repository", "value": "acme/app"},
    )
    assert float(by_repository["value"]) == approx(50.0)
