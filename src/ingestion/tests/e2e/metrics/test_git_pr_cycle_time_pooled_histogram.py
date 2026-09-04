"""The pooled histogram bins one distribution per dimension tuple over every
selected entity's events, instead of one distribution per entity.

Seeded over git.pr_cycle_time_h: in acme/api alice merges at 10 h and 30 h and bob
at 20 h and 40 h, so the pool spans [10, 40] where a per-entity shape would close
alice's at 30; in acme/web both merge at 5 h, so the tuple collapses to a single
[5, 5] bin whose count is both people's. Bins are fixed-width over the tuple's own
[min, max]. The dimensionless request still bins per entity; an empty window has no
tuple rows at all.
"""

from __future__ import annotations

import pytest
from lib.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "git_pr_cycle_time_pooled_histogram"

ALICE = "alice@example.com"
BOB = "bob@example.com"


def test_pooled_bins_per_repository(spec: SpecRun) -> None:
    """acme/api pools alice's [10, 30] with bob's [20, 40] so the distribution runs 10..40;
    acme/web's identical values collapse to one bin counting both people, with no entity grain."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ALICE, BOB]},
                "period": {"from": "2026-10-01", "to": "2026-10-03"},
                "metrics": [
                    {
                        "metric_key": "git.pr_cycle_time_h",
                        "views": [{"view": "histogram", "dimensions": ["repository"]}],
                    }
                ],
            },
        }
    )
    assert r.status == 200

    api = r.row("git.pr_cycle_time_h", "histogram", dimensions={"key": "repository", "value": "git-test:acme/api"})
    api.contains(bins={"lo": 10, "count": 1})
    api.contains(bins={"hi": 40, "count": 1})

    web = r.row("git.pr_cycle_time_h", "histogram", dimensions={"key": "repository", "value": "git-test:acme/web"})
    web.contains(bins={"lo": 5, "hi": 5, "count": 2})
    assert "entity_id" not in web.fields


def test_the_dimensionless_request_still_bins_per_entity(spec: SpecRun) -> None:
    """alice's own three merges span [5, 30]; bob's 40 h is not hers."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ALICE, BOB]},
                "period": {"from": "2026-10-01", "to": "2026-10-03"},
                "metrics": [{"metric_key": "git.pr_cycle_time_h", "views": [{"view": "histogram"}]}],
            },
        }
    )
    assert r.status == 200

    alice = r.row("git.pr_cycle_time_h", "histogram", entity_id=ALICE)
    alice.contains(bins={"lo": 5, "count": 1})
    alice.contains(bins={"hi": 30, "count": 1})
    r.row("git.pr_cycle_time_h", "histogram", entity_id=BOB).contains(bins={"hi": 40, "count": 1})


def test_empty_window(spec: SpecRun) -> None:
    """No events, so no tuple has a row: pooled absence, unlike the per-entity shape
    which still lists every requested entity."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ALICE, BOB]},
                "period": {"from": "2025-01-01", "to": "2025-01-31"},
                "metrics": [
                    {
                        "metric_key": "git.pr_cycle_time_h",
                        "views": [{"view": "histogram", "dimensions": ["repository"]}],
                    }
                ],
            },
        }
    )
    assert r.status == 200
    assert len(r.histogram("git.pr_cycle_time_h")) == 0
