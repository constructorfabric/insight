"""A branch-scoped merge is counted on the day it merged, not the day it was opened.

Which destination lands on which side of the split is owned by
`git_default_branch_prs_created`. What no fixture separated is the DATE: every request
touching these keys was created and merged on the same day, so a rule dating a merge by
the request's creation read identically. Here the two dates fall in different months, and
each month's merges and creations come from disjoint sets of requests. #3063
"""

from __future__ import annotations

import pytest
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "git_branch_prs_merge_window"

CAROL = "carol@example.com"

OCTOBER = {"from": "2026-10-01", "to": "2026-10-31"}
NOVEMBER = {"from": "2026-11-01", "to": "2026-11-30"}
DECEMBER = {"from": "2026-12-01", "to": "2026-12-31"}

MERGED_KEYS = (
    "git.default_branch_prs_merged",
    "git.non_default_branch_prs_merged",
    "git.prs_merged",
)
CREATED_KEYS = (
    "git.default_branch_prs_created",
    "git.non_default_branch_prs_created",
    "git.prs_created",
)


def both_pairs(period: dict[str, str]) -> dict[str, object]:
    return {
        "url": "/v1/metric-results",
        "method": "POST",
        "body": {
            "entity": {"type": "person", "ids": [CAROL]},
            "period": period,
            "metrics": [
                {"metric_key": key, "views": [{"view": "period"}]}
                for key in MERGED_KEYS + CREATED_KEYS
            ],
        },
    }


def test_a_months_merges_and_creations_are_different_requests(spec: SpecRun) -> None:
    """November merged two requests into the default branch and one elsewhere, all three
    opened in October; it opened one and three, none of which merged that month. Dating a
    merge by its creation would swap those figures into 1 and 2."""
    r = spec.call(both_pairs(NOVEMBER))
    assert r.status == 200

    r.row("git.default_branch_prs_merged", "period", entity_id=CAROL).equals(value=2)
    r.row("git.non_default_branch_prs_merged", "period", entity_id=CAROL).equals(value=1)
    r.row("git.prs_merged", "period", entity_id=CAROL).equals(value=3)

    r.row("git.default_branch_prs_created", "period", entity_id=CAROL).equals(value=1)
    r.row("git.non_default_branch_prs_created", "period", entity_id=CAROL).equals(value=3)
    r.row("git.prs_created", "period", entity_id=CAROL).equals(value=4)


def test_the_month_the_requests_were_opened_has_no_merges_in_it(spec: SpecRun) -> None:
    """October opened three requests and merged none. A creation-dated rule would report
    two and one here instead of nothing at all."""
    r = spec.call(both_pairs(OCTOBER))
    assert r.status == 200

    for key in MERGED_KEYS:
        r.row(key, "period", entity_id=CAROL).equals(value=None)

    r.row("git.default_branch_prs_created", "period", entity_id=CAROL).equals(value=2)
    r.row("git.non_default_branch_prs_created", "period", entity_id=CAROL).equals(value=1)
    r.row("git.prs_created", "period", entity_id=CAROL).equals(value=3)


def test_a_request_merged_after_its_month_counts_in_the_month_it_merged(spec: SpecRun) -> None:
    """The three opened in November merged in December, and December opened nothing. The
    request still open belongs to neither."""
    r = spec.call(both_pairs(DECEMBER))
    assert r.status == 200

    r.row("git.default_branch_prs_merged", "period", entity_id=CAROL).equals(value=1)
    r.row("git.non_default_branch_prs_merged", "period", entity_id=CAROL).equals(value=2)
    r.row("git.prs_merged", "period", entity_id=CAROL).equals(value=3)

    for key in CREATED_KEYS:
        r.row(key, "period", entity_id=CAROL).equals(value=None)


def test_each_merge_lands_on_its_own_day(spec: SpecRun) -> None:
    """The day, not only the month: a rule reading the creation would put these points in
    October, where the series has no days at all."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [CAROL]},
                "period": NOVEMBER,
                "metrics": [
                    {
                        "metric_key": key,
                        "views": [{"view": "timeseries", "bucket": "day"}],
                    }
                    for key in (
                        "git.default_branch_prs_merged",
                        "git.non_default_branch_prs_merged",
                    )
                ],
            },
        }
    )
    assert r.status == 200

    default_series = r.row("git.default_branch_prs_merged", "timeseries", entity_id=CAROL)
    default_series.contains(points={"bucket_start": "2026-11-05", "value": 1})
    default_series.contains(points={"bucket_start": "2026-11-07", "value": 1})

    r.row("git.non_default_branch_prs_merged", "timeseries", entity_id=CAROL).contains(
        points={"bucket_start": "2026-11-06", "value": 1}
    )
