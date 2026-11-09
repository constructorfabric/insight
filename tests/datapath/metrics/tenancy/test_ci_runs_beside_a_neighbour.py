"""A tenant-grain total counts this tenant's runs alone beside a neighbour.

The neighbour reports runs with the same ids from the same source and repository; a
tenant-grain read sums one tenant, and its breakdown groups one tenant's runs.
"""

from __future__ import annotations

from typing import Any

import pytest
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "ci_runs_beside_a_neighbour"
NEIGHBOUR = "../ci/ci_runs"

PERIOD = {"from": "2026-03-01", "to": "2026-03-07"}


def _trigger(row: dict[str, Any]) -> str:
    return next(str(d["value"]) for d in row["dimensions"] if d["key"] == "trigger")


def test_the_tenant_total_counts_only_this_tenants_runs(spec: SpecRun) -> None:
    """Two runs here, three in the neighbour under the same ids: the total is 2."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "tenant"},
                "period": PERIOD,
                "metrics": [{"metric_key": "ci.runs", "views": [{"view": "period"}]}],
            },
        }
    )
    assert r.status == 200, r.payload

    r.row("ci.runs", "period", entity_id=spec.tenant).equals(value=2.0)


def test_the_tenant_breakdown_holds_only_this_tenants_triggers(spec: SpecRun) -> None:
    """A push and a schedule run here; the neighbour's pull_request run, under the same
    source and repository, never becomes a group of this tenant."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "tenant"},
                "period": PERIOD,
                "metrics": [
                    {
                        "metric_key": "ci.runs",
                        "views": [{"view": "breakdown", "dimensions": ["trigger"]}],
                    }
                ],
            },
        }
    )
    assert r.status == 200, r.payload

    triggers = {_trigger(row): row["value"] for row in r.breakdown("ci.runs")}
    assert triggers == {"push": 1.0, "schedule": 1.0}, f"trigger groups: {triggers}"
