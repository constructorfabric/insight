"""Deployments created, with the outcome folded from the latest status event, at tenant grain.

Bronze: deployment records (no outcome) and deployment status events arrive as two
streams. Silver: both accumulate as-is; the status stream is an event log. Gold: each
deployment takes its latest status event as outcome; one with no event yet is
'pending' and stays visible, never rounded away. Seeded: a straight success to
production, a transient preview environment with no status yet, and a static staging
environment whose failure was later superseded by success -- success 2, pending 1.
"""

from __future__ import annotations

import pytest
from lib.metric_expect import one, some
from lib.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "ci_deployments"

TENANT = "11111111-1111-1111-1111-111111111111"


def test_deployments_fold_the_latest_status_and_keep_pending_visible(spec: SpecRun) -> None:
    """Period total 3; the outcome breakdown proves the fold (success 2) and the visible
    pending (1); the Mar 01 bucket holds the one production deployment."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "tenant"},
                "period": {"from": "2026-03-01", "to": "2026-03-07"},
                "metrics": [
                    {
                        "metric_key": "ci.deployments",
                        "views": [
                            {"view": "period"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["outcome"]},
                        ],
                    }
                ],
            },
        }
    )
    assert r.status == 200

    r.row("ci.deployments", "period", entity_id=TENANT).equals(value=3.0)
    by_outcome = r.breakdown("ci.deployments")
    assert float(one(by_outcome, dimensions={"key": "outcome", "value": "success"})["value"]) == 2.0
    assert float(one(by_outcome, dimensions={"key": "outcome", "value": "pending"})["value"]) == 1.0
    assert any(some(s["points"], bucket_start="2026-03-01", value=1.0) for s in r.series("ci.deployments"))


def test_env_kind_splits_production_transient_preview_and_static(spec: SpecRun) -> None:
    """The env_kind breakdown serves production, preview and static at one each."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "tenant"},
                "period": {"from": "2026-03-01", "to": "2026-03-07"},
                "metrics": [
                    {"metric_key": "ci.deployments", "views": [{"view": "breakdown", "dimensions": ["env_kind"]}]}
                ],
            },
        }
    )
    assert r.status == 200

    by_env_kind = r.breakdown("ci.deployments")
    assert float(one(by_env_kind, dimensions={"key": "env_kind", "value": "production"})["value"]) == 1.0
    assert float(one(by_env_kind, dimensions={"key": "env_kind", "value": "preview"})["value"]) == 1.0
    assert float(one(by_env_kind, dimensions={"key": "env_kind", "value": "static"})["value"]) == 1.0
