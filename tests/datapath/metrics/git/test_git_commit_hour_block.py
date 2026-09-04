"""Commits carry the two-hour block they landed in, so a repository screen can draw
when work happens without a second request.

Alice commits at 09:14, 09:52 and 15:03 UTC one day, at 09:30 the next and at 21:05
that evening on a branch that has not landed: blocks 08 three times, 14 once, 20 once.
The `hour_block` breakdown sums to the commit total, each branch scope answers over the
same dimension, and `git.active_days` stays 2 because the block rides on commit rows only.
"""

from __future__ import annotations

import pytest
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "git_commit_hour_block"

ALICE = "alice@example.com"


def hour_block(value: str) -> dict[str, str]:
    return {"key": "hour_block", "value": value}


def test_commits_break_down_by_the_block_they_landed_in(spec: SpecRun) -> None:
    """09:14, 09:52 and 09:30 all fall in the 08-10 block; the label is what a heatmap
    axis shows."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ALICE]},
                "period": {"from": "2026-10-01", "to": "2026-10-03"},
                "metrics": [
                    {
                        "metric_key": "git.commits",
                        "views": [
                            {"view": "period"},
                            {"view": "breakdown", "dimensions": ["hour_block"]},
                        ],
                    }
                ],
            },
        }
    )
    assert r.status == 200

    r.row("git.commits", "period", entity_id=ALICE).equals(value=5)
    r.row("git.commits", "breakdown", entity_id=ALICE, dimensions=hour_block("08")).equals(value=3)
    block_14 = r.row("git.commits", "breakdown", entity_id=ALICE, dimensions=hour_block("14"))
    block_14.equals(value=1)
    block_14.contains(dimensions={"key": "hour_block", "label": "14-16"})


def test_each_branch_scope_breaks_down_over_the_same_dimension(spec: SpecRun) -> None:
    """The trunk keeps the three 08-block commits; the block of the one still in flight
    belongs to the non-default reading."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ALICE]},
                "period": {"from": "2026-10-01", "to": "2026-10-03"},
                "metrics": [
                    {
                        "metric_key": "git.default_branch_commits",
                        "views": [{"view": "breakdown", "dimensions": ["hour_block"]}],
                    },
                    {
                        "metric_key": "git.non_default_branch_commits",
                        "views": [{"view": "breakdown", "dimensions": ["hour_block"]}],
                    },
                ],
            },
        }
    )
    assert r.status == 200

    r.row(
        "git.default_branch_commits", "breakdown", entity_id=ALICE, dimensions=hour_block("08")
    ).equals(value=3)
    r.row(
        "git.non_default_branch_commits", "breakdown", entity_id=ALICE, dimensions=hour_block("20")
    ).equals(value=1)


def test_the_block_does_not_split_an_active_day(spec: SpecRun) -> None:
    """Two calendar days, however many blocks the commits fell into."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ALICE]},
                "period": {"from": "2026-10-01", "to": "2026-10-03"},
                "metrics": [{"metric_key": "git.active_days", "views": [{"view": "period"}]}],
            },
        }
    )
    assert r.status == 200
    r.row("git.active_days", "period", entity_id=ALICE).equals(value=2)
