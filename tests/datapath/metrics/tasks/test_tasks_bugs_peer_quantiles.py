"""Peer quantiles for bugs_fixed and bugs_ratio once the pool clears MIN_PEER_N.

Six department members; alice..erin of rank r each close 5 issues on one day, r bugs
(each person's bugs carrying one bug-name alias: Defect, Regression, padded DEFECT,
translated name with untranslatedName Bug, literal Bug) and (5-r) tasks, so bugs_fixed
spreads {1..5} and bugs_ratio {20..100}. heidi closes 2 tasks and 0 bugs: sumIfOrNull
computes NULL for her, she drops out of both peer pools, and the 5 observed members
disclose non-NULL p25/median/p75/min/max.
"""

from __future__ import annotations

import pytest
from insight_datapath.metric_expect import approx, one
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "tasks_bugs_peer_quantiles"

ALICE = "alice@example.com"
BOB = "bob@example.com"
CAROL = "carol@example.com"
DAVE = "dave@example.com"
ERIN = "erin@example.com"
HEIDI = "heidi@example.com"


def test_peer_quantiles_disclosed(spec: SpecRun) -> None:
    """Erin (rank 5) closes 5 bugs of 5 issues; the peer pool of 5 observed members
    clears MIN_PEER_N, so the quantiles disclose over {1..5} and {20..100}."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-06-20", "to": "2026-06-30"},
                "metrics": [
                    {
                        "metric_key": "tasks.bugs_fixed",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                        ],
                    },
                    {
                        "metric_key": "tasks.bugs_ratio",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                        ],
                    },
                ],
            },
        }
    )
    assert r.status == 200

    # p25/p75 follow ClickHouse quantilesExactIf element selection over the
    # 5-member pool; confirmed against a live run.
    r.row("tasks.bugs_fixed", "period", entity_id=ERIN).equals(value=5)
    r.row("tasks.bugs_fixed", "peer", entity_id=ERIN).equals(
        target_value=5, p25=2, median=3, p75=4, min=1, max=5, n=5
    )
    fixed = one(r.series("tasks.bugs_fixed"), entity_id=ERIN)["points"]
    assert float(one(fixed, bucket_start="2026-06-25")["value"]) == approx(5.0)

    r.row("tasks.bugs_ratio", "period", entity_id=ERIN).equals(value=100)
    r.row("tasks.bugs_ratio", "peer", entity_id=ERIN).equals(
        target_value=100, p25=40, median=60, p75=80, min=20, max=100, n=5
    )


def test_zero_bug_member_has_no_observation(spec: SpecRun) -> None:
    """Heidi closes 2 tasks and 0 bugs: her bugs_fixed and bugs_ratio are NULL, she is
    excluded from the pool, yet the 5 observed members still disclose the quantiles."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [HEIDI]},
                "period": {"from": "2026-06-20", "to": "2026-06-30"},
                "metrics": [
                    {
                        "metric_key": "tasks.bugs_fixed",
                        "views": [{"view": "period"}, {"view": "peer"}],
                    },
                    {"metric_key": "tasks.bugs_ratio", "views": [{"view": "period"}]},
                    {"metric_key": "tasks.closed", "views": [{"view": "period"}]},
                ],
            },
        }
    )
    assert r.status == 200

    r.row("tasks.bugs_fixed", "period", entity_id=HEIDI).equals(value=None)
    r.row("tasks.bugs_ratio", "period", entity_id=HEIDI).equals(value=None)
    r.row("tasks.closed", "period", entity_id=HEIDI).equals(value=2)
    r.row("tasks.bugs_fixed", "peer", entity_id=HEIDI).equals(
        target_value=None, p25=2, median=3, p75=4, min=1, max=5, n=5
    )


@pytest.mark.parametrize(
    ("email", "bugs_fixed"),
    [
        (ALICE, 1),
        (BOB, 2),
        (CAROL, 3),
        (DAVE, 4),
        (ERIN, 5),
    ],
    ids=["Defect", "Regression", "padded DEFECT", "untranslatedName Bug", "literal Bug"],
)
def test_alias_types_classify_as_bug(spec: SpecRun, email: str, bugs_fixed: int) -> None:
    """Each member's bugs carry one bug-name alias; every alias counts as a bug."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [email]},
                "period": {"from": "2026-06-20", "to": "2026-06-30"},
                "metrics": [{"metric_key": "tasks.bugs_fixed", "views": [{"view": "period"}]}],
            },
        }
    )
    assert r.status == 200
    r.row("tasks.bugs_fixed", "period", entity_id=email).equals(value=bugs_fixed)
